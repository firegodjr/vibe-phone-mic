//! Tests of the CLI binary (`--scan`, `--headless`).

mod common;

use common::{Behavior, FakeConfig, FakePhone};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use vibe_phone_mic::dsp;

const BIN: &str = env!("CARGO_BIN_EXE_vibe-phone-mic");

fn run(args: &[&str]) -> (Output, String, String) {
    let out = Command::new(BIN).args(args).output().unwrap();
    let so = String::from_utf8_lossy(&out.stdout).to_string();
    let se = String::from_utf8_lossy(&out.stderr).to_string();
    (out, so, se)
}

fn summary_field<'a>(stdout: &'a str, key: &str) -> &'a str {
    let line = stdout.lines().find(|l| l.starts_with("SUMMARY ")).expect("SUMMARY line");
    line.split_whitespace()
        .find_map(|kv| kv.strip_prefix(&format!("{key}=")))
        .unwrap_or_else(|| panic!("no {key} in {line}"))
}

fn tmp(name: &str) -> std::path::PathBuf {
    let d = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("cli");
    std::fs::create_dir_all(&d).unwrap();
    d.join(name)
}

#[test]
fn headless_streams_to_wav_and_logs() {
    let phone = FakePhone::start(FakeConfig { name: "CLI Phone".into(), ..Default::default() });
    let wav = tmp("headless.wav");
    let (out, so, se) = run(&["--headless", "--connect", &phone.addr(), "--wav", wav.to_str().unwrap(), "--duration", "1.5", "--no-virtual-mic"]);
    assert!(out.status.success(), "{so}\n{se}");
    assert!(so.contains(r#"HELLO {"name":"CLI Phone","sampleRate":48000,"channels":1"#), "{so}");
    assert!(so.contains(r#"STATUS {"muted":false,"volume":100"#), "{so}");
    assert!(so.lines().any(|l| l.starts_with("CONNECTED ")));
    let frames: u64 = summary_field(&so, "frames").parse().unwrap();
    assert!((120..=160).contains(&frames), "frames={frames}");
    assert_eq!(summary_field(&so, "samples").parse::<u64>().unwrap(), frames * 480);
    assert!(summary_field(&so, "rms").parse::<f64>().unwrap() > 0.2);
    let hz: f64 = summary_field(&so, "dominant_hz").parse().unwrap();
    assert!((hz - 440.0).abs() < 3.0);
    let mut r = hound::WavReader::open(&wav).unwrap();
    assert_eq!(r.spec().sample_rate, 48000);
    let s: Vec<i16> = r.samples::<i16>().map(|x| x.unwrap()).collect();
    assert_eq!(s.len() as u64, frames * 480);
    let f = dsp::dominant_frequency(&dsp::to_mono(&s, 1), 48000).unwrap();
    assert!((f - 440.0).abs() < 3.0);
}

#[test]
fn headless_reports_muted_status() {
    let phone = FakePhone::start(FakeConfig::default());
    phone.set_status(true, 80);
    let (out, so, _) = run(&["--headless", "--connect", &phone.addr(), "--duration", "0.6", "--no-virtual-mic"]);
    assert!(out.status.success());
    assert!(so.contains(r#"STATUS {"muted":true,"volume":80"#), "{so}");
    assert_eq!(summary_field(&so, "rms"), "0.00000");
    assert_eq!(summary_field(&so, "dominant_hz"), "none");
    assert_eq!(summary_field(&so, "muted"), "true");
}

#[test]
fn headless_44100() {
    let phone = FakePhone::start(FakeConfig { rate: 44100, ..Default::default() });
    let wav = tmp("h44.wav");
    let (out, so, _) = run(&["--headless", "--connect", &phone.addr(), "--wav", wav.to_str().unwrap(), "--duration", "0.8", "--no-virtual-mic"]);
    assert!(out.status.success());
    assert!(so.contains(r#""sampleRate":44100"#));
    assert_eq!(hound::WavReader::open(&wav).unwrap().spec().sample_rate, 44100);
}

#[test]
fn headless_connection_refused_exits_1() {
    let free = common::closed_addr();
    let (out, so, se) = run(&["--headless", "--connect", &free.to_string(), "--duration", "2", "--no-virtual-mic"]);
    assert_eq!(out.status.code(), Some(1), "{so}\n{se}");
    assert_eq!(summary_field(&so, "frames"), "0");
    assert!(se.to_lowercase().contains("refused"), "{se}");
}

#[test]
fn headless_phone_closes_exits_0_with_partial_audio() {
    let phone = FakePhone::start(FakeConfig { behavior: Behavior::CloseAfter(30), ..Default::default() });
    let t = Instant::now();
    let (out, so, se) = run(&["--headless", "--connect", &phone.addr(), "--duration", "10", "--no-virtual-mic"]);
    assert!(t.elapsed() < Duration::from_secs(5), "should exit when the phone disconnects");
    assert!(out.status.success());
    assert_eq!(summary_field(&so, "frames"), "30");
    assert!(so.contains("DISCONNECTED phone closed the connection"), "{so}");
    assert!(se.contains("connection ended"));
}

#[test]
fn headless_argument_errors_exit_2() {
    let (out, _, se) = run(&["--headless"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(se.contains("--connect"));
    let (out, _, se) = run(&["--headless", "--connect", "300.1.1.1"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(se.contains("Invalid address"), "{se}");
    let (out, _, se) = run(&["--headless", "--connect", "1.2.3.4:99999"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(se.contains("Invalid port"), "{se}");
}

#[test]
fn scan_prints_json_array_within_budget() {
    let t = Instant::now();
    let (out, so, _) = run(&["--scan", "--timeout-ms", "600"]);
    assert!(out.status.success());
    assert!(t.elapsed() < Duration::from_millis(2500), "{:?}", t.elapsed());
    let v: serde_json::Value = serde_json::from_str(so.trim()).unwrap();
    for s in v.as_array().unwrap() {
        assert!(s["host"].is_string() && s["port"].is_u64() && s["name"].is_string() && s["source"].is_string());
    }
}

fn modules_loaded() -> bool {
    let o = Command::new("pactl").args(["list", "short", "modules"]).output().unwrap();
    String::from_utf8_lossy(&o.stdout).contains("source_name=vibe_phone_mic ")
}

/// Start headless with the virtual mic, wait for audio, send `signal`, check cleanup.
fn signal_cleanup(signal: &str) {
    let phone = FakePhone::start(FakeConfig::default());
    let child = Command::new(BIN)
        .args(["--headless", "--connect", &phone.addr()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let t = Instant::now();
    while !modules_loaded() {
        assert!(t.elapsed() < Duration::from_secs(5), "virtual mic not created");
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(500));
    Command::new("kill").args([signal, &child.id().to_string()]).status().unwrap();
    let out = child.wait_with_output().unwrap();
    let so = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{signal}: {so}");
    assert!(so.contains("VIRTUAL_MIC ready source=vibe_phone_mic"), "{so}");
    assert!(summary_field(&so, "frames").parse::<u64>().unwrap() > 20);
    assert!(!modules_loaded(), "modules must be unloaded after {signal}");
}

#[test]
fn headless_virtual_mic_lifecycle() {
    if !vibe_phone_mic::audio::pulse_available() {
        eprintln!("SKIP: no PulseAudio/PipeWire server");
        return;
    }
    // Tests in this binary run in parallel; the pulse ones share global modules, so run them in sequence.
    signal_cleanup("-INT");
    signal_cleanup("-TERM");

    // Created before connecting: present even if the phone is unreachable, removed on exit.
    let free = common::closed_addr();
    let (out, so, _) = run(&["--headless", "--connect", &free.to_string()]);
    assert_eq!(out.status.code(), Some(1));
    assert!(so.contains("VIRTUAL_MIC ready"), "{so}");
    assert!(!modules_loaded());

    // Phone drops the connection: headless exits and unloads.
    let phone = FakePhone::start(FakeConfig { behavior: Behavior::CloseAfter(20), ..Default::default() });
    let (out, so, _) = run(&["--headless", "--connect", &phone.addr()]);
    assert!(out.status.success(), "{so}");
    assert!(!modules_loaded());

    two_instances();
}

/// Second instance while the first owns the mic: it streams without a virtual
/// mic ("in-use") and never touches the first instance's modules.
fn two_instances() {
    let p1 = FakePhone::start(FakeConfig { freq: 523.0, ..Default::default() });
    let p2 = FakePhone::start(FakeConfig { freq: 880.0, ..Default::default() });
    let a = Command::new(BIN)
        .args(["--headless", "--connect", &p1.addr(), "--duration", "4"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let t = Instant::now();
    while !modules_loaded() {
        assert!(t.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(50));
    }
    std::thread::sleep(Duration::from_millis(300));
    let (out_b, so_b, se_b) = run(&["--headless", "--connect", &p2.addr(), "--duration", "1"]);
    assert!(out_b.status.success(), "{so_b}\n{se_b}");
    assert!(so_b.contains("VIRTUAL_MIC error in-use"), "{so_b}");
    assert!(!so_b.contains("VIRTUAL_MIC ready"), "{so_b}");
    assert!(summary_field(&so_b, "frames").parse::<u64>().unwrap() > 50);
    assert!(modules_loaded(), "second instance removed the first instance's virtual mic");
    // A's mic still carries A's 523 Hz tone.
    let mut rec = Command::new("parec")
        .args(["--latency-msec=20", "--device", "vibe_phone_mic", "--format=s16le", "--rate=48000", "--channels=1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let _ = rec.kill();
    let raw = rec.wait_with_output().unwrap().stdout;
    let pcm: Vec<i16> = raw.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
    let tail = &pcm[pcm.len().min(9600)..];
    assert!(tail.len() > 4800, "recorded {} samples", pcm.len());
    let f = dsp::dominant_frequency(&dsp::to_mono(tail, 1), 48000).unwrap();
    assert!((f - 523.0).abs() < 3.0, "first instance's mic carries {f} Hz");
    let out_a = a.wait_with_output().unwrap();
    assert!(out_a.status.success());
    assert!(String::from_utf8_lossy(&out_a.stdout).contains("VIRTUAL_MIC ready"));
    assert!(!modules_loaded(), "first instance unloads on exit");
}

#[test]
fn headless_taken_over_by_another_client() {
    let phone = FakePhone::start(FakeConfig::default());
    let a = Command::new(BIN)
        .args(["--headless", "--connect", &phone.addr(), "--duration", "6", "--no-virtual-mic"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    std::thread::sleep(Duration::from_millis(800));
    let t = Instant::now();
    let (out_b, so_b, _) = run(&["--headless", "--connect", &phone.addr(), "--duration", "1", "--no-virtual-mic"]);
    assert!(out_b.status.success(), "{so_b}");
    let out_a = a.wait_with_output().unwrap();
    assert!(t.elapsed() < Duration::from_secs(4), "A should stop when replaced, not run its full duration");
    let so_a = String::from_utf8_lossy(&out_a.stdout);
    assert!(out_a.status.success(), "{so_a}");
    assert!(so_a.contains(r#"BYE {"reason":"replaced"}"#), "{so_a}");
    assert!(so_a.contains("DISCONNECTED replaced"), "{so_a}");
    assert_eq!(phone.streams(), 2, "headless must not reconnect after being replaced");
}

#[test]
fn headless_rejects_invalid_hello() {
    let g = vibe_phone_mic::protocol::Frame::Unknown {
        kind: 0x01,
        payload: br#"{"name":"x","sampleRate":48000,"channels":257,"format":"s16le"}"#.to_vec(),
    }
    .encode();
    let phone = FakePhone::start(FakeConfig { behavior: Behavior::Garbage(g), ..Default::default() });
    let (out, so, se) = run(&["--headless", "--connect", &phone.addr(), "--duration", "2", "--no-virtual-mic"]);
    assert_eq!(out.status.code(), Some(1), "{so}");
    assert!(!so.contains("HELLO {"), "{so}");
    assert!(se.contains("channels 257 out of range"), "{se}");
}
