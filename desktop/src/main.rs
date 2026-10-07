use clap::Parser;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use vibe_phone_mic::discovery::{self, ScanConfig};
use vibe_phone_mic::session::{Phase, Session, SessionOptions};
use vibe_phone_mic::{audio, gui};

/// Vibe Phone Mic — use an Android phone as a microphone.
///
/// Without arguments the GUI starts.
#[derive(Parser, Debug)]
#[command(name = "vibe-phone-mic", version)]
struct Cli {
    /// Scan for phones and print a JSON array.
    #[arg(long)]
    scan: bool,
    /// Scan time budget in milliseconds.
    #[arg(long, default_value_t = 2000)]
    timeout_ms: u64,
    /// Run without a window (requires --connect).
    #[arg(long)]
    headless: bool,
    /// Phone address HOST[:PORT] (default port 48777).
    #[arg(long, value_name = "HOST[:PORT]")]
    connect: Option<String>,
    /// Write received audio to this WAV file.
    #[arg(long, value_name = "PATH")]
    wav: Option<PathBuf>,
    /// Stop after this many seconds (headless).
    #[arg(long, value_name = "SECS")]
    duration: Option<f64>,
    /// Don't create the PulseAudio virtual microphone.
    #[arg(long)]
    no_virtual_mic: bool,
    /// Render the GUI offscreen (no window, no display needed) to a PNG and exit.
    #[arg(long, value_name = "PATH")]
    screenshot: Option<PathBuf>,
    /// How long the UI runs (scanning/connecting) before the screenshot, in ms.
    #[arg(long, default_value_t = 2500)]
    screenshot_delay_ms: u64,
    /// Screenshot scale factor (pixels per point).
    #[arg(long, default_value_t = 2.0)]
    screenshot_scale: f32,
    /// Write the app icon PNG (used by the AppImage build) and exit.
    #[arg(long, value_name = "PATH", hide = true)]
    render_icon: Option<PathBuf>,
    /// GUI theme: system, light or dark.
    #[arg(long, default_value = "system")]
    theme: String,
    /// GUI: don't scan on startup.
    #[arg(long, hide = true)]
    no_scan: bool,
}

fn main() {
    let cli = Cli::parse();
    if let Some(p) = &cli.render_icon {
        if let Err(e) = gui::render_icon(p, 256) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return;
    }
    if cli.scan {
        let found = discovery::scan(&ScanConfig::system(Duration::from_millis(cli.timeout_ms)));
        println!("{}", serde_json::to_string(&found).unwrap());
        return;
    }
    if cli.headless {
        std::process::exit(headless(&cli));
    }
    let theme = match cli.theme.as_str() {
        "light" => Some(eframe::egui::Theme::Light),
        "dark" => Some(eframe::egui::Theme::Dark),
        _ => None,
    };
    let opts = gui::GuiOptions {
        auto_scan: !cli.no_scan,
        connect: cli.connect.clone(),
        virtual_mic: !cli.no_virtual_mic,
        ..Default::default()
    };
    if let Some(path) = &cli.screenshot {
        let t = theme.unwrap_or(eframe::egui::Theme::Light);
        let delay = Duration::from_millis(cli.screenshot_delay_ms);
        match gui::render_offscreen(opts, t, path, delay, [480.0, 900.0], cli.screenshot_scale) {
            Ok(()) => eprintln!("screenshot saved to {}", path.display()),
            Err(e) => {
                eprintln!("error: {e}");
                std::process::exit(1);
            }
        }
        return;
    }
    if let Err(e) = gui::run(opts, theme) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn headless(cli: &Cli) -> i32 {
    let Some(raw) = cli.connect.clone() else {
        eprintln!("error: --headless requires --connect HOST[:PORT]");
        return 2;
    };
    let addr = match vibe_phone_mic::client::parse_manual_address(&raw, "") {
        Ok(a) => a,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    {
        let stop = stop.clone();
        let _ = ctrlc::set_handler(move || stop.store(true, Ordering::SeqCst));
    }
    let mic = if cli.no_virtual_mic { None } else { create_mic() };
    if mic.is_some() {
        println!("VIRTUAL_MIC ready source={}", audio::SOURCE_NAME);
    }
    let opts = SessionOptions { mic: mic.clone(), wav: cli.wav.clone(), log: true, read_timeout: None };
    eprintln!("connecting to {addr} ...");
    let mut session = Session::start(&addr, opts, None);
    let start = Instant::now();
    let deadline = cli.duration.map(|d| start + Duration::from_secs_f64(d.max(0.0)));
    let mut user_stop = false;
    loop {
        std::thread::sleep(Duration::from_millis(20));
        if stop.load(Ordering::SeqCst) || deadline.is_some_and(|d| Instant::now() >= d) {
            user_stop = true;
            break;
        }
        if session.snapshot().phase == Phase::Disconnected {
            break;
        }
    }
    session.stop();
    let s = session.snapshot();
    drop(session);
    // Unload the virtual mic.
    if let Some(m) = mic {
        m.shutdown();
    }
    let hz = s.dominant_hz().map(|f| format!("{f:.1}")).unwrap_or_else(|| "none".into());
    println!(
        "SUMMARY frames={} samples={} rms={:.5} dominant_hz={} lost_frames={} muted={}",
        s.frames,
        s.samples,
        s.total_rms(),
        hz,
        s.lost_frames,
        s.status.as_ref().map(|x| x.muted).unwrap_or(false)
    );
    if s.frames == 0 {
        if let Some(e) = &s.error {
            eprintln!("error: {e}");
        }
        return 1;
    }
    if !user_stop {
        eprintln!("connection ended: {}", s.error.as_deref().unwrap_or("closed"));
    }
    0
}

/// Create the app-lifetime virtual mic (cleans up stale modules first).
/// If another instance owns it, continue without one (`--no-virtual-mic` semantics).
fn create_mic() -> Option<Arc<audio::MicHost>> {
    match audio::MicHost::create() {
        Ok(m) => Some(m),
        Err(e) => {
            println!("VIRTUAL_MIC error {e}");
            None
        }
    }
}
