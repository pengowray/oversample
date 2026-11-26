use serde::{Deserialize, Serialize};

/// A sign-encoded pitch-shift factor.
///
/// The historical convention packs three things into one number, and is kept on
/// the wire/UI (stored values and slider positions are unchanged):
///
/// - magnitude > 1, positive sign → shift DOWN (divide frequencies; 10 ⇒ 50 kHz → 5 kHz)
/// - magnitude > 1, negative sign → shift UP   (multiply frequencies; -10 ⇒ 5 kHz → 50 kHz)
/// - `|value| <= 1` → bypass (returns input unchanged)
///
/// Call sites use the named accessors below instead of re-deriving
/// `abs()`/`< 0.0`/`<= 1.0`, so the direction can no longer be inverted by
/// accident. `#[serde(transparent)]` keeps the serialized form a bare number.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PitchFactor(f64);

impl PitchFactor {
    /// Wrap a raw sign-encoded factor (the historical convention).
    pub fn from_signed(value: f64) -> Self {
        Self(value)
    }
    /// Shift DOWN by `magnitude`× (e.g. 50 kHz → 5 kHz at 10×).
    pub fn down(magnitude: f64) -> Self {
        Self(magnitude.abs())
    }
    /// Shift UP by `magnitude`× (e.g. 5 kHz → 50 kHz at 10×).
    pub fn up(magnitude: f64) -> Self {
        Self(-magnitude.abs())
    }
    /// The raw sign-encoded value (for storage, display, and UI sliders).
    pub fn signed(self) -> f64 {
        self.0
    }
    /// Effective magnitude, clamped to ≥ 1 (matches the old `abs().max(1.0)`).
    pub fn magnitude(self) -> f64 {
        self.0.abs().max(1.0)
    }
    /// True when `|value| <= 1`, i.e. the shift is a no-op.
    pub fn is_bypass(self) -> bool {
        self.0.abs() <= 1.0
    }
    /// True when the factor shifts frequencies UP (negative sign).
    pub fn is_up(self) -> bool {
        self.0 < 0.0
    }
}

impl From<PitchFactor> for f64 {
    fn from(f: PitchFactor) -> f64 {
        f.0
    }
}

/// Output samples in one grain. A grain is a short piece of the input played
/// at the new pitch. 256 samples is 1 ms at 256 kHz: short enough that a
/// bat's sweep is drawn as a line and not as a few long steps, and long
/// enough for two cycles of 2 kHz. At 44.1 kHz it is 5.8 ms.
///
/// Two runs of the shifter over the same audio that start at different
/// samples (two playback chunks, two spectrogram tiles) can line their
/// grains up differently, so their outputs have the same spectrum but not
/// always the same waveform. Runs that start a multiple of 128 samples apart
/// usually give the same samples once both have passed a call or some
/// noise, but not on a long steady tone. Where playback joins two chunks it
/// crossfades them over one grain.
pub const GRAIN: usize = 256;
/// Output samples between grain starts. Two grains overlap at every sample,
/// and their Hann windows add up to exactly 1.
const GRAIN_HOP: usize = GRAIN / 2;
/// How far, in output samples, a grain may be moved either way so its
/// waveform lines up with the grain before it.
const ALIGN_RANGE: usize = 64;
/// Samples compared when lining a grain up with the grain before it.
const ALIGN_WINDOW: usize = GRAIN - GRAIN_HOP;
/// Taps on each side of the resampling kernel.
pub(crate) const KERNEL_HALF: usize = 16;
/// Kernel rows per input sample: the fractional position is rounded to
/// 1/4096 of a sample, which leaves an error about 70 dB under the signal.
const KERNEL_PHASES: usize = 4096;

/// Windowed-sinc resampling kernel: `KERNEL_PHASES + 1` rows of
/// `2 * KERNEL_HALF` taps. Row `p` gives the input at a position `p /
/// KERNEL_PHASES` of a sample past tap `KERNEL_HALF - 1`.
fn kernel() -> &'static [f32] {
    static KERNEL: std::sync::OnceLock<Vec<f32>> = std::sync::OnceLock::new();
    KERNEL.get_or_init(|| {
        // Zeroth-order modified Bessel function, for the Kaiser window.
        fn bessel_i0(x: f64) -> f64 {
            let (mut sum, mut term, mut k) = (1.0, 1.0, 1.0);
            while term > 1e-12 * sum {
                term *= (x / (2.0 * k)).powi(2);
                sum += term;
                k += 1.0;
            }
            sum
        }
        let taps = 2 * KERNEL_HALF;
        let beta = 7.0;
        let mut table = vec![0.0f32; (KERNEL_PHASES + 1) * taps];
        for p in 0..=KERNEL_PHASES {
            let frac = p as f64 / KERNEL_PHASES as f64;
            let row = &mut table[p * taps..(p + 1) * taps];
            let mut sum = 0.0;
            let mut coefs = [0.0f64; 2 * KERNEL_HALF];
            for (t, c) in coefs.iter_mut().enumerate() {
                // Distance from this tap to the wanted position, in samples.
                let d = t as f64 - (KERNEL_HALF as f64 - 1.0) - frac;
                let sinc = if d.abs() < 1e-9 {
                    1.0
                } else {
                    (std::f64::consts::PI * d).sin() / (std::f64::consts::PI * d)
                };
                let u = d / KERNEL_HALF as f64;
                let window = if u.abs() >= 1.0 {
                    0.0
                } else {
                    bessel_i0(beta * (1.0 - u * u).sqrt()) / bessel_i0(beta)
                };
                *c = sinc * window;
                sum += *c;
            }
            // Each row adds up to 1, so a constant input stays constant.
            for (r, c) in row.iter_mut().zip(coefs) {
                *r = (c / sum) as f32;
            }
        }
        table
    })
}

/// The waveform of `samples` at position `pos` (in samples, may be
/// fractional), by the same windowed-sinc interpolation the shifter uses.
/// Samples outside the buffer are 0.
#[inline]
pub(crate) fn interpolate(samples: &[f32], pos: f64) -> f32 {
    sample_at(samples, pos, kernel())
}

/// The input at position `pos` (in samples, may be fractional or outside the
/// buffer), by windowed-sinc interpolation. Samples outside the buffer are 0.
#[inline]
fn sample_at(samples: &[f32], pos: f64, table: &[f32]) -> f32 {
    const TAPS: usize = 2 * KERNEL_HALF;
    // Round down, also for positions before the buffer.
    let mut base = pos as isize;
    if (base as f64) > pos {
        base -= 1;
    }
    let phase = ((pos - base as f64) * KERNEL_PHASES as f64 + 0.5) as usize;
    let row = &table[phase * TAPS..(phase + 1) * TAPS];
    let first = base - (KERNEL_HALF as isize - 1);
    if first >= 0 && (first as usize) + TAPS <= samples.len() {
        let first = first as usize;
        // Eight running sums, so the compiler can do eight taps at a time.
        let mut acc = [0.0f32; 8];
        for (s, c) in samples[first..first + TAPS]
            .chunks_exact(8)
            .zip(row.chunks_exact(8))
        {
            for i in 0..8 {
                acc[i] += s[i] * c[i];
            }
        }
        acc.iter().sum()
    } else {
        let mut acc = 0.0;
        for (t, c) in row.iter().enumerate() {
            let i = first + t as isize;
            if i >= 0 && (i as usize) < samples.len() {
                acc += samples[i as usize] * c;
            }
        }
        acc
    }
}

/// Pitch-shift audio by `factor` while preserving original duration.
/// See [`PitchFactor`] for the sign/bypass convention.
///
/// The output is made of short overlapping grains. Each grain is a piece of
/// the input, resampled so it plays `factor` times slower (shift down) or
/// faster (shift up), and placed at the time it was taken from. Shifted down
/// by 10, a 256-sample grain is made from 25.6 input samples and a grain
/// starts every 128 samples, so a fifth of the input is played. That is
/// what dividing the frequencies by 10 in the same duration means: there is
/// room for a tenth of the cycles. What the shifter must not do is leave
/// gaps in the output, add copies of the signal at other frequencies, or let
/// neighbouring grains cancel:
///
/// - grains start every [`GRAIN_HOP`] samples, from before the buffer's start
///   to its end, so every output sample is covered by two of them and the
///   gain is the same everywhere;
/// - the input is resampled with a windowed-sinc kernel, which adds no copies
///   (linear interpolation added one for each multiple of the factor), and
///   when shifting up it is low-passed first, so nothing folds back;
/// - each grain is moved by up to [`ALIGN_RANGE`] samples to where its
///   waveform best matches the grain before it (WSOLA), so a steady tone
///   comes out as a steady tone at any frequency.
pub fn pitch_shift_realtime(samples: &[f32], factor: PitchFactor) -> Vec<f32> {
    if samples.is_empty() || factor.is_bypass() {
        return samples.to_vec();
    }

    let magnitude = factor.magnitude();
    // Input samples read per output sample.
    let step = if factor.is_up() {
        magnitude
    } else {
        1.0 / magnitude
    };

    // Shifting up reads the input faster than it was sampled. Frequencies
    // over the output's Nyquist would fold back, so they are removed first.
    let filtered;
    let source: &[f32] = if factor.is_up() {
        let mut buf = samples.to_vec();
        crate::dsp::filters::Butterworth::lowpass(8, 1.0, (0.4 / magnitude) as f32)
            .process_in_place(&mut buf);
        filtered = buf;
        &filtered
    } else {
        samples
    };

    let table = kernel();
    let out_len = samples.len();
    let mut output = vec![0.0f32; out_len];

    // Hann window. Two of them, GRAIN_HOP apart, add up to 1.
    let hann: Vec<f32> = (0..GRAIN)
        .map(|i| {
            let x = std::f32::consts::PI * i as f32 / GRAIN as f32;
            x.sin().powi(2)
        })
        .collect();

    // A grain and the samples ALIGN_RANGE either side of it, at the new
    // pitch: `cur[j]` is the input at `(cur_start + j) * step`. These two
    // buffers are only compared with each other, to line grains up, so they
    // are made by linear interpolation, which is cheap. The copies linear
    // interpolation adds are far weaker than the signal and do not move the
    // best match.
    let span = GRAIN + 2 * ALIGN_RANGE;
    let mut cur = vec![0.0f32; span];
    let mut prev = vec![0.0f32; span];
    // Where in the previous grain's span the grain after it would start if
    // the input were simply played on.
    let mut prev_next: Option<usize> = None;
    let linear = |pos: f64| -> f32 {
        if pos < 0.0 {
            return 0.0;
        }
        let i = pos as usize;
        if i + 1 >= source.len() {
            return 0.0;
        }
        let frac = (pos - i as f64) as f32;
        source[i] + frac * (source[i + 1] - source[i])
    };
    // The comparison uses every `sub`-th sample. Shifted down by 4 or more,
    // nothing is left above a quarter of Nyquist, so every 4th is enough.
    let sub = if factor.is_up() {
        2
    } else {
        (magnitude as usize).clamp(2, 4)
    };
    // How well the grain starting at `cur[lag]` continues the previous
    // grain. Normalised, so a louder stretch is not preferred for its level
    // alone.
    let score = |cur: &[f32], target: &[f32], lag: usize| -> f32 {
        let cand = &cur[lag..lag + ALIGN_WINDOW];
        // Four running sums of each, so the additions do not wait on each
        // other.
        let (mut dot, mut energy) = ([0.0f32; 4], [0.0f32; 4]);
        for (t, c) in target.chunks_exact(4 * sub).zip(cand.chunks_exact(4 * sub)) {
            for i in 0..4 {
                dot[i] += t[i * sub] * c[i * sub];
                energy[i] += c[i * sub] * c[i * sub];
            }
        }
        dot.iter().sum::<f32>() / (energy.iter().sum::<f32>() + 1e-12).sqrt()
    };

    // Grain `k` covers output samples `k * GRAIN_HOP .. + GRAIN`. Grains
    // start before the buffer so its first samples are fully covered too.
    let first_grain = -((GRAIN / GRAIN_HOP) as isize - 1);
    let last_grain = (out_len / GRAIN_HOP) as isize;
    for k in first_grain..=last_grain {
        let out_start = k * GRAIN_HOP as isize;
        // The grain's middle plays the input sample at the same time.
        let centre = out_start as f64 + GRAIN as f64 / 2.0;
        let nominal = (centre / step).round() - (GRAIN / 2 + ALIGN_RANGE) as f64;
        // At the buffer's ends the grain is taken from just inside the
        // buffer instead of from the silence beyond it, so the first and
        // last grains are as loud as the rest.
        let last_start = ((source.len() - 1) as f64 / step).floor() - span as f64;
        let cur_start = if last_start > 0.0 {
            nominal.clamp(0.0, last_start)
        } else {
            nominal
        };
        for (j, c) in cur.iter_mut().enumerate() {
            *c = linear((cur_start + j as f64) * step);
        }

        // Move the grain to where it best matches what would have followed
        // the previous grain: try every `sub`-th position, then the
        // positions between the best one and its neighbours.
        let mut offset = ALIGN_RANGE;
        if let Some(next) = prev_next {
            let target = &prev[next..next + ALIGN_WINDOW];
            let mut best = f32::MIN;
            for lag in (0..=2 * ALIGN_RANGE).step_by(sub) {
                let s = score(&cur, target, lag);
                if s > best {
                    best = s;
                    offset = lag;
                }
            }
            let coarse = offset;
            for lag in coarse.saturating_sub(sub - 1)..=(coarse + sub - 1).min(2 * ALIGN_RANGE) {
                let s = score(&cur, target, lag);
                if s > best {
                    best = s;
                    offset = lag;
                }
            }
        }

        // The grain itself is resampled with the windowed-sinc kernel.
        let grain_start = cur_start + offset as f64;
        for (i, w) in hann.iter().enumerate() {
            let o = out_start + i as isize;
            if o >= 0 && (o as usize) < out_len {
                output[o as usize] += sample_at(source, (grain_start + i as f64) * step, table) * w;
            }
        }
        prev_next = Some(offset + GRAIN_HOP);
        std::mem::swap(&mut cur, &mut prev);
    }

    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(len: usize, cycles_per_sample: f32) -> Vec<f32> {
        (0..len)
            .map(|i| (std::f32::consts::TAU * cycles_per_sample * i as f32).sin())
            .collect()
    }

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
    }

    #[test]
    fn test_pitch_shift_down_preserves_length() {
        let input: Vec<f32> = (0..4096).map(|i| (i as f32 / 100.0).sin()).collect();
        let output = pitch_shift_realtime(&input, PitchFactor::from_signed(10.0));
        assert_eq!(output.len(), input.len());
    }

    #[test]
    fn test_pitch_shift_up_preserves_length() {
        let input: Vec<f32> = (0..4096).map(|i| (i as f32 / 100.0).sin()).collect();
        let output = pitch_shift_realtime(&input, PitchFactor::from_signed(-10.0));
        assert_eq!(output.len(), input.len());
    }

    #[test]
    fn test_pitch_shift_bypass() {
        let input: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0];
        assert_eq!(
            pitch_shift_realtime(&input, PitchFactor::from_signed(0.0)),
            input
        );
        assert_eq!(
            pitch_shift_realtime(&input, PitchFactor::from_signed(1.0)),
            input
        );
        assert_eq!(
            pitch_shift_realtime(&input, PitchFactor::from_signed(-1.0)),
            input
        );
    }

    #[test]
    fn test_pitch_shift_empty() {
        assert!(pitch_shift_realtime(&[], PitchFactor::from_signed(10.0)).is_empty());
        assert!(pitch_shift_realtime(&[], PitchFactor::from_signed(-10.0)).is_empty());
    }

    /// The output has the input's level from its first grain to its last
    /// sample. (The old overlap-add left up to 1023 zeros at the end.)
    #[test]
    fn the_output_is_filled_to_both_ends() {
        // An odd length, so the last grain is a partial one.
        let input = tone(20_011, 0.05);
        for factor in [2.0, 10.0, -4.0] {
            let out = pitch_shift_realtime(&input, PitchFactor::from_signed(factor));
            let whole = rms(&out);
            let start = rms(&out[GRAIN..2 * GRAIN]);
            let end = rms(&out[out.len() - GRAIN..]);
            assert!(
                start > 0.8 * whole,
                "factor {factor}: start {start} of {whole}"
            );
            assert!(end > 0.8 * whole, "factor {factor}: end {end} of {whole}");
        }
    }

    /// The shifter adds no gain, but it plays the waveform between the
    /// samples, so its peak is the input's true peak, not its sample peak.
    ///
    /// A tone at a quarter of the sample rate has 4 samples a cycle. Sampled
    /// at its crests, its sample peak is its true peak. Sampled 45 degrees
    /// later, every sample is at 0.707 of the crest, so scaled to a sample
    /// peak of 1 its true peak is 1.414 (+3 dB). Shifted down 10 times, it
    /// has 40 samples a cycle, which land near its crests, and the output
    /// peaks at +3 dB. Clipped bat recordings with 0 dBFS samples peak above
    /// 0 dBFS in pitch shift for this reason.
    #[test]
    fn the_output_peaks_at_the_inputs_true_peak() {
        let len = 20_000;
        for (phase, true_peak) in [(0.0f32, 1.0f32), (std::f32::consts::FRAC_PI_4, 2f32.sqrt())] {
            let raw: Vec<f32> = (0..len)
                .map(|i| (std::f32::consts::FRAC_PI_2 * i as f32 + phase).sin())
                .collect();
            let sample_peak = raw.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            let input: Vec<f32> = raw.iter().map(|v| v / sample_peak).collect();
            let out = pitch_shift_realtime(&input, PitchFactor::down(10.0));
            let out_peak = out[GRAIN..len - GRAIN]
                .iter()
                .fold(0.0f32, |m, v| m.max(v.abs()));
            let diff_db = 20.0 * (out_peak / true_peak).log10();
            assert!(
                diff_db.abs() < 0.5,
                "phase {phase}: output peak {out_peak}, true peak {true_peak} ({diff_db:+.2} dB)"
            );
        }
    }

    /// A short buffer (shorter than one grain) is still shifted.
    #[test]
    fn a_buffer_shorter_than_a_grain_is_shifted() {
        let input = tone(100, 0.2);
        let out = pitch_shift_realtime(&input, PitchFactor::down(2.0));
        assert_eq!(out.len(), 100);
        assert!(rms(&out) > 0.3);
    }
}
