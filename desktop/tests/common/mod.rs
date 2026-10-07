//! In-process fake phone server for integration tests (protocol v1).
#![allow(dead_code)]

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};
use vibe_phone_mic::protocol::{self, DiscoveryReply, Frame, Hello, Mode, Status};

#[derive(Clone)]
pub struct FakeConfig {
    pub name: String,
    pub freq: f64,
    pub rate: u32,
    pub channels: u16,
    pub amplitude: f64,
    pub behavior: Behavior,
}

/// Misbehaviours for error-path tests.
#[derive(Clone, Debug, PartialEq)]
pub enum Behavior {
    Normal,
    /// Send STATUS before HELLO (protocol error).
    BadFirstFrame,
    /// Accept the handshake and close without sending anything.
    CloseImmediately,
    /// Send these raw bytes instead of HELLO (e.g. oversized header, bad JSON).
    Garbage(Vec<u8>),
    /// Stream N audio frames then go silent but keep the socket open.
    StallAfter(u32),
    /// Stream N audio frames then close the socket.
    CloseAfter(u32),
    /// Insert an unknown frame type after HELLO (must be ignored).
    UnknownFrame,
}

impl Default for FakeConfig {
    fn default() -> Self {
        FakeConfig { name: "Fake Pixel".into(), freq: 440.0, rate: 48000, channels: 1, amplitude: 0.4, behavior: Behavior::Normal }
    }
}

struct Shared {
    status: Mutex<Status>,
    status_gen: AtomicU32,
    stream_gen: AtomicU32,
    current: Mutex<Option<TcpStream>>,
    /// BYE reason the current stream sends when it is ended on purpose (None = just drop).
    bye_reason: Mutex<Option<String>>,
    stop: AtomicBool,
    probes: AtomicU32,
    streams: AtomicU32,
}

pub struct FakePhone {
    pub tcp_port: u16,
    pub udp_port: u16,
    shared: Arc<Shared>,
}

impl FakePhone {
    pub fn start(cfg: FakeConfig) -> FakePhone {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let tcp_port = listener.local_addr().unwrap().port();
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        let udp_port = udp.local_addr().unwrap().port();
        let shared = Arc::new(Shared {
            status: Mutex::new(Status { muted: false, volume: 100, input: "Test mic".into() }),
            status_gen: AtomicU32::new(0),
            stream_gen: AtomicU32::new(0),
            current: Mutex::new(None),
            bye_reason: Mutex::new(None),
            stop: AtomicBool::new(false),
            probes: AtomicU32::new(0),
            streams: AtomicU32::new(0),
        });
        {
            let shared = shared.clone();
            let name = cfg.name.clone();
            udp.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
            thread::spawn(move || {
                let mut buf = [0u8; 256];
                while !shared.stop.load(Ordering::SeqCst) {
                    if let Ok((n, from)) = udp.recv_from(&mut buf)
                        && &buf[..n] == protocol::DISCOVER_MSG {
                            let r = DiscoveryReply { service: "vibe-phone-mic".into(), version: 1, name: name.clone(), port: tcp_port };
                            let _ = udp.send_to(&serde_json::to_vec(&r).unwrap(), from);
                        }
                }
            });
        }
        {
            let shared = shared.clone();
            listener.set_nonblocking(true).unwrap();
            thread::spawn(move || {
                while !shared.stop.load(Ordering::SeqCst) {
                    match listener.accept() {
                        Ok((s, _)) => {
                            let (shared, cfg) = (shared.clone(), cfg.clone());
                            thread::spawn(move || serve(s, shared, cfg));
                        }
                        Err(_) => thread::sleep(Duration::from_millis(5)),
                    }
                }
            });
        }
        FakePhone { tcp_port, udp_port, shared }
    }

    pub fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.tcp_port)
    }

    pub fn set_status(&self, muted: bool, volume: u32) {
        let mut s = self.shared.status.lock().unwrap();
        s.muted = muted;
        s.volume = volume;
        self.shared.status_gen.fetch_add(1, Ordering::SeqCst);
    }

    /// Drop the current streaming connection (simulates Wi-Fi loss); the
    /// server keeps listening so a client can reconnect.
    pub fn kick(&self) {
        *self.shared.bye_reason.lock().unwrap() = None;
        self.shared.stream_gen.fetch_add(1, Ordering::SeqCst);
        if let Some(s) = self.shared.current.lock().unwrap().take() {
            let _ = s.shutdown(Shutdown::Both);
        }
    }

    /// End the current stream on purpose with `BYE {"reason":"stopped"}`.
    pub fn stop_stream(&self) {
        *self.shared.bye_reason.lock().unwrap() = Some("stopped".into());
        self.shared.stream_gen.fetch_add(1, Ordering::SeqCst);
    }

    pub fn probes(&self) -> u32 {
        self.shared.probes.load(Ordering::SeqCst)
    }

    pub fn streams(&self) -> u32 {
        self.shared.streams.load(Ordering::SeqCst)
    }
}

impl Drop for FakePhone {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(s) = self.shared.current.lock().unwrap().take() {
            let _ = s.shutdown(Shutdown::Both);
        }
    }
}

fn serve(mut s: TcpStream, shared: Arc<Shared>, cfg: FakeConfig) {
    s.set_nonblocking(false).unwrap();
    s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let mut hs = [0u8; 8];
    if s.read_exact(&mut hs).is_err() {
        return;
    }
    let Some(mode) = protocol::decode_handshake(&hs) else { return };
    let hello = Hello { name: cfg.name.clone(), sample_rate: cfg.rate, channels: cfg.channels, ..Default::default() };
    if mode == Mode::Probe {
        shared.probes.fetch_add(1, Ordering::SeqCst);
        if let Behavior::Garbage(g) = &cfg.behavior {
            let _ = s.write_all(g);
            return;
        }
        let _ = Frame::Hello(hello).write_to(&mut s);
        return;
    }
    shared.streams.fetch_add(1, Ordering::SeqCst);
    // Replacing an existing client: it says BYE "replaced" from its own loop.
    *shared.bye_reason.lock().unwrap() = Some("replaced".into());
    let my_gen = shared.stream_gen.fetch_add(1, Ordering::SeqCst) + 1;
    shared.current.lock().unwrap().replace(s.try_clone().unwrap());
    let mut out = Vec::new();
    match &cfg.behavior {
        Behavior::BadFirstFrame => out.extend(Frame::Status(Status::default()).encode()),
        Behavior::CloseImmediately => {
            let _ = s.shutdown(Shutdown::Both);
            return;
        }
        Behavior::Garbage(g) => {
            let _ = s.write_all(g);
            thread::sleep(Duration::from_millis(200));
            return;
        }
        _ => {}
    }
    out.extend(Frame::Hello(hello).encode());
    if cfg.behavior == Behavior::UnknownFrame {
        out.extend(Frame::Unknown { kind: 0x42, payload: b"future".to_vec() }.encode());
    }
    out.extend(Frame::Status(shared.status.lock().unwrap().clone()).encode());
    if s.write_all(&out).is_err() {
        return;
    }
    let n = (cfg.rate / 100) as usize;
    let mut seq = 0u32;
    let mut phase = 0.0f64;
    let step = 2.0 * std::f64::consts::PI * cfg.freq / cfg.rate as f64;
    let mut seen_status = shared.status_gen.load(Ordering::SeqCst);
    let t0 = Instant::now();
    loop {
        if shared.stop.load(Ordering::SeqCst) {
            return;
        }
        if shared.stream_gen.load(Ordering::SeqCst) != my_gen {
            if let Some(r) = shared.bye_reason.lock().unwrap().clone() {
                let _ = Frame::Bye(protocol::Bye { reason: r }).write_to(&mut s);
            }
            let _ = s.shutdown(Shutdown::Both);
            return;
        }
        match cfg.behavior {
            Behavior::StallAfter(n) if seq >= n => {
                thread::sleep(Duration::from_millis(20));
                continue;
            }
            Behavior::CloseAfter(n) if seq >= n => {
                let _ = s.shutdown(Shutdown::Both);
                return;
            }
            _ => {}
        }
        let g = shared.status_gen.load(Ordering::SeqCst);
        let st = shared.status.lock().unwrap().clone();
        if g != seen_status {
            seen_status = g;
            if Frame::Status(st.clone()).write_to(&mut s).is_err() {
                return;
            }
        }
        let gain = if st.muted { 0.0 } else { cfg.amplitude * st.volume as f64 / 100.0 };
        let mut samples = Vec::with_capacity(n * cfg.channels as usize);
        for _ in 0..n {
            let v = ((phase.sin() * gain).clamp(-1.0, 1.0) * 32767.0) as i16;
            for _ in 0..cfg.channels {
                samples.push(v);
            }
            phase = (phase + step) % (2.0 * std::f64::consts::PI);
        }
        let f = Frame::Audio { seq, capture_time_micros: t0.elapsed().as_micros() as u64, samples };
        if f.write_to(&mut s).is_err() {
            return;
        }
        seq += 1;
        let target = t0 + Duration::from_millis(10 * seq as u64);
        if let Some(d) = target.checked_duration_since(Instant::now()) {
            thread::sleep(d);
        }
    }
}

/// An address nothing listens on. Uses 127.0.0.2 so a port freed here can't be
/// grabbed by fake phones of tests running in parallel (they bind 127.0.0.1).
pub fn closed_addr() -> std::net::SocketAddr {
    std::net::TcpListener::bind("127.0.0.2:0").unwrap().local_addr().unwrap()
}
