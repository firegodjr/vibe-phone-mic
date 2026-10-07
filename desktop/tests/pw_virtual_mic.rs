//! Virtual mic (native PipeWire source) against a PRIVATE PipeWire instance
//! (tools/pw-private.sh) — never the user's audio daemon. Skips with a message
//! when the private instance can't be started.
//!
//! `VPM_LONG_TESTS=1` also runs the 2-minute drift test.

mod common;

use common::pw::{private_pw, PrivatePw};
use common::{FakeConfig, FakePhone};
use std::io::Write;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use vibe_phone_mic::audio::{self, MicBackend, MicHost};
use vibe_phone_mic::dsp;
use eframe::egui::{self, epaint::Shape};
use vibe_phone_mic::gui::{GuiOptions, VpmApp};
use vibe_phone_mic::session::{Session, SessionOptions};

static MIC_LOCK: Mutex<()> = Mutex::new(());

/// Private instance + process env + exclusive use of the (global) mic.
fn setup() -> Option<(&'static PrivatePw, std::sync::MutexGuard<'static, ()>)> {
    let pw = private_pw()?;
    pw.apply_to_process();
    let g = MIC_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    Some((pw, g))
}

type Recording = (std::process::Child, std::thread::JoinHandle<(Option<Instant>, Vec<u8>)>);

/// Record s16le mono 48k from vibe_phone_mic; stdout drained continuously.
/// Also returns when the first bytes arrived (for latency estimates).
fn start_record(pw: &PrivatePw, latency_ms: u32) -> Recording {
    let lat = format!("--latency-msec={latency_ms}");
    let mut child = pw
        .cmd("parec")
        .args([lat.as_str(), "--device", audio::SOURCE_NAME, "--format=s16le", "--rate=48000", "--channels=1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn parec");
    let mut out = child.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut v = Vec::new();
        let mut first = None;
        let mut buf = [0u8; 4096];
        loop {
            match std::io::Read::read(&mut out, &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    first.get_or_insert_with(Instant::now);
                    v.extend_from_slice(&buf[..n]);
                }
            }
        }
        (first, v)
    });
    (child, reader)
}

fn stop_record_t((mut child, reader): Recording) -> (Option<Instant>, Vec<i16>) {
    let _ = child.kill();
    let _ = child.wait();
    let (first, bytes) = reader.join().unwrap();
    (first, bytes.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect())
}

fn stop_record(r: Recording) -> Vec<i16> {
    stop_record_t(r).1
}

fn record(pw: &PrivatePw, secs: f64) -> Vec<i16> {
    let r = start_record(pw, 20);
    std::thread::sleep(Duration::from_secs_f64(secs));
    stop_record(r)
}

/// Structural regression for the "mic plays system audio" bug: exactly one
/// source, no sink of ours, no modules of ours, nothing linked INTO it.
fn assert_structure(pw: &PrivatePw, what: &str) {
    assert_eq!(pw.sources_matching("vibe_phone_mic"), vec!["vibe_phone_mic".to_string()], "{what}: sources");
    let sinks = pw.pactl(&["list", "short", "sinks"]);
    assert!(!sinks.contains("vibe_phone_mic"), "{what}: a vibe_phone_mic sink exists:\n{sinks}");
    let mods = pw.pactl(&["list", "short", "modules"]);
    assert!(!mods.contains("vibe_phone_mic"), "{what}: modules of ours are loaded:\n{mods}");
    let out = pw.cmd("pw-link").arg("-l").output().expect("pw-link");
    let text = String::from_utf8_lossy(&out.stdout);
    let mut port = "";
    for line in text.lines() {
        if !line.starts_with(' ') {
            port = line.trim();
            continue;
        }
        let l = line.trim();
        if port.starts_with("vibe_phone_mic") && l.starts_with("|<-") {
            panic!("{what}: {port} is fed by {l} (system audio could leak in)\n{text}");
        }
        if l.starts_with("|->") && l.contains("vibe_phone_mic") {
            panic!("{what}: {port} links into our mic: {l}\n{text}");
        }
    }
}

/// Positions (s) of discontinuities in a pure tone at `f` (second-order sine
/// predictor residual > 300, same metric as e2e/verify_desktop/clicks.py).
fn discontinuities(x: &[i16], f: f64) -> Vec<f64> {
    let k = 2.0 - 2.0 * (2.0 * std::f64::consts::PI * f / 48000.0).cos();
    let mut groups: Vec<usize> = Vec::new();
    let mut last = 0usize;
    for n in 2..x.len() {
        let r = x[n] as f64 - 2.0 * x[n - 1] as f64 + x[n - 2] as f64 + k * x[n - 1] as f64;
        if r.abs() > 300.0 {
            if groups.is_empty() || n - last > 480 {
                groups.push(n);
            }
            last = n;
        }
    }
    groups.into_iter().map(|n| n as f64 / 48000.0).collect()
}

fn check_tone(rec: &[i16], f: f64, what: &str) {
    let tail = &rec[rec.len().min(14400)..];
    assert!(tail.len() > 9600, "{what}: recorded only {} samples", rec.len());
    let rms = dsp::rms(tail);
    assert!(rms > 0.02, "{what}: virtual mic is silent (rms={rms})");
    let got = dsp::dominant_frequency(&dsp::to_mono(tail, 1), 48000).unwrap();
    assert!((got - f).abs() <= 2.0, "{what}: dominant {got} Hz, expected {f}");
    let full: Vec<f64> = tail.chunks_exact(2400).map(dsp::rms).collect();
    let worst = full.iter().cloned().fold(f64::MAX, f64::min);
    assert!(worst > 0.25, "{what}: dropout, weakest 50 ms window rms={worst:.3}");
    let clicks = discontinuities(tail, f);
    assert!(clicks.is_empty(), "{what}: discontinuities at {clicks:?} s (after the first 0.3 s)");
    eprintln!("{what}: rms={rms:.4} dominant={got:.1} Hz, no clicks");
}

fn wait_streaming(s: &Session) {
    let t = Instant::now();
    while !(s.snapshot().mic_active && s.snapshot().frames > 10) {
        assert!(t.elapsed() < Duration::from_secs(10), "no audio into virtual mic: {:?}", s.snapshot().mic_error);
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Feed `secs` of a 48 kHz mono tone into the host in real time (10 ms chunks).
fn feed_tone(host: &MicHost, generation: u64, f: f64, secs: f64, amp: f64) {
    let t0 = Instant::now();
    let total = (48000.0 * secs) as u64;
    let mut n = 0u64;
    while n < total {
        let frame: Vec<i16> = (0..480)
            .map(|i| ((2.0 * std::f64::consts::PI * f * (n + i) as f64 / 48000.0).sin() * amp * 32767.0) as i16)
            .collect();
        n += 480;
        host.push(generation, &frame);
        let target = t0 + Duration::from_micros(n * 1_000_000 / 48000);
        if let Some(d) = target.checked_duration_since(Instant::now()) {
            std::thread::sleep(d);
        }
    }
}

#[test]
fn native_source_structure_tone_and_removal() {
    let Some((pw, _g)) = setup() else { return };
    let host = MicHost::create().expect("create virtual mic");
    assert_eq!(host.backend(), MicBackend::PipeWire);
    assert!(host.is_active() && host.node_id().is_some());
    assert_structure(pw, "idle");
    let g = host.begin();
    let rec = start_record(pw, 20);
    std::thread::scope(|s| {
        s.spawn(|| feed_tone(&host, g, 440.0, 2.5, 0.4));
        std::thread::sleep(Duration::from_millis(1200));
        assert_structure(pw, "streaming");
    });
    let rec = stop_record(rec);
    eprintln!("direct stats: {:?}", host.stats());
    check_tone(&rec, 440.0, "direct");
    // Large consumer buffer must not cause dropouts either.
    let rec = start_record(pw, 250);
    feed_tone(&host, g, 440.0, 2.5, 0.4);
    let rec = stop_record(rec);
    check_tone(&rec[..rec.len().saturating_sub(4800)], 440.0, "direct, 250 ms consumer");
    drop(host);
    pw.wait_for("source removal", Duration::from_secs(2), || pw.sources_matching("vibe_phone_mic").is_empty());
}

#[test]
fn session_formats_and_mic_survives_reconnects() {
    let Some((pw, _g)) = setup() else { return };
    let host = MicHost::create().unwrap();
    let opts = SessionOptions { mic: Some(host.clone()), ..Default::default() };
    for (rate, channels, f) in [(48000u32, 1u16, 440.0), (44100, 1, 523.0), (48000, 2, 660.0), (44100, 2, 880.0)] {
        let phone = FakePhone::start(FakeConfig { rate, channels, freq: f, ..Default::default() });
        let mut session = Session::start(&phone.addr(), opts.clone(), None);
        wait_streaming(&session);
        std::thread::sleep(Duration::from_millis(300));
        check_tone(&record(pw, 2.0), f, &format!("{rate} Hz x{channels}"));
        assert_structure(pw, "session streaming");
        session.stop();
        drop(session);
        assert_eq!(pw.sources_matching("vibe_phone_mic").len(), 1, "virtual mic must survive a disconnect");
        // Between connections: digital silence (after in-flight audio played out).
        std::thread::sleep(Duration::from_millis(300));
        let gap = record(pw, 1.2);
        let tail = &gap[gap.len().min(9600)..];
        assert!(tail.len() > 9600, "idle mic must keep producing samples ({} samples)", gap.len());
        let peak = tail.iter().map(|&x| (x as i32).abs()).max().unwrap();
        assert_eq!(peak, 0, "idle virtual mic must be digitally silent");
    }
    host.shutdown();
    pw.wait_for("source removal", Duration::from_secs(2), || pw.sources_matching("vibe_phone_mic").is_empty());
}

#[test]
fn underrun_gives_silence_then_resumes() {
    let Some((pw, _g)) = setup() else { return };
    let host = MicHost::create().unwrap();
    let g = host.begin();
    let rec = start_record(pw, 20);
    std::thread::sleep(Duration::from_millis(300));
    feed_tone(&host, g, 440.0, 1.5, 0.4);
    std::thread::sleep(Duration::from_millis(1000)); // phone stalls
    feed_tone(&host, g, 440.0, 1.5, 0.4);
    std::thread::sleep(Duration::from_millis(200));
    let rec = stop_record(rec);
    let win: Vec<f64> = rec.chunks(2400).map(dsp::rms).collect();
    let loud = |r: f64| r > 0.1;
    let first = win.iter().position(|&r| loud(r)).expect("tone before the stall");
    let gap_start = first + win[first..].iter().position(|&r| r == 0.0).expect("digital silence during the stall");
    let resume = gap_start + win[gap_start..].iter().position(|&r| loud(r)).expect("tone after the stall");
    let gap_len = resume - gap_start;
    assert!((14..=22).contains(&gap_len), "silent gap of {gap_len} x 50 ms (expected ~1 s)");
    let after = &rec[(resume + 2) * 2400..((resume + 22) * 2400).min(rec.len())];
    let f = dsp::dominant_frequency(&dsp::to_mono(after, 1), 48000).unwrap();
    assert!((f - 440.0).abs() <= 2.0, "after resume: {f}");
    assert!(host.stats().underruns >= 1);
}

#[test]
fn latency_is_low() {
    let Some((pw, _g)) = setup() else { return };
    let host = MicHost::create().unwrap();
    let g = host.begin();
    let mut results = Vec::new();
    for _ in 0..3 {
        let rec = start_record(pw, 10);
        std::thread::sleep(Duration::from_millis(800));
        let t_push = Instant::now();
        feed_tone(&host, g, 1000.0, 0.6, 0.5);
        std::thread::sleep(Duration::from_millis(300));
        let (first, rec) = stop_record_t(rec);
        let first = first.expect("recorded something");
        let onset = rec.iter().position(|&x| x.unsigned_abs() > 3000).expect("tone recorded") as f64 / 48000.0;
        // Sample 0 was captured no later than `first`, so this is an upper bound.
        // `first` arrived ~800 ms before the push, so `lead` is positive.
        let lead = t_push.duration_since(first).as_secs_f64();
        let latency = onset - lead;
        results.push(latency);
        host.clear(g);
        std::thread::sleep(Duration::from_millis(300));
    }
    let best = results.iter().cloned().fold(f64::MAX, f64::min);
    eprintln!("latency upper bounds (s): {results:?}");
    assert!(best <= 0.090, "end-to-end latency too high: {results:?}");
}

#[test]
fn stale_session_cannot_touch_new_one() {
    let Some((_pw, _g)) = setup() else { return };
    let host = MicHost::create().unwrap();
    let old = host.begin();
    let new = host.begin();
    assert!(host.accepts(new) && !host.accepts(old));
    // Nobody records here, so the buffer keeps only the newest 40 ms (1920 samples).
    host.push(new, &vec![1000i16; 4800]);
    host.clear(old);
    let q = host.queued();
    assert_eq!(q, 1920, "stale session cleared the new session's audio (queued {q})");
    host.push(old, &vec![5i16; 4800]);
    assert!(host.queued() <= q, "stale session fed the mic");
    host.clear(new);
    assert_eq!(host.queued(), 0);
}

#[test]
fn single_owner() {
    let Some((pw, _g)) = setup() else { return };
    let first = MicHost::create().unwrap();
    let err = MicHost::create().unwrap_err();
    assert!(err.starts_with(audio::IN_USE), "{err}");
    assert_eq!(pw.sources_matching("vibe_phone_mic").len(), 1, "no second Vibe Phone Mic");
    drop(first);
    pw.wait_for("source removal", Duration::from_secs(2), || pw.sources_matching("vibe_phone_mic").is_empty());
    drop(MicHost::create().unwrap());
}

#[test]
fn app_sigkill_leaves_nothing_behind() {
    let Some((pw, _g)) = setup() else { return };
    let phone = FakePhone::start(FakeConfig::default());
    let mut child = pw
        .cmd(env!("CARGO_BIN_EXE_vibe-phone-mic"))
        .args(["--headless", "--connect", &phone.addr()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    pw.wait_for("source from child", Duration::from_secs(5), || !pw.sources_matching("vibe_phone_mic").is_empty());
    child.kill().unwrap(); // SIGKILL
    child.wait().unwrap();
    pw.wait_for("source gone after SIGKILL", Duration::from_secs(3), || pw.sources_matching("vibe_phone_mic").is_empty());
    let mods = pw.pactl(&["list", "short", "modules"]);
    assert!(!mods.contains("vibe_phone_mic"), "{mods}");
    let host = MicHost::create().expect("lock released by the dead process");
    assert_structure(pw, "after crash");
    drop(host);
}

#[test]
fn no_system_audio_leak() {
    let Some((pw, _g)) = setup() else { return };
    // Distractor: 1000 Hz into the private default sink.
    let play = |secs: f64| {
        let mut p = pw
            .cmd("pacat")
            .args(["--playback", "--format=s16le", "--rate=48000", "--channels=1", "--latency-msec=50"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("pacat");
        let n = (48000.0 * secs) as usize;
        let pcm: Vec<u8> = (0..n)
            .flat_map(|i| (((2.0 * std::f64::consts::PI * 1000.0 * i as f64 / 48000.0).sin() * 0.5 * 32767.0) as i16).to_le_bytes())
            .collect();
        let mut stdin = p.stdin.take().unwrap();
        std::thread::spawn(move || {
            let _ = stdin.write_all(&pcm);
            drop(stdin);
            let _ = p.wait();
        })
    };
    let host = MicHost::create().unwrap();
    // Sanity: the distractor really plays (visible on the sink monitor).
    let d = play(2.0);
    std::thread::sleep(Duration::from_millis(400));
    let mon = {
        let r = pw
            .cmd("parec")
            .args(["--latency-msec=20", "--device", "private_out.monitor", "--format=s16le", "--rate=48000", "--channels=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut r = r;
        let mut out = r.stdout.take().unwrap();
        let th = std::thread::spawn(move || {
            let mut v = Vec::new();
            let _ = std::io::Read::read_to_end(&mut out, &mut v);
            v
        });
        std::thread::sleep(Duration::from_millis(800));
        let _ = r.kill();
        let _ = r.wait();
        th.join().unwrap()
    };
    let mon: Vec<i16> = mon.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
    let idle = record(pw, 0.8);
    d.join().unwrap();
    assert!(dsp::rms(&mon) > 0.05, "distractor not audible on the sink monitor (rms {})", dsp::rms(&mon));
    let peak = idle[idle.len().min(4800)..].iter().map(|&x| (x as i32).abs()).max().unwrap_or(0);
    assert_eq!(peak, 0, "idle mic not silent while system audio plays");

    let phone = FakePhone::start(FakeConfig { freq: 523.0, ..Default::default() });
    let session = Session::start(&phone.addr(), SessionOptions { mic: Some(host.clone()), ..Default::default() }, None);
    wait_streaming(&session);
    let d = play(3.0);
    std::thread::sleep(Duration::from_millis(400));
    let rec = record(pw, 2.0);
    d.join().unwrap();
    let tail: Vec<f64> = dsp::to_mono(&rec[rec.len().min(4800)..], 1);
    let ratio = dsp::goertzel_power(&tail, 48000, 1000.0) / dsp::goertzel_power(&tail, 48000, 523.0);
    eprintln!("leak check: monitor rms {:.3}, idle peak {peak}, P(1000)/P(523) = {ratio:e}", dsp::rms(&mon));
    assert!(ratio < 1e-6, "system audio leaked into the mic: {ratio:e}");
}

#[test]
fn gui_shows_mic_and_second_window_does_not_take_it() {
    let Some((pw, _g)) = setup() else { return };
    let ctx = eframe::egui::Context::default();
    let first = VpmApp::new(&ctx, GuiOptions { virtual_mic: true, ..Default::default() });
    assert!(first.mic_active());
    let second = VpmApp::new(&ctx, GuiOptions { virtual_mic: true, ..Default::default() });
    assert!(!second.mic_active());
    drop(second);
    assert_eq!(pw.sources_matching("vibe_phone_mic").len(), 1);
    drop(first);
    pw.wait_for("source removal", Duration::from_secs(2), || pw.sources_matching("vibe_phone_mic").is_empty());
}

#[test]
fn two_minutes_without_drift() {
    if std::env::var("VPM_LONG_TESTS").ok().as_deref() != Some("1") {
        eprintln!("SKIP: set VPM_LONG_TESTS=1 for the 2-minute drift test");
        return;
    }
    let Some((pw, _g)) = setup() else { return };
    let host = MicHost::create().unwrap();
    let phone = FakePhone::start(FakeConfig { freq: 440.0, ..Default::default() });
    let session = Session::start(&phone.addr(), SessionOptions { mic: Some(host.clone()), ..Default::default() }, None);
    wait_streaming(&session);
    let rec = start_record(pw, 20);
    std::thread::sleep(Duration::from_secs(1));
    let base = host.stats();
    let err0 = graph_errors(pw);
    let t0 = Instant::now();
    let mut max_q = 0;
    while t0.elapsed() < Duration::from_secs(120) {
        std::thread::sleep(Duration::from_millis(100));
        max_q = max_q.max(host.queued());
    }
    let end = host.stats();
    let elapsed = t0.elapsed();
    let rec = stop_record(rec);
    let st = vibe_phone_mic::pwmic::JitterStats {
        underruns: end.underruns - base.underruns,
        dropped: end.dropped - base.dropped,
        pulled: end.pulled - base.pulled,
        trimmed: end.trimmed - base.trimmed,
        idle_skipped: end.idle_skipped - base.idle_skipped,
    };
    eprintln!("drift: 120 s, max queue {max_q} samples, stats {st:?}, recorded {} s", rec.len() as f64 / 48000.0);
    assert_eq!(st.dropped, 0, "overflow drops (clock drift)");
    assert!(st.underruns <= 2, "underruns: {}", st.underruns);
    assert!(max_q < 100 * 48, "queue grew to {max_q}");
    assert!(st.trimmed <= 3 * 48 * 100, "latency trims: {}", st.trimmed);
    let clicks = discontinuities(&rec[4800..], 440.0);
    if !clicks.is_empty() {
        let p = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("drift.raw");
        let bytes: Vec<u8> = rec.iter().flat_map(|s| s.to_le_bytes()).collect();
        let _ = std::fs::write(&p, bytes);
        eprintln!("recording saved to {}", p.display());
    }
    let xruns = graph_errors(pw).saturating_sub(err0);
    eprintln!("graph xruns (pw-top ERR) during the run: {xruns}; clicks: {clicks:?}");
    // Our side is verified clean above (no underrun/drop/trim). A discontinuity
    // is only acceptable if PipeWire itself reported an xrun at the same time
    // (this container has no realtime scheduling).
    assert!(clicks.len() as u64 <= xruns, "clicks during 2 minutes at {clicks:?} s without graph xruns");
    let pulled_rate = st.pulled as f64 / elapsed.as_secs_f64();
    assert!((pulled_rate - 48000.0).abs() < 100.0, "pull rate {pulled_rate}");
    // The last 10 s are still a clean, continuous tone.
    let last = &rec[rec.len().saturating_sub(48000 * 10)..];
    check_tone(last, 440.0, "after 2 minutes");
}

fn gui_text(ctx: &egui::Context, app: &mut VpmApp) -> String {
    fn collect(shape: &Shape, out: &mut Vec<String>) {
        match shape {
            Shape::Text(t) => out.push(t.galley.text().to_string()),
            Shape::Vec(v) => v.iter().for_each(|s| collect(s, out)),
            _ => {}
        }
    }
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 1400.0))),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    out.textures_delta.clear();
    let mut v = Vec::new();
    for c in &out.shapes {
        collect(&c.shape, &mut v);
    }
    v.join("\n")
}

fn gui_until(ctx: &egui::Context, app: &mut VpmApp, what: &str, pred: impl Fn(&str) -> bool) -> String {
    let t0 = Instant::now();
    loop {
        let t = gui_text(ctx, app);
        if pred(&t) {
            return t;
        }
        assert!(t0.elapsed() < Duration::from_secs(8), "timed out waiting for {what}:\n{t}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn gui_mic_lives_for_app_lifetime() {
    let Some((pw, _g)) = setup() else { return };
    let phone = FakePhone::start(FakeConfig::default());
    let ctx = egui::Context::default();
    let mut app = VpmApp::new(
        &ctx,
        GuiOptions { virtual_mic: true, reconnect_interval: Some(Duration::from_millis(200)), ..Default::default() },
    );
    let one = || pw.sources_matching("vibe_phone_mic").len() == 1;
    // Created at startup, before any connection; hint shown while idle.
    assert!(app.mic_active() && one());
    let t = gui_text(&ctx, &mut app);
    assert!(t.contains("In your apps, select") && t.contains("Not connected"), "{t}");
    app.set_manual("127.0.0.1", &phone.tcp_port.to_string());
    app.connect(&ctx);
    gui_until(&ctx, &mut app, "streaming", |t| t.contains("Connected to"));
    // Survives user disconnect, reconnect, connection loss / auto-reconnect.
    app.disconnect();
    gui_text(&ctx, &mut app);
    assert!(app.mic_active() && one());
    app.connect(&ctx);
    gui_until(&ctx, &mut app, "reconnected", |t| t.contains("Connected to"));
    phone.kick();
    gui_until(&ctx, &mut app, "reconnecting", |t| t.contains("Reconnecting"));
    assert!(one());
    gui_until(&ctx, &mut app, "back", |t| t.contains("Connected to"));
    assert!(one());
    assert_structure(pw, "gui streaming");
    // Removed on app exit.
    drop(app);
    pw.wait_for("source removal on exit", Duration::from_secs(2), || pw.sources_matching("vibe_phone_mic").is_empty());
}

/// Regression: a recorder attaching while a phone streams used to get a click
/// exactly 2 s later (queue had grown while nobody pulled, then was trimmed at
/// once) and up to 250 ms extra latency meanwhile.
#[test]
fn late_attach_no_click_and_low_latency() {
    let Some((pw, _g)) = setup() else { return };
    let host = MicHost::create().unwrap();
    for delay in [0.3, 1.0, 3.0, 5.5] {
        let phone = FakePhone::start(FakeConfig { freq: 440.0, ..Default::default() });
        let session = Session::start(&phone.addr(), SessionOptions { mic: Some(host.clone()), ..Default::default() }, None);
        let t = Instant::now();
        while session.snapshot().frames < 3 {
            assert!(t.elapsed() < Duration::from_secs(5));
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_secs_f64(delay));
        let rec = start_record(pw, 20);
        // Queue (≈ our share of the latency) during the first second after attach.
        let t0 = Instant::now();
        let mut max_q = 0;
        while t0.elapsed() < Duration::from_millis(1000) {
            max_q = max_q.max(host.queued());
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(2500)); // past the old 2 s trim point
        let rec = stop_record(rec);
        drop(session);
        let start = rec.iter().position(|&x| x.unsigned_abs() > 3000).expect("tone recorded");
        let body = &rec[start..];
        let clicks = discontinuities(&body[FADE_SKIP..], 440.0);
        eprintln!("late attach after {delay} s: max queue {max_q} samples ({} ms), clicks {clicks:?}", max_q / 48);
        assert!(clicks.is_empty(), "attach after {delay} s: discontinuities at {clicks:?} s");
        assert!(max_q <= 70 * 48, "attach after {delay} s: queue {} ms in the first second", max_q / 48);
        let f = dsp::dominant_frequency(&dsp::to_mono(&body[body.len() - 48000..], 1), 48000).unwrap();
        assert!((f - 440.0).abs() <= 2.0);
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Samples to skip at a recording's tone onset (the 5 ms fade-in).
const FADE_SKIP: usize = 480;

/// Sum of the ERR (xrun) column of `pw-top` for the private instance.
fn graph_errors(pw: &PrivatePw) -> u64 {
    let out = pw.cmd("pw-top").args(["-b", "-n", "2"]).output().expect("pw-top");
    let text = String::from_utf8_lossy(&out.stdout);
    let last_block: Vec<&str> = text.lines().rev().take_while(|l| !l.starts_with("S   ID")).collect();
    last_block.iter().filter_map(|l| l.split_whitespace().nth(8)).filter_map(|e| e.parse::<u64>().ok()).sum()
}
