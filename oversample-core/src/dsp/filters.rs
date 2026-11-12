use crate::dsp::fft::{hann_window, plan_fft_forward, plan_fft_inverse};

/// How the multi-band EQ splits the spectrum, relative to the selected
/// `[freq_low, freq_high]` range. The boolean accessors replace the magic
/// `band_mode {<=2, >=3, >=4}` comparisons that were previously duplicated
/// across the filter functions and `harmonics_band_bounds`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum BandMode {
    /// Below + selected only — everything at/above `freq_low` uses the selected gain.
    TwoBand,
    /// Below + selected + above.
    #[default]
    ThreeBand,
    /// Below + selected + harmonics + above.
    FourBand,
}

impl BandMode {
    /// Whether the spectrum above `freq_high` gets its own "above" gain band
    /// rather than being folded into the selected band. (Was `band_mode >= 3`.)
    pub fn has_above_band(self) -> bool {
        !matches!(self, BandMode::TwoBand)
    }

    /// Whether a distinct harmonics band exists between the selected range and
    /// twice `freq_high`. (Was `band_mode >= 4`.)
    pub fn has_harmonics(self) -> bool {
        matches!(self, BandMode::FourBand)
    }
}

pub fn harmonics_band_bounds(
    freq_low: f64,
    freq_high: f64,
    band_mode: BandMode,
) -> Option<(f64, f64)> {
    if !band_mode.has_harmonics() || freq_high <= 0.0 || freq_low >= freq_high {
        return None;
    }

    let band_low = (freq_low * 2.0).max(freq_high);
    let band_high = freq_high * 2.0;
    if band_low < band_high {
        Some((band_low, band_high))
    } else {
        None
    }
}

/// Apply a multi-band EQ filter in the frequency domain using overlap-add FFT processing.
///
/// Bands are defined relative to the "selected" frequency range [freq_low, freq_high]:
/// - Below: 0 to freq_low
/// - Selected: freq_low to freq_high
/// - Harmonics: max(freq_high, freq_low*2) to freq_high*2 (only `BandMode::FourBand`)
/// - Above: everything above (`BandMode::ThreeBand` and up)
///
/// In 2-band mode (`BandMode::TwoBand`), everything at or above freq_low uses db_selected.
pub fn apply_eq_filter(
    samples: &[f32],
    sample_rate: u32,
    freq_low: f64,
    freq_high: f64,
    db_below: f64,
    db_selected: f64,
    db_harmonics: f64,
    db_above: f64,
    band_mode: BandMode,
) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }

    let fft_size = 4096;
    let hop_size = fft_size / 2;
    let len = samples.len();

    let window = hann_window(fft_size);

    // Build per-bin gain table
    let num_bins = fft_size / 2 + 1;
    let freq_per_bin = sample_rate as f64 / fft_size as f64;
    let harmonics_bounds = harmonics_band_bounds(freq_low, freq_high, band_mode);

    let gains: Vec<f32> = (0..num_bins)
        .map(|i| {
            let freq = i as f64 * freq_per_bin;
            let db = if freq < freq_low {
                db_below
            } else if freq <= freq_high {
                db_selected
            } else if !band_mode.has_above_band() {
                // 2-band: everything above uses selected
                db_selected
            } else if let Some((harmonics_lower, harmonics_upper)) = harmonics_bounds {
                if freq >= harmonics_lower && freq <= harmonics_upper {
                    db_harmonics
                } else {
                    db_above
                }
            } else {
                db_above
            };
            10.0_f64.powf(db / 20.0) as f32
        })
        .collect();

    let (fft_fwd, fft_inv) = (plan_fft_forward(fft_size), plan_fft_inverse(fft_size));

    // Overlap-add output buffer
    let mut output = vec![0.0f32; len];
    let mut window_sum = vec![0.0f32; len];

    // Pre-allocate FFT buffers once and reuse across frames
    let mut frame = fft_fwd.make_input_vec();
    let mut spectrum = fft_fwd.make_output_vec();
    let mut time_out = fft_inv.make_output_vec();

    let mut pos = 0;
    while pos < len {
        // Zero and fill frame in-place (no allocation)
        frame.fill(0.0);
        for (i, &w) in window.iter().enumerate() {
            if pos + i < len {
                frame[i] = samples[pos + i] * w;
            }
        }

        // Forward FFT
        fft_fwd
            .process(&mut frame, &mut spectrum)
            .expect("FFT forward failed");

        // Apply per-bin gains
        for (bin, gain) in gains.iter().enumerate() {
            if bin < spectrum.len() {
                spectrum[bin] *= *gain;
            }
        }

        // Inverse FFT
        fft_inv
            .process(&mut spectrum, &mut time_out)
            .expect("FFT inverse failed");

        // Normalize (realfft inverse doesn't normalize)
        let norm = 1.0 / fft_size as f32;

        // Overlap-add with window
        for i in 0..fft_size {
            if pos + i < len {
                output[pos + i] += time_out[i] * norm * window[i];
                window_sum[pos + i] += window[i] * window[i];
            }
        }

        pos += hop_size;
    }

    // Normalize by window sum to avoid amplitude changes
    for i in 0..len {
        if window_sum[i] > 1e-6 {
            output[i] /= window_sum[i];
        }
    }

    output
}

/// Fast IIR-based multi-band EQ using cascaded lowpass band-splitting.
/// Lower latency than FFT version, suitable for live parameter changes.
/// Band transitions are softer (~24 dB/octave) compared to FFT's brick-wall.
pub fn apply_eq_filter_fast(
    samples: &[f32],
    sample_rate: u32,
    freq_low: f64,
    freq_high: f64,
    db_below: f64,
    db_selected: f64,
    db_harmonics: f64,
    db_above: f64,
    band_mode: BandMode,
) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }

    let gain = |db: f64| 10.0_f64.powf(db / 20.0) as f32;
    let gain_below = gain(db_below);
    let gain_selected = gain(db_selected);
    // Two-band mode folds everything above freq_low into the selected band.
    let gain_above = if band_mode.has_above_band() {
        gain(db_above)
    } else {
        gain_selected
    };

    // Crossovers and the gain of the band above each one, in ascending order.
    // The harmonics band sits between freq_high and 2 × freq_high, with the
    // "above" gain on both sides of it.
    let mut splits: Vec<(f64, f32)> = vec![(freq_low, gain_selected), (freq_high, gain_above)];
    if let Some((h_lo, h_hi)) = harmonics_band_bounds(freq_low, freq_high, band_mode) {
        splits.push((h_lo, gain(db_harmonics)));
        splits.push((h_hi, gain_above));
    }
    let nyquist = sample_rate as f64 / 2.0;
    let mut freqs: Vec<f64> = Vec::new();
    let mut gains: Vec<f32> = vec![gain_below];
    for (f, g) in splits {
        if f <= 0.0 {
            // No band below 0 Hz: the band above starts at the bottom.
            gains[0] = g;
        } else if f < nyquist * 0.98 && freqs.last().is_none_or(|&last| f > last) {
            freqs.push(f);
            gains.push(g);
        } else if f < nyquist * 0.98 {
            // Same crossover as the last one (harmonics starting at freq_high).
            *gains.last_mut().unwrap() = g;
        }
    }

    // Linkwitz-Riley bands add back to the input level, so equal gains leave
    // the sound unchanged, and a band with the others muted stays flat
    // inside (a 4th-order Butterworth pair loses 8.6 dB mid-band in a
    // 30-50 kHz band; this loses about 2 dB there, less in wider bands).
    let bands = lr8_split(samples, sample_rate, &freqs);
    let mut output = vec![0.0f32; samples.len()];
    for (band, g) in bands.iter().zip(&gains) {
        if *g == 0.0 {
            continue;
        }
        for (o, b) in output.iter_mut().zip(band) {
            *o += b * g;
        }
    }
    output
}

/// Split a signal into three brick-wall frequency bands via overlap-add FFT.
/// Returns (below freq_low, between freq_low..=freq_high, above freq_high).
pub fn split_three_bands_fft(
    samples: &[f32],
    sample_rate: u32,
    freq_low: f64,
    freq_high: f64,
) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    use realfft::num_complex::Complex;

    let len = samples.len();
    if len == 0 {
        return (Vec::new(), Vec::new(), Vec::new());
    }

    let fft_size = 4096usize;
    let hop_size = fft_size / 2;
    let window = hann_window(fft_size);
    let num_bins = fft_size / 2 + 1;
    let freq_per_bin = sample_rate as f64 / fft_size as f64;

    let (fft_fwd, fft_inv) = (plan_fft_forward(fft_size), plan_fft_inverse(fft_size));

    let mut below = vec![0.0f32; len];
    let mut middle = vec![0.0f32; len];
    let mut above = vec![0.0f32; len];
    let mut window_sum = vec![0.0f32; len];

    let mut frame = fft_fwd.make_input_vec();
    let mut spectrum = fft_fwd.make_output_vec();
    let mut scratch = fft_fwd.make_output_vec();
    let mut time_out = fft_inv.make_output_vec();
    let norm = 1.0 / fft_size as f32;
    let zero = Complex::new(0.0f32, 0.0f32);

    let lo_bin = (freq_low / freq_per_bin).round() as usize;
    let hi_bin = (freq_high / freq_per_bin).round() as usize;

    let mut pos = 0;
    while pos < len {
        frame.fill(0.0);
        for (i, &w) in window.iter().enumerate() {
            if pos + i < len {
                frame[i] = samples[pos + i] * w;
            }
        }

        fft_fwd
            .process(&mut frame, &mut spectrum)
            .expect("FFT forward failed");

        let emit = |target: &mut [f32],
                    bin_gate: &dyn Fn(usize) -> bool,
                    scratch: &mut [Complex<f32>],
                    time_out: &mut [f32]| {
            for bin in 0..num_bins {
                scratch[bin] = if bin_gate(bin) { spectrum[bin] } else { zero };
            }
            fft_inv
                .process(scratch, time_out)
                .expect("FFT inverse failed");
            for i in 0..fft_size {
                if pos + i < len {
                    target[pos + i] += time_out[i] * norm * window[i];
                }
            }
        };

        emit(&mut below, &|bin| bin < lo_bin, &mut scratch, &mut time_out);
        emit(
            &mut middle,
            &|bin| bin >= lo_bin && bin <= hi_bin,
            &mut scratch,
            &mut time_out,
        );
        emit(&mut above, &|bin| bin > hi_bin, &mut scratch, &mut time_out);

        for i in 0..fft_size {
            if pos + i < len {
                window_sum[pos + i] += window[i] * window[i];
            }
        }

        pos += hop_size;
    }

    for i in 0..len {
        if window_sum[i] > 1e-6 {
            let inv = 1.0 / window_sum[i];
            below[i] *= inv;
            middle[i] *= inv;
            above[i] *= inv;
        }
    }

    (below, middle, above)
}

/// One RBJ biquad section in transposed direct form II.
#[derive(Clone, Copy, Debug)]
struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    z1: f32,
    z2: f32,
}

impl Biquad {
    fn new(sample_rate: f32, freq_hz: f32, q: f32, highpass: bool) -> Self {
        let w = std::f32::consts::TAU * freq_hz / sample_rate;
        let (cos_w, alpha) = (w.cos(), w.sin() / (2.0 * q));
        let (b0, b1, b2) = if highpass {
            let b1 = -(1.0 + cos_w);
            (-b1 / 2.0, b1, -b1 / 2.0)
        } else {
            let b1 = 1.0 - cos_w;
            (b1 / 2.0, b1, b1 / 2.0)
        };
        let a0 = 1.0 + alpha;
        Self {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: -2.0 * cos_w / a0,
            a2: (1.0 - alpha) / a0,
            z1: 0.0,
            z2: 0.0,
        }
    }

    fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.z1;
        self.z1 = self.b1 * x - self.a1 * y + self.z2;
        self.z2 = self.b2 * x - self.a2 * y;
        y
    }
}

/// Butterworth low- or high-pass filter of even `order`, as `order / 2`
/// biquads: flat in the passband, −3 dB at the cutoff and `6 × order` dB per
/// octave beyond it.
#[derive(Clone, Debug)]
pub struct Butterworth(Vec<Biquad>);

impl Butterworth {
    /// Q of each biquad section: 1 / (2·cos((2k − 1)·π / (2·order))).
    fn qs(order: usize) -> impl Iterator<Item = f32> {
        let n = order.max(2) / 2 * 2;
        (1..=n / 2).map(move |k| {
            let angle = (2 * k - 1) as f32 * std::f32::consts::PI / (2 * n) as f32;
            1.0 / (2.0 * angle.cos())
        })
    }

    pub fn lowpass(order: usize, sample_rate: f32, cutoff_hz: f32) -> Self {
        Self(
            Self::qs(order)
                .map(|q| Biquad::new(sample_rate, cutoff_hz, q, false))
                .collect(),
        )
    }

    pub fn highpass(order: usize, sample_rate: f32, cutoff_hz: f32) -> Self {
        Self(
            Self::qs(order)
                .map(|q| Biquad::new(sample_rate, cutoff_hz, q, true))
                .collect(),
        )
    }

    pub fn process_in_place(&mut self, buf: &mut [f32]) {
        for x in buf {
            let mut y = *x;
            for section in &mut self.0 {
                y = section.process(y);
            }
            *x = y;
        }
    }
}

/// 8th-order Linkwitz-Riley low-pass (a 4th-order Butterworth applied twice):
/// −6 dB at `fc`. With [`lr8_highpass`] at the same `fc` it sums to an
/// all-pass, so the two halves add back to the original level.
fn lr8_lowpass(x: &[f32], sample_rate: u32, fc: f64) -> Vec<f32> {
    let mut y = x.to_vec();
    for _ in 0..2 {
        Butterworth::lowpass(4, sample_rate as f32, fc as f32).process_in_place(&mut y);
    }
    y
}

/// 8th-order Linkwitz-Riley high-pass; see [`lr8_lowpass`].
fn lr8_highpass(x: &[f32], sample_rate: u32, fc: f64) -> Vec<f32> {
    let mut y = x.to_vec();
    for _ in 0..2 {
        Butterworth::highpass(4, sample_rate as f32, fc as f32).process_in_place(&mut y);
    }
    y
}

/// Split `x` into bands at the ascending crossover frequencies `splits`, with
/// 8th-order Linkwitz-Riley filters. Band `i` covers `splits[i-1]..splits[i]`.
/// Each band also passes through the all-pass of every later crossover, so
/// the bands stay in phase and add back to the original level at every
/// frequency. Each crossover slope is 48 dB per octave.
fn lr8_split(x: &[f32], sample_rate: u32, splits: &[f64]) -> Vec<Vec<f32>> {
    let mut bands = Vec::with_capacity(splits.len() + 1);
    let mut rest = x.to_vec();
    for &fc in splits {
        bands.push(lr8_lowpass(&rest, sample_rate, fc));
        rest = lr8_highpass(&rest, sample_rate, fc);
    }
    bands.push(rest);
    for i in 0..splits.len() {
        for &fc in &splits[i + 1..] {
            let low = lr8_lowpass(&bands[i], sample_rate, fc);
            let high = lr8_highpass(&bands[i], sample_rate, fc);
            bands[i] = low.iter().zip(&high).map(|(a, b)| a + b).collect();
        }
    }
    bands
}

/// Keep `freq_low`..`freq_high` Hz: a 4th-order Butterworth high-pass at
/// `freq_low` and low-pass at `freq_high`. An edge at 0 Hz or at Nyquist is
/// left open.
pub fn butterworth_bandpass(
    samples: &[f32],
    sample_rate: u32,
    freq_low: f64,
    freq_high: f64,
) -> Vec<f32> {
    let mut out = samples.to_vec();
    let sr = sample_rate as f32;
    let nyquist = sample_rate as f64 / 2.0;
    if freq_low > 0.0 && freq_low < nyquist {
        Butterworth::highpass(4, sr, freq_low as f32).process_in_place(&mut out);
    }
    if freq_high > 0.0 && freq_high < nyquist {
        // Keep the cutoff just under Nyquist, where the biquad stays stable.
        let fc = freq_high.min(nyquist * 0.98) as f32;
        Butterworth::lowpass(4, sr, fc).process_in_place(&mut out);
    }
    out
}

/// Simple single-pole IIR low-pass filter (first-order exponential moving average).
///
/// Transfer function: y[n] = alpha * x[n] + (1 - alpha) * y[n-1]
/// where alpha = dt / (rc + dt), rc = 1 / (2 * PI * cutoff_hz)
///
/// For production use, upgrade to a higher-order Butterworth or Chebyshev filter
/// for sharper rolloff.
pub fn lowpass_filter(samples: &[f32], cutoff_hz: f64, sample_rate: u32) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }

    let dt = 1.0 / sample_rate as f64;
    let rc = 1.0 / (2.0 * std::f64::consts::PI * cutoff_hz);
    let alpha = (dt / (rc + dt)) as f32;

    let mut output = Vec::with_capacity(samples.len());
    let mut prev = samples[0];
    output.push(prev);

    for &sample in &samples[1..] {
        let filtered = alpha * sample + (1.0 - alpha) * prev;
        output.push(filtered);
        prev = filtered;
    }

    output
}

/// Decimate (downsample) audio from `source_rate` to `target_rate`.
///
/// Applies an 8th-order Butterworth anti-aliasing low-pass, then takes every
/// M-th sample where M = source_rate / target_rate.
///
/// Returns the original samples unchanged if `target_rate >= source_rate`.
/// Rounds to the nearest integer decimation factor.
pub fn decimate(samples: &[f32], source_rate: u32, target_rate: u32) -> Vec<f32> {
    if target_rate >= source_rate || target_rate == 0 || samples.is_empty() {
        return samples.to_vec();
    }

    let factor = source_rate as f64 / target_rate as f64;
    let m = factor.round() as usize;
    if m <= 1 {
        return samples.to_vec();
    }

    // Anti-aliasing: 8th-order Butterworth low-pass at 45% of the rate the
    // output actually gets (source / m, which can be below target_rate), so
    // little above its Nyquist folds back and the band below stays flat.
    let out_rate = source_rate as f64 / m as f64;
    let mut filtered = samples.to_vec();
    Butterworth::lowpass(8, source_rate as f32, (0.45 * out_rate) as f32)
        .process_in_place(&mut filtered);

    // Take every M-th sample
    filtered.iter().step_by(m).copied().collect()
}

/// Compute the effective sample rate after decimation (integer-ratio rounded).
pub fn decimated_rate(source_rate: u32, target_rate: u32) -> u32 {
    if target_rate >= source_rate || target_rate == 0 {
        return source_rate;
    }
    let m = (source_rate as f64 / target_rate as f64).round() as u32;
    if m <= 1 {
        return source_rate;
    }
    source_rate / m
}

#[cfg(test)]
mod tests {

    /// Peak level (dB) of `y` after the first 10 ms.
    fn level_db(y: &[f32]) -> f64 {
        let peak = y[3840..].iter().fold(0.0f32, |p, s| p.max(s.abs()));
        20.0 * (peak as f64).log10()
    }

    fn tone(freq: f64, sr: u32, secs: f64) -> Vec<f32> {
        (0..(sr as f64 * secs) as usize)
            .map(|i| (std::f64::consts::TAU * freq * i as f64 / sr as f64).sin() as f32)
            .collect()
    }

    fn fast_eq(x: &[f32], band: (f64, f64), db: [f64; 4], mode: BandMode) -> Vec<f32> {
        apply_eq_filter_fast(x, 384_000, band.0, band.1, db[0], db[1], db[2], db[3], mode)
    }

    #[test]
    fn fast_eq_at_0_db_leaves_every_frequency_at_its_level() {
        for f in [
            5_000.0, 29_000.0, 30_000.0, 40_000.0, 50_000.0, 75_000.0, 100_000.0, 150_000.0,
        ] {
            let y = fast_eq(
                &tone(f, 384_000, 0.25),
                (30_000.0, 50_000.0),
                [0.0; 4],
                BandMode::FourBand,
            );
            let db = level_db(&y);
            assert!(db.abs() < 0.2, "{f} Hz: {db:.2} dB");
        }
    }

    #[test]
    fn fast_eq_band_alone_is_flat_inside_and_steep_outside() {
        let muted = [-120.0, 0.0, -120.0, -120.0];
        let band = (30_000.0, 50_000.0);
        let mid = level_db(&fast_eq(
            &tone(40_000.0, 384_000, 0.25),
            band,
            muted,
            BandMode::ThreeBand,
        ));
        assert!(mid > -2.5, "40 kHz in 30-50 kHz: {mid:.1} dB");
        let wide = level_db(&fast_eq(
            &tone(40_000.0, 384_000, 0.25),
            (20_000.0, 80_000.0),
            muted,
            BandMode::ThreeBand,
        ));
        assert!(wide > -0.3, "40 kHz in 20-80 kHz: {wide:.1} dB");
        for f in [15_000.0, 100_000.0] {
            let out = level_db(&fast_eq(
                &tone(f, 384_000, 0.25),
                band,
                muted,
                BandMode::ThreeBand,
            ));
            assert!(out < -40.0, "{f} Hz: {out:.1} dB");
        }
    }

    #[test]
    fn fast_eq_two_band_cut_below_has_no_bump_at_the_edge() {
        // Cutting only below 30 kHz: no boost at the crossover, -6 dB at it.
        let cut = [-120.0, 0.0, 0.0, 0.0];
        for (f, lo, hi) in [
            (30_000.0, -7.0, -5.0),
            (45_000.0, -1.0, 0.2),
            (120_000.0, -0.2, 0.2),
        ] {
            let db = level_db(&fast_eq(
                &tone(f, 384_000, 0.25),
                (30_000.0, 60_000.0),
                cut,
                BandMode::TwoBand,
            ));
            assert!((lo..hi).contains(&db), "{f} Hz: {db:.2} dB");
        }
    }

    #[test]
    fn decimate_keeps_the_band_and_blocks_aliases() {
        // 384 kHz to 48 kHz: 20 kHz stays, 40 kHz (would alias to 8 kHz) goes.
        let kept = decimate(&tone(18_000.0, 384_000, 0.25), 384_000, 48_000);
        let db = 20.0 * (kept[600..].iter().fold(0.0f32, |p, s| p.max(s.abs())) as f64).log10();
        assert!(db > -1.0, "18 kHz after decimation: {db:.1} dB");
        let alias = decimate(&tone(40_000.0, 384_000, 0.25), 384_000, 48_000);
        let db = 20.0 * (alias[600..].iter().fold(0.0f32, |p, s| p.max(s.abs())) as f64).log10();
        assert!(db < -40.0, "40 kHz folded to 8 kHz: {db:.1} dB");
    }

    fn tone_level_db(freq: f64, band: (f64, f64)) -> f64 {
        let sr = 384_000u32;
        let x: Vec<f32> = (0..sr as usize / 4)
            .map(|i| (std::f64::consts::TAU * freq * i as f64 / sr as f64).sin() as f32)
            .collect();
        let y = butterworth_bandpass(&x, sr, band.0, band.1);
        // Skip the first 10 ms while the filters settle.
        let peak = y[3840..].iter().fold(0.0f32, |p, s| p.max(s.abs()));
        20.0 * (peak as f64).log10()
    }

    #[test]
    fn bandpass_keeps_the_band_and_cuts_an_octave_away() {
        let band = (30_000.0, 50_000.0);
        let inside = tone_level_db(40_000.0, band);
        assert!(inside > -1.5, "40 kHz in a 30-50 kHz band: {inside:.1} dB");
        assert!(tone_level_db(15_000.0, band) < -20.0);
        assert!(tone_level_db(100_000.0, band) < -20.0);
        // An open edge passes everything on that side.
        assert!(tone_level_db(150_000.0, (30_000.0, 192_000.0)) > -0.5);
    }

    use super::*;

    #[test]
    fn harmonics_band_uses_doubled_low_when_it_exceeds_focus_high() {
        assert_eq!(
            harmonics_band_bounds(20_000.0, 30_000.0, BandMode::FourBand),
            Some((40_000.0, 60_000.0))
        );
    }

    #[test]
    fn harmonics_band_uses_focus_high_as_lower_floor() {
        assert_eq!(
            harmonics_band_bounds(20_000.0, 50_000.0, BandMode::FourBand),
            Some((50_000.0, 100_000.0))
        );
    }

    #[test]
    fn harmonics_band_is_disabled_for_invalid_ranges() {
        assert_eq!(
            harmonics_band_bounds(50_000.0, 50_000.0, BandMode::FourBand),
            None
        );
        assert_eq!(
            harmonics_band_bounds(20_000.0, 50_000.0, BandMode::ThreeBand),
            None
        );
    }

    #[test]
    fn test_lowpass_attenuates_high_frequency() {
        let sample_rate = 192_000u32;
        let num_samples = 19200; // 100ms

        // Generate a 50 kHz signal
        let high: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (2.0 * std::f64::consts::PI * 50_000.0 * t).sin() as f32
            })
            .collect();

        let filtered = lowpass_filter(&high, 10_000.0, sample_rate);

        let rms_in: f64 =
            (high.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / high.len() as f64).sqrt();
        let rms_out: f64 = (filtered.iter().map(|s| (*s as f64).powi(2)).sum::<f64>()
            / filtered.len() as f64)
            .sqrt();

        assert!(
            rms_out < rms_in * 0.3,
            "50 kHz should be attenuated by LPF at 10 kHz: in={rms_in}, out={rms_out}"
        );
    }

    #[test]
    fn test_lowpass_passes_low_frequency() {
        let sample_rate = 192_000u32;
        let num_samples = 19200;

        // Generate a 1 kHz signal (well below 10 kHz cutoff)
        let low: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (2.0 * std::f64::consts::PI * 1_000.0 * t).sin() as f32
            })
            .collect();

        let filtered = lowpass_filter(&low, 10_000.0, sample_rate);

        let rms_in: f64 =
            (low.iter().map(|s| (*s as f64).powi(2)).sum::<f64>() / low.len() as f64).sqrt();
        let rms_out: f64 = (filtered.iter().map(|s| (*s as f64).powi(2)).sum::<f64>()
            / filtered.len() as f64)
            .sqrt();

        assert!(
            rms_out > rms_in * 0.8,
            "1 kHz should pass through LPF at 10 kHz: in={rms_in}, out={rms_out}"
        );
    }

    #[test]
    fn test_empty_input() {
        let result = lowpass_filter(&[], 1000.0, 44100);
        assert!(result.is_empty());
    }
}
