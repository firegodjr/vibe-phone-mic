//! Lock-free single-producer/single-consumer jitter buffer for the realtime
//! PipeWire callback.
//!
//! * Producer: the network thread (`push`, `clear`). Never blocks the consumer.
//! * Consumer: the PipeWire process callback (`pull`). No locks, no allocation,
//!   no panics.
//! * While nobody pulls (no app records, so PipeWire doesn't run the callback)
//!   the producer keeps only the newest `PREBUF_MS`; the first pull after an
//!   idle gap therefore starts at ~`PREBUF_MS` latency.
//! * Latency correction (queue stayed high, e.g. after a stall) and underruns
//!   are crossfaded / faded so they never produce a discontinuity (click).

use std::sync::atomic::{AtomicBool, AtomicI16, AtomicU32, AtomicU64, Ordering::*};
use std::time::{Duration, Instant};

/// Queue target: playback (re)starts once this much is buffered.
pub const PREBUF_MS: usize = 40;
/// Soft latency control: if the queue stays above `HIGH_MS` for `HIGH_FOR`, it
/// is brought back to `PREBUF_MS` with a crossfade.
pub const HIGH_MS: usize = 2 * PREBUF_MS;
pub const HIGH_FOR: Duration = Duration::from_secs(2);
/// No pull for this long = nobody is recording. Shorter scheduling pauses
/// (e.g. the graph reconfiguring when a recorder joins) are NOT treated as idle:
/// playback just continues where it was (no skip, no click) and the latency
/// control crossfades any excess away later.
pub const IDLE: Duration = Duration::from_millis(250);
/// Crossfade / fade length.
pub const FADE: usize = 240; // 5 ms
const SAMPLES_PER_MS: usize = 48;
const CAP: usize = 1 << 15; // 32768 samples ≈ 680 ms, preallocated
const MASK: u64 = (CAP - 1) as u64;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct JitterStats {
    /// Times playback ran dry after having started (faded out).
    pub underruns: u64,
    /// Samples dropped because the ring was full.
    pub dropped: u64,
    /// Samples handed to PipeWire (including silence).
    pub pulled: u64,
    /// Samples skipped (with a crossfade) by the latency control while playing.
    pub trimmed: u64,
    /// Samples discarded while nobody was recording (keeps only the newest).
    pub idle_skipped: u64,
}

pub struct Jitter {
    buf: Box<[AtomicI16]>,
    /// Total samples written (producer-owned).
    head: AtomicU64,
    /// Total samples consumed/discarded (monotonic, advanced with fetch_max).
    tail: AtomicU64,
    epoch: Instant,
    /// Micros since `epoch` of the last pull + 1 (0 = never).
    last_pull: AtomicU64,
    // Consumer state.
    primed: AtomicBool,
    fade_in: AtomicU32,
    high_since: AtomicU64,
    // Stats.
    underruns: AtomicU64,
    dropped: AtomicU64,
    pulled: AtomicU64,
    trimmed: AtomicU64,
    idle_skipped: AtomicU64,
}

impl Default for Jitter {
    fn default() -> Self {
        Jitter {
            buf: (0..CAP).map(|_| AtomicI16::new(0)).collect(),
            head: AtomicU64::new(0),
            tail: AtomicU64::new(0),
            epoch: Instant::now(),
            last_pull: AtomicU64::new(0),
            primed: AtomicBool::new(false),
            fade_in: AtomicU32::new(0),
            high_since: AtomicU64::new(0),
            underruns: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            pulled: AtomicU64::new(0),
            trimmed: AtomicU64::new(0),
            idle_skipped: AtomicU64::new(0),
        }
    }
}

const PREBUF: u64 = (PREBUF_MS * SAMPLES_PER_MS) as u64;
const HIGH: u64 = (HIGH_MS * SAMPLES_PER_MS) as u64;

impl Jitter {
    fn now_us(&self) -> u64 {
        self.epoch.elapsed().as_micros() as u64
    }

    fn consumer_idle(&self, now: u64) -> bool {
        let last = self.last_pull.load(Acquire);
        last == 0 || now.saturating_sub(last - 1) > IDLE.as_micros() as u64
    }

    #[inline]
    fn at(&self, i: u64) -> i16 {
        self.buf[(i & MASK) as usize].load(Relaxed)
    }

    /// Producer: append samples (never blocks).
    pub fn push(&self, samples: &[i16]) {
        let h = self.head.load(Relaxed);
        let t = self.tail.load(Acquire);
        let free = CAP as u64 - (h - t).min(CAP as u64);
        let n = (samples.len() as u64).min(free);
        if n < samples.len() as u64 {
            self.dropped.fetch_add(samples.len() as u64 - n, Relaxed);
        }
        for (i, &s) in samples[..n as usize].iter().enumerate() {
            self.buf[((h + i as u64) & MASK) as usize].store(s, Relaxed);
        }
        let h2 = h + n;
        self.head.store(h2, Release);
        // Nobody records: keep only the newest PREBUF so a recorder attaching
        // later starts at normal latency.
        if self.consumer_idle(self.now_us()) && h2 - self.tail.load(Acquire) > PREBUF {
            let before = self.tail.fetch_max(h2 - PREBUF, AcqRel);
            if h2 - PREBUF > before {
                self.idle_skipped.fetch_add(h2 - PREBUF - before, Relaxed);
            }
        }
    }

    /// Producer: connection ended — drop everything (consumer fades to silence).
    pub fn clear(&self) {
        let h = self.head.load(Acquire);
        self.tail.fetch_max(h, AcqRel);
    }

    pub fn queued(&self) -> usize {
        (self.head.load(Acquire).saturating_sub(self.tail.load(Acquire))) as usize
    }

    pub fn stats(&self) -> JitterStats {
        JitterStats {
            underruns: self.underruns.load(Relaxed),
            dropped: self.dropped.load(Relaxed),
            pulled: self.pulled.load(Relaxed),
            trimmed: self.trimmed.load(Relaxed),
            idle_skipped: self.idle_skipped.load(Relaxed),
        }
    }

    /// Consumer (realtime): fill `out` completely. Lock-free, allocation-free.
    pub fn pull(&self, out: &mut [i16]) {
        let n = out.len() as u64;
        self.pulled.fetch_add(n, Relaxed);
        let now = self.now_us();
        let was_idle = self.consumer_idle(now);
        self.last_pull.store(now + 1, Release);
        let h = self.head.load(Acquire);
        if was_idle {
            // First pull after an idle gap: start from the newest PREBUF.
            self.primed.store(false, Relaxed);
            self.high_since.store(0, Relaxed);
            if h > PREBUF {
                let before = self.tail.fetch_max(h - PREBUF, AcqRel);
                if h - PREBUF > before {
                    self.idle_skipped.fetch_add(h - PREBUF - before, Relaxed);
                }
            }
        }
        let mut t = self.tail.load(Acquire);
        let avail = h.saturating_sub(t);
        if !self.primed.load(Relaxed) {
            if avail < PREBUF {
                out.fill(0);
                return;
            }
            self.primed.store(true, Relaxed);
            self.fade_in.store(FADE as u32, Relaxed);
        }

        // Latency control: queue stayed high → crossfade forward to PREBUF.
        let mut skip = 0u64;
        if avail > HIGH {
            let since = self.high_since.load(Relaxed);
            if since == 0 {
                self.high_since.store(now + 1, Relaxed);
            } else if now.saturating_sub(since - 1) >= HIGH_FOR.as_micros() as u64 {
                skip = avail - PREBUF;
                self.high_since.store(0, Relaxed);
            }
        } else {
            self.high_since.store(0, Relaxed);
        }

        if skip > 0 && avail - skip >= n {
            let f = (FADE as u64).min(n);
            for i in 0..n {
                let b = self.at(t + skip + i) as f32;
                out[i as usize] = if i < f {
                    let w = (i as f32 + 0.5) / f as f32;
                    (self.at(t + i) as f32 * (1.0 - w) + b * w).round() as i16
                } else {
                    b as i16
                };
            }
            self.trimmed.fetch_add(skip, Relaxed);
            t += skip + n;
        } else {
            let take = avail.min(n);
            for i in 0..take {
                out[i as usize] = self.at(t + i);
            }
            if take < n {
                // Underrun: fade out what we have, then silence; re-prime later.
                let f = (FADE as u64).min(take);
                for i in 0..f {
                    let idx = (take - f + i) as usize;
                    let w = 1.0 - (i as f32 + 0.5) / f as f32;
                    out[idx] = (out[idx] as f32 * w).round() as i16;
                }
                out[take as usize..].fill(0);
                self.primed.store(false, Relaxed);
                self.underruns.fetch_add(1, Relaxed);
            }
            t += take;
        }
        // Fade in after (re)starting.
        let fi = self.fade_in.load(Relaxed) as usize;
        if fi > 0 {
            let done = FADE - fi;
            let k = fi.min(out.len());
            for (j, o) in out[..k].iter_mut().enumerate() {
                let w = (done + j) as f32 / FADE as f32;
                *o = (*o as f32 * w).round() as i16;
            }
            self.fade_in.store((fi - k) as u32, Relaxed);
        }
        self.tail.fetch_max(t, AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(n: usize, start: usize) -> Vec<i16> {
        (start..start + n)
            .map(|i| ((2.0 * std::f64::consts::PI * 440.0 * i as f64 / 48000.0).sin() * 16000.0) as i16)
            .collect()
    }

    /// Max |second-order sine-predictor residual| (the verifier's click metric).
    fn max_residual(x: &[i16]) -> f64 {
        let k = 2.0 - 2.0 * (2.0 * std::f64::consts::PI * 440.0 / 48000.0).cos();
        (2..x.len())
            .map(|n| (x[n] as f64 - 2.0 * x[n - 1] as f64 + x[n - 2] as f64 + k * x[n - 1] as f64).abs())
            .fold(0.0, f64::max)
    }

    #[test]
    fn prebuffers_then_plays_and_fades_on_underrun() {
        let j = Jitter::default();
        let mut out = vec![1i16; 480];
        j.pull(&mut out); // consumer becomes active
        j.push(&tone(960, 0));
        j.pull(&mut out);
        assert!(out.iter().all(|&s| s == 0), "silence while buffering");
        j.push(&tone(960, 960));
        j.pull(&mut out); // primed: fade-in over the first 5 ms
        assert_eq!(out[0], 0);
        assert_eq!(&out[FADE..], &tone(1920, 0)[FADE..480]);
        let mut big = vec![1i16; 2000];
        j.pull(&mut big); // only 1440 left → underrun, faded out, then zeros
        assert!(big[1440..].iter().all(|&s| s == 0));
        assert!(big[1439].abs() < 200, "fade-out reaches ~0: {}", big[1439]);
        assert_eq!(j.stats().underruns, 1);
        j.clear();
        assert_eq!(j.queued(), 0);
    }

    #[test]
    fn idle_consumer_keeps_only_newest_prebuf() {
        let j = Jitter::default();
        for k in 0..100 {
            j.push(&tone(480, k * 480)); // 1 s while nobody pulls
        }
        assert_eq!(j.queued(), PREBUF as usize);
        assert!(j.stats().idle_skipped > 40000);
        assert_eq!(j.stats().dropped, 0);
    }

    #[test]
    fn latency_trim_is_crossfaded_without_click() {
        let j = Jitter::default();
        let mut out = vec![0i16; 480];
        j.pull(&mut out);
        // Stream 120 ms ahead (as after a stall), then keep pace for > HIGH_FOR.
        let mut pos = 0;
        j.push(&tone(5760, pos));
        pos += 5760;
        let mut rec = Vec::new();
        let t0 = Instant::now();
        while t0.elapsed() < HIGH_FOR + Duration::from_millis(300) {
            j.pull(&mut out);
            rec.extend_from_slice(&out);
            j.push(&tone(480, pos));
            pos += 480;
            std::thread::sleep(Duration::from_millis(5));
        }
        let st = j.stats();
        assert!(st.trimmed > 0, "latency control should have trimmed: {st:?}");
        assert!(j.queued() <= (PREBUF + 960) as usize);
        // Skip the initial fade-in; the trim itself must not click.
        let r = max_residual(&rec[FADE + 10..]);
        eprintln!("crossfade max residual {r}");
        assert!(r < 100.0, "discontinuity at trim: residual {r}");
    }
}
