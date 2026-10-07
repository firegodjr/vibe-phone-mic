//! Offscreen pixel tests: render the real UI with the built-in CPU rasterizer
//! (no window, GPU or display) and check colours / palette rules. PNGs are
//! written to $CARGO_TARGET_TMPDIR/gui-snapshots for visual review.

mod common;

use common::{Behavior, FakeConfig, FakePhone};
use eframe::egui::{self, Color32, ColorImage, Theme};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use vibe_phone_mic::gui::{save_png, GuiOptions, VpmApp};
use vibe_phone_mic::render::{render_ui, SoftRenderer};

const SIZE: [f32; 2] = [480.0, 900.0];

fn bg(theme: Theme) -> Color32 {
    if theme == Theme::Dark { Color32::from_rgb(0x1C, 0x1C, 0x1E) } else { Color32::WHITE }
}

struct Shot {
    ctx: egui::Context,
    app: VpmApp,
    r: SoftRenderer,
    theme: Theme,
}

impl Shot {
    fn new(theme: Theme, opts: GuiOptions) -> Shot {
        let ctx = egui::Context::default();
        ctx.set_theme(theme);
        let app = VpmApp::new(&ctx, opts);
        Shot { ctx, app, r: SoftRenderer::new(), theme }
    }
    fn step(&mut self) {
        let (ctx, app) = (&self.ctx, &mut self.app);
        render_ui(ctx, &mut self.r, SIZE, 1.0, bg(self.theme), false, |ui| app.draw(ui));
    }
    fn settle(&mut self, pred: impl Fn(&VpmApp) -> bool) {
        let t = Instant::now();
        while !pred(&self.app) {
            assert!(t.elapsed() < Duration::from_secs(8), "state never reached");
            self.step();
            std::thread::sleep(Duration::from_millis(20));
        }
        // A few more frames so the level meter animates in.
        for _ in 0..10 {
            self.step();
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    fn render(&mut self, name: &str) -> ColorImage {
        let (ctx, app) = (&self.ctx, &mut self.app);
        let img = render_ui(ctx, &mut self.r, SIZE, 1.0, bg(self.theme), true, |ui| app.draw(ui)).unwrap();
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("gui-snapshots");
        save_png(&dir.join(format!("{name}.png")), &img).unwrap();
        img
    }
}

/// Pixels that are clearly "coloured" (not gray).
fn saturated(c: Color32) -> bool {
    let (r, g, b) = (c.r() as i32, c.g() as i32, c.b() as i32);
    r.max(g).max(b) - r.min(g).min(b) > 40
}

/// Orange→red family: red dominant, blue lowest.
fn brand_hue(c: Color32) -> bool {
    c.r() >= c.g() && c.g() as i32 >= c.b() as i32 - 8
}

fn check_palette(img: &ColorImage, theme: Theme) -> usize {
    assert_eq!(img.pixels[5 * img.size[0] + 5], bg(theme), "background colour");
    let mut n = 0;
    for (i, &c) in img.pixels.iter().enumerate() {
        if saturated(c) {
            assert!(brand_hue(c), "non-brand accent colour {c:?} at {:?}", (i % img.size[0], i / img.size[0]));
            n += 1;
        }
    }
    n
}

fn opts() -> GuiOptions {
    GuiOptions { virtual_mic: false, ..Default::default() }
}

#[test]
fn idle_has_gradient_connect_button_and_logo_only() {
    for theme in [Theme::Light, Theme::Dark] {
        let mut s = Shot::new(theme, opts());
        s.app.set_manual("192.168.1.23", "48777");
        s.step();
        let img = s.render(&format!("idle-{theme:?}"));
        let n = check_palette(&img, theme);
        assert!(n > 5000, "expected gradient button + logo, got {n} coloured px");
        // The Connect button is a horizontal gradient: orange on its left, red on its right.
        let (w, _) = (img.size[0], img.size[1]);
        let btn_row = (0..img.size[1]).find(|&y| {
            let c = img.pixels[y * w + w / 2];
            c.r() > 200 && c.g() < 120
        });
        let y = btn_row.expect("gradient Connect button") + 5;
        let left = img.pixels[y * w + 60];
        let right = img.pixels[y * w + w - 60];
        assert!(left.g() > right.g() + 50, "left {left:?} should be more orange than right {right:?}");
    }
}

#[test]
fn connected_shows_gradient_indicator_and_meter() {
    let phone = FakePhone::start(FakeConfig { name: "Pixel 7".into(), ..Default::default() });
    for theme in [Theme::Light, Theme::Dark] {
        let mut s = Shot::new(theme, GuiOptions { connect: Some(phone.addr()), ..opts() });
        s.settle(|a| a.session_state().is_some_and(|st| st.frames > 20));
        let img = s.render(&format!("connected-{theme:?}"));
        let n = check_palette(&img, theme);
        assert!(n > 2000, "{n}");
        // Gradient goes from orange (#FF8A00-ish) to red (#E52E2E-ish) somewhere on screen.
        assert!(img.pixels.iter().any(|c| c.r() > 245 && (120..=150).contains(&c.g()) && c.b() < 30), "orange end");
        assert!(img.pixels.iter().any(|c| (215..=240).contains(&c.r()) && c.g() < 70 && c.b() < 70), "red end");
        s.app.disconnect();
    }
}

#[test]
fn muted_and_error_states_render() {
    let phone = FakePhone::start(FakeConfig { name: "Pixel 7".into(), ..Default::default() });
    let mut s = Shot::new(Theme::Light, GuiOptions { connect: Some(phone.addr()), ..opts() });
    s.settle(|a| a.session_state().is_some_and(|st| st.frames > 5));
    phone.set_status(true, 120);
    s.settle(|a| a.session_state().is_some_and(|st| st.status.as_ref().is_some_and(|x| x.muted)));
    let img = s.render("muted-Light");
    check_palette(&img, Theme::Light);
    s.app.disconnect();

    let bad = FakePhone::start(FakeConfig { behavior: Behavior::CloseAfter(5), ..Default::default() });
    let mut s = Shot::new(
        Theme::Dark,
        GuiOptions { connect: Some(bad.addr()), reconnect_interval: Some(Duration::from_secs(60)), ..opts() },
    );
    s.settle(|a| a.is_reconnecting());
    let img = s.render("reconnecting-Dark");
    check_palette(&img, Theme::Dark);
}

/// Max per-channel jump between horizontally adjacent pixels in `rect` (x0..x1, y0..y1).
fn max_step(img: &ColorImage, x0: usize, x1: usize, y0: usize, y1: usize) -> i32 {
    let mut m = 0;
    for y in y0..y1 {
        for x in x0 + 1..x1 {
            let (a, b) = (img.pixels[y * img.size[0] + x - 1], img.pixels[y * img.size[0] + x]);
            for (p, q) in [(a.r(), b.r()), (a.g(), b.g()), (a.b(), b.b())] {
                m = m.max((p as i32 - q as i32).abs());
            }
        }
    }
    m
}

#[test]
fn gradient_fill_is_smooth_without_speckles() {
    // Semi-transparent and opaque gradients over both backgrounds, at many sub-pixel
    // positions/sizes and scales: a pixel covered by two triangles of the fan mesh
    // would be blended twice and show up as a jump between neighbours.
    for theme in [Theme::Light, Theme::Dark] {
        for opacity in [1.0f32, 0.45] {
            for ppp in [1.0f32, 1.5, 2.0] {
                for k in 0..6 {
                    let off = k as f32 * 0.37;
                    let rect = egui::Rect::from_min_size(egui::pos2(24.0 + off, 20.0 + off * 0.5), egui::vec2(380.0 + off * 3.0, 46.0));
                    let ctx = egui::Context::default();
                    let mut r = SoftRenderer::new();
                    let draw = |ui: &mut egui::Ui| {
                        vibe_phone_mic::gui::paint_gradient_rect(ui.painter(), rect, 10.0, opacity);
                    };
                    render_ui(&ctx, &mut r, [440.0, 90.0], ppp, bg(theme), false, draw);
                    let img = render_ui(&ctx, &mut r, [440.0, 90.0], ppp, bg(theme), true, draw).unwrap();
                    // Interior only (stay clear of the rounded, anti-aliased border).
                    let inner = rect.shrink(12.0);
                    let (x0, x1) = ((inner.min.x * ppp) as usize, (inner.max.x * ppp) as usize);
                    let (y0, y1) = ((inner.min.y * ppp) as usize, (inner.max.y * ppp) as usize);
                    let step = max_step(&img, x0, x1, y0, y1);
                    assert!(step <= 3, "{theme:?} opacity {opacity} ppp {ppp} off {off}: speckle/seam step {step}");
                }
            }
        }
    }
}

#[test]
fn disabled_connect_button_is_neutral_gray() {
    for theme in [Theme::Light, Theme::Dark] {
        let mut s = Shot::new(theme, opts());
        s.step();
        let img = s.render(&format!("idle-disabled-{theme:?}"));
        let w = img.size[0];
        // Only the logo (top ~80 px) may be coloured; the disabled button must not be.
        for (i, &c) in img.pixels.iter().enumerate() {
            if i / w > 80 {
                assert!(!saturated(c), "{theme:?}: coloured pixel {c:?} at {:?} with Connect disabled", (i % w, i / w));
            }
        }
    }
}
