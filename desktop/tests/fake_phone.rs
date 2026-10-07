//! Integration tests against an in-process fake phone.

mod common;

use common::{Behavior, FakeConfig, FakePhone};
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};
use vibe_phone_mic::client::{Client, ClientEvent};
use vibe_phone_mic::discovery::{self, ScanConfig};
use vibe_phone_mic::dsp;
use vibe_phone_mic::session::{Phase, Session, SessionOptions};

fn start_client(addr: &str) -> (Client, Receiver<ClientEvent>) {
    let (tx, rx) = mpsc::channel();
    let c = Client::connect(addr, move |e| {
        let _ = tx.send(e);
    });
    (c, rx)
}

/// Collect `secs` worth of audio samples (and statuses seen) from the receiver.
fn collect_audio(rx: &Receiver<ClientEvent>, samples_wanted: usize) -> Vec<i16> {
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut out = Vec::new();
    while out.len() < samples_wanted {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(ClientEvent::Audio { samples, .. }) => out.extend(samples),
            Ok(ClientEvent::Disconnected { reason, .. }) => panic!("disconnected: {reason}"),
            Ok(_) => {}
            Err(_) => panic!("timed out collecting audio ({} samples)", out.len()),
        }
    }
    out
}

fn wait_for<F: FnMut(&ClientEvent) -> bool>(rx: &Receiver<ClientEvent>, mut pred: F) -> ClientEvent {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let e = rx.recv_timeout(deadline.saturating_duration_since(Instant::now())).expect("timed out waiting for event");
        if pred(&e) {
            return e;
        }
    }
}

#[test]
fn streams_sine_status_and_mute() {
    let phone = FakePhone::start(FakeConfig::default());
    let (mut client, rx) = start_client(&phone.addr());

    assert!(matches!(wait_for(&rx, |_| true), ClientEvent::Connected { .. }));
    let ClientEvent::Hello(h) = wait_for(&rx, |_| true) else { panic!("HELLO must come first") };
    assert_eq!(h.name, "Fake Pixel");
    assert_eq!(h.sample_rate, 48000);
    let ClientEvent::Status(s) = wait_for(&rx, |e| matches!(e, ClientEvent::Status(_))) else { unreachable!() };
    assert!(!s.muted);
    assert_eq!(s.volume, 100);

    let audio = collect_audio(&rx, 24000);
    let rms = dsp::rms(&audio);
    assert!(rms > 0.2, "audio should be loud, rms={rms}");
    let f = dsp::dominant_frequency(&dsp::to_mono(&audio, 1), 48000).unwrap();
    assert!((f - 440.0).abs() <= 3.0, "dominant frequency {f}");

    // Mute on the phone: STATUS muted + zero PCM.
    phone.set_status(true, 100);
    let ClientEvent::Status(s) = wait_for(&rx, |e| matches!(e, ClientEvent::Status(_))) else { unreachable!() };
    assert!(s.muted);
    let muted = collect_audio(&rx, 4800);
    assert!(muted.iter().all(|&x| x == 0), "muted audio must be zeros");

    // Volume change is reflected in STATUS and level.
    phone.set_status(false, 50);
    let ClientEvent::Status(s) = wait_for(&rx, |e| matches!(e, ClientEvent::Status(_))) else { unreachable!() };
    assert_eq!((s.muted, s.volume), (false, 50));
    let half = collect_audio(&rx, 9600);
    let r2 = dsp::rms(&half[4800..]);
    assert!((r2 / rms - 0.5).abs() < 0.05, "half volume rms ratio {}", r2 / rms);

    client.disconnect();
    let e = wait_for(&rx, |e| matches!(e, ClientEvent::Disconnected { .. }));
    assert!(matches!(e, ClientEvent::Disconnected { requested: true, .. }));
    assert!(!client.is_running());
}

#[test]
fn honors_hello_rate_and_channels_in_wav() {
    let phone = FakePhone::start(FakeConfig { rate: 44100, channels: 2, freq: 1000.0, ..Default::default() });
    let dir = std::env::temp_dir().join(format!("vpm-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("out.wav");
    let mut session = Session::start(
        &phone.addr(),
        SessionOptions { wav: Some(wav.clone()), ..Default::default() },
        None,
    );
    let t = Instant::now();
    while session.snapshot().frames < 60 {
        assert!(t.elapsed() < Duration::from_secs(10), "no audio");
        std::thread::sleep(Duration::from_millis(20));
    }
    let snap = session.snapshot();
    assert_eq!(snap.phase, Phase::Streaming);
    assert_eq!(snap.lost_frames, 0);
    let hz = snap.dominant_hz().unwrap();
    assert!((hz - 1000.0).abs() <= 3.0, "{hz}");
    session.stop();
    assert_eq!(session.snapshot().phase, Phase::Disconnected);

    let mut r = hound::WavReader::open(&wav).unwrap();
    let spec = r.spec();
    assert_eq!((spec.sample_rate, spec.channels, spec.bits_per_sample), (44100, 2, 16));
    let samples: Vec<i16> = r.samples::<i16>().map(|s| s.unwrap()).collect();
    assert!(samples.len() >= 60 * 441 * 2);
    let f = dsp::dominant_frequency(&dsp::to_mono(&samples, 2), 44100).unwrap();
    assert!((f - 1000.0).abs() <= 3.0, "{f}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn probe_mode_returns_hello() {
    let phone = FakePhone::start(FakeConfig { name: "Probe Me".into(), ..Default::default() });
    let addr: SocketAddr = phone.addr().parse().unwrap();
    let h = discovery::probe(addr, Duration::from_millis(500)).unwrap();
    assert_eq!(h.name, "Probe Me");
    assert_eq!(phone.probes(), 1);
    assert_eq!(phone.streams(), 0);
}

#[test]
fn udp_discovery_on_loopback() {
    let phone = FakePhone::start(FakeConfig { name: "UDP Phone".into(), ..Default::default() });
    let mut cfg = ScanConfig::empty(Duration::from_millis(500));
    cfg.udp_targets = vec![SocketAddr::from((Ipv4Addr::LOCALHOST, phone.udp_port))];
    let t = Instant::now();
    let found = discovery::scan(&cfg);
    assert!(t.elapsed() < Duration::from_millis(1500));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].host, "127.0.0.1");
    assert_eq!(found[0].port, phone.tcp_port);
    assert_eq!(found[0].name, "UDP Phone");
    assert_eq!(found[0].source, "udp");
}

#[test]
fn scan_combines_and_dedupes_udp_probe_and_sweep() {
    let phone = FakePhone::start(FakeConfig { name: "Multi".into(), ..Default::default() });
    let mut cfg = ScanConfig::empty(Duration::from_millis(1500));
    cfg.udp_targets = vec![SocketAddr::from((Ipv4Addr::LOCALHOST, phone.udp_port))];
    cfg.probe_targets = vec![SocketAddr::from((Ipv4Addr::LOCALHOST, phone.tcp_port))];
    // Sweep 127.0.0.1..254 on the fake's port: only .1 answers.
    cfg.sweep_networks = vec![Ipv4Addr::new(127, 0, 0, 0)];
    cfg.sweep_port = phone.tcp_port;
    cfg.probe_timeout = Duration::from_millis(200);
    let found = discovery::scan(&cfg);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].source, "udp");
    assert_eq!(found[0].name, "Multi");
    // Both the localhost probe and the sweep reached it.
    assert!(phone.probes() >= 2, "probes={}", phone.probes());
}

#[test]
fn sweep_alone_finds_server() {
    let phone = FakePhone::start(FakeConfig { name: "Swept".into(), ..Default::default() });
    let mut cfg = ScanConfig::empty(Duration::from_millis(1500));
    cfg.sweep_networks = vec![Ipv4Addr::new(127, 0, 0, 0)];
    cfg.sweep_port = phone.tcp_port;
    cfg.probe_timeout = Duration::from_millis(200);
    cfg.concurrency = 16;
    let found = discovery::scan(&cfg);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!((found[0].host.as_str(), found[0].source.as_str(), found[0].name.as_str()), ("127.0.0.1", "sweep", "Swept"));
}

#[test]
fn scan_with_nothing_listening_is_empty_and_bounded() {
    let mut cfg = ScanConfig::empty(Duration::from_millis(400));
    let free = common::closed_addr();
    cfg.probe_targets = vec![free];
    cfg.udp_targets = vec![free];
    let t = Instant::now();
    assert!(discovery::scan(&cfg).is_empty());
    assert!(t.elapsed() < Duration::from_secs(2));
}

#[test]
fn disconnect_then_reconnect_and_replacement() {
    let phone = FakePhone::start(FakeConfig::default());
    for _ in 0..3 {
        let (mut c, rx) = start_client(&phone.addr());
        collect_audio(&rx, 960);
        c.disconnect();
        c.disconnect(); // idempotent
        wait_for(&rx, |e| matches!(e, ClientEvent::Disconnected { requested: true, .. }));
    }
    // A second streaming client replaces the first; the first sees a non-requested disconnect.
    let (mut a, rxa) = start_client(&phone.addr());
    collect_audio(&rxa, 480);
    let (mut b, rxb) = start_client(&phone.addr());
    collect_audio(&rxb, 480);
    // The phone says BYE "replaced" to the old client.
    let e = wait_for(&rxa, |e| matches!(e, ClientEvent::Bye { .. }));
    assert_eq!(e, ClientEvent::Bye { reason: "replaced".into() });
    let e = wait_for(&rxa, |e| matches!(e, ClientEvent::Disconnected { .. }));
    assert_eq!(e, ClientEvent::Disconnected { reason: "replaced".into(), requested: false });
    collect_audio(&rxb, 4800);
    b.disconnect();
    a.disconnect();
    assert_eq!(phone.streams(), 5);
}

#[test]
fn protocol_error_when_first_frame_not_hello() {
    let phone = FakePhone::start(FakeConfig { behavior: Behavior::BadFirstFrame, ..Default::default() });
    let (_c, rx) = start_client(&phone.addr());
    let e = wait_for(&rx, |e| matches!(e, ClientEvent::Disconnected { .. }));
    let ClientEvent::Disconnected { reason, requested } = e else { unreachable!() };
    assert!(!requested);
    assert!(reason.contains("HELLO"), "{reason}");
}

#[test]
fn disconnect_while_connecting_does_not_block() {
    // 10.255.255.1 is normally unroutable → connect hangs until timeout.
    let (mut c, _rx) = start_client("10.255.255.1:48777");
    std::thread::sleep(Duration::from_millis(50));
    let t = Instant::now();
    c.disconnect();
    assert!(t.elapsed() < Duration::from_millis(500));
}

fn disconnect_reason(rx: &Receiver<ClientEvent>) -> (String, bool) {
    match wait_for(rx, |e| matches!(e, ClientEvent::Disconnected { .. })) {
        ClientEvent::Disconnected { reason, requested } => (reason, requested),
        _ => unreachable!(),
    }
}

#[test]
fn server_closes_connection_mid_stream() {
    let phone = FakePhone::start(FakeConfig { behavior: Behavior::CloseAfter(20), ..Default::default() });
    let (_c, rx) = start_client(&phone.addr());
    let mut frames = 0;
    let (reason, requested) = loop {
        match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
            ClientEvent::Audio { .. } => frames += 1,
            ClientEvent::Disconnected { reason, requested } => break (reason, requested),
            _ => {}
        }
    };
    assert_eq!(frames, 20);
    assert!(!requested);
    assert_eq!(reason, "phone closed the connection");
}

#[test]
fn server_closes_before_hello() {
    let phone = FakePhone::start(FakeConfig { behavior: Behavior::CloseImmediately, ..Default::default() });
    let (_c, rx) = start_client(&phone.addr());
    assert!(matches!(wait_for(&rx, |_| true), ClientEvent::Connected { .. }));
    // The very next event must be the disconnect: no HELLO, no audio.
    let e = rx.recv_timeout(Duration::from_secs(5)).unwrap();
    assert!(matches!(e, ClientEvent::Disconnected { requested: false, .. }), "{e:?}");
}

#[test]
fn oversized_payload_is_protocol_error() {
    let mut g = vec![0x01];
    g.extend_from_slice(&(2u32 << 20).to_be_bytes());
    let phone = FakePhone::start(FakeConfig { behavior: Behavior::Garbage(g), ..Default::default() });
    let (_c, rx) = start_client(&phone.addr());
    let (reason, requested) = disconnect_reason(&rx);
    assert!(!requested);
    assert!(reason.contains("too large"), "{reason}");
}

#[test]
fn bad_hello_json_is_protocol_error() {
    let g = vibe_phone_mic::protocol::Frame::Unknown { kind: 0x01, payload: b"{not json".to_vec() }.encode();
    let phone = FakePhone::start(FakeConfig { behavior: Behavior::Garbage(g), ..Default::default() });
    let (_c, rx) = start_client(&phone.addr());
    let (reason, _) = disconnect_reason(&rx);
    assert!(reason.contains("bad HELLO"), "{reason}");
}

#[test]
fn unsupported_format_rejected() {
    let g = vibe_phone_mic::protocol::Frame::Unknown {
        kind: 0x01,
        payload: br#"{"name":"x","sampleRate":48000,"channels":1,"format":"f32le","frameMs":10,"version":1}"#.to_vec(),
    }
    .encode();
    let phone = FakePhone::start(FakeConfig { behavior: Behavior::Garbage(g), ..Default::default() });
    let (_c, rx) = start_client(&phone.addr());
    let (reason, _) = disconnect_reason(&rx);
    assert!(reason.contains("unsupported format"), "{reason}");
}

#[test]
fn unknown_frame_types_are_ignored() {
    let phone = FakePhone::start(FakeConfig { behavior: Behavior::UnknownFrame, ..Default::default() });
    let (_c, rx) = start_client(&phone.addr());
    let audio = collect_audio(&rx, 4800);
    assert!(dsp::rms(&audio) > 0.1);
}

#[test]
fn stalled_phone_times_out() {
    let phone = FakePhone::start(FakeConfig { behavior: Behavior::StallAfter(5), ..Default::default() });
    let (tx, rx) = mpsc::channel();
    let t = Instant::now();
    let _c = Client::connect_with(&phone.addr(), Duration::from_millis(400), move |e| {
        let _ = tx.send(e);
    });
    let (reason, requested) = disconnect_reason(&rx);
    assert!(!requested);
    assert!(reason.contains("timed out"), "{reason}");
    assert!(t.elapsed() < Duration::from_secs(3));
}

#[test]
fn connection_refused() {
    let free = common::closed_addr();
    let (_c, rx) = start_client(&free.to_string());
    let (reason, requested) = disconnect_reason(&rx);
    assert!(!requested);
    assert!(reason.to_lowercase().contains("refused"), "{reason}");
}

#[test]
fn unresolvable_host_reports_error() {
    let (_c, rx) = start_client("no-such-host.invalid");
    let (_reason, requested) = disconnect_reason(&rx);
    assert!(!requested);
}

#[test]
fn hello_48000_mono_and_44100_mono() {
    for rate in [48000u32, 44100] {
        let phone = FakePhone::start(FakeConfig { rate, freq: 700.0, ..Default::default() });
        let (mut c, rx) = start_client(&phone.addr());
        let ClientEvent::Hello(h) = wait_for(&rx, |e| matches!(e, ClientEvent::Hello(_))) else { unreachable!() };
        assert_eq!((h.sample_rate, h.channels), (rate, 1));
        // 10 ms frames at the advertised rate.
        let ClientEvent::Audio { samples, .. } = wait_for(&rx, |e| matches!(e, ClientEvent::Audio { .. })) else { unreachable!() };
        assert_eq!(samples.len(), rate as usize / 100);
        let audio = collect_audio(&rx, rate as usize / 2);
        let f = dsp::dominant_frequency(&dsp::to_mono(&audio, 1), rate).unwrap();
        assert!((f - 700.0).abs() <= 3.0, "rate {rate}: {f}");
        c.disconnect();
    }
}

#[test]
fn session_phases_and_stats() {
    let phone = FakePhone::start(FakeConfig::default());
    let mut s = Session::start(&phone.addr(), SessionOptions::default(), None);
    let first = s.snapshot().phase;
    assert!(matches!(first, Phase::Connecting | Phase::Handshaking | Phase::Streaming));
    let t = Instant::now();
    while s.snapshot().frames < 30 {
        assert!(t.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    let snap = s.snapshot();
    assert_eq!(snap.phase, Phase::Streaming);
    assert!(snap.status.is_some());
    assert!(snap.last_rms > 0.2 && snap.last_peak > 0.35);
    assert!(!snap.mic_active, "virtual mic disabled in this test");
    phone.set_status(true, 100);
    let t = Instant::now();
    while !s.snapshot().status.unwrap().muted {
        assert!(t.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(s.snapshot().last_rms, 0.0);
    // Tone analysis ignores the muted (all-zero) frames.
    let hz = s.snapshot().dominant_hz().expect("tone from before the mute");
    assert!((hz - 440.0).abs() <= 2.0, "{hz}");
    s.stop();
    let snap = s.snapshot();
    assert_eq!(snap.phase, Phase::Disconnected);
    assert!(snap.error.is_none(), "user disconnect is not an error");
}

#[test]
fn session_records_error_when_phone_drops() {
    let phone = FakePhone::start(FakeConfig { behavior: Behavior::CloseAfter(10), ..Default::default() });
    let s = Session::start(&phone.addr(), SessionOptions::default(), None);
    let t = Instant::now();
    while s.snapshot().phase != Phase::Disconnected {
        assert!(t.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(s.snapshot().error.as_deref(), Some("phone closed the connection"));
}

#[test]
fn scan_localhost_only() {
    let phone = FakePhone::start(FakeConfig { name: "USB Phone".into(), ..Default::default() });
    let mut cfg = ScanConfig::empty(Duration::from_millis(500));
    cfg.probe_targets = vec![SocketAddr::from((Ipv4Addr::LOCALHOST, phone.tcp_port))];
    let found = discovery::scan(&cfg);
    assert_eq!(found.len(), 1);
    assert_eq!((found[0].source.as_str(), found[0].name.as_str()), ("localhost", "USB Phone"));
    assert_eq!(phone.streams(), 0, "scan must only use probe mode");
}

#[test]
fn scan_finds_multiple_distinct_phones() {
    let a = FakePhone::start(FakeConfig { name: "A".into(), ..Default::default() });
    let b = FakePhone::start(FakeConfig { name: "B".into(), ..Default::default() });
    let mut cfg = ScanConfig::empty(Duration::from_millis(500));
    cfg.udp_targets = vec![
        SocketAddr::from((Ipv4Addr::LOCALHOST, a.udp_port)),
        SocketAddr::from((Ipv4Addr::LOCALHOST, b.udp_port)),
    ];
    let mut names: Vec<String> = discovery::scan(&cfg).into_iter().map(|s| s.name).collect();
    names.sort();
    assert_eq!(names, ["A", "B"]);
}

#[test]
fn bye_stopped_is_reported() {
    let phone = FakePhone::start(FakeConfig::default());
    let (_c, rx) = start_client(&phone.addr());
    collect_audio(&rx, 960);
    phone.stop_stream();
    let e = wait_for(&rx, |e| matches!(e, ClientEvent::Bye { .. }));
    assert_eq!(e, ClientEvent::Bye { reason: "stopped".into() });
    let (reason, requested) = disconnect_reason(&rx);
    assert_eq!((reason.as_str(), requested), ("stopped", false));
}

#[test]
fn session_records_bye_and_hello_time() {
    let phone = FakePhone::start(FakeConfig::default());
    let s = Session::start(&phone.addr(), SessionOptions::default(), None);
    let t = Instant::now();
    while s.snapshot().frames < 5 {
        assert!(t.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(s.snapshot().hello_at.is_some());
    let (_c2, _rx2) = start_client(&phone.addr()); // takes over
    while s.snapshot().phase != Phase::Disconnected {
        assert!(t.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    let snap = s.snapshot();
    assert_eq!(snap.bye.as_deref(), Some("replaced"));
    assert_eq!(snap.error.as_deref(), Some("replaced"));
}

#[test]
fn non_vpm_server_is_identified() {
    for garbage in [&b"SSH-2.0-OpenSSH_9.6\r\n"[..], b"HTTP/1.1 400 Bad Request\r\n\r\n", b"\x00\x00\x00\x00\x00"] {
        let phone = FakePhone::start(FakeConfig { behavior: Behavior::Garbage(garbage.to_vec()), ..Default::default() });
        let (_c, rx) = start_client(&phone.addr());
        let (reason, _) = disconnect_reason(&rx);
        assert!(reason.starts_with("Not a Vibe Phone Mic server"), "{reason}");
        // Probe (scan) rejects it too.
        let addr: SocketAddr = phone.addr().parse().unwrap();
        assert!(discovery::probe(addr, Duration::from_millis(500)).is_err());
    }
}

#[test]
fn invalid_hello_values_rejected() {
    for (json, expect) in [
        (r#"{"name":"x","sampleRate":48000,"channels":257,"format":"s16le"}"#, "channels 257"),
        (r#"{"name":"x","sampleRate":48000,"channels":0,"format":"s16le"}"#, "channels 0"),
        (r#"{"name":"x","sampleRate":1000,"channels":1,"format":"s16le"}"#, "sampleRate 1000"),
        (r#"{"name":"x","sampleRate":500000,"channels":1,"format":"s16le"}"#, "sampleRate 500000"),
        ("[]", "not a JSON object"),
        (r#"{"name":"x","channels":1,"format":"s16le"}"#, "sampleRate"),
        (r#"{"name":"x","sampleRate":48000,"format":"s16le"}"#, "channels"),
        (r#"{"name":"x","sampleRate":48000,"channels":1}"#, "format"),
    ] {
        let g = vibe_phone_mic::protocol::Frame::Unknown { kind: 0x01, payload: json.as_bytes().to_vec() }.encode();
        let phone = FakePhone::start(FakeConfig { behavior: Behavior::Garbage(g), ..Default::default() });
        let (_c, rx) = start_client(&phone.addr());
        let mut saw_hello = false;
        let reason = loop {
            match rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                ClientEvent::Hello(_) => saw_hello = true,
                ClientEvent::Disconnected { reason, .. } => break reason,
                _ => {}
            }
        };
        assert!(!saw_hello, "{json}");
        assert!(reason.contains("HELLO") && reason.contains(expect), "{json}: {reason}");
    }
}
