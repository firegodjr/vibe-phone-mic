//! Streaming client: connects to a phone, reads frames on a background thread and
//! delivers events to a handler. `disconnect()` is safe to call at any time and
//! joins the thread, so a new connection can be started right after.

use crate::protocol::{self, Frame, Hello, Mode, Status, DEFAULT_PORT};
use std::io::{self, Write};
use std::net::{Shutdown, SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub enum ClientEvent {
    /// TCP connected and handshake sent.
    Connected { peer: SocketAddr },
    Hello(Hello),
    Status(Status),
    Audio { seq: u32, samples: Vec<i16> },
    /// The phone said goodbye (`reason` = "replaced" | "stopped" | other).
    Bye { reason: String },
    /// Always the last event. `requested` is true when `disconnect()` caused it.
    Disconnected { reason: String, requested: bool },
}

pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The phone sends audio every ~10 ms, so silence this long means the link is dead.
pub const READ_TIMEOUT: Duration = Duration::from_secs(5);

/// Normalise `HOST`, `HOST:PORT`, `[v6]:PORT` into something `ToSocketAddrs` accepts.
pub fn normalize_addr(input: &str, default_port: u16) -> String {
    let s = input.trim();
    if s.starts_with('[') {
        if s.contains("]:") { s.to_string() } else { format!("{s}:{default_port}") }
    } else if s.matches(':').count() == 1 {
        s.to_string()
    } else if s.matches(':').count() > 1 {
        format!("[{s}]:{default_port}")
    } else {
        format!("{s}:{default_port}")
    }
}

/// Validate manual GUI entry (host field + port field) into `HOST:PORT`.
/// The host may itself carry a `:PORT`, which then wins over the port field.
/// An empty port field means the default port.
pub fn parse_manual_address(host: &str, port: &str) -> Result<String, String> {
    let host = host.trim();
    let port = port.trim();
    if host.is_empty() {
        return Err("Enter the phone's IP address".into());
    }
    let (h, embedded_port) = split_host_port(host)?;
    let port_num = match (embedded_port, port) {
        (Some(p), _) => p,
        (None, "") => DEFAULT_PORT,
        (None, p) => parse_port(p)?,
    };
    validate_host(&h)?;
    Ok(if h.contains(':') { format!("[{h}]:{port_num}") } else { format!("{h}:{port_num}") })
}

fn parse_port(p: &str) -> Result<u16, String> {
    match p.parse::<u32>() {
        Ok(n) if (1..=65535).contains(&n) => Ok(n as u16),
        _ => Err(format!("Invalid port \"{p}\" (use 1-65535)")),
    }
}

fn split_host_port(s: &str) -> Result<(String, Option<u16>), String> {
    if let Some(rest) = s.strip_prefix('[') {
        let (h, after) = rest.split_once(']').ok_or("Invalid IPv6 address")?;
        return match after.strip_prefix(':') {
            Some(p) => Ok((h.to_string(), Some(parse_port(p)?))),
            None if after.is_empty() => Ok((h.to_string(), None)),
            None => Err("Invalid address".into()),
        };
    }
    match s.matches(':').count() {
        0 => Ok((s.to_string(), None)),
        1 => {
            let (h, p) = s.split_once(':').unwrap();
            Ok((h.to_string(), Some(parse_port(p)?)))
        }
        _ => Ok((s.to_string(), None)), // bare IPv6
    }
}

fn validate_host(h: &str) -> Result<(), String> {
    if h.parse::<std::net::IpAddr>().is_ok() {
        return Ok(());
    }
    let bad = || Err(format!("Invalid address \"{h}\""));
    // Things that look like a dotted IPv4 must parse as one (rejects 300.1.2.3, 1.2.3).
    if h.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return bad();
    }
    let ok_label = |l: &str| {
        !l.is_empty() && l.len() <= 63 && !l.starts_with('-') && !l.ends_with('-')
            && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    };
    if h.len() > 253 || !h.trim_end_matches('.').split('.').all(ok_label) {
        return bad();
    }
    Ok(())
}

pub fn resolve(input: &str) -> io::Result<SocketAddr> {
    let norm = normalize_addr(input, DEFAULT_PORT);
    norm.to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("cannot resolve {input}")))
}

pub struct Client {
    stop: Arc<AtomicBool>,
    stream: Arc<Mutex<Option<TcpStream>>>,
    thread: Option<JoinHandle<()>>,
}

impl Client {
    /// Start connecting to `addr` (HOST[:PORT]) in the background.
    pub fn connect<F>(addr: &str, handler: F) -> Client
    where
        F: FnMut(ClientEvent) + Send + 'static,
    {
        Client::connect_with(addr, READ_TIMEOUT, handler)
    }

    /// Like `connect` with a custom "no data" timeout.
    pub fn connect_with<F>(addr: &str, read_timeout: Duration, mut handler: F) -> Client
    where
        F: FnMut(ClientEvent) + Send + 'static,
    {
        let stop = Arc::new(AtomicBool::new(false));
        let stream: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
        let addr = addr.to_string();
        let (stop2, stream2) = (stop.clone(), stream.clone());
        let thread = thread::Builder::new()
            .name("vpm-client".into())
            .spawn(move || {
                let res = run(&addr, read_timeout, &stop2, &stream2, &mut handler);
                let requested = stop2.load(Ordering::SeqCst);
                let reason = match res {
                    _ if requested => "disconnected".to_string(),
                    Ok(Some(bye)) if !bye.is_empty() => bye,
                    Ok(_) => "phone closed the connection".to_string(),
                    Err(e) => e.to_string(),
                };
                stream2.lock().unwrap().take();
                handler(ClientEvent::Disconnected { reason, requested });
            })
            .expect("spawn client thread");
        Client { stop, stream, thread: Some(thread) }
    }

    /// True while the background thread is alive.
    pub fn is_running(&self) -> bool {
        self.thread.as_ref().is_some_and(|t| !t.is_finished())
    }

    /// Stop streaming and wait for the thread to exit. Idempotent.
    pub fn disconnect(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        let connected = match self.stream.lock().unwrap().as_ref() {
            Some(s) => {
                let _ = s.shutdown(Shutdown::Both);
                true
            }
            None => false,
        };
        if let Some(t) = self.thread.take()
            && (connected || t.is_finished()) {
                let _ = t.join();
            }
            // Otherwise the thread is still inside connect(): it checks the stop
            // flag as soon as connect returns and exits without touching audio,
            // so we detach instead of blocking the caller for the connect timeout.
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.disconnect();
    }
}

fn timeout_err(e: io::Error) -> io::Error {
    if e.kind() == io::ErrorKind::WouldBlock || e.kind() == io::ErrorKind::TimedOut {
        io::Error::new(io::ErrorKind::TimedOut, "no data from phone (timed out)")
    } else {
        e
    }
}

/// Returns `Ok(Some(reason))` when the phone sent BYE, `Ok(None)` on plain EOF.
fn run<F: FnMut(ClientEvent)>(
    addr: &str,
    read_timeout: Duration,
    stop: &AtomicBool,
    slot: &Mutex<Option<TcpStream>>,
    handler: &mut F,
) -> io::Result<Option<String>> {
    let sa = resolve(addr)?;
    if stop.load(Ordering::SeqCst) {
        return Ok(None);
    }
    let mut s = TcpStream::connect_timeout(&sa, CONNECT_TIMEOUT)?;
    s.set_nodelay(true)?;
    s.set_read_timeout(Some(read_timeout))?;
    s.set_write_timeout(Some(CONNECT_TIMEOUT))?;
    *slot.lock().unwrap() = Some(s.try_clone()?);
    // disconnect() may have run between connect and storing the clone.
    if stop.load(Ordering::SeqCst) {
        return Ok(None);
    }
    s.write_all(&protocol::encode_handshake(Mode::Stream))?;
    handler(ClientEvent::Connected { peer: sa });
    let mut reader = io::BufReader::with_capacity(64 * 1024, s);
    let Some(h) = protocol::read_first_frame(&mut reader).map_err(timeout_err)? else {
        return Ok(None);
    };
    if stop.load(Ordering::SeqCst) {
        return Ok(None);
    }
    if h.format != "s16le" {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("unsupported format {}", h.format)));
    }
    handler(ClientEvent::Hello(h));
    loop {
        let frame = match protocol::read_frame(&mut reader).map_err(timeout_err)? {
            Some(f) => f,
            None => return Ok(None),
        };
        if stop.load(Ordering::SeqCst) {
            return Ok(None);
        }
        match frame {
            Frame::Hello(_) => {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "protocol error: unexpected second HELLO"));
            }
            Frame::Status(st) => handler(ClientEvent::Status(st)),
            Frame::Audio { seq, samples, .. } => handler(ClientEvent::Audio { seq, samples }),
            Frame::Bye(b) => {
                handler(ClientEvent::Bye { reason: b.reason.clone() });
                return Ok(Some(b.reason));
            }
            Frame::Unknown { .. } => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize() {
        assert_eq!(normalize_addr("1.2.3.4", 48777), "1.2.3.4:48777");
        assert_eq!(normalize_addr(" 1.2.3.4:5 ", 48777), "1.2.3.4:5");
        assert_eq!(normalize_addr("::1", 48777), "[::1]:48777");
        assert_eq!(normalize_addr("[::1]:9", 48777), "[::1]:9");
        assert_eq!(normalize_addr("[::1]", 48777), "[::1]:48777");
        assert_eq!(normalize_addr("localhost", 1), "localhost:1");
    }

    #[test]
    fn manual_address_validation() {
        assert_eq!(parse_manual_address("192.168.1.5", "").unwrap(), "192.168.1.5:48777");
        assert_eq!(parse_manual_address(" 192.168.1.5 ", "48777").unwrap(), "192.168.1.5:48777");
        assert_eq!(parse_manual_address("192.168.1.5", "5000").unwrap(), "192.168.1.5:5000");
        assert_eq!(parse_manual_address("192.168.1.5:6000", "48777").unwrap(), "192.168.1.5:6000");
        assert_eq!(parse_manual_address("my-phone.local", "").unwrap(), "my-phone.local:48777");
        assert_eq!(parse_manual_address("localhost", "1").unwrap(), "localhost:1");
        assert_eq!(parse_manual_address("fe80::1", "").unwrap(), "[fe80::1]:48777");
        assert_eq!(parse_manual_address("[::1]:9", "").unwrap(), "[::1]:9");
        for (h, p) in [
            ("", "48777"),
            ("300.1.2.3", ""),
            ("1.2.3", ""),
            ("192.168.1.5", "0"),
            ("192.168.1.5", "70000"),
            ("192.168.1.5", "abc"),
            ("192.168.1.5:", ""),
            ("bad host", ""),
            ("under_score", ""),
            ("-x.com", ""),
            ("[::1", ""),
        ] {
            assert!(parse_manual_address(h, p).is_err(), "{h:?} {p:?} should be rejected");
        }
    }

    #[test]
    fn connect_refused_reports_disconnect() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        let (tx, rx) = std::sync::mpsc::channel();
        let mut c = Client::connect(&format!("127.0.0.1:{port}"), move |e| tx.send(e).unwrap());
        let ev = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(ev, ClientEvent::Disconnected { requested: false, .. }), "{ev:?}");
        c.disconnect();
    }
}
