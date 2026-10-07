//! Private PipeWire instance for tests (never the user's audio daemon).
//!
//! Started via `tools/pw-private.sh` in a short directory under the user's
//! runtime dir; a watchdog stops it when the test process exits.
#![allow(dead_code)]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

pub struct PrivatePw {
    pub dir: PathBuf,
    script: PathBuf,
    pub env: Vec<(String, String)>,
}

static PW: OnceLock<Option<PrivatePw>> = OnceLock::new();
static ENV_APPLIED: OnceLock<()> = OnceLock::new();

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tools/pw-private.sh")
}

fn parse_env(out: &str) -> Vec<(String, String)> {
    out.lines()
        .filter_map(|l| l.strip_prefix("export "))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.trim_matches('"').to_string()))
        .collect()
}

impl PrivatePw {
    fn start_new() -> Option<PrivatePw> {
        let base = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("/tmp"));
        // Socket paths must stay short (< 108 bytes).
        let dir = base.join(format!("vpmt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let out = Command::new(script()).arg("start").arg(&dir).stderr(Stdio::piped()).output().ok()?;
        if !out.status.success() {
            eprintln!(
                "SKIP: cannot start a private PipeWire instance: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
            let _ = Command::new(script()).arg("stop").arg(&dir).status();
            return None;
        }
        let env = parse_env(&String::from_utf8_lossy(&out.stdout));
        // Watchdog: stop the instance when this test process exits.
        let me = std::process::id();
        let _ = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "while kill -0 {me} 2>/dev/null; do sleep 0.3; done; '{}' stop '{}'; rm -rf '{}'",
                script().display(),
                dir.display(),
                dir.display()
            ))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        Some(PrivatePw { dir, script: script(), env })
    }

    /// A command that talks to the private instance (env set explicitly).
    pub fn cmd(&self, program: &str) -> Command {
        let mut c = Command::new(program);
        c.envs(self.env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        c
    }

    pub fn pactl(&self, args: &[&str]) -> String {
        let out = self.cmd("pactl").args(args).output().unwrap();
        String::from_utf8_lossy(&out.stdout).to_string()
    }

    /// Point this process's libpipewire/libpulse at the private instance.
    /// Must run before any audio code in the process.
    pub fn apply_to_process(&self) {
        ENV_APPLIED.get_or_init(|| {
            for (k, v) in &self.env {
                // SAFETY: called at the start of every test of the binaries that use
                // it, guarded by a OnceLock, before any thread reads these variables.
                unsafe { std::env::set_var(k, v) };
            }
        });
    }

    /// Stop the daemons (simulates PipeWire going away).
    pub fn stop_daemons(&self) {
        let _ = Command::new(&self.script).arg("stop").arg(&self.dir).status();
    }

    /// Start the daemons again in the same directory.
    pub fn start_daemons(&self) {
        let out = Command::new(&self.script).arg("start").arg(&self.dir).output().unwrap();
        assert!(out.status.success(), "restart failed: {}", String::from_utf8_lossy(&out.stderr));
    }

    /// Names of sources whose name contains `pat`.
    pub fn sources_matching(&self, pat: &str) -> Vec<String> {
        self.pactl(&["list", "short", "sources"])
            .lines()
            .filter_map(|l| l.split('\t').nth(1))
            .filter(|n| n.contains(pat))
            .map(str::to_string)
            .collect()
    }

    pub fn wait_for(&self, what: &str, timeout: Duration, mut f: impl FnMut() -> bool) {
        let t = Instant::now();
        while !f() {
            assert!(t.elapsed() < timeout, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

/// The private instance for this test binary (None = skip the test).
pub fn private_pw() -> Option<&'static PrivatePw> {
    let pw = PW.get_or_init(PrivatePw::start_new).as_ref();
    if pw.is_none() {
        eprintln!("SKIP: no private PipeWire instance (see tools/pw-private.sh / tools/fetch-pw-test-deps.sh)");
    }
    pw
}
