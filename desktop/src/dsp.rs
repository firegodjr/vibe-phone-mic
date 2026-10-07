//! Small signal helpers (level metering and tone detection).

/// RMS of s16 samples, normalised to 0.0..=1.0.
pub fn rms(samples: &[i16]) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum: f64 = samples.iter().map(|&s| (s as f64 / 32768.0).powi(2)).sum();
    (sum / samples.len() as f64).sqrt()
}

/// Peak absolute value, normalised to 0.0..=1.0.
pub fn peak(samples: &[i16]) -> f64 {
    samples.iter().map(|&s| (s as i32).unsigned_abs()).max().unwrap_or(0) as f64 / 32768.0
}

/// Goertzel power of `freq` in mono `samples`.
pub fn goertzel_power(samples: &[f64], sample_rate: u32, freq: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * freq / sample_rate as f64;
    let coeff = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &x in samples {
        let s0 = x + coeff * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    s1 * s1 + s2 * s2 - coeff * s1 * s2
}

/// Downmix interleaved s16 to mono f64.
pub fn to_mono(samples: &[i16], channels: u16) -> Vec<f64> {
    let ch = channels.max(1) as usize;
    samples
        .chunks_exact(ch)
        .map(|c| c.iter().map(|&s| s as f64 / 32768.0).sum::<f64>() / ch as f64)
        .collect()
}

/// Hann-windowed copy of `x` (reduces spectral leakage).
fn hann(x: &[f64]) -> Vec<f64> {
    let n = x.len();
    if n < 2 {
        return x.to_vec();
    }
    x.iter()
        .enumerate()
        .map(|(i, v)| v * (0.5 - 0.5 * (2.0 * std::f64::consts::PI * i as f64 / (n - 1) as f64).cos()))
        .collect()
}

fn best_in(win: &[f64], rate: u32, lo: f64, hi: f64, step: f64) -> (f64, f64) {
    let (mut bf, mut bp) = (lo, -1.0);
    let mut f = lo;
    while f <= hi + 1e-9 {
        let p = goertzel_power(win, rate, f);
        if p > bp {
            bp = p;
            bf = f;
        }
        f += step;
    }
    (bf, bp)
}

/// Dominant frequency in 50..=8000 Hz (capped below Nyquist), accurate to well
/// under ±2 Hz for a steady tone.
///
/// Coarse-to-fine: a short window (wide main lobe) is scanned on a 10 Hz grid so
/// no tone can fall between grid points, then progressively longer windows narrow
/// the search, and a parabolic fit refines the final peak. Uses at most the last
/// second of audio.
pub fn dominant_frequency(mono: &[f64], sample_rate: u32) -> Option<f64> {
    let n = mono.len().min(sample_rate as usize);
    if n < (sample_rate as usize / 20).max(64) {
        return None;
    }
    let full = &mono[mono.len() - n..];
    let hi = 8000.0f64.min(sample_rate as f64 / 2.0 - 50.0);
    // Stage 1: 25 ms window → Hann main lobe ±80 Hz ≫ 10 Hz grid.
    let short = hann(&full[full.len() - (sample_rate as usize / 40).min(full.len())..]);
    let (f1, p1) = best_in(&short, sample_rate, 50.0, hi, 10.0);
    if p1 <= 0.0 {
        return None;
    }
    // Stage 2: 200 ms window → lobe ±10 Hz; search ±40 Hz on a 2 Hz grid.
    let mid = hann(&full[full.len() - (sample_rate as usize / 5).min(full.len())..]);
    let (f2, _) = best_in(&mid, sample_rate, (f1 - 40.0).max(1.0), f1 + 40.0, 2.0);
    // Stage 3: full window → search ±4 Hz on a 0.25 Hz grid, then parabolic refine.
    let long = hann(full);
    let step = 0.25;
    let (f3, p3) = best_in(&long, sample_rate, (f2 - 4.0).max(1.0), f2 + 4.0, step);
    let pl = goertzel_power(&long, sample_rate, f3 - step);
    let pr = goertzel_power(&long, sample_rate, f3 + step);
    let denom = pl - 2.0 * p3 + pr;
    let delta = if denom.abs() > 0.0 { (0.5 * (pl - pr) / denom).clamp(-0.5, 0.5) } else { 0.0 };
    Some(f3 + delta * step)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_sine() {
        let rate = 48000;
        let s: Vec<i16> = (0..rate)
            .map(|i| ((2.0 * std::f64::consts::PI * 440.0 * i as f64 / rate as f64).sin() * 8000.0) as i16)
            .collect();
        let f = dominant_frequency(&to_mono(&s, 1), rate).unwrap();
        assert!((f - 440.0).abs() <= 2.0, "{f}");
        let r = rms(&s);
        assert!((r - 8000.0 / 32768.0 / 2f64.sqrt()).abs() < 0.01, "{r}");
        assert_eq!(rms(&[0; 100]), 0.0);
        assert!(peak(&[-32768, 5]) == 1.0);
    }

    fn tone(rate: u32, f: f64, secs: f64) -> Vec<f64> {
        (0..(rate as f64 * secs) as usize)
            .map(|i| (2.0 * std::f64::consts::PI * f * i as f64 / rate as f64 + 0.3).sin() * 0.3)
            .collect()
    }

    #[test]
    fn finds_any_tone_50_to_8000_within_2hz() {
        let mut freqs = vec![50.0, 55.5, 100.0, 440.0, 523.25, 997.0, 1234.0, 1450.0, 3333.3, 4001.0, 6789.0, 7999.0];
        // Plus a sweep of awkward off-grid values.
        let mut f = 61.7;
        while f < 8000.0 {
            freqs.push(f);
            f *= 1.37;
        }
        for rate in [48000u32, 44100] {
            for &f in &freqs {
                let got = dominant_frequency(&tone(rate, f, 1.0), rate).unwrap();
                assert!((got - f).abs() <= 2.0, "rate {rate}: tone {f} Hz detected as {got}");
            }
        }
        // Short input (100 ms) and noisy input still work.
        let got = dominant_frequency(&tone(48000, 1234.0, 0.1), 48000).unwrap();
        assert!((got - 1234.0).abs() <= 2.0, "{got}");
        let mut x = tone(48000, 880.0, 1.0);
        let mut seed = 12345u32;
        for v in x.iter_mut() {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
            *v += ((seed >> 16) as f64 / 65536.0 - 0.5) * 0.2;
        }
        let got = dominant_frequency(&x, 48000).unwrap();
        assert!((got - 880.0).abs() <= 2.0, "noisy: {got}");
    }
}
