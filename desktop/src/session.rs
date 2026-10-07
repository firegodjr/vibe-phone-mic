//! A streaming session: a `Client` plus the outputs (virtual mic, WAV) and shared
//! state used by both the GUI and the headless CLI.

use crate::audio::{AudioOutput, MicHost, WavOutput};
use crate::client::{Client, ClientEvent};
use crate::dsp;
use crate::protocol::{Hello, Status};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

#[derive(Debug, Clone, Default)]
pub struct SessionOptions {
    /// App-lifetime virtual mic to play audio into (`None` = no virtual mic).
    pub mic: Option<Arc<MicHost>>,
    /// Also record the received PCM to this WAV file.
    pub wav: Option<PathBuf>,
    /// Print machine-readable event lines (HELLO/STATUS/...) to stdout.
    pub log: bool,
    /// "No data" timeout; `None` = default (5 s).
    pub read_timeout: Option<std::time::Duration>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Connecting,
    /// TCP connected, waiting for HELLO.
    Handshaking,
    Streaming,
    Disconnected,
}

#[derive(Debug, Clone)]
pub struct SessionState {
    pub addr: String,
    pub phase: Phase,
    pub hello: Option<Hello>,
    pub status: Option<Status>,
    /// RMS / peak of the most recent audio frame (0..1).
    pub last_rms: f32,
    pub last_peak: f32,
    pub last_audio: Option<Instant>,
    pub frames: u64,
    pub samples: u64,
    pub sum_sq: f64,
    pub lost_frames: u64,
    pub last_seq: Option<u32>,
    /// Set when the virtual mic is up.
    pub mic_active: bool,
    pub mic_error: Option<String>,
    pub wav_error: Option<String>,
    /// Last disconnect reason (if not user-requested).
    pub error: Option<String>,
    /// When HELLO arrived (None = never connected successfully).
    pub hello_at: Option<Instant>,
    /// BYE reason sent by the phone, if any ("replaced" / "stopped").
    pub bye: Option<String>,
    /// Last ~1 s of non-silent samples (interleaved) for tone analysis.
    pub tail: VecDeque<i16>,
}

impl SessionState {
    fn new(addr: &str) -> SessionState {
        SessionState {
            addr: addr.to_string(),
            phase: Phase::Connecting,
            hello: None,
            status: None,
            last_rms: 0.0,
            last_peak: 0.0,
            last_audio: None,
            frames: 0,
            samples: 0,
            sum_sq: 0.0,
            lost_frames: 0,
            last_seq: None,
            mic_active: false,
            mic_error: None,
            wav_error: None,
            error: None,
            hello_at: None,
            bye: None,
            tail: VecDeque::new(),
        }
    }

    /// Overall RMS of everything received (0..1).
    pub fn total_rms(&self) -> f64 {
        if self.samples == 0 { 0.0 } else { (self.sum_sq / self.samples as f64).sqrt() }
    }

    /// Dominant frequency of the last second of audio.
    pub fn dominant_hz(&self) -> Option<f64> {
        let h = self.hello.as_ref()?;
        let v: Vec<i16> = self.tail.iter().copied().collect();
        if dsp::rms(&v) < 1e-4 {
            return None;
        }
        dsp::dominant_frequency(&dsp::to_mono(&v, h.channels), h.sample_rate)
    }
}

pub struct Session {
    client: Client,
    pub state: Arc<Mutex<SessionState>>,
}

type Notify = Box<dyn Fn() + Send>;

impl Session {
    /// Start connecting. `notify` is called (from the network thread) after each state change.
    pub fn start(addr: &str, opts: SessionOptions, notify: Option<Notify>) -> Session {
        let state = Arc::new(Mutex::new(SessionState::new(addr)));
        let st = state.clone();
        let mut wav: Option<WavOutput> = None;
        // Claim the virtual mic for this session; a late event from an older
        // session's thread can then no longer clear or feed it.
        let generation = opts.mic.as_ref().map(|m| m.begin()).unwrap_or(0);
        let timeout = opts.read_timeout.unwrap_or(crate::client::READ_TIMEOUT);
        let client = Client::connect_with(addr, timeout, move |ev| {
            handle(&st, &opts, generation, &mut wav, ev);
            if let Some(n) = &notify {
                n();
            }
        });
        Session { client, state }
    }

    pub fn snapshot(&self) -> SessionState {
        self.state.lock().unwrap().clone()
    }

    pub fn is_running(&self) -> bool {
        self.client.is_running()
    }

    /// Disconnect; outputs (virtual mic, WAV) are released by the client thread.
    pub fn stop(&mut self) {
        self.client.disconnect();
    }
}

fn log_line(opts: &SessionOptions, line: String) {
    if opts.log {
        println!("{line}");
    }
}

fn handle(
    st: &Mutex<SessionState>,
    opts: &SessionOptions,
    generation: u64,
    wav: &mut Option<WavOutput>,
    ev: ClientEvent,
) {
    match ev {
        ClientEvent::Connected { peer } => {
            log_line(opts, format!("CONNECTED {peer}"));
            st.lock().unwrap().phase = Phase::Handshaking;
        }
        ClientEvent::Hello(h) => {
            log_line(opts, format!("HELLO {}", serde_json::to_string(&h).unwrap_or_default()));
            let (mut mic_active, mut mic_error, mut wav_error) = (false, None, None);
            if let Some(mic) = &opts.mic {
                match mic.set_format(generation, h.sample_rate, h.channels) {
                    Ok(()) => mic_active = true,
                    Err(e) => {
                        log_line(opts, format!("VIRTUAL_MIC error {e}"));
                        mic_error = Some(e);
                    }
                }
            }
            if let Some(p) = &opts.wav {
                if let Some(mut old) = wav.take() {
                    let _ = old.finalize();
                }
                match WavOutput::create(p, h.sample_rate, h.channels) {
                    Ok(w) => *wav = Some(w),
                    Err(e) => {
                        log_line(opts, format!("WAV error {e}"));
                        wav_error = Some(e);
                    }
                }
            }
            let mut s = st.lock().unwrap();
            s.phase = Phase::Streaming;
            s.hello = Some(h);
            s.hello_at = Some(Instant::now());
            s.mic_active = mic_active;
            s.mic_error = mic_error;
            s.wav_error = wav_error;
        }
        ClientEvent::Status(status) => {
            log_line(opts, format!("STATUS {}", serde_json::to_string(&status).unwrap_or_default()));
            st.lock().unwrap().status = Some(status);
        }
        ClientEvent::Audio { seq, samples } => {
            if let Some(mic) = &opts.mic {
                mic.push(generation, &samples);
            }
            if let Some(w) = wav {
                w.push(&samples);
            }
            let mut s = st.lock().unwrap();
            if let Some(prev) = s.last_seq {
                let gap = seq.wrapping_sub(prev);
                if gap > 1 && gap < 1_000_000 {
                    s.lost_frames += (gap - 1) as u64;
                }
            }
            s.last_seq = Some(seq);
            s.frames += 1;
            s.samples += samples.len() as u64;
            s.sum_sq += samples.iter().map(|&x| (x as f64 / 32768.0).powi(2)).sum::<f64>();
            s.last_rms = dsp::rms(&samples) as f32;
            s.last_peak = dsp::peak(&samples) as f32;
            s.last_audio = Some(Instant::now());
            let cap = s.hello.as_ref().map(|h| h.sample_rate as usize * h.channels as usize).unwrap_or(48000);
            // Tone analysis uses the last second of non-silent audio (muted frames are all zeros).
            if samples.iter().any(|&x| x != 0) {
                s.tail.extend(samples.iter().copied());
                let excess = s.tail.len().saturating_sub(cap);
                s.tail.drain(..excess);
            }
        }
        ClientEvent::Bye { reason } => {
            log_line(opts, format!("BYE {}", serde_json::json!({ "reason": reason })));
            st.lock().unwrap().bye = Some(reason);
        }
        ClientEvent::Disconnected { reason, requested } => {
            // The virtual mic stays loaded (silence until the next connection).
            if let Some(mic) = &opts.mic {
                mic.clear(generation);
            }
            if let Some(mut w) = wav.take() {
                let _ = w.finalize();
            }
            log_line(opts, format!("DISCONNECTED {reason}"));
            let mut s = st.lock().unwrap();
            s.phase = Phase::Disconnected;
            s.mic_active = false;
            s.last_rms = 0.0;
            s.last_peak = 0.0;
            s.error = (!requested).then_some(reason);
        }
    }
}
