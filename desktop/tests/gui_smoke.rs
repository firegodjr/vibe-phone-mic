//! Headless GUI smoke test: run the egui UI logic for several frames (no window)
//! in each theme and state, and make sure it lays out and tessellates.

mod common;

use common::{FakeConfig, FakePhone};
use eframe::egui::{self, Theme};
use std::time::{Duration, Instant};
use vibe_phone_mic::discovery::{ScanConfig, Server};
use vibe_phone_mic::gui::{GuiOptions, VpmApp};
use vibe_phone_mic::session::Phase;

fn frame(ctx: &egui::Context, app: &mut VpmApp) -> usize {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(480.0, 780.0))),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    out.textures_delta.clear();
    let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
    prims.len()
}

#[test]
fn renders_idle_and_connected_in_both_themes() {
    let phone = FakePhone::start(FakeConfig { name: "Smoke Phone".into(), ..Default::default() });
    for theme in [Theme::Light, Theme::Dark] {
        let ctx = egui::Context::default();
        ctx.set_theme(theme);
        // Idle, with a (loopback-only) scan.
        let mut cfg = ScanConfig::empty(Duration::from_millis(300));
        cfg.probe_targets = vec![phone.addr().parse().unwrap()];
        let mut app = VpmApp::new(&ctx, GuiOptions { auto_scan: true, scan_config: Some(cfg), ..Default::default() });
        assert!(frame(&ctx, &mut app) > 0);
        let t = Instant::now();
        while t.elapsed() < Duration::from_millis(800) {
            frame(&ctx, &mut app);
            std::thread::sleep(Duration::from_millis(30));
        }
        app.set_servers(vec![Server { host: "192.168.1.23".into(), port: 48777, name: "Pixel 7".into(), source: "udp".into() }]);
        assert!(frame(&ctx, &mut app) > 0);

        // Connected (no virtual mic so it runs anywhere).
        let mut app = VpmApp::new(
            &ctx,
            GuiOptions { connect: Some(phone.addr()), virtual_mic: false, ..Default::default() },
        );
        let t = Instant::now();
        loop {
            frame(&ctx, &mut app);
            if app.session_state().is_some_and(|s| s.phase == Phase::Streaming && s.frames > 10) {
                break;
            }
            assert!(t.elapsed() < Duration::from_secs(10), "GUI never reached streaming");
            std::thread::sleep(Duration::from_millis(20));
        }
        phone.set_status(true, 150);
        std::thread::sleep(Duration::from_millis(100));
        assert!(frame(&ctx, &mut app) > 0);
        assert!(app.session_state().unwrap().status.unwrap().muted);
        phone.set_status(false, 100);
        app.disconnect();
        assert!(frame(&ctx, &mut app) > 0);
        assert!(app.session_state().is_none());
    }
}
