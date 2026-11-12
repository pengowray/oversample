//! Deterministic simulator of the path from a phone's speaker to the X8's input.
//!
//! Every function takes and returns the signal as `Vec<f64>`, so a test can apply
//! them one after another and call [`to_f32`] at the end. Functions that use random
//! numbers take a seed and always give the same output for the same seed.
//!
//! Levels given in dBFS are peak amplitudes: -6 dBFS means an amplitude of 0.5.
//!
//! The tests of this module are in `tests/channel_sim.rs`.

use std::f64::consts::PI;

// -- Random numbers ----------------------------------------------------------

/// Small xorshift64* random number generator.
pub struct Xorshift {
    state: u64,
}

impl Xorshift {
    /// Any seed works, including 0.
    pub fn new(seed: u64) -> Self {
        let scrambled = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xD1B5_4A32_D192_ED03;
        let mut generator = Self {
            state: if scrambled == 0 { 1 } else { scrambled },
        };
        for _ in 0..8 {
            generator.next_u64();
        }
        generator
    }

    pub fn next_u64(&mut self) -> u64 {
        self.state ^= self.state >> 12;
        self.state ^= self.state << 25;
        self.state ^= self.state >> 27;
        self.state.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// Uniformly distributed in 0.0 <= x < 1.0.
    pub fn uniform(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Gaussian with mean 0 and standard deviation 1, from the Box-Muller transform.
    pub fn gaussian(&mut self) -> f64 {
        let radius = (-2.0 * (1.0 - self.uniform()).ln()).sqrt();
        radius * (2.0 * PI * self.uniform()).cos()
    }

    /// +1.0 or -1.0.
    fn sign(&mut self) -> f64 {
        if self.next_u64() >> 63 == 0 {
            1.0
        } else {
            -1.0
        }
    }

    /// Exponentially distributed, for the gaps between events that arrive at
    /// random times. `mean` is the average gap.
    fn exponential(&mut self, mean: f64) -> f64 {
        -(1.0 - self.uniform()).ln() * mean
    }
}

// -- Measurements and conversions --------------------------------------------

pub fn to_f32(x: &[f64]) -> Vec<f32> {
    x.iter().map(|&s| s as f32).collect()
}

pub fn rms(x: &[f64]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|s| s * s).sum::<f64>() / x.len() as f64).sqrt()
}

/// Converts a linear level to decibels relative to 1.0.
pub fn db(level: f64) -> f64 {
    20.0 * level.log10()
}

fn amplitude_from_db(level_db: f64) -> f64 {
    10f64.powf(level_db / 20.0)
}

/// The part of `x` between `low_hz` and `high_hz`, from a brick-wall filter in the
/// frequency domain.
pub fn band_limited(x: &[f64], sample_rate: u32, low_hz: f64, high_hz: f64) -> Vec<f64> {
    let size = (x.len() + 1).next_power_of_two();
    let mut re = x.to_vec();
    re.resize(size, 0.0);
    let mut im = vec![0.0; size];
    fft(&mut re, &mut im, false);
    let hz_per_bin = sample_rate as f64 / size as f64;
    for bin in 0..size {
        let distance = bin.min(size - bin);
        let hz = distance as f64 * hz_per_bin;
        if hz < low_hz || hz > high_hz {
            re[bin] = 0.0;
            im[bin] = 0.0;
        }
    }
    fft(&mut re, &mut im, true);
    re.truncate(x.len());
    re
}

/// RMS level of the 17 to 19 kHz band while the chime is playing.
///
/// The function finds the chime as the span where the band's 20 ms moving RMS is
/// above half power (-3 dB) relative to the moving RMS's peak, so silence around
/// the chime does not lower the result. The mix of a chime and the carrier alone
/// give the same result, because the melody is outside the band.
pub fn carrier_rms(x: &[f64], sample_rate: u32) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    let band = band_limited(x, sample_rate, 17_000.0, 19_000.0);
    let window = ((0.020 * sample_rate as f64) as usize).clamp(1, band.len());

    // Running sum of squares over `window` samples, with an index that points to
    // the middle of the window.
    let mut sum = 0.0;
    let mut moving_power = vec![0.0; band.len()];
    for (end, &sample) in band.iter().enumerate() {
        sum += sample * sample;
        if end >= window {
            sum -= band[end - window] * band[end - window];
        }
        if end + 1 >= window {
            moving_power[end - window / 2] = sum.max(0.0);
        }
    }
    let peak = moving_power.iter().cloned().fold(0.0, f64::max);
    let threshold = peak / 2.0;
    let first = moving_power
        .iter()
        .position(|&p| p >= threshold)
        .unwrap_or(0);
    let last = moving_power
        .iter()
        .rposition(|&p| p >= threshold)
        .unwrap_or(0);
    rms(&band[first..=last])
}

// -- FFT and convolution -----------------------------------------------------

/// In-place radix-2 FFT. The length must be a power of two. The inverse transform
/// divides by the length.
pub fn fft(re: &mut [f64], im: &mut [f64], inverse: bool) {
    let size = re.len();
    assert!(size.is_power_of_two() && im.len() == size);
    if size < 2 {
        return;
    }

    let mut j = 0;
    for i in 1..size {
        let mut bit = size >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }

    let sign = if inverse { 1.0 } else { -1.0 };
    let twiddles: Vec<(f64, f64)> = (0..size / 2)
        .map(|k| {
            let angle = sign * 2.0 * PI * k as f64 / size as f64;
            (angle.cos(), angle.sin())
        })
        .collect();

    let mut half = 1;
    while half < size {
        let step = size / (2 * half);
        let mut start = 0;
        while start < size {
            for k in 0..half {
                let (w_re, w_im) = twiddles[k * step];
                let (a, b) = (start + k, start + k + half);
                let t_re = re[b] * w_re - im[b] * w_im;
                let t_im = re[b] * w_im + im[b] * w_re;
                re[b] = re[a] - t_re;
                im[b] = im[a] - t_im;
                re[a] += t_re;
                im[a] += t_im;
            }
            start += 2 * half;
        }
        half *= 2;
    }

    if inverse {
        let scale = 1.0 / size as f64;
        for value in re.iter_mut().chain(im.iter_mut()) {
            *value *= scale;
        }
    }
}

/// Linear convolution. The result has `x.len() + h.len() - 1` samples.
pub fn convolve(x: &[f64], h: &[f64]) -> Vec<f64> {
    if x.is_empty() || h.is_empty() {
        return Vec::new();
    }
    let length = x.len() + h.len() - 1;
    let size = length.next_power_of_two();
    let mut x_re = x.to_vec();
    x_re.resize(size, 0.0);
    let mut x_im = vec![0.0; size];
    let mut h_re = h.to_vec();
    h_re.resize(size, 0.0);
    let mut h_im = vec![0.0; size];
    fft(&mut x_re, &mut x_im, false);
    fft(&mut h_re, &mut h_im, false);
    for i in 0..size {
        let re = x_re[i] * h_re[i] - x_im[i] * h_im[i];
        let im = x_re[i] * h_im[i] + x_im[i] * h_re[i];
        x_re[i] = re;
        x_im[i] = im;
    }
    fft(&mut x_re, &mut x_im, true);
    x_re.truncate(length);
    x_re
}

// -- Windowed-sinc interpolation ---------------------------------------------

/// Kaiser-windowed sinc kernel, stored in a table and read with linear
/// interpolation. It has 32 samples on each side and a Kaiser beta of 10. Its error
/// on an 18 kHz sine at a 48 kHz input rate is below -60 dB, which
/// `tests/channel_sim.rs` checks.
struct SincKernel {
    table: Vec<f64>,
}

const SINC_HALF_WIDTH: usize = 32;
const SINC_STEPS_PER_SAMPLE: usize = 512;
const KAISER_BETA: f64 = 10.0;

impl SincKernel {
    fn new() -> Self {
        fn bessel_i0(x: f64) -> f64 {
            let mut sum = 1.0;
            let mut term = 1.0;
            for k in 1..40 {
                term *= (x / (2.0 * k as f64)).powi(2);
                sum += term;
            }
            sum
        }

        let normaliser = bessel_i0(KAISER_BETA);
        let entries = SINC_HALF_WIDTH * SINC_STEPS_PER_SAMPLE + 2;
        let table = (0..entries)
            .map(|i| {
                let t = i as f64 / SINC_STEPS_PER_SAMPLE as f64;
                let ratio = t / SINC_HALF_WIDTH as f64;
                if ratio >= 1.0 {
                    return 0.0;
                }
                let sinc = if i == 0 {
                    1.0
                } else {
                    (PI * t).sin() / (PI * t)
                };
                sinc * bessel_i0(KAISER_BETA * (1.0 - ratio * ratio).sqrt()) / normaliser
            })
            .collect();
        Self { table }
    }

    /// The kernel's value at an offset of `t` samples from its centre.
    fn at(&self, t: f64) -> f64 {
        let position = t.abs() * SINC_STEPS_PER_SAMPLE as f64;
        let index = position as usize;
        if index + 1 >= self.table.len() {
            return 0.0;
        }
        let fraction = position - index as f64;
        self.table[index] + (self.table[index + 1] - self.table[index]) * fraction
    }

    /// The value of the band-limited signal through `x` at a fractional sample
    /// position. Samples outside `x` count as zero.
    fn interpolate(&self, x: &[f64], position: f64) -> f64 {
        let centre = position.floor() as i64;
        let half = SINC_HALF_WIDTH as i64;
        let first = (centre - half + 1).max(0);
        let last = (centre + half).min(x.len() as i64 - 1);
        (first..=last)
            .map(|k| x[k as usize] * self.at(position - k as f64))
            .sum()
    }
}

// -- Channel effects ---------------------------------------------------------

pub fn gain_db(mut x: Vec<f64>, gain: f64) -> Vec<f64> {
    let factor = amplitude_from_db(gain);
    x.iter_mut().for_each(|s| *s *= factor);
    x
}

/// Polarity inversion, as on some of the X8's inputs.
pub fn invert(mut x: Vec<f64>) -> Vec<f64> {
    x.iter_mut().for_each(|s| *s = -*s);
    x
}

/// Hard clipping at +-`limit`. Apply it after [`gain_db`] to model an input stage
/// that overloads.
pub fn clip(mut x: Vec<f64>, limit: f64) -> Vec<f64> {
    x.iter_mut().for_each(|s| *s = s.clamp(-limit, limit));
    x
}

pub fn dc_offset(mut x: Vec<f64>, offset: f64) -> Vec<f64> {
    x.iter_mut().for_each(|s| *s += offset);
    x
}

/// Adds silence before and after the signal, given in samples.
pub fn pad(x: Vec<f64>, before: usize, after: usize) -> Vec<f64> {
    let mut padded = vec![0.0; before];
    padded.extend_from_slice(&x);
    padded.resize(before + x.len() + after, 0.0);
    padded
}

/// Adds white Gaussian noise so that the carrier-to-noise ratio in the 2 kHz band
/// from 17 to 19 kHz is `snr_db_in_band`.
///
/// The SNR is the carrier RMS from [`carrier_rms`] over the noise RMS in that band.
/// White noise spreads over the whole spectrum from 0 to half the sample rate, so
/// the total noise power is the band's noise power times (`sample_rate` / 2) / 2000.
///
/// The carrier level is measured from `x` when this function runs, so apply gain,
/// speaker and reverb effects first and add clicks and bat calls afterwards.
///
/// Panics if `x` has nothing in the band, because the noise level would be 0. Use
/// [`add_white_noise_rms`] for a signal with no chime in it.
pub fn add_white_noise(x: Vec<f64>, sample_rate: u32, snr_db_in_band: f64, seed: u64) -> Vec<f64> {
    let carrier = carrier_rms(&x, sample_rate);
    assert!(
        carrier > 0.0,
        "the signal has nothing in the 17 to 19 kHz band"
    );
    let band_noise_rms = carrier / amplitude_from_db(snr_db_in_band);
    let total_noise_rms = band_noise_rms * (sample_rate as f64 / 2.0 / 2000.0).sqrt();
    add_white_noise_rms(x, total_noise_rms, seed)
}

/// Adds white Gaussian noise with the given total RMS level, for tests of a signal
/// that has no chime in it.
pub fn add_white_noise_rms(mut x: Vec<f64>, total_noise_rms: f64, seed: u64) -> Vec<f64> {
    let mut random = Xorshift::new(seed);
    x.iter_mut()
        .for_each(|s| *s += total_noise_rms * random.gaussian());
    x
}

/// Second-order Butterworth low-pass filter, for a speaker that rolls off near the
/// carrier. The coefficients are from the RBJ Audio EQ Cookbook.
pub fn speaker_lowpass(mut x: Vec<f64>, sample_rate: u32, cutoff_hz: f64) -> Vec<f64> {
    let omega = 2.0 * PI * cutoff_hz / sample_rate as f64;
    let alpha = omega.sin() / (2.0 * std::f64::consts::FRAC_1_SQRT_2);
    let cos_omega = omega.cos();
    let a0 = 1.0 + alpha;
    let b0 = (1.0 - cos_omega) / 2.0 / a0;
    let b1 = (1.0 - cos_omega) / a0;
    let b2 = b0;
    let a1 = -2.0 * cos_omega / a0;
    let a2 = (1.0 - alpha) / a0;
    let (mut z1, mut z2) = (0.0, 0.0);
    for sample in x.iter_mut() {
        let input = *sample;
        let output = b0 * input + z1;
        z1 = b1 * input - a1 * output + z2;
        z2 = b2 * input - a2 * output;
        *sample = output;
    }
    x
}

/// Memoryless speaker distortion: y = x + a2 x^2 + a3 x^3.
pub fn speaker_nonlinearity(mut x: Vec<f64>, a2: f64, a3: f64) -> Vec<f64> {
    x.iter_mut().for_each(|s| {
        let v = *s;
        *s = v + a2 * v * v + a3 * v * v * v;
    });
    x
}

/// Clock error of the playback relative to the recorder. A positive `ppm` means
/// the signal plays faster than it should, so the carrier is higher and the chime
/// is shorter. Uses windowed-sinc interpolation.
pub fn rate_error_ppm(x: Vec<f64>, ppm: f64) -> Vec<f64> {
    let kernel = SincKernel::new();
    let speed = 1.0 + ppm * 1e-6;
    let output_length = (x.len() as f64 / speed).floor() as usize;
    (0..output_length)
        .map(|n| kernel.interpolate(&x, n as f64 * speed))
        .collect()
}

/// Ideal interpolation by a whole-number factor, for a 48 kHz render that reaches a
/// 192 kHz input through a good output stage. The output has `factor` times as many
/// samples, and every `factor`th output sample equals an input sample.
pub fn upsample_sinc(x: Vec<f64>, factor: usize) -> Vec<f64> {
    let kernel = SincKernel::new();
    (0..x.len() * factor)
        .map(|n| kernel.interpolate(&x, n as f64 / factor as f64))
        .collect()
}

/// Zero-order hold by a whole-number factor, for a poor output stage. Each input
/// sample is repeated `factor` times, which leaves images of the carrier at the
/// input sample rate minus 18 kHz, plus 18 kHz, and so on.
pub fn zoh_upsample(x: Vec<f64>, factor: usize) -> Vec<f64> {
    x.into_iter()
        .flat_map(|s| std::iter::repeat_n(s, factor))
        .collect()
}

/// Adds mains hum at `freq_hz` with harmonics 2 to 5 at 1/2 to 1/5 of the
/// fundamental. `dbfs` is the peak amplitude of the fundamental.
pub fn hum(mut x: Vec<f64>, sample_rate: u32, freq_hz: f64, dbfs: f64) -> Vec<f64> {
    let amplitude = amplitude_from_db(dbfs);
    for (n, sample) in x.iter_mut().enumerate() {
        let t = n as f64 / sample_rate as f64;
        for harmonic in 1..=5 {
            let k = harmonic as f64;
            *sample += amplitude / k * (2.0 * PI * freq_hz * k * t + 0.7 * k).sin();
        }
    }
    x
}

/// Adds single-sample impulses of random polarity at random times, on average
/// `rate_per_second` of them each second. `dbfs` is their peak amplitude.
pub fn clicks(
    mut x: Vec<f64>,
    sample_rate: u32,
    rate_per_second: f64,
    dbfs: f64,
    seed: u64,
) -> Vec<f64> {
    let mut random = Xorshift::new(seed);
    let amplitude = amplitude_from_db(dbfs);
    let mean_gap = sample_rate as f64 / rate_per_second;
    let mut position = random.exponential(mean_gap);
    while (position as usize) < x.len() {
        x[position as usize] += amplitude * random.sign();
        position += random.exponential(mean_gap);
    }
    x
}

/// Length of a simulated bat call in seconds.
const BAT_CALL_SECONDS: f64 = 0.003;
const BAT_CALL_START_HZ: f64 = 80_000.0;
const BAT_CALL_END_HZ: f64 = 40_000.0;

/// Adds bat calls at random times, on average `rate_per_second` of them each second.
/// Each call is a 3 ms downward FM sweep from 80 to 40 kHz with 0.3 ms sin^2 ramps.
/// `dbfs` is the peak amplitude. `sample_rate` must be above 160 kHz; the X8's bat
/// recordings are 192 kHz.
pub fn bat_calls(
    mut x: Vec<f64>,
    sample_rate: u32,
    rate_per_second: f64,
    dbfs: f64,
    seed: u64,
) -> Vec<f64> {
    assert!(
        sample_rate > 160_000,
        "an 80 kHz sweep needs a sample rate above 160 kHz"
    );
    let rate = sample_rate as f64;
    let mut random = Xorshift::new(seed);
    let amplitude = amplitude_from_db(dbfs);
    let mean_gap = rate / rate_per_second;
    let call_samples = (BAT_CALL_SECONDS * rate) as usize;
    let ramp_seconds = 0.0003;
    let sweep_rate = (BAT_CALL_END_HZ - BAT_CALL_START_HZ) / BAT_CALL_SECONDS;

    let mut position = random.exponential(mean_gap);
    while (position as usize) < x.len() {
        let start = position as usize;
        let polarity = random.sign();
        for n in 0..call_samples {
            let Some(sample) = x.get_mut(start + n) else {
                break;
            };
            let t = n as f64 / rate;
            let edge = t.min(BAT_CALL_SECONDS - t) / ramp_seconds;
            let envelope = (edge.min(1.0) * PI / 2.0).sin().powi(2);
            let phase = 2.0 * PI * (BAT_CALL_START_HZ * t + sweep_rate * t * t / 2.0);
            *sample += polarity * amplitude * envelope * phase.sin();
        }
        position += call_samples as f64 + random.exponential(mean_gap);
    }
    x
}

// -- Reverb ------------------------------------------------------------------

/// Acoustic surroundings for [`reverb`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReverbProfile {
    /// A phone held near the recorder outdoors: 5 reflections from 1 to 8 ms at -6
    /// to -20 dB.
    Small,
    /// A furnished room: 6 reflections up to 30 ms, and a noise tail with an RT60
    /// of 150 ms and energy 9 dB below the direct sound.
    Medium,
    /// A hard, empty room: 5 strong reflections from 3 to 12 ms, and a noise tail
    /// with an RT60 of 500 ms and energy 3 dB below the direct sound.
    Large,
}

/// A decaying noise tail in an impulse response.
#[derive(Clone, Copy)]
struct Tail {
    rt60_seconds: f64,
    start_seconds: f64,
    energy_db: f64,
    seed: u64,
}

struct ProfileDefinition {
    /// Delay in seconds, gain in dB, and polarity.
    reflections: &'static [(f64, f64, f64)],
    tail: Option<Tail>,
}

// The delays are not whole numbers of carrier periods, so the reflections arrive
// at various phases of the 18 kHz carrier.
const SMALL_REFLECTIONS: [(f64, f64, f64); 5] = [
    (0.00107, -6.0, 1.0),
    (0.00233, -10.0, -1.0),
    (0.00371, -13.0, 1.0),
    (0.00529, -16.0, -1.0),
    (0.00793, -20.0, 1.0),
];

const MEDIUM_REFLECTIONS: [(f64, f64, f64); 6] = [
    (0.00213, -5.0, 1.0),
    (0.00471, -7.0, -1.0),
    (0.00937, -9.0, 1.0),
    (0.01463, -11.0, -1.0),
    (0.02119, -13.0, 1.0),
    (0.02947, -15.0, -1.0),
];

const LARGE_REFLECTIONS: [(f64, f64, f64); 5] = [
    (0.00311, -2.0, 1.0),
    (0.00463, -3.0, -1.0),
    (0.00671, -4.0, 1.0),
    (0.00893, -4.0, -1.0),
    (0.01177, -5.0, 1.0),
];

fn profile_definition(profile: ReverbProfile) -> ProfileDefinition {
    match profile {
        ReverbProfile::Small => ProfileDefinition {
            reflections: &SMALL_REFLECTIONS,
            tail: None,
        },
        ReverbProfile::Medium => ProfileDefinition {
            reflections: &MEDIUM_REFLECTIONS,
            tail: Some(Tail {
                rt60_seconds: 0.150,
                start_seconds: 0.010,
                energy_db: -9.0,
                seed: 0x4D45_4449,
            }),
        },
        ReverbProfile::Large => ProfileDefinition {
            reflections: &LARGE_REFLECTIONS,
            tail: Some(Tail {
                rt60_seconds: 0.500,
                start_seconds: 0.005,
                energy_db: -3.0,
                seed: 0x4C41_5247,
            }),
        },
    }
}

/// Natural logarithm of 10^3: the amplitude of a signal falls by this many nepers
/// in one RT60 (60 dB).
const NEPERS_IN_60_DB: f64 = 6.907_755_278_982_137;

/// The impulse response of a profile, with the direct sound of gain 1 at sample 0.
/// Reflections are placed at their exact delay with the windowed-sinc kernel, so a
/// profile has the same effect on the carrier at every sample rate.
pub fn impulse_response(sample_rate: u32, profile: ReverbProfile) -> Vec<f64> {
    let rate = sample_rate as f64;
    let definition = profile_definition(profile);
    let kernel = SincKernel::new();

    let reflections_end = definition
        .reflections
        .iter()
        .map(|r| r.0)
        .fold(0.0, f64::max);
    let tail_end = definition.tail.map_or(0.0, |tail| tail.rt60_seconds * 1.2);
    let length = (reflections_end.max(tail_end) * rate) as usize + SINC_HALF_WIDTH + 1;
    let mut response = vec![0.0; length];
    response[0] = 1.0;

    for &(delay_seconds, gain, polarity) in definition.reflections {
        let delay = delay_seconds * rate;
        let amplitude = polarity * amplitude_from_db(gain);
        let centre = delay.floor() as i64;
        let half = SINC_HALF_WIDTH as i64;
        for n in (centre - half + 1).max(0)..=(centre + half).min(length as i64 - 1) {
            response[n as usize] += amplitude * kernel.at(n as f64 - delay);
        }
    }

    if let Some(tail) = definition.tail {
        // The energy of A exp(-k t) noise from t0 onwards is
        // A^2 rate exp(-2 k t0) / (2 k), and this solves that for A.
        let decay = NEPERS_IN_60_DB / tail.rt60_seconds;
        let energy = 10f64.powf(tail.energy_db / 10.0);
        let start_amplitude =
            (energy * 2.0 * decay * (2.0 * decay * tail.start_seconds).exp() / rate).sqrt();
        let mut random = Xorshift::new(tail.seed);
        let first = (tail.start_seconds * rate) as usize;
        for (n, sample) in response.iter_mut().enumerate().skip(first) {
            let t = n as f64 / rate;
            *sample += start_amplitude * (-decay * t).exp() * random.gaussian();
        }
    }

    response
}

/// Applies a reverb profile. The direct sound stays where it was in time, and the
/// output is longer than the input by the length of the impulse response minus 1.
pub fn reverb(x: Vec<f64>, sample_rate: u32, profile: ReverbProfile) -> Vec<f64> {
    convolve(&x, &impulse_response(sample_rate, profile))
}
