//! Format conversion for the virtual mic: downmix to mono and resample to the
//! mic's fixed rate with a streaming windowed-sinc interpolator.

use std::f64::consts::PI;

/// Converts interleaved s16 at (`in_rate`, `channels`) to mono s16 at `out_rate`.
/// Stateful: feed consecutive chunks of one stream through the same converter.
pub struct Converter {
    in_rate: u32,
    out_rate: u32,
    channels: u16,
    /// Input step per output sample.
    step: f64,
    /// Low-pass cutoff in input-sample units (cycles/sample * 2).
    cutoff: f64,
    /// Kernel half width in input samples.
    half: usize,
    /// Mono input history (f64, -1..1).
    buf: Vec<f64>,
    /// Position of the next output sample, in input samples relative to `buf[0]`.
    pos: f64,
}

impl Converter {
    pub fn new(in_rate: u32, channels: u16, out_rate: u32) -> Converter {
        let step = in_rate as f64 / out_rate as f64;
        // Pass band up to 95 % of the lower Nyquist frequency.
        let cutoff = (out_rate as f64 / in_rate as f64).min(1.0) * 0.95;
        let half = (12.0 / cutoff).ceil() as usize;
        Converter {
            in_rate,
            out_rate,
            channels: channels.max(1),
            step,
            cutoff,
            half,
            // Pre-fill with silence so the first samples have history.
            buf: vec![0.0; half],
            pos: half as f64,
        }
    }

    pub fn is_passthrough(&self) -> bool {
        self.in_rate == self.out_rate && self.channels == 1
    }

    /// Algorithmic delay in output samples (for tests/latency estimates).
    pub fn delay(&self) -> usize {
        if self.in_rate == self.out_rate { 0 } else { (self.half as f64 / self.step).round() as usize }
    }

    fn kernel(&self, x: f64) -> f64 {
        let h = self.half as f64;
        if x.abs() >= h {
            return 0.0;
        }
        let sinc = if x == 0.0 { 1.0 } else { (PI * self.cutoff * x).sin() / (PI * self.cutoff * x) };
        // Blackman window.
        let n = (x + h) / (2.0 * h);
        let w = 0.42 - 0.5 * (2.0 * PI * n).cos() + 0.08 * (4.0 * PI * n).cos();
        self.cutoff * sinc * w
    }

    /// Convert a chunk; output may be slightly shorter/longer than the ideal ratio
    /// per chunk, but the stream as a whole keeps the exact rate.
    pub fn process(&mut self, input: &[i16], out: &mut Vec<i16>) {
        let ch = self.channels as usize;
        if self.is_passthrough() {
            out.extend_from_slice(input);
            return;
        }
        let mono = input.chunks_exact(ch).map(|f| f.iter().map(|&s| s as f64).sum::<f64>() / ch as f64 / 32768.0);
        if self.in_rate == self.out_rate {
            out.extend(mono.map(to_i16));
            return;
        }
        self.buf.extend(mono);
        let h = self.half as f64;
        // Produce outputs while the kernel's right edge is inside the buffer.
        while self.pos + h < self.buf.len() as f64 {
            let center = self.pos.floor() as isize;
            let frac = self.pos - center as f64;
            let mut acc = 0.0;
            let lo = center - self.half as isize + 1;
            let hi = center + self.half as isize;
            for k in lo..=hi {
                if k < 0 {
                    continue;
                }
                let x = (k - center) as f64 - frac;
                acc += self.buf[k as usize] * self.kernel(x);
            }
            out.push(to_i16(acc));
            self.pos += self.step;
        }
        // Drop history we no longer need (keep `half` samples before pos).
        let keep_from = (self.pos.floor() as usize).saturating_sub(self.half);
        if keep_from > 0 {
            self.buf.drain(..keep_from);
            self.pos -= keep_from as f64;
        }
    }
}

fn to_i16(v: f64) -> i16 {
    (v * 32768.0).round().clamp(-32768.0, 32767.0) as i16
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp;

    fn tone(rate: u32, ch: u16, f: f64, secs: f64, amp: f64) -> Vec<i16> {
        let n = (rate as f64 * secs) as usize;
        let mut v = Vec::with_capacity(n * ch as usize);
        for i in 0..n {
            let s = ((2.0 * PI * f * i as f64 / rate as f64).sin() * amp * 32767.0) as i16;
            for _ in 0..ch {
                v.push(s);
            }
        }
        v
    }

    /// Feed in 10 ms chunks (like the network does) and convert.
    fn convert(input: &[i16], rate: u32, ch: u16) -> Vec<i16> {
        let mut c = Converter::new(rate, ch, 48000);
        let mut out = Vec::new();
        let chunk = (rate / 100) as usize * ch as usize;
        for part in input.chunks(chunk) {
            c.process(part, &mut out);
        }
        out
    }

    /// Fraction of power NOT at `f` (THD+N), measured on the steady-state middle.
    fn impurity(x: &[i16], f: f64) -> f64 {
        let mid = &x[x.len() / 4..x.len() * 3 / 4];
        let v: Vec<f64> = mid.iter().map(|&s| s as f64 / 32768.0).collect();
        let n = v.len() as f64;
        let total: f64 = v.iter().map(|s| s * s).sum::<f64>() / n;
        // Least-squares fit of a sinusoid at f, then residual.
        let (mut a, mut b) = (0.0, 0.0);
        for (i, s) in v.iter().enumerate() {
            let w = 2.0 * PI * f * i as f64 / 48000.0;
            a += s * w.sin();
            b += s * w.cos();
        }
        a *= 2.0 / n;
        b *= 2.0 / n;
        let resid: f64 = v
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let w = 2.0 * PI * f * i as f64 / 48000.0;
                let e = s - (a * w.sin() + b * w.cos());
                e * e
            })
            .sum::<f64>()
            / n;
        resid / total
    }

    #[test]
    fn passthrough_48k_mono_is_bit_exact() {
        let x = tone(48000, 1, 440.0, 0.1, 0.5);
        assert_eq!(convert(&x, 48000, 1), x);
    }

    #[test]
    fn stereo_48k_downmixes() {
        let mut x = Vec::new();
        for i in 0..4800 {
            let s = ((2.0 * PI * 440.0 * i as f64 / 48000.0).sin() * 16000.0) as i16;
            x.push(s);
            x.push(s / 2);
        }
        let y = convert(&x, 48000, 2);
        assert_eq!(y.len(), 4800);
        assert_eq!(y[100], ((x[200] as i32 + x[201] as i32) / 2) as i16);
    }

    #[test]
    fn resamples_44100_cleanly() {
        for ch in [1u16, 2] {
            let x = tone(44100, ch, 440.0, 2.0, 0.5);
            let y = convert(&x, 44100, ch);
            // Exact overall rate (within the kernel delay).
            let ideal = 2.0 * 48000.0;
            assert!((y.len() as f64 - ideal).abs() < 200.0, "len {}", y.len());
            let mono: Vec<f64> = y.iter().map(|&s| s as f64 / 32768.0).collect();
            let f = dsp::dominant_frequency(&mono, 48000).unwrap();
            assert!((f - 440.0).abs() < 1.0, "{f}");
            let imp = impurity(&y, 440.0);
            assert!(imp < 1e-5, "THD+N too high: {imp:e} ({:.1} dB)", 10.0 * imp.log10());
            // Level preserved.
            let r = dsp::rms(&y[y.len() / 4..y.len() * 3 / 4]);
            assert!((r - 0.5 / 2f64.sqrt()).abs() < 0.01, "{r}");
        }
    }

    #[test]
    fn resamples_other_rates() {
        for (rate, f) in [(8000u32, 1000.0), (16000, 3000.0), (32000, 1234.0), (96000, 5000.0), (192000, 440.0)] {
            let x = tone(rate, 1, f, 1.5, 0.4);
            let y = convert(&x, rate, 1);
            assert!((y.len() as f64 - 72000.0).abs() < 400.0, "{rate}: len {}", y.len());
            let imp = impurity(&y, f);
            assert!(imp < 1e-4, "{rate} Hz: THD+N {imp:e}");
        }
    }
}
