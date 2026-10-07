//! Native PipeWire virtual source (see `pw_shim.c`) and the pull-model jitter
//! buffer that feeds it.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::sync::Arc;

mod ffi {
    use super::*;
    pub type FillFn = extern "C" fn(*mut c_void, *mut i16, u32);
    unsafe extern "C" {
        pub fn vpm_pw_new(
            node_name: *const c_char,
            description: *const c_char,
            rate: u32,
            latency_frames: u32,
            fill: FillFn,
            ud: *mut c_void,
            err: *mut c_char,
            errlen: usize,
        ) -> *mut c_void;
        pub fn vpm_pw_destroy(p: *mut c_void);
        pub fn vpm_pw_broken(p: *mut c_void) -> c_int;
        pub fn vpm_pw_node_id(p: *mut c_void) -> u32;
        pub fn vpm_pw_error(p: *mut c_void, buf: *mut c_char, len: usize);
    }
}

pub use crate::jitter::{Jitter, JitterStats};

/// Requested graph quantum (node.latency = 1024/48000 ≈ 21 ms). A smaller value
/// (we used 480) makes PipeWire run the WHOLE graph at a 256-sample quantum
/// while the mic is linked, which costs CPU and causes xruns (heard as clicks).
pub const NODE_LATENCY_FRAMES: u32 = 1024;

extern "C" fn fill_cb(ud: *mut c_void, dst: *mut i16, n: u32) {
    if ud.is_null() || dst.is_null() || n == 0 {
        return;
    }
    // SAFETY: `ud` is the `Jitter` kept alive by `PwSource` for the stream's
    // lifetime; `dst` points to `n` writable samples (checked in the shim).
    let jitter = unsafe { &*(ud as *const Jitter) };
    let out = unsafe { std::slice::from_raw_parts_mut(dst, n as usize) };
    // `pull` is lock-free and allocation-free; never unwind into C regardless.
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| jitter.pull(out))).is_err() {
        out.fill(0);
    }
}

/// One connected PipeWire source node. Dropping it removes the node.
pub struct PwSource {
    ptr: *mut c_void,
    _jitter: Arc<Jitter>,
}

// SAFETY: the shim is thread-safe (its own thread loop + atomics); the pointer
// is only freed in Drop.
unsafe impl Send for PwSource {}
unsafe impl Sync for PwSource {}

impl PwSource {
    pub fn connect(node_name: &str, description: &str, rate: u32, jitter: Arc<Jitter>) -> Result<PwSource, String> {
        let name = CString::new(node_name).unwrap();
        let desc = CString::new(description).unwrap();
        let mut err = vec![0 as c_char; 256];
        let ud = Arc::as_ptr(&jitter) as *mut c_void;
        // SAFETY: valid C strings and buffer; `ud` outlives the stream (held in self).
        let ptr = unsafe {
            ffi::vpm_pw_new(name.as_ptr(), desc.as_ptr(), rate, NODE_LATENCY_FRAMES, fill_cb, ud, err.as_mut_ptr(), err.len())
        };
        if ptr.is_null() {
            // SAFETY: the shim always NUL-terminates `err`.
            let msg = unsafe { CStr::from_ptr(err.as_ptr()) }.to_string_lossy().to_string();
            return Err(msg);
        }
        Ok(PwSource { ptr, _jitter: jitter })
    }

    /// True once PipeWire went away / the stream failed.
    pub fn is_broken(&self) -> bool {
        // SAFETY: ptr valid until Drop.
        unsafe { ffi::vpm_pw_broken(self.ptr) != 0 }
    }

    pub fn node_id(&self) -> u32 {
        // SAFETY: ptr valid until Drop.
        unsafe { ffi::vpm_pw_node_id(self.ptr) }
    }

    pub fn error(&self) -> String {
        let mut buf = vec![0 as c_char; 256];
        // SAFETY: ptr valid; buffer sized.
        unsafe { ffi::vpm_pw_error(self.ptr, buf.as_mut_ptr(), buf.len()) };
        unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().to_string()
    }
}

/// Upper bound for tearing a source down (a hung daemon must not block exit).
pub const DESTROY_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1000);

struct SendPtr(*mut c_void);
// SAFETY: only moved to the teardown thread, which is its sole user.
unsafe impl Send for SendPtr {}

impl Drop for PwSource {
    fn drop(&mut self) {
        let ptr = SendPtr(std::mem::replace(&mut self.ptr, std::ptr::null_mut()));
        // The jitter buffer must outlive the stream: keep a reference in the
        // teardown thread. If PipeWire is hung we give up waiting after
        // DESTROY_TIMEOUT (the thread finishes or dies with the process).
        let jitter = self._jitter.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new().name("vpm-pw-teardown".into()).spawn(move || {
            let ptr = ptr;
            // SAFETY: ptr came from vpm_pw_new and is destroyed exactly once.
            unsafe { ffi::vpm_pw_destroy(ptr.0) };
            drop(jitter);
            let _ = tx.send(());
        });
        if spawned.is_ok() {
            let _ = rx.recv_timeout(DESTROY_TIMEOUT);
        }
    }
}
