//! Click sounds: drive the real `VpmApp` with pointer events and check which samples it asks to play.

mod common;

use eframe::egui::{self, epaint::Shape, Pos2};
use vibe_phone_mic::gui::{GuiOptions, VpmApp};
use vibe_phone_mic::sfx::{PlayLog, Sfx, Sound};

type Played = Vec<(Sound, bool)>;

struct Harness {
    ctx: egui::Context,
    app: VpmApp,
}

fn find_text(shape: &Shape, needle: &str) -> Option<Pos2> {
    match shape {
        Shape::Text(t) if t.galley.text() == needle => Some(t.pos + t.galley.rect.center().to_vec2()),
        Shape::Vec(v) => v.iter().find_map(|s| find_text(s, needle)),
        _ => None,
    }
}

impl Harness {
    fn frame(&mut self, events: Vec<egui::Event>) -> Vec<egui::epaint::ClippedShape> {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(480.0, 1400.0))),
            events,
            ..Default::default()
        };
        let mut out = self.ctx.run_ui(input, |ui| self.app.draw(ui));
        out.textures_delta.clear(); // no renderer: drop pending texture uploads (epaint asserts otherwise)
        out.shapes
    }

    fn locate(&mut self, label: &str) -> Pos2 {
        let shapes = self.frame(vec![]);
        shapes.iter().find_map(|s| find_text(&s.shape, label)).unwrap_or_else(|| panic!("{label:?} not drawn"))
    }

    fn button(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    /// Press, hold for a couple of frames, release. Returns what was played after the press and after the release.
    fn click(&mut self, label: &str, log: &PlayLog) -> (Played, Played) {
        let pos = self.locate(label);
        self.frame(vec![egui::Event::PointerMoved(pos)]);
        log.lock().unwrap().clear();
        self.frame(vec![Self::button(pos, true)]);
        self.frame(vec![]);
        let pressed = std::mem::take(&mut *log.lock().unwrap());
        self.frame(vec![Self::button(pos, false)]);
        self.frame(vec![]);
        (pressed, std::mem::take(&mut *log.lock().unwrap()))
    }
}

fn harness() -> (Harness, PlayLog) {
    let ctx = egui::Context::default();
    let mut app = VpmApp::new(&ctx, GuiOptions { virtual_mic: false, ..Default::default() });
    let (sfx, log) = Sfx::recorder();
    app.set_sfx(sfx);
    (Harness { ctx, app }, log)
}

#[test]
fn rescan_plays_tiny_click_down_then_up() {
    let (mut h, log) = harness();
    let (down, up) = h.click("Rescan", &log);
    assert_eq!(down, vec![(Sound::CarsonMicroflip, true)]);
    assert_eq!(up, vec![(Sound::CarsonMicroflip, false)]);
}

#[test]
fn connect_plays_kvm_switch() {
    let (mut h, log) = harness();
    let free = common::closed_addr();
    h.app.set_manual(&free.ip().to_string(), &free.port().to_string());
    let (down, up) = h.click("Connect", &log);
    assert_eq!(down, vec![(Sound::KvmSwitch, true)]);
    assert_eq!(up, vec![(Sound::KvmSwitch, false)]);
}

#[test]
fn no_sounds_by_default() {
    let ctx = egui::Context::default();
    let app = VpmApp::new(&ctx, GuiOptions::default());
    let mut h = Harness { ctx, app };
    // Disabled player: clicking must not panic or play (nothing to observe beyond running cleanly).
    let pos = h.locate("Rescan");
    h.frame(vec![Harness::button(pos, true)]);
    h.frame(vec![Harness::button(pos, false)]);
}
