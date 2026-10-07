//! PipeWire restart recovery and legacy-module handling, on a PRIVATE PipeWire
//! instance (its own test binary because it kills/restarts the daemons).

mod common;

use common::pw::{private_pw, PrivatePw};
use common::{FakeConfig, FakePhone};
use eframe::egui::{self, epaint::Shape};
use std::io::BufRead;
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use vibe_phone_mic::audio::{self, MicHost};
use vibe_phone_mic::dsp;
use vibe_phone_mic::gui::{GuiOptions, VpmApp};

static LOCK: Mutex<()> = Mutex::new(());

fn setup() -> Option<(&'static PrivatePw, std::sync::MutexGuard<'static, ()>)> {
    let pw = private_pw()?;
    pw.apply_to_process();
    Some((pw, LOCK.lock().unwrap_or_else(|e| e.into_inner())))
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

fn record(pw: &PrivatePw, secs: f64) -> Vec<i16> {
    let mut child = pw
        .cmd("parec")
        .args(["--latency-msec=20", "--device", audio::SOURCE_NAME, "--format=s16le", "--rate=48000", "--channels=1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut out = child.stdout.take().unwrap();
    let th = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut v);
        v
    });
    std::thread::sleep(Duration::from_secs_f64(secs));
    let _ = child.kill();
    let _ = child.wait();
    th.join().unwrap().chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
}

#[test]
fn recovers_after_pipewire_restart_gui_and_audio() {
    let Some((pw, _g)) = setup() else { return };
    let ctx = egui::Context::default();
    let mut app = VpmApp::new(&ctx, GuiOptions { virtual_mic: true, ..Default::default() });
    assert!(app.mic_active());
    let t = gui_text(&ctx, &mut app);
    assert!(t.contains("In your apps, select"), "{t}");

    pw.stop_daemons();
    let t0 = Instant::now();
    let mut t = String::new();
    while app.mic_active() || !t.contains("reconnecting to PipeWire") {
        assert!(t0.elapsed() < Duration::from_secs(5), "GUI did not notice PipeWire going away:\n{t}");
        std::thread::sleep(Duration::from_millis(50));
        t = gui_text(&ctx, &mut app);
    }
    assert!(t.contains("Virtual microphone unavailable — reconnecting to PipeWire…"), "{t}");
    assert!(!t.contains("In your apps, select"), "{t}");

    pw.start_daemons();
    let t0 = Instant::now();
    while !app.mic_active() {
        assert!(t0.elapsed() < Duration::from_secs(8), "mic not re-created after PipeWire came back");
        std::thread::sleep(Duration::from_millis(50));
    }
    let t = gui_text(&ctx, &mut app);
    assert!(t.contains("In your apps, select") && !t.contains("reconnecting to PipeWire"), "{t}");
    pw.wait_for("source back", Duration::from_secs(3), || pw.sources_matching("vibe_phone_mic").len() == 1);

    // Audio flows through the re-created source.
    let phone = FakePhone::start(FakeConfig { freq: 660.0, ..Default::default() });
    app.set_manual("127.0.0.1", &phone.tcp_port.to_string());
    app.connect(&ctx);
    let t0 = Instant::now();
    while !app.session_state().is_some_and(|s| s.frames > 20 && s.mic_active) {
        assert!(t0.elapsed() < Duration::from_secs(5));
        gui_text(&ctx, &mut app);
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(300));
    let rec = record(pw, 1.5);
    let tail = &rec[rec.len().min(9600)..];
    let f = dsp::dominant_frequency(&dsp::to_mono(tail, 1), 48000).unwrap();
    assert!((f - 660.0).abs() <= 2.0 && dsp::rms(tail) > 0.2, "after restart: {f} Hz rms {}", dsp::rms(tail));
    app.shutdown();
}

#[test]
fn headless_reports_mic_loss_and_return() {
    let Some((pw, _g)) = setup() else { return };
    let phone = FakePhone::start(FakeConfig::default());
    let mut child = pw
        .cmd(env!("CARGO_BIN_EXE_vibe-phone-mic"))
        .args(["--headless", "--connect", &phone.addr(), "--duration", "8"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let stdout = child.stdout.take().unwrap();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for l in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
            let _ = tx.send(l);
        }
    });
    let wait_line = |pat: &str| {
        let t0 = Instant::now();
        loop {
            let l = rx.recv_timeout(Duration::from_secs(8).saturating_sub(t0.elapsed())).unwrap_or_else(|_| panic!("no line {pat}"));
            if l.starts_with(pat) {
                return l;
            }
        }
    };
    wait_line("VIRTUAL_MIC ready");
    wait_line("HELLO");
    pw.stop_daemons();
    let l = wait_line("VIRTUAL_MIC unavailable");
    assert!(l.contains("retrying"), "{l}");
    pw.start_daemons();
    wait_line("VIRTUAL_MIC ready");
    let out = child.wait().unwrap();
    assert!(out.success());
}

#[test]
fn legacy_modules_from_older_versions() {
    let Some((pw, _g)) = setup() else { return };
    // Old null-sink + remap pair: safe to unload, removed automatically.
    pw.pactl(&["load-module", "module-null-sink", "sink_name=vibe_phone_mic_sink"]);
    pw.pactl(&["load-module", "module-remap-source", "master=vibe_phone_mic_sink.monitor", "source_name=vibe_phone_mic"]);
    assert!(pw.pactl(&["list", "short", "modules"]).contains("sink_name=vibe_phone_mic_sink"));
    let host = MicHost::create().unwrap();
    let mods = pw.pactl(&["list", "short", "modules"]);
    assert!(!mods.contains("vibe_phone_mic"), "legacy modules not removed:\n{mods}");
    assert!(host.legacy_hint().is_none());
    assert_eq!(pw.sources_matching("vibe_phone_mic").len(), 1);
    drop(host);
    pw.wait_for("source removal", Duration::from_secs(2), || pw.sources_matching("vibe_phone_mic").is_empty());

    // Old pipe-source: NOT unloaded (that can crash PipeWire 1.6.8); a hint is shown instead.
    let fifo = pw.dir.join("run/legacy.fifo");
    let m = pw.pactl(&[
        "load-module",
        "module-pipe-source",
        "source_name=vibe_phone_mic",
        &format!("file={}", fifo.display()),
        "format=s16le",
        "rate=48000",
        "channels=1",
    ]);
    let m = m.trim().to_string();
    assert!(!m.is_empty());
    let host = MicHost::create().unwrap();
    let hint = host.legacy_hint().expect("hint about the old pipe-source").to_string();
    assert!(hint.contains("restart PipeWire") && hint.contains(&m), "{hint}");
    assert!(pw.pactl(&["list", "short", "modules"]).contains("module-pipe-source"), "pipe-source must be left alone");
    assert!(audio::cleanup_stale_modules().unwrap_err().starts_with(audio::IN_USE)); // in-use while host lives
    let ctx = egui::Context::default();
    drop(host);
    let mut app = VpmApp::new(&ctx, GuiOptions { virtual_mic: true, ..Default::default() });
    let t = gui_text(&ctx, &mut app);
    assert!(t.contains("An old \"Vibe Phone Mic\" pipe-source"), "{t}");
    app.shutdown();
    // cleanup_stale_modules() also leaves it alone on PipeWire.
    assert_eq!(audio::cleanup_stale_modules().unwrap(), 0);
    assert!(pw.pactl(&["list", "short", "modules"]).contains("module-pipe-source"));
    // Private instance only: remove it so other tests start clean.
    pw.pactl(&["unload-module", &m]);
}
