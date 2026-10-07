//! Audio outputs: the `AudioOutput` trait, a jitter-buffered PulseAudio player,
//! the PulseAudio virtual microphone (null sink + remap source) and a WAV writer.

use libpulse_binding as pulse;
use libpulse_simple_binding::Simple;
use pulse::context::{Context, FlagSet as CtxFlags, State as CtxState};
use pulse::mainloop::standard::{IterateResult, Mainloop};
use pulse::operation::{Operation, State as OpState};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Name of the virtual microphone source apps should select.
pub const SOURCE_NAME: &str = "vibe_phone_mic";
/// Internal null sink that the received audio is played into.
pub const SINK_NAME: &str = "vibe_phone_mic_sink";
/// Human readable name of the virtual mic.
pub const SOURCE_DESCRIPTION: &str = "Vibe Phone Mic";
/// Total jitter buffer target (server-side buffer + our queue).
pub const JITTER_MS: u32 = 60;

/// Something that consumes received interleaved s16 PCM.
pub trait AudioOutput: Send {
    fn push(&mut self, samples: &[i16]);
}

// ---------------------------------------------------------------------------
// libpulse context helper (module management)

struct PaConn {
    ml: Mainloop,
    ctx: Context,
}

impl PaConn {
    fn new() -> Result<PaConn, String> {
        let ml = Mainloop::new().ok_or("cannot create PulseAudio mainloop")?;
        let mut ctx = Context::new(&ml, "Vibe Phone Mic").ok_or("cannot create PulseAudio context")?;
        ctx.connect(None, CtxFlags::NOAUTOSPAWN, None).map_err(|e| format!("PulseAudio connect failed: {e}"))?;
        let mut conn = PaConn { ml, ctx };
        loop {
            conn.iterate()?;
            match conn.ctx.get_state() {
                CtxState::Ready => return Ok(conn),
                CtxState::Failed | CtxState::Terminated => {
                    return Err("cannot connect to PulseAudio/PipeWire server".into());
                }
                _ => {}
            }
        }
    }

    fn iterate(&mut self) -> Result<(), String> {
        match self.ml.iterate(true) {
            IterateResult::Success(_) => Ok(()),
            IterateResult::Quit(_) => Err("PulseAudio mainloop quit".into()),
            IterateResult::Err(e) => Err(format!("PulseAudio mainloop error: {e}")),
        }
    }

    fn wait<T: ?Sized>(&mut self, op: Operation<T>) -> Result<(), String> {
        loop {
            match op.get_state() {
                OpState::Running => self.iterate()?,
                OpState::Done => return Ok(()),
                OpState::Cancelled => return Err("PulseAudio operation cancelled".into()),
            }
        }
    }

    fn load_module(&mut self, name: &str, args: &str) -> Result<u32, String> {
        let res = Rc::new(RefCell::new(None));
        let r2 = res.clone();
        let op = self.ctx.introspect().load_module(name, args, move |idx| *r2.borrow_mut() = Some(idx));
        self.wait(op)?;
        match *res.borrow() {
            Some(idx) if idx != u32::MAX => Ok(idx),
            _ => Err(format!("failed to load {name} ({args})")),
        }
    }

    fn unload_module(&mut self, idx: u32) -> Result<bool, String> {
        let res = Rc::new(RefCell::new(false));
        let r2 = res.clone();
        let op = self.ctx.introspect().unload_module(idx, move |ok| *r2.borrow_mut() = ok);
        self.wait(op)?;
        Ok(*res.borrow())
    }

    fn list_modules(&mut self) -> Result<Vec<(u32, String, String)>, String> {
        let res = Rc::new(RefCell::new(Vec::new()));
        let r2 = res.clone();
        let op = self.ctx.introspect().get_module_info_list(move |item| {
            if let pulse::callbacks::ListResult::Item(m) = item {
                r2.borrow_mut().push((
                    m.index,
                    m.name.as_deref().unwrap_or("").to_string(),
                    m.argument.as_deref().unwrap_or("").to_string(),
                ));
            }
        });
        self.wait(op)?;
        Ok(res.take())
    }
}

impl Drop for PaConn {
    fn drop(&mut self) {
        self.ctx.disconnect();
    }
}

/// True when a PulseAudio-compatible server is reachable.
pub fn pulse_available() -> bool {
    PaConn::new().is_ok()
}

fn is_ours(name: &str, args: &str) -> bool {
    (name == "module-null-sink" && args.contains(&format!("sink_name={SINK_NAME}")))
        || (name == "module-remap-source" && args.contains(&format!("source_name={SOURCE_NAME}")))
}

/// Unload virtual-mic modules left behind by a previous (crashed) run.
/// Returns the number of modules unloaded. Refuses (`in-use` error) while a
/// running instance owns the virtual mic, so it never removes a live mic.
pub fn cleanup_stale_modules() -> Result<usize, String> {
    let _lock = acquire_lock()?;
    unload_our_modules()
}

fn unload_our_modules() -> Result<usize, String> {
    let mut c = PaConn::new()?;
    let mut mods = c.list_modules()?;
    // Unload sources (remap) before the sinks they depend on.
    mods.sort_by_key(|(_, name, _)| name != "module-remap-source");
    let mut n = 0;
    for (idx, name, args) in mods {
        if is_ours(&name, &args) && c.unload_module(idx)? {
            n += 1;
        }
    }
    Ok(n)
}

/// The "Vibe Phone Mic" virtual source. Modules are unloaded on drop.
pub struct VirtualMic {
    sink_module: u32,
    source_module: u32,
}

impl VirtualMic {
    /// Load the modules. Callers must hold the instance lock (see `MicHost`),
    /// because this first removes any existing (stale) Vibe Phone Mic modules.
    fn create(rate: u32, channels: u16) -> Result<VirtualMic, String> {
        match unload_our_modules() {
            Ok(n) if n > 0 => eprintln!("removed {n} stale virtual-mic module(s) from a previous run"),
            _ => {}
        }
        let mut c = PaConn::new()?;
        let sink_args = format!(
            "sink_name={SINK_NAME} sink_properties=\"device.description='Vibe Phone Mic (internal)'\" rate={rate} channels={channels}"
        );
        let sink_module = c.load_module("module-null-sink", &sink_args)?;
        let src_args = format!(
            "master={SINK_NAME}.monitor source_name={SOURCE_NAME} source_properties=\"device.description='{SOURCE_DESCRIPTION}'\" rate={rate} channels={channels}"
        );
        match c.load_module("module-remap-source", &src_args) {
            Ok(source_module) => Ok(VirtualMic { sink_module, source_module }),
            Err(e) => {
                let _ = c.unload_module(sink_module);
                Err(e)
            }
        }
    }

    /// Sink to play audio into.
    pub fn sink_name(&self) -> &'static str {
        SINK_NAME
    }
}

impl Drop for VirtualMic {
    fn drop(&mut self) {
        if let Ok(mut c) = PaConn::new() {
            let _ = c.unload_module(self.source_module);
            let _ = c.unload_module(self.sink_module);
        }
    }
}

// ---------------------------------------------------------------------------
// Jitter-buffered playback

struct Ring {
    buf: VecDeque<i16>,
    closed: bool,
}

/// Plays PCM into a PulseAudio sink from a dedicated thread.
///
/// Jitter handling: the server buffer holds ~`JITTER_MS`; after each blocking
/// write we wait a little for network data before falling back to silence, and
/// we trim the queue if it grows too large (clock drift), keeping latency bounded.
pub struct PulsePlayer {
    shared: Arc<(Mutex<Ring>, Condvar)>,
    thread: Option<JoinHandle<()>>,
    max_queue: usize,
    trim_to: usize,
}

impl PulsePlayer {
    pub fn new(sink: Option<&str>, rate: u32, channels: u16) -> Result<PulsePlayer, String> {
        let shared = Arc::new((Mutex::new(Ring { buf: VecDeque::new(), closed: false }), Condvar::new()));
        let (tx, rx) = mpsc::channel::<Result<(), String>>();
        let sink = sink.map(str::to_string);
        let sh = shared.clone();
        let samples_per_ms = (rate as usize * channels as usize) / 1000;
        let thread = thread::Builder::new()
            .name("vpm-playback".into())
            .spawn(move || {
                let spec = pulse::sample::Spec { format: pulse::sample::Format::S16le, rate, channels: channels as u8 };
                if !spec.is_valid() {
                    let _ = tx.send(Err(format!("invalid sample spec {rate} Hz x{channels}")));
                    return;
                }
                let bytes_per_ms = samples_per_ms as u32 * 2;
                let attr = pulse::def::BufferAttr {
                    maxlength: u32::MAX,
                    tlength: bytes_per_ms * JITTER_MS,
                    prebuf: u32::MAX,
                    minreq: u32::MAX,
                    fragsize: u32::MAX,
                };
                let simple = match Simple::new(
                    None,
                    "Vibe Phone Mic",
                    pulse::stream::Direction::Playback,
                    sink.as_deref(),
                    "Phone microphone",
                    &spec,
                    None,
                    Some(&attr),
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = tx.send(Err(format!("cannot open playback stream: {e}")));
                        return;
                    }
                };
                let _ = tx.send(Ok(()));
                let chunk = (samples_per_ms * 10).max(channels as usize);
                let (lock, cv) = &*sh;
                let mut out: Vec<i16> = Vec::with_capacity(chunk);
                let mut bytes: Vec<u8> = Vec::with_capacity(chunk * 2);
                loop {
                    out.clear();
                    {
                        let mut g = lock.lock().unwrap();
                        if g.buf.len() < chunk && !g.closed {
                            g = cv.wait_timeout_while(g, Duration::from_millis(40), |r| r.buf.len() < chunk && !r.closed).unwrap().0;
                        }
                        if g.closed {
                            break;
                        }
                        let n = g.buf.len().min(chunk);
                        let n = n - n % channels as usize;
                        out.extend(g.buf.drain(..n));
                    }
                    if out.is_empty() {
                        out.resize(chunk, 0); // underrun: keep the stream fed with silence
                    }
                    bytes.clear();
                    for s in &out {
                        bytes.extend_from_slice(&s.to_le_bytes());
                    }
                    if simple.write(&bytes).is_err() {
                        break;
                    }
                }
                let _ = simple.flush();
            })
            .map_err(|e| e.to_string())?;
        match rx.recv() {
            Ok(Ok(())) => Ok(PulsePlayer {
                shared,
                thread: Some(thread),
                max_queue: samples_per_ms * 150,
                trim_to: samples_per_ms * 30,
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => Err("playback thread died".into()),
        }
    }
}

impl PulsePlayer {
    pub fn queued(&self) -> usize {
        self.shared.0.lock().unwrap().buf.len()
    }

    /// Discard queued audio.
    pub fn clear(&mut self) {
        self.shared.0.lock().unwrap().buf.clear();
    }
}

impl AudioOutput for PulsePlayer {
    fn push(&mut self, samples: &[i16]) {
        let (lock, cv) = &*self.shared;
        let mut g = lock.lock().unwrap();
        g.buf.extend(samples.iter().copied());
        if g.buf.len() > self.max_queue {
            let drop_n = g.buf.len() - self.trim_to;
            g.buf.drain(..drop_n);
        }
        cv.notify_one();
    }
}

impl Drop for PulsePlayer {
    fn drop(&mut self) {
        {
            let (lock, cv) = &*self.shared;
            lock.lock().unwrap().closed = true;
            cv.notify_all();
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The virtual microphone for the whole app lifetime.
///
/// The modules are loaded once (48 kHz mono) and stay loaded across
/// disconnects/reconnects so apps keep "Vibe Phone Mic" selected. A playback
/// stream is always running into the sink: it carries phone audio while
/// connected and silence otherwise. The stream is re-opened with the phone's
/// rate/channels on each HELLO (PulseAudio converts). Dropping the host stops
/// playback and unloads the modules.
pub struct MicHost {
    inner: Mutex<Option<MicInner>>,
    /// Session generation currently allowed to feed/clear the mic.
    generation: std::sync::atomic::AtomicU64,
}

struct MicInner {
    rate: u32,
    channels: u16,
    // Field order matters: the player must be dropped before the sink goes
    // away, and the instance lock is released only after the modules are gone.
    player: PulsePlayer,
    _mic: VirtualMic,
    _lock: std::fs::File,
}

/// Path of the single-instance lock that guards the virtual mic modules.
pub fn lock_path() -> std::path::PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|d| d.is_dir())
        .unwrap_or_else(std::env::temp_dir);
    dir.join("vibe-phone-mic.lock")
}

/// Error prefix when another instance owns the virtual mic.
pub const IN_USE: &str = "in-use";

/// Take the single-instance lock (released automatically if the process dies).
fn acquire_lock() -> Result<std::fs::File, String> {
    use std::io::{Read, Seek, Write};
    let path = lock_path();
    let mut f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    match f.try_lock() {
        Ok(()) => {
            let _ = f.set_len(0);
            let _ = f.rewind();
            let _ = write!(f, "{}", std::process::id());
            Ok(f)
        }
        Err(std::fs::TryLockError::WouldBlock) => {
            let mut pid = String::new();
            let _ = f.read_to_string(&mut pid);
            Err(format!(
                "{IN_USE}: the Vibe Phone Mic virtual microphone is owned by another running instance (pid {})",
                pid.trim()
            ))
        }
        Err(std::fs::TryLockError::Error(e)) => Err(format!("cannot lock {}: {e}", path.display())),
    }
}

impl std::fmt::Debug for MicHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MicHost")
    }
}

pub const MIC_RATE: u32 = 48000;
pub const MIC_CHANNELS: u16 = 1;

impl MicHost {
    /// Create the virtual mic. Fails with an `in-use` error (and touches no
    /// modules) when another instance already owns it.
    pub fn create() -> Result<Arc<MicHost>, String> {
        let lock = acquire_lock()?;
        let mic = VirtualMic::create(MIC_RATE, MIC_CHANNELS)?;
        let player = PulsePlayer::new(Some(mic.sink_name()), MIC_RATE, MIC_CHANNELS)?;
        let inner = MicInner { rate: MIC_RATE, channels: MIC_CHANNELS, player, _mic: mic, _lock: lock };
        Ok(Arc::new(MicHost { inner: Mutex::new(Some(inner)), generation: Default::default() }))
    }

    /// Start a new session; returns its generation. Calls with an older
    /// generation (late events of a replaced session) are ignored.
    pub fn begin(&self) -> u64 {
        let g = self.generation.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        self.clear(g);
        g
    }

    fn current(&self, generation: u64) -> bool {
        self.generation.load(std::sync::atomic::Ordering::SeqCst) == generation
    }

    /// Prepare for a stream with this format (re-opens playback if it differs).
    pub fn set_format(&self, generation: u64, rate: u32, channels: u16) -> Result<(), String> {
        if !self.current(generation) {
            return Ok(());
        }
        let mut g = self.inner.lock().unwrap();
        let Some(inner) = g.as_mut() else { return Err("virtual mic was shut down".into()) };
        if inner.rate == rate && inner.channels == channels {
            inner.player.clear();
            return Ok(());
        }
        inner.player = PulsePlayer::new(Some(SINK_NAME), rate, channels)?;
        inner.rate = rate;
        inner.channels = channels;
        Ok(())
    }

    pub fn push(&self, generation: u64, samples: &[i16]) {
        if !self.current(generation) {
            return;
        }
        if let Some(inner) = self.inner.lock().unwrap().as_mut() {
            inner.player.push(samples);
        }
    }

    /// Drop queued audio (connection ended): the mic falls back to silence.
    pub fn clear(&self, generation: u64) {
        if !self.current(generation) {
            return;
        }
        if let Some(inner) = self.inner.lock().unwrap().as_mut() {
            inner.player.clear();
        }
    }

    /// Samples queued for playback (diagnostics/tests).
    pub fn queued(&self) -> usize {
        self.inner.lock().unwrap().as_ref().map(|i| i.player.queued()).unwrap_or(0)
    }

    pub fn is_active(&self) -> bool {
        self.inner.lock().unwrap().is_some()
    }

    /// Stop playback and unload the modules now, even if other references exist.
    pub fn shutdown(&self) {
        let inner = self.inner.lock().unwrap().take();
        drop(inner);
    }
}

// ---------------------------------------------------------------------------
// WAV

pub struct WavOutput {
    writer: Option<hound::WavWriter<std::io::BufWriter<std::fs::File>>>,
}

impl WavOutput {
    pub fn create(path: &Path, rate: u32, channels: u16) -> Result<WavOutput, String> {
        let spec = hound::WavSpec { channels, sample_rate: rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let writer = hound::WavWriter::create(path, spec).map_err(|e| format!("cannot create {}: {e}", path.display()))?;
        Ok(WavOutput { writer: Some(writer) })
    }

    pub fn finalize(&mut self) -> Result<(), String> {
        match self.writer.take() {
            Some(w) => w.finalize().map_err(|e| e.to_string()),
            None => Ok(()),
        }
    }
}

impl AudioOutput for WavOutput {
    fn push(&mut self, samples: &[i16]) {
        if let Some(w) = &mut self.writer {
            for &s in samples {
                let _ = w.write_sample(s);
            }
        }
    }
}

impl Drop for WavOutput {
    fn drop(&mut self) {
        let _ = self.finalize();
    }
}
