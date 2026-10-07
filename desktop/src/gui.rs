//! eframe/egui desktop UI.

use crate::audio::MicHost;
use crate::discovery::{self, ScanConfig, Server};
use crate::protocol::DEFAULT_PORT;
use crate::session::{Phase, Session, SessionOptions, SessionState};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontFamily, FontId, Layout, Margin, Mesh, Pos2, Rect, RichText, Sense,
    Shape, Stroke, StrokeKind, TextStyle, Theme, Vec2,
};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const GRAD_START: Color32 = Color32::from_rgb(0xFF, 0x8A, 0x00);
pub const GRAD_END: Color32 = Color32::from_rgb(0xE5, 0x2E, 0x2E);

/// Neutral palette for one theme. The orange→red gradient is the only accent.
#[derive(Clone, Copy)]
struct Palette {
    bg: Color32,
    card: Color32,
    border: Color32,
    text: Color32,
    weak: Color32,
    track: Color32,
    selected: Color32,
}

fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            bg: Color32::from_rgb(0x1C, 0x1C, 0x1E),
            card: Color32::from_rgb(0x26, 0x26, 0x29),
            border: Color32::from_rgb(0x38, 0x38, 0x3C),
            text: Color32::from_rgb(0xF2, 0xF2, 0xF4),
            weak: Color32::from_rgb(0xA1, 0xA1, 0xA8),
            track: Color32::from_rgb(0x3A, 0x3A, 0x3E),
            selected: Color32::from_rgb(0x34, 0x34, 0x38),
        }
    } else {
        Palette {
            bg: Color32::from_rgb(0xFF, 0xFF, 0xFF),
            card: Color32::from_rgb(0xF6, 0xF6, 0xF7),
            border: Color32::from_rgb(0xE4, 0xE4, 0xE7),
            text: Color32::from_rgb(0x1D, 0x1D, 0x1F),
            weak: Color32::from_rgb(0x6E, 0x6E, 0x76),
            track: Color32::from_rgb(0xE4, 0xE4, 0xE7),
            selected: Color32::from_rgb(0xEC, 0xEC, 0xEF),
        }
    }
}

/// Install fonts sizes, spacing and neutral visuals for both themes.
pub fn setup_style(ctx: &egui::Context) {
    for theme in [Theme::Light, Theme::Dark] {
        let p = palette(theme == Theme::Dark);
        ctx.style_mut_of(theme, |style| {
            style.text_styles = [
                (TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional)),
                (TextStyle::Body, FontId::new(15.0, FontFamily::Proportional)),
                (TextStyle::Button, FontId::new(15.0, FontFamily::Proportional)),
                (TextStyle::Small, FontId::new(12.5, FontFamily::Proportional)),
                (TextStyle::Monospace, FontId::new(14.0, FontFamily::Monospace)),
            ]
            .into();
            style.spacing.item_spacing = Vec2::new(10.0, 10.0);
            style.spacing.button_padding = Vec2::new(14.0, 7.0);
            style.spacing.interact_size.y = 34.0;
            let v = &mut style.visuals;
            v.panel_fill = p.bg;
            v.window_fill = p.bg;
            v.extreme_bg_color = p.bg;
            v.text_edit_bg_color = Some(p.bg);
            v.faint_bg_color = p.card;
            v.override_text_color = None;
            v.hyperlink_color = p.text;
            v.selection.bg_fill = p.track;
            v.selection.stroke = Stroke::new(1.0, p.text);
            v.text_cursor.stroke = Stroke::new(2.0, p.text);
            let r = CornerRadius::same(8);
            for w in [
                &mut v.widgets.noninteractive,
                &mut v.widgets.inactive,
                &mut v.widgets.hovered,
                &mut v.widgets.active,
                &mut v.widgets.open,
            ] {
                w.corner_radius = r;
            }
            v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
            v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
            v.widgets.inactive.weak_bg_fill = p.card;
            v.widgets.inactive.bg_fill = p.card;
            v.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
            v.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
            v.widgets.hovered.weak_bg_fill = p.selected;
            v.widgets.hovered.bg_fill = p.selected;
            v.widgets.hovered.bg_stroke = Stroke::new(1.0, p.weak);
            v.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
            v.widgets.active.weak_bg_fill = p.track;
            v.widgets.active.bg_fill = p.track;
            v.widgets.active.bg_stroke = Stroke::new(1.0, p.text);
            v.widgets.active.fg_stroke = Stroke::new(1.0, p.text);
        });
    }
}

// ---------------------------------------------------------------------------
// Gradient drawing (egui mesh with per-vertex colours + 1px AA fringe)

fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

fn grad_at(rect: Rect, p: Pos2) -> Color32 {
    // Diagonal gradient: top-left orange → bottom-right red.
    let d = rect.size();
    let t = ((p.x - rect.min.x) * d.x + (p.y - rect.min.y) * d.y * 0.35) / (d.x * d.x + d.y * d.y * 0.35).max(1.0);
    lerp_color(GRAD_START, GRAD_END, t)
}

/// Fill a convex polygon (counter-clockwise or clockwise) with the brand gradient.
fn gradient_polygon(points: &[Pos2], bounds: Rect, opacity: f32) -> Mesh {
    let mut mesh = Mesh::default();
    let n = points.len() as u32;
    let center = bounds.center();
    let col = |p: Pos2| grad_at(bounds, p).gamma_multiply(opacity);
    mesh.colored_vertex(center, col(center));
    for &p in points {
        mesh.colored_vertex(p, col(p));
    }
    // Fringe: points pushed 1px outward, fully transparent → anti-aliased edge.
    for &p in points {
        let dir = (p - center).normalized();
        mesh.colored_vertex(p + dir * 1.0, Color32::TRANSPARENT);
    }
    for i in 0..n {
        let j = (i + 1) % n;
        mesh.add_triangle(0, 1 + i, 1 + j);
        mesh.add_triangle(1 + i, 1 + n + i, 1 + j);
        mesh.add_triangle(1 + j, 1 + n + i, 1 + n + j);
    }
    mesh
}

fn rounded_rect_points(rect: Rect, r: f32) -> Vec<Pos2> {
    let r = r.min(rect.width() / 2.0).min(rect.height() / 2.0);
    let corners = [
        (Pos2::new(rect.max.x - r, rect.min.y + r), -90.0f32),
        (Pos2::new(rect.max.x - r, rect.max.y - r), 0.0),
        (Pos2::new(rect.min.x + r, rect.max.y - r), 90.0),
        (Pos2::new(rect.min.x + r, rect.min.y + r), 180.0),
    ];
    let mut pts = Vec::new();
    for (c, start) in corners {
        for k in 0..=8 {
            let a = (start + 90.0 * k as f32 / 8.0).to_radians();
            pts.push(c + Vec2::new(a.cos(), a.sin()) * r);
        }
    }
    pts
}

pub fn paint_gradient_rect(painter: &egui::Painter, rect: Rect, radius: f32, opacity: f32) {
    painter.add(Shape::mesh(gradient_polygon(&rounded_rect_points(rect, radius), rect, opacity)));
}

pub fn paint_gradient_circle(painter: &egui::Painter, center: Pos2, radius: f32) {
    let pts: Vec<Pos2> = (0..48)
        .map(|i| {
            let a = i as f32 / 48.0 * std::f32::consts::TAU;
            center + Vec2::new(a.cos(), a.sin()) * radius
        })
        .collect();
    let bounds = Rect::from_center_size(center, Vec2::splat(radius * 2.0));
    painter.add(Shape::mesh(gradient_polygon(&pts, bounds, 1.0)));
}

/// Logo: gradient disc with a white microphone glyph.
pub fn paint_logo(painter: &egui::Painter, center: Pos2, radius: f32) {
    paint_gradient_circle(painter, center, radius);
    let s = radius / 20.0;
    let white = Color32::WHITE;
    let capsule = Rect::from_center_size(center + Vec2::new(0.0, -4.0 * s), Vec2::new(10.0 * s, 17.0 * s));
    painter.rect_filled(capsule, CornerRadius::same((5.0 * s) as u8), white);
    let stroke = Stroke::new(2.2 * s, white);
    let arc_c = center + Vec2::new(0.0, -3.0 * s);
    let arc: Vec<Pos2> = (0..=16)
        .map(|i| {
            let a = (i as f32 / 16.0) * std::f32::consts::PI;
            arc_c + Vec2::new(a.cos() * 8.5 * s, a.sin() * 8.5 * s)
        })
        .collect();
    painter.add(Shape::line(arc, stroke));
    painter.line_segment([center + Vec2::new(0.0, 5.5 * s), center + Vec2::new(0.0, 10.5 * s)], stroke);
    painter.line_segment([center + Vec2::new(-5.0 * s, 10.5 * s), center + Vec2::new(5.0 * s, 10.5 * s)], stroke);
}

/// Small microphone glyph (optionally struck through for "muted").
pub fn paint_mic_glyph(painter: &egui::Painter, center: Pos2, size: f32, color: Color32, slashed: bool) {
    let s = size / 16.0;
    let stroke = Stroke::new(1.5 * s, color);
    let cap = Rect::from_center_size(center + Vec2::new(0.0, -2.5 * s), Vec2::new(5.0 * s, 8.5 * s));
    painter.rect_stroke(cap, CornerRadius::same((2.5 * s) as u8), stroke, StrokeKind::Middle);
    let c = center + Vec2::new(0.0, -2.0 * s);
    let arc: Vec<Pos2> = (0..=12)
        .map(|i| {
            let a = (i as f32 / 12.0) * std::f32::consts::PI;
            c + Vec2::new(a.cos() * 4.8 * s, a.sin() * 4.8 * s)
        })
        .collect();
    painter.add(Shape::line(arc, stroke));
    painter.line_segment([center + Vec2::new(0.0, 2.8 * s), center + Vec2::new(0.0, 6.0 * s)], stroke);
    if slashed {
        painter.line_segment([center + Vec2::new(-6.0 * s, -7.0 * s), center + Vec2::new(6.0 * s, 7.0 * s)], Stroke::new(1.8 * s, color));
    }
}

/// Primary button filled with the gradient.
fn gradient_button(ui: &mut egui::Ui, text: &str, enabled: bool) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 46.0), if enabled { Sense::click() } else { Sense::hover() });
    if !enabled {
        // Disabled: neutral, no gradient.
        let p = palette(ui.visuals().dark_mode);
        ui.painter().rect_filled(rect, CornerRadius::same(10), p.track);
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::new(16.0, FontFamily::Proportional), p.weak);
        return resp;
    }
    if ui.is_rect_visible(rect) {
        let opacity = if resp.is_pointer_button_down_on() { 0.85 } else { 1.0 };
        paint_gradient_rect(ui.painter(), rect, 10.0, opacity);
        if resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(10), Color32::from_white_alpha(18));
        }
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::new(16.0, FontFamily::Proportional), Color32::WHITE);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Secondary (neutral, outlined) full-width button.
fn secondary_button(ui: &mut egui::Ui, p: &Palette, text: &str) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 46.0), Sense::click());
    let fill = if resp.hovered() { p.selected } else { p.bg };
    ui.painter().rect(rect, CornerRadius::same(10), fill, Stroke::new(1.0, p.border), StrokeKind::Inside);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, FontId::new(16.0, FontFamily::Proportional), p.text);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn card<R>(ui: &mut egui::Ui, p: &Palette, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn section_title(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.label(RichText::new(text.to_uppercase()).size(12.0).color(p.weak).strong());
}

/// Horizontal bar: track + fill (gradient or neutral).
fn bar(ui: &mut egui::Ui, p: &Palette, frac: f32, gradient: bool, height: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, height), Sense::hover());
    let r = height / 2.0;
    ui.painter().rect_filled(rect, CornerRadius::same(r as u8), p.track);
    let frac = frac.clamp(0.0, 1.0);
    if frac > 0.005 {
        let fill = Rect::from_min_size(rect.min, Vec2::new((w * frac).max(height), height));
        if gradient {
            paint_gradient_rect(ui.painter(), fill, r, 1.0);
        } else {
            ui.painter().rect_filled(fill, CornerRadius::same(r as u8), p.weak);
        }
    }
}

fn key_label(ui: &mut egui::Ui, p: &Palette, key: &str) {
    ui.allocate_ui_with_layout(Vec2::new(116.0, 20.0), Layout::left_to_right(Align::Center), |ui| {
        ui.set_min_width(116.0);
        ui.label(RichText::new(key).color(p.weak));
    });
}

fn kv_row(ui: &mut egui::Ui, p: &Palette, key: &str, value: RichText) {
    ui.horizontal(|ui| {
        key_label(ui, p, key);
        ui.label(value);
    });
}

// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct GuiOptions {
    /// Scan automatically on startup.
    pub auto_scan: bool,
    /// Connect to this address on startup.
    pub connect: Option<String>,
    /// Create the PulseAudio virtual mic at startup (kept until the app exits).
    pub virtual_mic: bool,
    /// Delay between automatic reconnect attempts (default 2 s).
    pub reconnect_interval: Option<Duration>,
    /// Scan configuration override (tests).
    pub scan_config: Option<ScanConfig>,
    /// "No data" timeout override (tests).
    pub read_timeout: Option<Duration>,
}

pub struct VpmApp {
    opts: GuiOptions,
    servers: Vec<Server>,
    selected: Option<usize>,
    host: String,
    port: String,
    scan_result: Arc<Mutex<Option<Vec<Server>>>>,
    scanning: bool,
    scanned_once: bool,
    input_error: Option<String>,
    session: Option<Session>,
    last_state: Option<SessionState>,
    level: f32,
    last_frame: Instant,
    reconnect: Option<Reconnect>,
    /// Consecutive connections that dropped within `QUICK_DROP` of HELLO.
    quick_drops: u32,
    /// Message shown while idle after a connection ended (failed / taken over / gave up).
    notice: Option<String>,
    mic: Option<Arc<MicHost>>,
    mic_error: Option<String>,
}

/// Automatic reconnect after a connection was lost (not user-initiated).
#[derive(Debug, Clone)]
struct Reconnect {
    addr: String,
    name: String,
    next: Instant,
    attempts: u32,
    last_error: String,
}

pub const RECONNECT_INTERVAL: Duration = Duration::from_secs(2);
/// A connection lost this soon after HELLO counts as a "quick drop".
pub const QUICK_DROP: Duration = Duration::from_secs(3);
/// Stop auto-reconnecting after this many quick drops in a row.
pub const MAX_QUICK_DROPS: u32 = 3;
pub const TAKEN_OVER: &str = "Another computer took over this phone.";

fn friendly_reason(r: &str) -> String {
    match r {
        crate::protocol::BYE_STOPPED => "the phone stopped the microphone".into(),
        crate::protocol::BYE_REPLACED => TAKEN_OVER.into(),
        // "Connection refused (os error 111)" → "Connection refused".
        _ => match r.find(" (os error") {
            Some(i) => r[..i].to_string(),
            None => r.to_string(),
        },
    }
}

impl VpmApp {
    pub fn new(ctx: &egui::Context, opts: GuiOptions) -> VpmApp {
        setup_style(ctx);
        let mut app = VpmApp {
            servers: Vec::new(),
            selected: None,
            host: String::new(),
            port: DEFAULT_PORT.to_string(),
            scan_result: Arc::new(Mutex::new(None)),
            scanning: false,
            scanned_once: false,
            input_error: None,
            session: None,
            last_state: None,
            level: 0.0,
            last_frame: Instant::now(),
            reconnect: None,
            quick_drops: 0,
            notice: None,
            mic: None,
            mic_error: None,
            opts,
        };
        if app.opts.virtual_mic {
            match MicHost::create() {
                Ok(m) => app.mic = Some(m),
                Err(e) => app.mic_error = Some(e),
            }
        }
        if let Some(addr) = app.opts.connect.clone() {
            let norm = crate::client::normalize_addr(&addr, DEFAULT_PORT);
            if let Some((h, port)) = norm.rsplit_once(':') {
                app.host = h.trim_start_matches('[').trim_end_matches(']').to_string();
                app.port = port.to_string();
            }
            app.connect(ctx);
        }
        if app.opts.auto_scan {
            app.start_scan(ctx);
        }
        app
    }

    /// Test hook: set the discovered list.
    pub fn set_servers(&mut self, servers: Vec<Server>) {
        self.servers = servers;
        self.scanned_once = true;
    }

    pub fn session_state(&self) -> Option<SessionState> {
        self.session.as_ref().map(|s| s.snapshot())
    }

    pub fn start_scan(&mut self, ctx: &egui::Context) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        let slot = self.scan_result.clone();
        let ctx = ctx.clone();
        let cfg = self.opts.scan_config.clone().unwrap_or_else(|| ScanConfig::system(Duration::from_millis(1500)));
        std::thread::spawn(move || {
            let found = discovery::scan(&cfg);
            *slot.lock().unwrap() = Some(found);
            ctx.request_repaint();
        });
    }

    /// Set the manual address fields (as if typed).
    pub fn set_manual(&mut self, host: &str, port: &str) {
        self.host = host.to_string();
        self.port = port.to_string();
        self.selected = None;
        self.input_error = None;
    }

    /// Validation error shown under the address fields, if any.
    pub fn input_error(&self) -> Option<&str> {
        self.input_error.as_deref()
    }

    pub fn is_reconnecting(&self) -> bool {
        self.reconnect.is_some()
    }

    /// Number of failed reconnect attempts so far (0 when not reconnecting).
    pub fn reconnect_attempts(&self) -> u32 {
        self.reconnect.as_ref().map(|r| r.attempts).unwrap_or(0)
    }

    /// Idle message after a connection ended, if any.
    pub fn notice(&self) -> Option<&str> {
        self.notice.as_deref()
    }

    pub fn mic_active(&self) -> bool {
        self.mic.as_ref().is_some_and(|m| m.is_active())
    }

    /// Disconnect and unload the virtual mic (app exit).
    pub fn shutdown(&mut self) {
        self.disconnect();
        if let Some(m) = self.mic.take() {
            m.shutdown();
        }
    }

    fn reconnect_interval(&self) -> Duration {
        self.opts.reconnect_interval.unwrap_or(RECONNECT_INTERVAL)
    }

    pub fn is_scanning(&self) -> bool {
        self.scanning
    }

    pub fn servers(&self) -> &[Server] {
        &self.servers
    }

    /// Same as pressing Connect.
    pub fn connect(&mut self, ctx: &egui::Context) {
        let addr = match crate::client::parse_manual_address(&self.host, &self.port) {
            Ok(a) => a,
            Err(e) => {
                self.input_error = Some(e);
                return;
            }
        };
        self.input_error = None;
        self.disconnect();
        self.quick_drops = 0;
        self.notice = None;
        self.start_session(ctx, &addr);
    }

    fn start_session(&mut self, ctx: &egui::Context, addr: &str) {
        let opts = SessionOptions {
            mic: self.mic.clone(),
            wav: None,
            log: false,
            read_timeout: self.opts.read_timeout,
        };
        let c = ctx.clone();
        self.session = Some(Session::start(addr, opts, Some(Box::new(move || c.request_repaint()))));
    }

    /// Same as pressing Disconnect (also cancels automatic reconnects).
    pub fn disconnect(&mut self) {
        self.reconnect = None;
        self.notice = None;
        if let Some(mut s) = self.session.take() {
            s.stop();
            let mut st = s.snapshot();
            st.phase = Phase::Disconnected;
            st.error = None;
            self.last_state = Some(st);
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        let scan_done = self.scan_result.lock().unwrap().take();
        if let Some(found) = scan_done {
            self.scanning = false;
            self.scanned_once = true;
            let prev = self.selected.and_then(|i| self.servers.get(i)).map(|s| s.addr());
            self.servers = found;
            self.selected = prev.and_then(|a| self.servers.iter().position(|s| s.addr() == a));
            if self.selected.is_none() {
                if self.host.is_empty() && !self.servers.is_empty() {
                    self.select(0);
                } else {
                    let (h, p) = (self.host.trim().to_string(), self.port.trim().to_string());
                    self.selected = self.servers.iter().position(|s| s.host == h && s.port.to_string() == p);
                }
            }
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        let state = self.session.as_ref().map(|s| s.snapshot());
        let target = match &state {
            Some(s) if s.phase == Phase::Streaming && s.last_audio.is_some_and(|t| t.elapsed() < Duration::from_millis(300)) => {
                level_to_meter(s.last_rms)
            }
            _ => 0.0,
        };
        // Fast attack, ~300 ms release.
        self.level = if target > self.level { target } else { self.level + (target - self.level) * (dt / 0.3).min(1.0) };
        let interval = self.reconnect_interval();
        if let Some(s) = state {
            if s.phase == Phase::Disconnected {
                self.session = None;
                self.on_session_ended(&s, now, interval);
            } else if s.phase == Phase::Streaming {
                self.reconnect = None;
            }
            self.last_state = Some(s);
        }
        if self.session.is_none()
            && let Some(r) = &self.reconnect {
                if now >= r.next {
                    let addr = r.addr.clone();
                    self.start_session(ctx, &addr);
                } else {
                    ctx.request_repaint_after((r.next - now).min(Duration::from_millis(100)));
                }
            }
        if self.session.is_some() || self.level > 0.001 {
            ctx.request_repaint_after(Duration::from_millis(33));
        }
    }

    /// Decide what happens after a session ended on its own (not via Disconnect).
    fn on_session_ended(&mut self, s: &SessionState, now: Instant, interval: Duration) {
        let Some(err) = s.error.clone() else { return };
        // The phone gave this stream to another computer: never fight over it.
        if s.bye.as_deref() == Some(crate::protocol::BYE_REPLACED) {
            self.reconnect = None;
            self.notice = Some(TAKEN_OVER.into());
            return;
        }
        let reason = friendly_reason(&err);
        let had_hello = s.hello_at.is_some();
        if let Some(t) = s.hello_at {
            if now.duration_since(t) < QUICK_DROP {
                self.quick_drops += 1;
            } else {
                self.quick_drops = 0;
            }
        }
        if self.quick_drops >= MAX_QUICK_DROPS {
            self.reconnect = None;
            self.notice = Some(format!(
                "Stopped reconnecting to {}: the connection dropped right after connecting {} times in a row ({reason}).",
                s.addr, self.quick_drops
            ));
            return;
        }
        if self.reconnect.is_none() && !had_hello {
            // A connect that never succeeded: report it and stay idle.
            self.notice = Some(format!("Couldn't connect to {}: {reason}", s.addr));
            return;
        }
        let name = s.hello.as_ref().map(|h| h.name.clone()).filter(|n| !n.is_empty());
        let r = self.reconnect.get_or_insert_with(|| Reconnect {
            addr: s.addr.clone(),
            name: String::new(),
            next: now,
            attempts: 0,
            last_error: String::new(),
        });
        if let Some(n) = name {
            r.name = n;
        }
        if !had_hello {
            r.attempts += 1;
        }
        r.last_error = reason;
        r.next = now + interval;
    }

    fn select(&mut self, i: usize) {
        if let Some(s) = self.servers.get(i) {
            self.selected = Some(i);
            self.host = s.host.clone();
            self.port = s.port.to_string();
        }
    }

    /// Draw the whole UI into `ui`.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        let p = palette(ui.visuals().dark_mode);
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg).inner_margin(Margin::symmetric(24, 20)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_max_width(ui.available_width());
                    self.header(ui, &p);
                    ui.add_space(14.0);
                    self.phones_card(ui, &p, &ctx);
                    ui.add_space(12.0);
                    self.connect_section(ui, &p, &ctx);
                    ui.add_space(12.0);
                    self.status_card(ui, &p);
                    ui.add_space(4.0);
                });
            });
    }

    fn header(&self, ui: &mut egui::Ui, p: &Palette) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(44.0), Sense::hover());
            paint_logo(ui.painter(), rect.center(), 22.0);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new("Vibe Phone Mic").heading().strong().color(p.text));
                ui.label(RichText::new("Use your phone as a microphone").color(p.weak));
            });
        });
    }

    fn phones_card(&mut self, ui: &mut egui::Ui, p: &Palette, ctx: &egui::Context) {
        card(ui, p, |ui| {
            ui.horizontal(|ui| {
                section_title(ui, p, "Phones on your network");
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if self.scanning {
                        ui.add(egui::Spinner::new().size(16.0).color(p.weak));
                        ui.label(RichText::new("Scanning…").color(p.weak));
                    } else if ui.button("Rescan").clicked() {
                        self.start_scan(ctx);
                    }
                });
            });
            ui.add_space(2.0);
            if self.servers.is_empty() {
                let msg = if self.scanning {
                    "Looking for phones…"
                } else if !self.scanned_once {
                    "Press Rescan to search your network."
                } else {
                    "No phones found. Open Vibe Phone Mic on your phone (same Wi-Fi), or enter its address below. For USB, run: adb forward tcp:48777 tcp:48777"
                };
                ui.label(RichText::new(msg).color(p.weak));
            }
            let mut clicked = None;
            let mut dbl = None;
            for (i, s) in self.servers.iter().enumerate() {
                let selected = self.selected == Some(i);
                let w = ui.available_width();
                let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 52.0), Sense::click());
                let fill = if selected {
                    p.bg
                } else if resp.hovered() {
                    p.selected
                } else {
                    Color32::TRANSPARENT
                };
                let stroke = if selected { Stroke::new(1.5, p.text) } else { Stroke::new(1.0, p.border) };
                ui.painter().rect(rect, CornerRadius::same(10), fill, stroke, StrokeKind::Inside);
                let name = if s.name.is_empty() { "Phone" } else { s.name.as_str() };
                let how = match s.source.as_str() {
                    "localhost" => "USB / this computer",
                    "udp" => "Wi-Fi",
                    _ => "Wi-Fi (scan)",
                };
                ui.painter().text(
                    rect.left_top() + Vec2::new(14.0, 9.0),
                    Align2::LEFT_TOP,
                    name,
                    FontId::new(15.0, FontFamily::Proportional),
                    p.text,
                );
                ui.painter().text(
                    rect.left_bottom() + Vec2::new(14.0, -9.0),
                    Align2::LEFT_BOTTOM,
                    format!("{}  ·  {how}", s.addr()),
                    FontId::new(12.5, FontFamily::Proportional),
                    p.weak,
                );
                if resp.clicked() {
                    clicked = Some(i);
                }
                if resp.double_clicked() {
                    dbl = Some(i);
                }
            }
            if let Some(i) = clicked {
                self.select(i);
            }
            if let Some(i) = dbl {
                self.select(i);
                self.connect(ctx);
            }
        });
    }

    fn connect_section(&mut self, ui: &mut egui::Ui, p: &Palette, ctx: &egui::Context) {
        let active = self.session.is_some() || self.reconnect.is_some();
        card(ui, p, |ui| {
            section_title(ui, p, "Phone address");
            ui.horizontal(|ui| {
                let port_w = 90.0;
                let host_w = (ui.available_width() - port_w - ui.spacing().item_spacing.x).max(80.0);
                let host = ui.add_enabled(
                    !active,
                    egui::TextEdit::singleline(&mut self.host)
                        .hint_text("IP address, e.g. 192.168.1.23")
                        .desired_width(host_w)
                        .margin(Margin::symmetric(10, 8)),
                );
                let port = ui.add_enabled(
                    !active,
                    egui::TextEdit::singleline(&mut self.port)
                        .hint_text("Port")
                        .desired_width(port_w)
                        .margin(Margin::symmetric(10, 8)),
                );
                if host.changed() || port.changed() {
                    self.selected = None;
                    self.input_error = None;
                }
                if (host.lost_focus() || port.lost_focus()) && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !active {
                    self.connect(ctx);
                }
            });
            if let Some(e) = &self.input_error {
                ui.label(RichText::new(e).color(p.text).strong());
            }
            ui.add_space(4.0);
            if active {
                if secondary_button(ui, p, "Disconnect").clicked() {
                    self.disconnect();
                }
            } else {
                let can = !self.host.trim().is_empty();
                if gradient_button(ui, "Connect", can).clicked() {
                    self.connect(ctx);
                }
            }
        });
    }

    /// Virtual-mic hint / status line (shown in every state).
    fn mic_hint(&self, ui: &mut egui::Ui, p: &Palette) {
        ui.add_space(4.0);
        if self.mic_active() {
            egui::Frame::new()
                .fill(p.bg)
                .stroke(Stroke::new(1.0, p.border))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(12, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(RichText::new("In your apps, select").color(p.weak));
                        ui.label(RichText::new("Vibe Phone Mic").strong().color(p.text));
                        ui.label(RichText::new("as the microphone.").color(p.weak));
                    });
                });
        } else if let Some(e) = &self.mic_error {
            let msg = if e.starts_with(crate::audio::IN_USE) {
                "Vibe Phone Mic is already running in another window, which owns the virtual microphone. \
                 This window can connect, but its audio won't reach the mic."
                    .to_string()
            } else {
                format!("Virtual microphone unavailable: {e}")
            };
            ui.label(RichText::new(msg).color(p.text));
        } else if !self.opts.virtual_mic {
            ui.label(RichText::new("Virtual microphone disabled (--no-virtual-mic).").color(p.weak));
        }
    }

    fn status_card(&self, ui: &mut egui::Ui, p: &Palette) {
        let st = self.last_state.as_ref();
        let phase = st.map(|s| s.phase).unwrap_or(Phase::Disconnected);
        let reconnecting = self.reconnect.as_ref().filter(|_| phase != Phase::Streaming);
        card(ui, p, |ui| {
            section_title(ui, p, "Status");
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
                match phase {
                    Phase::Streaming => paint_gradient_circle(ui.painter(), rect.center(), 7.5),
                    _ if reconnecting.is_some() => {
                        ui.painter().circle_stroke(rect.center(), 5.5, Stroke::new(2.0, p.weak));
                    }
                    Phase::Connecting | Phase::Handshaking => {
                        ui.painter().circle_stroke(rect.center(), 5.5, Stroke::new(2.0, p.weak));
                    }
                    Phase::Disconnected => {
                        ui.painter().circle_filled(rect.center(), 6.0, p.track);
                    }
                }
                let text = match (phase, st, reconnecting) {
                    (Phase::Streaming, Some(s), _) => {
                        let name = s.hello.as_ref().map(|h| h.name.as_str()).filter(|n| !n.is_empty()).unwrap_or("phone");
                        format!("Connected to {name}")
                    }
                    (_, _, Some(r)) => {
                        let who = if r.name.is_empty() { r.addr.as_str() } else { r.name.as_str() };
                        format!("Reconnecting to {who}…")
                    }
                    (Phase::Connecting | Phase::Handshaking, Some(s), _) => format!("Connecting to {}…", s.addr),
                    _ => "Not connected".to_string(),
                };
                ui.label(RichText::new(text).size(16.0).strong().color(p.text));
            });
            if let Some(r) = reconnecting {
                ui.label(RichText::new(format!("Connection lost: {}", r.last_error)).color(p.weak));
                let secs = self.reconnect_interval().as_secs_f32();
                let mut line = format!("Retrying {} every {secs:.0} s", r.addr);
                if r.attempts > 0 {
                    line.push_str(&format!(" · {} failed attempt{}", r.attempts, if r.attempts == 1 { "" } else { "s" }));
                }
                ui.label(RichText::new(line).color(p.weak));
                ui.label(RichText::new("Press Disconnect to stop.").color(p.weak));
                self.mic_hint(ui, p);
                return;
            }
            if phase != Phase::Streaming {
                if let Some(n) = self.notice.as_ref().filter(|_| self.session.is_none()) {
                    ui.label(RichText::new(n).color(p.text));
                }
                ui.label(RichText::new("Pick a phone above or enter its IP address.").color(p.weak));
                self.mic_hint(ui, p);
                return;
            }
            let s = st.unwrap();
            ui.spacing_mut().interact_size.y = 22.0;
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.add_space(2.0);
            kv_row(ui, p, "Address", RichText::new(&s.addr).color(p.text));
            if let Some(h) = &s.hello {
                let ch = match h.channels {
                    1 => "mono".to_string(),
                    2 => "stereo".to_string(),
                    n => format!("{n} ch"),
                };
                kv_row(ui, p, "Audio", RichText::new(format!("{:.1} kHz · {ch} · 16-bit", h.sample_rate as f32 / 1000.0)).color(p.text));
            }
            let status = s.status.clone().unwrap_or_default();
            ui.horizontal(|ui| {
                key_label(ui, p, "Phone mic");
                let (r, _) = ui.allocate_exact_size(Vec2::new(14.0, 18.0), Sense::hover());
                paint_mic_glyph(ui.painter(), r.center(), 16.0, p.text, status.muted);
                let text = if status.muted { "Muted on phone" } else { "Live" };
                ui.label(RichText::new(text).strong().color(p.text));
            });
            if !status.input.is_empty() {
                kv_row(ui, p, "Input", RichText::new(&status.input).color(p.text));
            }
            ui.horizontal(|ui| {
                key_label(ui, p, "Phone volume");
                ui.label(RichText::new(format!("{} %", status.volume)).color(p.text));
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.add_space(8.0);
                    bar(ui, p, status.volume as f32 / 200.0, false, 6.0);
                });
            });
            ui.add_space(4.0);
            if status.muted {
                ui.label(RichText::new("Input level · muted on phone").color(p.weak));
                let w = ui.available_width();
                let (rect, _) = ui.allocate_exact_size(Vec2::new(w, 10.0), Sense::hover());
                ui.painter().rect_stroke(rect, CornerRadius::same(5), Stroke::new(1.0, p.track), StrokeKind::Inside);
            } else {
                ui.label(RichText::new("Input level").color(p.weak));
                bar(ui, p, self.level, true, 10.0);
            }
            ui.add_space(8.0);
            self.mic_hint(ui, p);
        });
    }

}

/// Map RMS (0..1) to a meter fraction using a -60..0 dBFS scale.
pub fn level_to_meter(rms: f32) -> f32 {
    if rms <= 0.0 {
        return 0.0;
    }
    ((20.0 * rms.log10() + 60.0) / 60.0).clamp(0.0, 1.0)
}

pub fn save_png(path: &std::path::Path, image: &egui::ColorImage) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let f = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(f), image.size[0] as u32, image.size[1] as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut w = enc.write_header().map_err(|e| e.to_string())?;
    let data: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
    w.write_image_data(&data).map_err(|e| e.to_string())
}

impl eframe::App for VpmApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }

    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        palette(visuals.dark_mode).bg.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown();
    }
}

/// Run the native GUI window.
pub fn run(opts: GuiOptions, theme: Option<Theme>) -> Result<(), String> {
    let icon = icon_image(128);
    let icon = egui::IconData {
        rgba: icon.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect(),
        width: icon.size[0] as u32,
        height: icon.size[1] as u32,
    };
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_icon(Arc::new(icon))
            .with_title("Vibe Phone Mic")
            .with_app_id("vibe-phone-mic")
            .with_inner_size([480.0, 900.0])
            .with_min_inner_size([380.0, 520.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Vibe Phone Mic",
        native,
        Box::new(move |cc| {
            if let Some(t) = theme {
                cc.egui_ctx.set_theme(t);
            }
            let ctx = cc.egui_ctx.clone();
            let _ = ctrlc::set_handler(move || {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                ctx.request_repaint();
            });
            Ok(Box::new(VpmApp::new(&cc.egui_ctx, opts)))
        }),
    )
    .map_err(|e| e.to_string())
}

/// Render the app fully offscreen (no window/GPU/display) to a PNG.
/// The UI logic runs for `settle` first so scans/connections can progress.
pub fn render_offscreen(
    opts: GuiOptions,
    theme: Theme,
    path: &std::path::Path,
    settle: Duration,
    size_pts: [f32; 2],
    ppp: f32,
) -> Result<(), String> {
    let ctx = egui::Context::default();
    ctx.set_theme(theme);
    let mut app = VpmApp::new(&ctx, opts);
    let mut r = crate::render::SoftRenderer::new();
    let clear = palette(theme == Theme::Dark).bg;
    let start = Instant::now();
    while start.elapsed() < settle {
        crate::render::render_ui(&ctx, &mut r, size_pts, ppp, clear, false, |ui| app.draw(ui));
        std::thread::sleep(Duration::from_millis(30));
    }
    let img = crate::render::render_ui(&ctx, &mut r, size_pts, ppp, clear, true, |ui| app.draw(ui)).unwrap();
    app.shutdown();
    save_png(path, &img)
}

/// Render the app icon (gradient disc + mic glyph) to a square PNG.
pub fn render_icon(path: &std::path::Path, size: u32) -> Result<(), String> {
    save_png(path, &icon_image(size))
}

/// The app icon as an image (unpremultiplied conversion happens on save).
pub fn icon_image(size: u32) -> egui::ColorImage {
    let ctx = egui::Context::default();
    let mut r = crate::render::SoftRenderer::new();
    let pts = 64.0;
    let ppp = size as f32 / pts;
    let draw = |ui: &mut egui::Ui| paint_logo(ui.painter(), Pos2::new(pts / 2.0, pts / 2.0), pts / 2.0 - 1.0);
    crate::render::render_ui(&ctx, &mut r, [pts, pts], ppp, Color32::TRANSPARENT, false, draw);
    crate::render::render_ui(&ctx, &mut r, [pts, pts], ppp, Color32::TRANSPARENT, true, draw).unwrap()
}

impl Drop for VpmApp {
    fn drop(&mut self) {
        self.shutdown();
    }
}
