//! Virtual-mic path through a real PulseAudio/PipeWire server. Skips (passes with a
//! message) when no server or no `parec` binary is available.

mod common;

use common::{FakeConfig, FakePhone};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use vibe_phone_mic::audio::{self, MicHost};
use vibe_phone_mic::dsp;
use vibe_phone_mic::session::{Session, SessionOptions};

fn have_tools() -> bool {
    if !audio::pulse_available() {
        eprintln!("SKIP: no PulseAudio/PipeWire server");
        return false;
    }
    let ok = |c: &str| Command::new(c).arg("--version").stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok();
    if !ok("parec") || !ok("pactl") {
        eprintln!("SKIP: parec/pactl not installed");
        return false;
    }
    true
}

/// Record `secs` of s16le mono 48k from the vibe_phone_mic source.
fn record(secs: f64) -> Vec<i16> {
    let mut child = Command::new("parec")
        .args(["--latency-msec=20", "--device", audio::SOURCE_NAME, "--format=s16le", "--rate=48000", "--channels=1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn parec");
    std::thread::sleep(Duration::from_secs_f64(secs));
    let _ = child.kill();
    let out = child.wait_with_output().unwrap();
    out.stdout.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
}

fn our_modules_loaded() -> bool {
    let out = Command::new("pactl").args(["list", "short", "modules"]).output().unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    s.contains("sink_name=vibe_phone_mic_sink") || s.contains("source_name=vibe_phone_mic ")
}

fn check_tone(rec: &[i16], what: &str) {
    // Skip the first 300 ms (stream start-up).
    let tail = &rec[rec.len().min(14400)..];
    assert!(tail.len() > 9600, "{what}: recorded only {} samples", rec.len());
    let rms = dsp::rms(tail);
    assert!(rms > 0.02, "{what}: virtual mic is silent (rms={rms})");
    let f = dsp::dominant_frequency(&dsp::to_mono(tail, 1), 48000).unwrap();
    assert!((f - 440.0).abs() <= 5.0, "{what}: dominant {f} Hz");
    eprintln!("{what}: rms={rms:.4} dominant={f:.1} Hz");
}

fn wait_mic_active(s: &Session) {
    let t = Instant::now();
    while !(s.snapshot().mic_active && s.snapshot().frames > 10) {
        assert!(t.elapsed() < Duration::from_secs(10), "no audio into virtual mic: {:?}", s.snapshot().mic_error);
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn virtual_mic_end_to_end() {
    if !have_tools() {
        return;
    }

    // 1) Directly: play a tone into the host and record from the virtual source.
    {
        let host = MicHost::create().expect("create virtual mic");
        assert!(our_modules_loaded());
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (stop2, h2) = (stop.clone(), host.clone());
        let generation = host.begin();
        let feeder = std::thread::spawn(move || {
            let t0 = Instant::now();
            let mut n = 0u64;
            while !stop2.load(std::sync::atomic::Ordering::SeqCst) {
                let frame: Vec<i16> = (0..480)
                    .map(|i| {
                        let t = (n + i) as f64 / 48000.0;
                        ((2.0 * std::f64::consts::PI * 440.0 * t).sin() * 0.4 * 32767.0) as i16
                    })
                    .collect();
                n += 480;
                h2.push(generation, &frame);
                let target = t0 + Duration::from_micros(n * 1_000_000 / 48000);
                if let Some(d) = target.checked_duration_since(Instant::now()) {
                    std::thread::sleep(d);
                }
            }
        });
        std::thread::sleep(Duration::from_millis(300));
        let rec = record(2.5);
        stop.store(true, std::sync::atomic::Ordering::SeqCst);
        feeder.join().unwrap();
        check_tone(&rec, "direct");
        drop(host);
        assert!(!our_modules_loaded(), "modules must be unloaded when the host is dropped");
    }

    // 2) Full path, and the mic survives disconnect + reconnect (silence in between).
    {
        let host = MicHost::create().unwrap();
        let phone = FakePhone::start(FakeConfig::default());
        let opts = SessionOptions { mic: Some(host.clone()), ..Default::default() };
        let mut session = Session::start(&phone.addr(), opts.clone(), None);
        wait_mic_active(&session);
        std::thread::sleep(Duration::from_millis(300));
        check_tone(&record(2.5), "session");
        session.stop();
        drop(session);
        assert!(our_modules_loaded(), "virtual mic must survive a disconnect");

        // Between connections the source is still there and outputs silence.
        let gap = record(2.0);
        let tail = &gap[gap.len().min(14400)..];
        assert!(tail.len() > 9600, "virtual mic must keep producing samples while idle ({} samples)", gap.len());
        assert!(dsp::rms(tail) < 0.001, "idle virtual mic must be silent, rms={}", dsp::rms(tail));

        // Reconnect (also at a different rate): audio flows again through the same modules.
        let phone2 = FakePhone::start(FakeConfig { rate: 44100, ..Default::default() });
        let session = Session::start(&phone2.addr(), opts, None);
        wait_mic_active(&session);
        std::thread::sleep(Duration::from_millis(300));
        check_tone(&record(2.5), "reconnected@44.1k");
        drop(session);
        assert!(our_modules_loaded());
        host.shutdown();
        assert!(!our_modules_loaded(), "shutdown must unload even with other references alive");
        drop(host);
    }

    // 3) A late event from a replaced session must not clear or feed the new one.
    {
        let host = MicHost::create().unwrap();
        let old = host.begin();
        let new = host.begin();
        // Let the playback stream reach steady state (server buffer full) so it
        // only drains ~10 ms at a time while we measure.
        std::thread::sleep(Duration::from_millis(400));
        // 100 ms queued (below the 150 ms drift-trim threshold).
        host.push(new, &vec![1000i16; 4800]);
        let q = host.queued();
        assert!(q > 3000, "{q}");
        host.clear(old);
        host.push(old, &vec![5i16; 4800]);
        let q2 = host.queued();
        assert!(q2 <= q && q2 + 2400 > q, "stale session touched the mic: {q} -> {q2}");
        host.clear(new);
        assert_eq!(host.queued(), 0);
    }

    // 4) Only one owner: a second MicHost is refused without touching the first's modules.
    {
        let first = MicHost::create().unwrap();
        let err = MicHost::create().unwrap_err();
        assert!(err.starts_with(audio::IN_USE), "{err}");
        let err = audio::cleanup_stale_modules().unwrap_err();
        assert!(err.starts_with(audio::IN_USE), "{err}");
        assert!(our_modules_loaded(), "second instance must not remove the first's mic");
        drop(first);
        assert!(!our_modules_loaded());
        // Lock released: a new owner can start.
        drop(MicHost::create().unwrap());
    }

    // 5) Stale-module cleanup after a crash (SIGKILL: no chance to unload).
    {
        let phone = FakePhone::start(FakeConfig::default());
        let mut child = Command::new(env!("CARGO_BIN_EXE_vibe-phone-mic"))
            .args(["--headless", "--connect", &phone.addr()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let t = Instant::now();
        while !our_modules_loaded() {
            assert!(t.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(50));
        }
        child.kill().unwrap(); // SIGKILL
        child.wait().unwrap();
        assert!(our_modules_loaded(), "a killed process leaves its modules behind");
        let n = audio::cleanup_stale_modules().unwrap();
        assert_eq!(n, 2);
        assert!(!our_modules_loaded());
    }
}
