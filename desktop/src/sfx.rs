//! UI click sounds from github.com/joshwcomeau/button-sounds (MIT, see `../sounds/`).
//!
//! Each sound is several recordings of the same switch. A press plays a random "down" sample and the release a
//! random "up" sample (never the same sample twice in a row), so repeated clicks don't sound robotic. The clips
//! are cut by `sounds/extract.py` into `assets/sfx/*.wav` (48 kHz stereo s16) and embedded in the binary.
//!
//! Playback goes through libpulse-simple (PipeWire's pulse server or real PulseAudio) on a background thread. The
//! stream is opened on the first click and closed after a few idle seconds, so no stream sits in the mixer while
//! nothing plays. Any audio error just drops the sound: click sounds are a nice-to-have.

use eframe::egui;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// (down samples, up samples), each interleaved stereo 48 kHz s16.
pub type Clips = (Vec<Vec<i16>>, Vec<Vec<i16>>);
/// What a recorder `Sfx` logged: (sound, press).
pub type PlayLog = Arc<Mutex<Vec<(Sound, bool)>>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Sound {
    /// Small buttons: Connect / Disconnect.
    KvmSwitch,
    /// Tiny buttons: Rescan, phone list rows. Kept quiet.
    CarsonMicroflip,
}

impl Sound {
    pub const ALL: [Sound; 2] = [Sound::KvmSwitch, Sound::CarsonMicroflip];

    /// Gain applied to the recording (button-sounds recommends 25–50 % for UI sounds).
    pub fn gain(self) -> f32 {
        match self {
            Sound::KvmSwitch => 0.9, // the recording itself is quiet (peaks around -17 dBFS)
            Sound::CarsonMicroflip => 0.25,
        }
    }

    fn index(self) -> usize {
        self as usize
    }

    /// Decoded samples (embedded WAVs, decoded once).
    pub fn clips(self) -> &'static Clips {
        static BANKS: [OnceLock<Clips>; 2] = [OnceLock::new(), OnceLock::new()];
        BANKS[self.index()].get_or_init(|| {
            let (downs, ups): (&[&[u8]], &[&[u8]]) = match self {
                Sound::KvmSwitch => (&KVM_DOWN, &KVM_UP),
                Sound::CarsonMicroflip => (&CARSON_DOWN, &CARSON_UP),
            };
            (downs.iter().map(|b| decode(b)).collect(), ups.iter().map(|b| decode(b)).collect())
        })
    }
}

macro_rules! clips {
    ($name:literal, $kind:literal, $($n:literal),+) => {
        [$(include_bytes!(concat!("../assets/sfx/", $name, "_", $kind, $n, ".wav")) as &[u8]),+]
    };
}
const KVM_DOWN: [&[u8]; 5] = clips!("kvm_switch", "down", "1", "2", "3", "4", "5");
const KVM_UP: [&[u8]; 5] = clips!("kvm_switch", "up", "1", "2", "3", "4", "5");
const CARSON_DOWN: [&[u8]; 5] = clips!("carson_microflip", "down", "1", "2", "3", "4", "5");
const CARSON_UP: [&[u8]; 5] = clips!("carson_microflip", "up", "1", "2", "3", "4", "5");

pub const RATE: u32 = 48_000;
pub const CHANNELS: u16 = 2;

fn decode(wav: &[u8]) -> Vec<i16> {
    let r = hound::WavReader::new(std::io::Cursor::new(wav)).expect("embedded sfx clip is a valid WAV");
    let spec = r.spec();
    assert_eq!((spec.sample_rate, spec.channels, spec.bits_per_sample), (RATE, CHANNELS, 16), "sfx clip format");
    r.into_samples::<i16>().map(|s| s.expect("sfx sample")).collect()
}

/// Random sample index that never repeats the previous one (xorshift; no `rand` dependency).
#[derive(Debug, Clone)]
pub struct VariantPicker {
    count: usize,
    last: Option<usize>,
    state: u64,
}

impl VariantPicker {
    pub fn new(count: usize, seed: u64) -> VariantPicker {
        assert!(count > 0, "count must be > 0");
        VariantPicker { count, last: None, state: seed | 1 }
    }

    fn rand(&mut self, n: usize) -> usize {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        (self.state % n as u64) as usize
    }

    pub fn pick(&mut self) -> usize {
        let i = match self.last {
            Some(last) if self.count > 1 => {
                let r = self.rand(self.count - 1);
                if r >= last { r + 1 } else { r }
            }
            _ => self.rand(self.count),
        };
        self.last = Some(i);
        i
    }
}

enum Sink {
    Off,
    Pulse(Sender<(Sound, bool, usize)>),
    /// Tests: record what would have played.
    Record(PlayLog),
}

/// Click-sound player owned by the GUI. Methods take `&self` so they can be called while drawing.
pub struct Sfx {
    sink: Sink,
    /// [sound][0 = down, 1 = up]
    pickers: Mutex<Vec<[VariantPicker; 2]>>,
}

impl Sfx {
    fn with_sink(sink: Sink) -> Sfx {
        let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
        let pickers = Sound::ALL
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let (d, u) = s.clips();
                let seed = seed.wrapping_add(i as u64 * 0x9E37_79B9_7F4A_7C15);
                [VariantPicker::new(d.len(), seed), VariantPicker::new(u.len(), seed.rotate_left(17))]
            })
            .collect();
        Sfx { sink, pickers: Mutex::new(pickers) }
    }

    /// Silent (tests, offscreen rendering, `--no-sounds`).
    pub fn disabled() -> Sfx {
        Sfx { sink: Sink::Off, pickers: Mutex::new(Vec::new()) }
    }

    /// Plays through the desktop's PulseAudio/PipeWire server.
    pub fn pulse() -> Sfx {
        let (tx, rx) = mpsc::channel();
        let spawned = std::thread::Builder::new().name("vpm-sfx".into()).spawn(move || player_thread(rx));
        match spawned {
            Ok(_) => Sfx::with_sink(Sink::Pulse(tx)),
            Err(_) => Sfx::disabled(),
        }
    }

    /// Records (sound, press) pairs instead of playing them.
    pub fn recorder() -> (Sfx, PlayLog) {
        let log = Arc::new(Mutex::new(Vec::new()));
        (Sfx::with_sink(Sink::Record(log.clone())), log)
    }

    /// `press` = true plays a "down" sample, false an "up" sample.
    pub fn play(&self, sound: Sound, press: bool) {
        match &self.sink {
            Sink::Off => {}
            Sink::Record(log) => log.lock().unwrap().push((sound, press)),
            Sink::Pulse(tx) => {
                let i = self.pickers.lock().unwrap()[sound.index()][if press { 0 } else { 1 }].pick();
                let _ = tx.send((sound, press, i));
            }
        }
    }

    /// Call right after adding a clickable widget: plays the down sample when the pointer goes down on it and the
    /// up sample when it is released (wherever the pointer is by then). A keyboard activation (no pointer press)
    /// plays both.
    pub fn button(&self, ui: &egui::Ui, resp: &egui::Response, sound: Sound) {
        if matches!(self.sink, Sink::Off) {
            return;
        }
        let id = resp.id.with("vpm_sfx_down");
        let down = resp.is_pointer_button_down_on();
        let was = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
        if down != was {
            ui.data_mut(|d| d.insert_temp(id, down));
            self.play(sound, down);
        } else if !down && resp.clicked() {
            self.play(sound, true);
            self.play(sound, false);
        }
    }
}

/// How long the playback stream stays open after the last click.
const IDLE_CLOSE: Duration = Duration::from_secs(4);
/// A release sound starts at least this long after its press sound (a quick click still sounds like a
/// substantial two-part switch; keyboard activation plays press then release this far apart).
pub const MIN_PRESS_RELEASE_GAP: Duration = Duration::from_millis(50);

/// How long to wait before a release sound requested at `now` whose press sound started at `press_at`.
pub fn release_delay(press_at: Option<Instant>, now: Instant) -> Duration {
    press_at.map_or(Duration::ZERO, |p| (p + MIN_PRESS_RELEASE_GAP).saturating_duration_since(now))
}

fn open_stream() -> Result<libpulse_simple_binding::Simple, String> {
    use libpulse_binding::def::BufferAttr;
    use libpulse_binding::sample::{Format, Spec};
    use libpulse_binding::stream::Direction;
    let spec = Spec { format: Format::S16le, channels: CHANNELS as u8, rate: RATE };
    let ms = |m: u32| m * RATE / 1000 * CHANNELS as u32 * 2;
    // Small target/prebuffer so a click starts within a few ms instead of after the default ~2 s buffer fills.
    let attr = BufferAttr { maxlength: u32::MAX, tlength: ms(30), prebuf: ms(5), minreq: u32::MAX, fragsize: u32::MAX };
    libpulse_simple_binding::Simple::new(None, "Vibe Phone Mic", Direction::Playback, None, "Button sounds", &spec, None, Some(&attr))
        .map_err(|e| e.to_string().unwrap_or_else(|| "unknown PulseAudio error".into()))
}

fn player_thread(rx: Receiver<(Sound, bool, usize)>) {
    let mut warned = false;
    // Wait for a click with no stream open.
    while let Ok(first) = rx.recv() {
        let stream = match open_stream() {
            Ok(s) => s,
            Err(e) => {
                if !warned {
                    eprintln!("button sounds unavailable: {e}");
                    warned = true;
                }
                continue;
            }
        };
        let mut next = Some(first);
        let mut press_at: [Option<Instant>; Sound::ALL.len()] = [None; Sound::ALL.len()];
        while let Some((sound, press, i)) = next {
            if press {
                press_at[sound.index()] = Some(Instant::now());
            } else {
                std::thread::sleep(release_delay(press_at[sound.index()].take(), Instant::now()));
            }
            let (downs, ups) = sound.clips();
            let clip = if press { &downs[i] } else { &ups[i] };
            let g = sound.gain();
            let bytes: Vec<u8> = clip.iter().flat_map(|&s| ((s as f32 * g) as i16).to_le_bytes()).collect();
            if stream.write(&bytes).is_err() {
                break; // server went away; reopen on the next click
            }
            next = match rx.recv_timeout(IDLE_CLOSE) {
                Ok(m) => Some(m),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picker_never_repeats_and_covers_all() {
        let mut p = VariantPicker::new(5, 12345);
        let mut seen = [false; 5];
        let mut prev = usize::MAX;
        for _ in 0..500 {
            let i = p.pick();
            assert!(i < 5);
            assert_ne!(i, prev);
            seen[i] = true;
            prev = i;
        }
        assert!(seen.iter().all(|&s| s));
    }

    #[test]
    fn release_waits_for_minimum_gap_after_press() {
        let t = Instant::now();
        assert_eq!(release_delay(None, t), Duration::ZERO);
        assert_eq!(release_delay(Some(t), t), MIN_PRESS_RELEASE_GAP);
        assert_eq!(release_delay(Some(t), t + Duration::from_millis(20)), Duration::from_millis(30));
        assert_eq!(release_delay(Some(t), t + Duration::from_millis(50)), Duration::ZERO);
        assert_eq!(release_delay(Some(t), t + Duration::from_millis(400)), Duration::ZERO);
    }

    #[test]
    fn single_sample_picker() {
        let mut p = VariantPicker::new(1, 7);
        assert!((0..5).all(|_| p.pick() == 0));
    }

    #[test]
    fn embedded_clips_decode() {
        for s in Sound::ALL {
            let (downs, ups) = s.clips();
            assert_eq!(downs.len(), 5);
            assert_eq!(ups.len(), 5);
            for c in downs.iter().chain(ups) {
                // Non-empty, whole stereo frames, at most the 790 ms sprite slot.
                assert!(!c.is_empty() && c.len() % 2 == 0 && c.len() <= (RATE as usize * 79 / 100) * 2);
                assert!(c.iter().any(|&x| x.unsigned_abs() > 1000), "clip is silent");
            }
        }
    }
}
