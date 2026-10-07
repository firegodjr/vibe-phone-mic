//! State-to-view tests: drive the real `VpmApp` (no window) through every client
//! state and assert on the text it actually paints.

mod common;

use common::{Behavior, FakeConfig, FakePhone};
use eframe::egui::{self, epaint::Shape};
use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};
use vibe_phone_mic::discovery::{ScanConfig, Server};
use vibe_phone_mic::gui::{GuiOptions, VpmApp};

struct Harness {
    ctx: egui::Context,
    app: VpmApp,
}

fn collect_text(shape: &Shape, out: &mut Vec<String>) {
    match shape {
        Shape::Text(t) => out.push(t.galley.text().to_string()),
        Shape::Vec(v) => v.iter().for_each(|s| collect_text(s, out)),
        _ => {}
    }
}

impl Harness {
    fn new(opts: GuiOptions) -> Harness {
        let ctx = egui::Context::default();
        let app = VpmApp::new(&ctx, opts);
        Harness { ctx, app }
    }

    /// Run one frame and return all painted text joined by newlines.
    fn frame(&mut self) -> String {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 1400.0))),
            ..Default::default()
        };
        let mut out = self.ctx.run_ui(input, |ui| self.app.draw(ui));
        out.textures_delta.clear();
        let mut texts = Vec::new();
        for cs in &out.shapes {
            collect_text(&cs.shape, &mut texts);
        }
        texts.join("\n")
    }

    /// Run frames until `pred(text)` holds (or panic after 8 s).
    fn until(&mut self, what: &str, mut pred: impl FnMut(&str, &VpmApp) -> bool) -> String {
        let t = Instant::now();
        loop {
            let text = self.frame();
            if pred(&text, &self.app) {
                return text;
            }
            assert!(t.elapsed() < Duration::from_secs(8), "timed out waiting for {what}; last view:\n{text}");
            std::thread::sleep(Duration::from_millis(15));
        }
    }

    fn connect(&mut self, host: &str, port: &str) {
        self.app.set_manual(host, port);
        let ctx = self.ctx.clone();
        self.app.connect(&ctx);
    }
}

fn opts() -> GuiOptions {
    GuiOptions { virtual_mic: false, ..Default::default() }
}

fn has(text: &str, needle: &str) -> bool {
    text.contains(needle)
}

#[test]
fn idle_view() {
    let mut h = Harness::new(opts());
    let t = h.frame();
    for s in ["Vibe Phone Mic", "PHONES ON YOUR NETWORK", "PHONE ADDRESS", "Connect", "Not connected", "48777", "Rescan"] {
        assert!(has(&t, s), "missing {s:?} in:\n{t}");
    }
    assert!(has(&t, "Press Rescan to search"), "{t}");
    assert!(has(&t, "Pick a phone above or enter its IP address."), "{t}");
    assert!(!has(&t, "Disconnect"));
}

#[test]
fn scanning_then_none_found() {
    let free = common::closed_addr();
    let mut cfg = ScanConfig::empty(Duration::from_millis(300));
    cfg.probe_targets = vec![free];
    cfg.udp_targets = vec![free];
    let mut h = Harness::new(GuiOptions { auto_scan: true, scan_config: Some(cfg), ..opts() });
    let t = h.frame();
    assert!(h.app.is_scanning());
    assert!(has(&t, "Scanning…") && has(&t, "Looking for phones") && !has(&t, "Rescan"), "{t}");
    let t = h.until("scan end", |_, a| !a.is_scanning());
    let t = if has(&t, "No phones found") { t } else { h.frame() };
    assert!(has(&t, "No phones found"), "{t}");
    assert!(has(&t, "adb forward tcp:48777 tcp:48777"), "{t}");
    assert!(has(&t, "Rescan"));
}

#[test]
fn scan_results_localhost_udp_sweep_and_dedupe() {
    let usb = FakePhone::start(FakeConfig { name: "USB Phone".into(), ..Default::default() });
    let wifi = FakePhone::start(FakeConfig { name: "WiFi Phone".into(), ..Default::default() });
    let swept = FakePhone::start(FakeConfig { name: "Swept Phone".into(), ..Default::default() });
    let mut cfg = ScanConfig::empty(Duration::from_millis(1200));
    // USB phone reachable both by probe and by its own UDP responder: must appear once.
    cfg.probe_targets = vec![SocketAddr::from((Ipv4Addr::LOCALHOST, usb.tcp_port))];
    cfg.udp_targets = vec![
        SocketAddr::from((Ipv4Addr::LOCALHOST, wifi.udp_port)),
        SocketAddr::from((Ipv4Addr::LOCALHOST, usb.udp_port)),
    ];
    cfg.sweep_networks = vec![Ipv4Addr::new(127, 0, 0, 0)];
    cfg.sweep_port = swept.tcp_port;
    cfg.probe_timeout = Duration::from_millis(200);
    let mut h = Harness::new(GuiOptions { auto_scan: true, scan_config: Some(cfg), ..opts() });
    h.until("scan results", |_, a| !a.is_scanning() && !a.servers().is_empty());
    let t = h.frame();
    assert_eq!(h.app.servers().len(), 3, "{:?}", h.app.servers());
    assert_eq!(h.app.servers().iter().filter(|s| s.name == "USB Phone").count(), 1, "deduped");
    for s in ["USB Phone", "WiFi Phone", "Swept Phone", "Wi-Fi (scan)", "Wi-Fi"] {
        assert!(has(&t, s), "missing {s:?}:\n{t}");
    }
    assert!(has(&t, &format!("127.0.0.1:{}", swept.tcp_port)));
    // First result is auto-selected into the address fields.
    let first = h.app.servers()[0].clone();
    assert!(has(&t, &first.host));
}

#[test]
fn localhost_result_labelled_usb() {
    let mut h = Harness::new(opts());
    h.app.set_servers(vec![Server { host: "127.0.0.1".into(), port: 48777, name: "Pixel 7".into(), source: "localhost".into() }]);
    let t = h.frame();
    assert!(has(&t, "Pixel 7") && has(&t, "127.0.0.1:48777") && has(&t, "USB / this computer"), "{t}");
}

#[test]
fn manual_entry_validation() {
    let mut h = Harness::new(opts());
    for (host, port, err) in [
        ("", "48777", "Enter the phone"),
        ("300.1.2.3", "", "Invalid address"),
        ("bad host", "", "Invalid address"),
        ("192.168.1.5", "abc", "Invalid port"),
        ("192.168.1.5", "70000", "Invalid port"),
        ("192.168.1.5", "0", "Invalid port"),
    ] {
        h.connect(host, port);
        assert!(h.app.session_state().is_none(), "{host}:{port} must not connect");
        let t = h.frame();
        assert!(has(&t, err), "{host:?}/{port:?}: expected {err:?} in:\n{t}");
        assert!(has(&t, "Not connected"));
    }
    // Host without port → default 48777; host:port in the host field wins.
    h.connect("127.0.0.1", "");
    assert_eq!(h.app.session_state().unwrap().addr, "127.0.0.1:48777");
    assert!(h.app.input_error().is_none());
    h.app.disconnect();
    h.connect("127.0.0.1:5555", "48777");
    assert_eq!(h.app.session_state().unwrap().addr, "127.0.0.1:5555");
    h.app.disconnect();
}

#[test]
fn connecting_view() {
    let mut h = Harness::new(opts());
    h.connect("10.255.255.1", "48777"); // unroutable → stays connecting
    let t = h.frame();
    assert!(has(&t, "Connecting to 10.255.255.1:48777…"), "{t}");
    assert!(has(&t, "Disconnect") && !has(&t, "Connected to"), "{t}");
    h.app.disconnect();
    let t = h.frame();
    assert!(has(&t, "Not connected") && !has(&t, "Connection lost"), "{t}");
}

#[test]
fn streaming_muted_volume_and_user_disconnect_and_reconnect() {
    let phone = FakePhone::start(FakeConfig { name: "Pixel 7".into(), ..Default::default() });
    let mut h = Harness::new(opts());
    h.connect("127.0.0.1", &phone.tcp_port.to_string());
    let t = h.until("streaming", |t, a| has(t, "Connected to Pixel 7") && a.session_state().is_some_and(|s| s.frames > 5));
    for s in ["48.0 kHz · mono · 16-bit", "Live", "100 %", "Test mic", "Input level", "Disconnect", "Virtual microphone disabled"] {
        assert!(has(&t, s), "missing {s:?}:\n{t}");
    }
    assert!(!has(&t, "Not connected"));

    phone.set_status(true, 100);
    let t = h.until("muted", |t, _| has(t, "Muted on phone"));
    assert!(!has(&t, "\nLive"));
    assert!(has(&t, "Input level · muted on phone"), "{t}");
    phone.set_status(false, 150);
    let t = h.until("volume 150", |t, _| has(t, "150 %"));
    assert!(has(&t, "Live"));
    phone.set_status(false, 0);
    h.until("volume 0", |t, _| has(t, "0 %") && !has(t, "150 %"));

    // User disconnect: back to idle without an error.
    h.app.disconnect();
    let t = h.frame();
    assert!(has(&t, "Not connected") && !has(&t, "Connection lost") && has(&t, "Connect"), "{t}");

    // Reconnect works.
    h.connect("127.0.0.1", &phone.tcp_port.to_string());
    h.until("reconnected", |t, _| has(t, "Connected to Pixel 7"));
    h.app.disconnect();
}

#[test]
fn hello_44100_stereo_view() {
    let phone = FakePhone::start(FakeConfig { name: "Old Phone".into(), rate: 44100, channels: 2, ..Default::default() });
    let mut h = Harness::new(opts());
    h.connect("127.0.0.1", &phone.tcp_port.to_string());
    let t = h.until("streaming", |t, _| has(t, "Connected to Old Phone"));
    assert!(has(&t, "44.1 kHz · stereo"), "{t}");
    h.app.disconnect();
}

fn error_view(behavior: Behavior, read_timeout: Option<Duration>, expect: &str) {
    let phone = FakePhone::start(FakeConfig { behavior, ..Default::default() });
    let mut h = Harness::new(GuiOptions { read_timeout, reconnect_interval: Some(Duration::from_secs(30)), ..opts() });
    h.connect("127.0.0.1", &phone.tcp_port.to_string());
    let t = h.until(expect, |t, _| has(t, "Connection lost"));
    assert!(has(&t, expect), "expected {expect:?}:\n{t}");
    // Lost connections are retried automatically, with a visible state.
    assert!(h.app.is_reconnecting());
    assert!(has(&t, "Reconnecting to") && has(&t, "Retrying 127.0.0.1:") && has(&t, "every 30 s"), "{t}");
    assert!(has(&t, "Disconnect") && !has(&t, "Not connected"), "{t}");
    h.app.disconnect();
    let t = h.frame();
    assert!(!h.app.is_reconnecting());
    assert!(has(&t, "Not connected") && !has(&t, "Reconnecting"), "{t}");
}

#[test]
fn phone_closes_connection_view() {
    error_view(Behavior::CloseAfter(15), None, "Connection lost: phone closed the connection");
}

#[test]
fn timeout_view() {
    error_view(Behavior::StallAfter(10), Some(Duration::from_millis(400)), "timed out");
}

/// A first connect that never got HELLO: "Couldn't connect …", idle, no retries.
fn first_connect_failure(host: &str, addr_port: u16, expect: &str) {
    let mut h = Harness::new(GuiOptions { reconnect_interval: Some(Duration::from_millis(100)), ..opts() });
    h.connect(host, &addr_port.to_string());
    let t = h.until("failure", |t, _| has(t, "Couldn't connect"));
    assert!(has(&t, &format!("Couldn't connect to {host}:{addr_port}: ")), "{t}");
    assert!(t.contains(expect), "expected {expect:?}:\n{t}");
    assert!(has(&t, "Not connected") && !has(&t, "Reconnecting") && !has(&t, "Disconnect"), "{t}");
    assert!(!h.app.is_reconnecting());
    // Stays idle: no automatic retries.
    for _ in 0..15 {
        let t = h.frame();
        assert!(!has(&t, "Reconnecting") && !has(&t, "Connecting to"), "{t}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(h.app.session_state().is_none());
    // A new Connect clears the message.
    h.connect("10.255.255.1", "48777");
    let t = h.frame();
    assert!(!has(&t, "Couldn't connect"), "{t}");
    h.app.disconnect();
}

#[test]
fn connection_refused_on_first_connect_view() {
    let free = common::closed_addr();
    first_connect_failure("127.0.0.2", free.port(), "refused");
}

#[test]
fn protocol_error_on_first_connect_view() {
    let phone = FakePhone::start(FakeConfig { behavior: Behavior::BadFirstFrame, ..Default::default() });
    first_connect_failure("127.0.0.1", phone.tcp_port, "Not a Vibe Phone Mic server");
}

#[test]
fn non_vpm_server_view() {
    let phone = FakePhone::start(FakeConfig { behavior: Behavior::Garbage(b"SSH-2.0-OpenSSH_9.6\r\n".to_vec()), ..Default::default() });
    first_connect_failure("127.0.0.1", phone.tcp_port, "Not a Vibe Phone Mic server");
}

#[test]
fn invalid_hello_view() {
    let g = vibe_phone_mic::protocol::Frame::Unknown {
        kind: 0x01,
        payload: br#"{"name":"x","sampleRate":48000,"channels":257,"format":"s16le"}"#.to_vec(),
    }
    .encode();
    let phone = FakePhone::start(FakeConfig { behavior: Behavior::Garbage(g), ..Default::default() });
    first_connect_failure("127.0.0.1", phone.tcp_port, "channels 257 out of range");
}

#[test]
fn taken_over_by_another_computer_view() {
    let phone = FakePhone::start(FakeConfig { name: "Shared Phone".into(), ..Default::default() });
    let interval = Some(Duration::from_millis(100));
    let mut a = Harness::new(GuiOptions { reconnect_interval: interval, ..opts() });
    let mut b = Harness::new(GuiOptions { reconnect_interval: interval, ..opts() });
    a.connect("127.0.0.1", &phone.tcp_port.to_string());
    a.until("A streaming", |t, _| has(t, "Connected to Shared Phone"));
    b.connect("127.0.0.1", &phone.tcp_port.to_string());
    b.until("B streaming", |t, _| has(t, "Connected to Shared Phone"));
    let t = a.until("A taken over", |t, _| has(t, "Another computer took over this phone."));
    assert!(has(&t, "Not connected") && !has(&t, "Reconnecting"), "{t}");
    assert!(!a.app.is_reconnecting());
    // A must not fight back: no new streams for well over the retry interval.
    for _ in 0..20 {
        a.frame();
        b.frame();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(phone.streams(), 2, "A reconnected and kicked B");
    let t = b.frame();
    assert!(has(&t, "Connected to Shared Phone"), "{t}");
    b.app.disconnect();
}

#[test]
fn phone_stopped_bye_keeps_retrying_quietly() {
    let phone = FakePhone::start(FakeConfig { name: "Pixel 7".into(), ..Default::default() });
    let mut h = Harness::new(GuiOptions { reconnect_interval: Some(Duration::from_millis(300)), ..opts() });
    h.connect("127.0.0.1", &phone.tcp_port.to_string());
    h.until("streaming", |t, _| has(t, "Connected to Pixel 7"));
    std::thread::sleep(Duration::from_millis(3100)); // not a "quick drop"
    phone.stop_stream();
    let t = h.until("reconnecting", |t, _| has(t, "Reconnecting to Pixel 7…"));
    assert!(has(&t, "Connection lost: the phone stopped the microphone"), "{t}");
    h.until("back", |t, _| has(t, "Connected to Pixel 7"));
    h.app.disconnect();
}

#[test]
fn gives_up_after_three_quick_drops() {
    // Each connection dies 200 ms after HELLO.
    let phone = FakePhone::start(FakeConfig { name: "Flaky".into(), behavior: Behavior::CloseAfter(20), ..Default::default() });
    let mut h = Harness::new(GuiOptions { reconnect_interval: Some(Duration::from_millis(100)), ..opts() });
    h.connect("127.0.0.1", &phone.tcp_port.to_string());
    let t = h.until("gave up", |t, _| has(t, "Stopped reconnecting"));
    assert!(has(&t, "dropped right after connecting 3 times in a row"), "{t}");
    assert!(has(&t, "phone closed the connection") && has(&t, "Not connected"), "{t}");
    assert!(!h.app.is_reconnecting());
    for _ in 0..15 {
        h.frame();
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(phone.streams(), 3);
}

#[test]
fn auto_reconnects_after_connection_loss() {
    let phone = FakePhone::start(FakeConfig { name: "Pixel 7".into(), ..Default::default() });
    let mut h = Harness::new(GuiOptions { reconnect_interval: Some(Duration::from_millis(300)), ..opts() });
    h.connect("127.0.0.1", &phone.tcp_port.to_string());
    h.until("streaming", |t, _| has(t, "Connected to Pixel 7"));
    for round in 0..2 {
        phone.kick();
        let t = h.until("reconnecting", |t, _| has(t, "Reconnecting to Pixel 7…"));
        assert!(has(&t, "Connection lost: phone closed the connection"), "round {round}: {t}");
        assert_eq!(h.app.reconnect_attempts(), 0, "the loss itself is not a failed attempt");
        let t = h.until("reconnected", |t, a| has(t, "Connected to Pixel 7") && !a.is_reconnecting());
        assert!(!has(&t, "Connection lost"), "{t}");
    }
    assert_eq!(phone.streams(), 3);
    h.app.disconnect();
}

#[test]
fn keeps_retrying_until_user_disconnects() {
    let phone = FakePhone::start(FakeConfig { name: "Gone Phone".into(), ..Default::default() });
    let port = phone.tcp_port;
    let mut h = Harness::new(GuiOptions { reconnect_interval: Some(Duration::from_millis(150)), ..opts() });
    h.connect("127.0.0.1", &port.to_string());
    h.until("streaming", |t, _| has(t, "Connected to Gone Phone"));
    drop(phone); // phone app closed: port now refuses
    let t = h.until("several failed attempts", |_, a| a.reconnect_attempts() >= 3);
    let t = if has(&t, "failed attempts") { t } else { h.frame() };
    assert!(has(&t, "Reconnecting to Gone Phone…") && has(&t, "failed attempts"), "{t}");
    h.app.disconnect();
    let t = h.frame();
    assert!(has(&t, "Not connected") && !has(&t, "Reconnecting"), "{t}");
    // No more attempts after Disconnect.
    std::thread::sleep(Duration::from_millis(400));
    h.frame();
    assert!(h.app.session_state().is_none() && !h.app.is_reconnecting());
}

#[test]
fn user_disconnect_does_not_reconnect() {
    let phone = FakePhone::start(FakeConfig::default());
    let mut h = Harness::new(GuiOptions { reconnect_interval: Some(Duration::from_millis(100)), ..opts() });
    h.connect("127.0.0.1", &phone.tcp_port.to_string());
    h.until("streaming", |t, _| has(t, "Connected to"));
    h.app.disconnect();
    for _ in 0..20 {
        let t = h.frame();
        assert!(has(&t, "Not connected") && !has(&t, "Reconnecting"), "{t}");
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(phone.streams(), 1);
}

// Virtual-mic GUI tests live in tests/pw_virtual_mic.rs (private PipeWire instance).
