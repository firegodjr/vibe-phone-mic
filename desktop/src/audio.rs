//! Audio outputs: the `AudioOutput` trait, the PulseAudio/PipeWire virtual
//! microphone (`module-pipe-source` fed through a FIFO, with a jitter buffer)
//! and a WAV writer.

use libpulse_binding as pulse;
use pulse::context::{Context, FlagSet as CtxFlags, State as CtxState};
use pulse::mainloop::standard::{IterateResult, Mainloop};
use pulse::operation::{Operation, State as OpState};
use std::cell::RefCell;
use std::collections::VecDeque;
use std::path::Path;
use std::rc::Rc;
use crate::pwmic::{Jitter, JitterStats, PwSource};
use crate::resample::Converter;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

/// Name of the virtual microphone source apps should select.
pub const SOURCE_NAME: &str = "vibe_phone_mic";
/// Null sink used by versions before 0.2 (cleaned up if found).
pub const LEGACY_SINK_NAME: &str = "vibe_phone_mic_sink";
/// Human readable name of the virtual mic.
pub const SOURCE_DESCRIPTION: &str = "Vibe Phone Mic";

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

    fn server_name(&mut self) -> Result<String, String> {
        let res = Rc::new(RefCell::new(String::new()));
        let r2 = res.clone();
        let op = self.ctx.introspect().get_server_info(move |info| {
            *r2.borrow_mut() = info.server_name.as_deref().unwrap_or("").to_string();
        });
        self.wait(op)?;
        Ok(res.take())
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

/// Is this module one of ours (current pipe-source, or the null-sink/remap pair
/// of older versions)?
fn is_ours(name: &str, args: &str) -> bool {
    let has_arg = |k: &str, v: &str| args.split_whitespace().any(|a| a == format!("{k}={v}"));
    (name == "module-pipe-source" && has_arg("source_name", SOURCE_NAME))
        || (name == "module-null-sink" && has_arg("sink_name", LEGACY_SINK_NAME))
        || (name == "module-remap-source" && has_arg("source_name", SOURCE_NAME))
}

/// Unload virtual-mic modules left behind by a previous (crashed) run and
/// remove stale FIFOs. Returns the number of modules unloaded. Refuses
/// (`in-use` error) while a running instance owns the virtual mic.
///
/// On PipeWire, old pipe-source modules are left alone (unloading them can
/// crash PipeWire 1.6.8); only legacy null-sink/remap modules are removed.
pub fn cleanup_stale_modules() -> Result<usize, String> {
    let _lock = acquire_lock()?;
    let mut c = PaConn::new()?;
    if c.server_name().unwrap_or_default().contains("PipeWire") {
        let before = c.list_modules()?.iter().filter(|(_, n, a)| is_ours(n, a)).count();
        let _ = cleanup_legacy_on_pipewire(&mut c);
        let after = c.list_modules()?.iter().filter(|(_, n, a)| is_ours(n, a)).count();
        return Ok(before - after);
    }
    drop(c);
    let n = unload_our_modules()?;
    remove_stale_fifos();
    Ok(n)
}

fn unload_our_modules() -> Result<usize, String> {
    let mut c = PaConn::new()?;
    let mut mods = c.list_modules()?;
    // Legacy pair: unload the remap source before the sink it reads from.
    mods.sort_by_key(|(_, name, _)| name != "module-remap-source");
    let mut n = 0;
    for (idx, name, args) in mods {
        if is_ours(&name, &args) && c.unload_module(idx)? {
            n += 1;
        }
    }
    Ok(n)
}

fn runtime_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(std::path::PathBuf::from)
        .filter(|d| d.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

/// FIFO path for this process (the pid tags the owner).
pub fn fifo_path() -> std::path::PathBuf {
    runtime_dir().join(format!("vibe-phone-mic-{}.fifo", std::process::id()))
}

/// Remove `vibe-phone-mic-*.fifo` files (call only while holding the lock).
fn remove_stale_fifos() -> usize {
    let mine = fifo_path();
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(runtime_dir()) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with("vibe-phone-mic-") && name.ends_with(".fifo") && e.path() != mine && std::fs::remove_file(e.path()).is_ok() {
                n += 1;
            }
        }
    }
    n
}

/// The "Vibe Phone Mic" source: one `module-pipe-source` fed through a FIFO.
///
/// Unlike a null-sink + remap-source pair there is no capture stream and no
/// links, so nothing can ever fall back to the default sink's monitor (system
/// audio), and no extra sink shows up in output device lists. The module is
/// unloaded on drop (it removes its FIFO itself).
pub struct VirtualMic {
    module: u32,
    path: std::path::PathBuf,
}

impl VirtualMic {
    /// Load the module. `node.always-process=true` keeps PipeWire reading the FIFO
    /// even when no app records, so nothing stale piles up inside the source
    /// (otherwise the next app to open the mic would first hear ~0.3 s of old
    /// audio) and our writes never stall.
    ///
    /// Callers must hold the instance lock (see `MicHost`),
    /// because this first removes any existing (stale) Vibe Phone Mic modules.
    fn create(rate: u32, channels: u16) -> Result<VirtualMic, String> {
        match unload_our_modules() {
            Ok(n) if n > 0 => eprintln!("removed {n} stale virtual-mic module(s) from a previous run"),
            _ => {}
        }
        remove_stale_fifos();
        let path = fifo_path();
        let _ = std::fs::remove_file(&path);
        let mut c = PaConn::new()?;
        let args = format!(
            "source_name={SOURCE_NAME} file={} format=s16le rate={rate} channels={channels} source_properties=\"device.description='{SOURCE_DESCRIPTION}' node.always-process=true\"",
            path.display()
        );
        let module = c.load_module("module-pipe-source", &args)?;
        Ok(VirtualMic { module, path })
    }
}

impl Drop for VirtualMic {
    fn drop(&mut self) {
        if let Ok(mut c) = PaConn::new() {
            let _ = c.unload_module(self.module);
        }
        let _ = std::fs::remove_file(&self.path);
    }
}

// ---------------------------------------------------------------------------
// FIFO writer with jitter buffer

struct Ring {
    buf: VecDeque<i16>,
    closed: bool,
}

/// Writes mono 48 kHz s16le into the pipe-source FIFO from its own thread.
///
/// * Jitter buffer: after an underrun we wait until `PREBUF_MS` is queued before
///   writing again; the FIFO itself (shrunk to one page, ~42 ms) plus the queue
///   give ~60 ms of latency.
/// * The network thread only appends to the queue (never blocks); if the queue
///   grows past `MAX_QUEUE_MS` (e.g. nobody records, so the source is suspended
///   and stops reading) the oldest audio is dropped.
/// * Writes are non-blocking + poll with a timeout, so shutdown never hangs on
///   a full pipe. EPIPE (module unloaded externally) stops the writer.
/// * When nothing is written the source produces digital silence.
pub struct FifoWriter {
    shared: Arc<(Mutex<Ring>, Condvar)>,
    thread: Option<JoinHandle<()>>,
    dead: Arc<std::sync::atomic::AtomicBool>,
}

pub const PREBUF_MS: usize = 60;
pub const MAX_QUEUE_MS: usize = 150;
const SAMPLES_PER_MS: usize = 48;
/// FIFO capacity requested with F_SETPIPE_SZ (one page).
pub const FIFO_BYTES: i32 = 4096;

impl FifoWriter {
    pub fn open(path: &Path) -> Result<FifoWriter, String> {
        use std::os::unix::fs::OpenOptionsExt;
        use std::os::unix::io::AsRawFd;
        // The module holds the read end, so a non-blocking write-only open
        // succeeds immediately (and fails with ENXIO instead of hanging if not).
        let file = std::fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
            .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
        // SAFETY: plain fcntl on a valid fd we own.
        unsafe {
            libc::fcntl(file.as_raw_fd(), libc::F_SETPIPE_SZ, FIFO_BYTES);
        }
        let shared = Arc::new((Mutex::new(Ring { buf: VecDeque::new(), closed: false }), Condvar::new()));
        let dead = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (sh, dead2) = (shared.clone(), dead.clone());
        let thread = thread::Builder::new()
            .name("vpm-fifo".into())
            .spawn(move || writer_loop(file, &sh, &dead2))
            .map_err(|e| e.to_string())?;
        Ok(FifoWriter { shared, thread: Some(thread), dead })
    }

    pub fn push(&mut self, samples: &[i16]) {
        let (lock, cv) = &*self.shared;
        let mut g = lock.lock().unwrap();
        g.buf.extend(samples.iter().copied());
        let max = MAX_QUEUE_MS * SAMPLES_PER_MS;
        if g.buf.len() > max {
            let drop_n = g.buf.len() - PREBUF_MS * SAMPLES_PER_MS;
            g.buf.drain(..drop_n);
        }
        cv.notify_one();
    }

    pub fn clear(&mut self) {
        self.shared.0.lock().unwrap().buf.clear();
    }

    pub fn queued(&self) -> usize {
        self.shared.0.lock().unwrap().buf.len()
    }

    /// True if the FIFO broke (e.g. module unloaded by someone else).
    pub fn is_dead(&self) -> bool {
        self.dead.load(std::sync::atomic::Ordering::SeqCst)
    }
}

fn writer_loop(file: std::fs::File, shared: &(Mutex<Ring>, Condvar), dead: &std::sync::atomic::AtomicBool) {
    use std::os::unix::io::AsRawFd;
    let (lock, cv) = shared;
    let fd = file.as_raw_fd();
    let chunk = 10 * SAMPLES_PER_MS;
    let prebuf = PREBUF_MS * SAMPLES_PER_MS;
    let mut primed = false;
    let mut bytes: Vec<u8> = Vec::with_capacity(chunk * 2);
    loop {
        bytes.clear();
        {
            let mut g = lock.lock().unwrap();
            let need = if primed { 1 } else { prebuf };
            if g.buf.len() < need && !g.closed {
                g = cv.wait_timeout_while(g, Duration::from_millis(40), |r| r.buf.len() < need && !r.closed).unwrap().0;
            }
            if g.closed {
                return;
            }
            if g.buf.len() < need {
                // Underrun (or still buffering): the source plays silence; re-prime.
                primed = false;
                continue;
            }
            primed = true;
            let n = g.buf.len().min(chunk);
            for s in g.buf.drain(..n) {
                bytes.extend_from_slice(&s.to_le_bytes());
            }
        }
        // Write the chunk; wait for space with poll() so we can notice shutdown.
        let mut off = 0;
        let mut blocked_since: Option<std::time::Instant> = None;
        while off < bytes.len() {
            // SAFETY: writing from a valid buffer to our fd.
            let r = unsafe { libc::write(fd, bytes[off..].as_ptr() as *const libc::c_void, bytes.len() - off) };
            if r > 0 {
                off += r as usize;
                blocked_since = None;
                continue;
            }
            let err = std::io::Error::last_os_error();
            match err.raw_os_error() {
                Some(libc::EAGAIN) | Some(libc::EINTR) => {
                    if lock.lock().unwrap().closed {
                        return;
                    }
                    let since = *blocked_since.get_or_insert_with(std::time::Instant::now);
                    if since.elapsed() > Duration::from_millis(100) {
                        // Nobody is reading (source suspended): drop this chunk and
                        // the queue instead of building latency; re-prime later.
                        lock.lock().unwrap().buf.clear();
                        primed = false;
                        break;
                    }
                    let mut pfd = libc::pollfd { fd, events: libc::POLLOUT, revents: 0 };
                    // SAFETY: valid pollfd array of length 1.
                    unsafe {
                        libc::poll(&mut pfd, 1, 20);
                    }
                }
                _ => {
                    // EPIPE: the module (reader) is gone.
                    dead.store(true, std::sync::atomic::Ordering::SeqCst);
                    return;
                }
            }
        }
    }
}

impl Drop for FifoWriter {
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

// ---------------------------------------------------------------------------
/// The virtual microphone for the whole app lifetime.
///
/// On PipeWire it is a native PipeWire stream that is itself an
/// `Audio/Source` node (node.virtual=true) ("Vibe Phone Mic"): no modules are loaded, so
/// nothing can fall back to system audio, nothing has to be unloaded (unloading
/// pipe-source modules crashes PipeWire 1.6.8), and the node disappears by
/// itself if the process dies. If PipeWire goes away the host keeps retrying
/// and re-creates the node when it is back.
///
/// On a real PulseAudio server (no PipeWire) it falls back to a
/// `module-pipe-source` fed through a FIFO.
///
/// Phone audio is downmixed/resampled to 48 kHz mono here; between
/// connections the source outputs digital silence.
pub struct MicHost {
    conv: Mutex<Converter>,
    backend: Backend,
    /// Session generation currently allowed to feed/clear the mic.
    generation: std::sync::atomic::AtomicU64,
    closed: Arc<std::sync::atomic::AtomicBool>,
    lock: Mutex<Option<std::fs::File>>,
    legacy_hint: Option<String>,
}

enum Backend {
    PipeWire {
        jitter: Arc<Jitter>,
        source: Arc<Mutex<Option<PwSource>>>,
        last_error: Arc<Mutex<String>>,
        supervisor: Mutex<Option<JoinHandle<()>>>,
    },
    Pulse {
        // Field order: the writer closes before the module goes away.
        inner: Mutex<Option<(FifoWriter, VirtualMic)>>,
    },
}

/// Which audio server implementation the virtual mic uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MicBackend {
    PipeWire,
    PulseAudio,
}

/// How often the PipeWire supervisor retries after losing the daemon.
pub const PW_RETRY: Duration = Duration::from_millis(1000);

/// Path of the single-instance lock that guards the virtual mic.
pub fn lock_path() -> std::path::PathBuf {
    runtime_dir().join("vibe-phone-mic.lock")
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

/// Legacy modules (from older builds) found on a PipeWire server: null-sink /
/// remap-source are unloaded (safe); pipe-sources are NOT unloaded because
/// unloading them can crash PipeWire 1.6.8 — a hint is returned instead.
fn cleanup_legacy_on_pipewire(c: &mut PaConn) -> Option<String> {
    let mods = c.list_modules().ok()?;
    let mut pipe_sources = Vec::new();
    let mut legacy: Vec<(u32, String)> = Vec::new();
    for (idx, name, args) in mods {
        if !is_ours(&name, &args) {
            continue;
        }
        if name == "module-pipe-source" {
            pipe_sources.push(idx);
        } else {
            legacy.push((idx, name));
        }
    }
    legacy.sort_by_key(|(_, name)| name != "module-remap-source");
    let mut n = 0;
    for (idx, _) in legacy {
        if c.unload_module(idx).unwrap_or(false) {
            n += 1;
        }
    }
    if n > 0 {
        eprintln!("removed {n} legacy virtual-mic module(s) from an older version");
    }
    remove_stale_fifos();
    if pipe_sources.is_empty() {
        None
    } else {
        Some(format!(
            "An old \"Vibe Phone Mic\" pipe-source from a previous version is still loaded (module {}). \
             It is not removed automatically because unloading it can crash PipeWire; \
             restart PipeWire (systemctl --user restart pipewire pipewire-pulse) or log out and back in to remove it.",
            pipe_sources.iter().map(|i| i.to_string()).collect::<Vec<_>>().join(", ")
        ))
    }
}

impl MicHost {
    /// Create the virtual mic. Fails with an `in-use` error (and touches
    /// nothing) when another instance already owns it.
    pub fn create() -> Result<Arc<MicHost>, String> {
        let lock = acquire_lock()?;
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let jitter = Arc::new(Jitter::default());
        // Prefer native PipeWire.
        let (first, pw_err) = match PwSource::connect(SOURCE_NAME, SOURCE_DESCRIPTION, MIC_RATE, jitter.clone()) {
            Ok(s) => (Some(s), None),
            Err(e) => (None, Some(e)),
        };
        let mut pulse = PaConn::new().ok();
        let pulse_is_real_pa = first.is_none()
            && pulse.as_mut().is_some_and(|c| !c.server_name().unwrap_or_default().contains("PipeWire"));
        let mut legacy_hint = None;
        let backend = match (first, pulse) {
            (Some(src), mut pulse) => {
                if let Some(c) = pulse.as_mut() {
                    legacy_hint = cleanup_legacy_on_pipewire(c);
                }
                Self::pipewire_backend(jitter, Some(src), String::new(), &closed)
            }
            (None, Some(c)) if pulse_is_real_pa => {
                // Real PulseAudio: modules are safe to (un)load there.
                drop(c);
                let mic = VirtualMic::create(MIC_RATE, MIC_CHANNELS)?;
                let writer = FifoWriter::open(&mic.path)?;
                Backend::Pulse { inner: Mutex::new(Some((writer, mic))) }
            }
            (None, _) => {
                // PipeWire not reachable right now: keep retrying in the background.
                let err = pw_err.unwrap_or_default();
                eprintln!("virtual mic: PipeWire unavailable ({err}); will retry");
                Self::pipewire_backend(jitter, None, err, &closed)
            }
        };
        if let Some(h) = &legacy_hint {
            eprintln!("warning: {h}");
        }
        Ok(Arc::new(MicHost {
            conv: Mutex::new(Converter::new(MIC_RATE, MIC_CHANNELS, MIC_RATE)),
            backend,
            generation: Default::default(),
            closed,
            lock: Mutex::new(Some(lock)),
            legacy_hint,
        }))
    }

    fn pipewire_backend(
        jitter: Arc<Jitter>,
        first: Option<PwSource>,
        err: String,
        closed: &Arc<std::sync::atomic::AtomicBool>,
    ) -> Backend {
        let source = Arc::new(Mutex::new(first));
        let last_error = Arc::new(Mutex::new(err));
        let (src2, err2, jit2, closed2) = (source.clone(), last_error.clone(), jitter.clone(), closed.clone());
        let supervisor = thread::Builder::new()
            .name("vpm-pw-supervisor".into())
            .spawn(move || {
                use std::sync::atomic::Ordering::SeqCst;
                let mut next_try = std::time::Instant::now();
                while !closed2.load(SeqCst) {
                    let broken = src2.lock().unwrap().as_ref().map(|s| s.is_broken()).unwrap_or(true);
                    if broken {
                        let old = src2.lock().unwrap().take();
                        if let Some(old) = old {
                            *err2.lock().unwrap() = old.error();
                            eprintln!("virtual mic: lost PipeWire ({}); reconnecting", old.error());
                            drop(old);
                        }
                        if std::time::Instant::now() >= next_try {
                            match PwSource::connect(SOURCE_NAME, SOURCE_DESCRIPTION, MIC_RATE, jit2.clone()) {
                                Ok(s) if closed2.load(SeqCst) => drop(s),
                                Ok(s) => {
                                    eprintln!("virtual mic: PipeWire is back, source re-created");
                                    err2.lock().unwrap().clear();
                                    *src2.lock().unwrap() = Some(s);
                                }
                                Err(e) => {
                                    *err2.lock().unwrap() = e;
                                    next_try = std::time::Instant::now() + PW_RETRY;
                                }
                            }
                        }
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            })
            .expect("spawn supervisor");
        Backend::PipeWire { jitter, source, last_error, supervisor: Mutex::new(Some(supervisor)) }
    }

    pub fn backend(&self) -> MicBackend {
        match self.backend {
            Backend::PipeWire { .. } => MicBackend::PipeWire,
            Backend::Pulse { .. } => MicBackend::PulseAudio,
        }
    }

    /// Hint about an old pipe-source module that could not be removed safely.
    pub fn legacy_hint(&self) -> Option<&str> {
        self.legacy_hint.as_deref()
    }

    /// Why the mic is currently down (empty while active).
    pub fn last_error(&self) -> String {
        match &self.backend {
            Backend::PipeWire { last_error, .. } => last_error.lock().unwrap().clone(),
            Backend::Pulse { .. } => {
                if self.is_active() { String::new() } else { "virtual mic FIFO is closed".into() }
            }
        }
    }

    /// PipeWire node id of the source (None while down / on PulseAudio).
    pub fn node_id(&self) -> Option<u32> {
        match &self.backend {
            Backend::PipeWire { source, .. } => source.lock().unwrap().as_ref().filter(|s| !s.is_broken()).map(|s| s.node_id()),
            Backend::Pulse { .. } => None,
        }
    }

    /// Jitter-buffer statistics (PipeWire backend).
    pub fn stats(&self) -> JitterStats {
        match &self.backend {
            Backend::PipeWire { jitter, .. } => jitter.stats(),
            Backend::Pulse { .. } => JitterStats::default(),
        }
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

    /// Whether calls from this session generation are still honoured.
    pub fn accepts(&self, generation: u64) -> bool {
        self.current(generation)
    }

    /// Prepare for a stream with this format (new converter: downmix/resample to 48 kHz mono).
    pub fn set_format(&self, generation: u64, rate: u32, channels: u16) -> Result<(), String> {
        if !self.current(generation) {
            return Ok(());
        }
        if self.closed.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("virtual mic was shut down".into());
        }
        self.clear(generation);
        *self.conv.lock().unwrap() = Converter::new(rate, channels, MIC_RATE);
        Ok(())
    }

    pub fn push(&self, generation: u64, samples: &[i16]) {
        if !self.current(generation) {
            return;
        }
        let mut out = Vec::with_capacity(samples.len() * 2);
        self.conv.lock().unwrap().process(samples, &mut out);
        match &self.backend {
            Backend::PipeWire { jitter, .. } => jitter.push(&out),
            Backend::Pulse { inner } => {
                if let Some((w, _)) = inner.lock().unwrap().as_mut() {
                    w.push(&out);
                }
            }
        }
    }

    /// Drop queued audio (connection ended): the mic falls back to silence.
    pub fn clear(&self, generation: u64) {
        if !self.current(generation) {
            return;
        }
        match &self.backend {
            Backend::PipeWire { jitter, .. } => jitter.clear(),
            Backend::Pulse { inner } => {
                if let Some((w, _)) = inner.lock().unwrap().as_mut() {
                    w.clear();
                }
            }
        }
    }

    /// Samples queued for playback (diagnostics/tests).
    pub fn queued(&self) -> usize {
        match &self.backend {
            Backend::PipeWire { jitter, .. } => jitter.queued(),
            Backend::Pulse { inner } => inner.lock().unwrap().as_ref().map(|(w, _)| w.queued()).unwrap_or(0),
        }
    }

    /// True while the source exists (false while PipeWire is down or after shutdown).
    pub fn is_active(&self) -> bool {
        if self.closed.load(std::sync::atomic::Ordering::SeqCst) {
            return false;
        }
        match &self.backend {
            Backend::PipeWire { source, .. } => source.lock().unwrap().as_ref().is_some_and(|s| !s.is_broken()),
            Backend::Pulse { inner } => inner.lock().unwrap().as_ref().is_some_and(|(w, _)| !w.is_dead()),
        }
    }

    /// True once `shutdown()` ran.
    pub fn is_shut_down(&self) -> bool {
        self.closed.load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Remove the source now (even if other references exist) and release the lock.
    pub fn shutdown(&self) {
        self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
        match &self.backend {
            Backend::PipeWire { source, supervisor, .. } => {
                if let Some(h) = supervisor.lock().unwrap().take() {
                    // Bounded: the supervisor may be inside a connect attempt to
                    // a hung daemon; don't block exit for more than ~1 s.
                    let t0 = std::time::Instant::now();
                    while !h.is_finished() && t0.elapsed() < Duration::from_millis(1000) {
                        thread::sleep(Duration::from_millis(10));
                    }
                    if h.is_finished() {
                        let _ = h.join();
                    }
                }
                drop(source.lock().unwrap().take());
            }
            Backend::Pulse { inner } => drop(inner.lock().unwrap().take()),
        }
        drop(self.lock.lock().unwrap().take());
    }
}

impl Drop for MicHost {
    fn drop(&mut self) {
        self.shutdown();
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
