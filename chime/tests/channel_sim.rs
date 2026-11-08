//! Tests of the channel simulator in `support/channel.rs`. They live in their own
//! file because code in `support` is compiled into every test binary, and tests
//! placed there would run once for each binary.

mod support;

use std::f64::consts::PI;

use support::channel::*;
use support::generator;

const FS: u32 = 48_000;

fn sine(frequency: f64, amplitude: f64, length: usize, sample_rate: u32) -> Vec<f64> {
    (0..length)
        .map(|n| amplitude * (2.0 * PI * frequency * n as f64 / sample_rate as f64).sin())
        .collect()
}

/// RMS of the part of `x` between `low_hz` and `high_hz`, summed from FFT bins by
/// Parseval's theorem. `x.len()` must be a power of two.
fn fft_band_rms(x: &[f64], sample_rate: u32, low_hz: f64, high_hz: f64) -> f64 {
    let mut re = x.to_vec();
    let mut im = vec![0.0; x.len()];
    fft(&mut re, &mut im, false);
    let size = x.len();
    let hz_per_bin = sample_rate as f64 / size as f64;
    let mut power = 0.0;
    for bin in 1..size / 2 {
        let hz = bin as f64 * hz_per_bin;
        if hz >= low_hz && hz < high_hz {
            power += 2.0 * (re[bin] * re[bin] + im[bin] * im[bin]) / (size as f64 * size as f64);
        }
    }
    power.sqrt()
}

#[test]
fn fft_finds_a_sine_at_its_level() {
    let x = sine(18_000.0, 0.5, 1 << 14, FS);
    let band = fft_band_rms(&x, FS, 17_000.0, 19_000.0);
    assert!((band - 0.5 / 2f64.sqrt()).abs() < 1e-3, "{band}");
    let outside = fft_band_rms(&x, FS, 1_000.0, 16_000.0);
    assert!(outside < 1e-3, "{outside}");
}

#[test]
fn inverse_fft_undoes_fft() {
    let original = sine(1000.0, 1.0, 256, FS);
    let mut re = original.clone();
    let mut im = vec![0.0; 256];
    fft(&mut re, &mut im, false);
    fft(&mut re, &mut im, true);
    for (a, b) in original.iter().zip(&re) {
        assert!((a - b).abs() < 1e-12);
    }
}

#[test]
fn convolve_matches_direct_convolution() {
    let x = sine(1234.0, 1.0, 300, FS);
    let h: Vec<f64> = (0..50).map(|n| (-(n as f64) / 10.0).exp()).collect();
    let fast = convolve(&x, &h);
    assert_eq!(fast.len(), 349);
    for n in 0..fast.len() {
        let direct: f64 = (0..h.len())
            .filter(|k| *k <= n && n - k < x.len())
            .map(|k| h[k] * x[n - k])
            .sum();
        assert!((direct - fast[n]).abs() < 1e-10, "sample {n}");
    }
}

#[test]
fn random_numbers_are_repeatable_and_have_the_right_statistics() {
    let draw = |seed| -> Vec<f64> {
        let mut random = Xorshift::new(seed);
        (0..100_000).map(|_| random.gaussian()).collect()
    };
    let first = draw(1);
    assert_eq!(first, draw(1));
    assert_ne!(first, draw(2));
    let mean = first.iter().sum::<f64>() / first.len() as f64;
    assert!(mean.abs() < 0.02, "mean {mean}");
    assert!((rms(&first) - 1.0).abs() < 0.02, "rms {}", rms(&first));
    let mut zero_seed = Xorshift::new(0);
    assert_ne!(zero_seed.next_u64(), zero_seed.next_u64());
}

fn check_band_snr(sample_rate: u32, snr_db: f64) {
    let size = 1 << 17;
    let carrier_amplitude = 0.25;
    let clean = sine(18_000.0, carrier_amplitude, size, sample_rate);
    let carrier = carrier_amplitude / 2f64.sqrt();

    let noisy = add_white_noise(clean.clone(), sample_rate, snr_db, 7);
    let noise: Vec<f64> = noisy.iter().zip(&clean).map(|(a, b)| a - b).collect();

    let band_noise = fft_band_rms(&noise, sample_rate, 17_000.0, 19_000.0);
    let measured_snr = db(carrier / band_noise);
    assert!(
        (measured_snr - snr_db).abs() < 0.3,
        "{sample_rate} Hz: wanted {snr_db} dB, measured {measured_snr} dB"
    );

    // All of the noise compared with the 2 kHz band: the ratio of the powers is
    // (fs / 2) / 2000.
    let expected_ratio = sample_rate as f64 / 2.0 / 2000.0;
    let measured_ratio = (rms(&noise) / band_noise).powi(2);
    assert!(
        (db(measured_ratio.sqrt()) - db(expected_ratio.sqrt())).abs() < 0.3,
        "{sample_rate} Hz: wanted power ratio {expected_ratio}, measured {measured_ratio}"
    );
}

#[test]
fn band_snr_scaling_is_right_at_48_khz() {
    check_band_snr(48_000, 10.0);
    check_band_snr(48_000, -3.0);
}

#[test]
fn band_snr_scaling_is_right_at_192_khz() {
    check_band_snr(192_000, 10.0);
    check_band_snr(192_000, -3.0);
}

#[test]
fn carrier_rms_measures_only_the_carrier_during_the_chime() {
    let payload = generator::time_payload(1_793_000_000, 660);
    let render = generator::render(FS, &payload, generator::TUNE_TIME);
    let carrier_alone: Vec<f64> = render.carrier.iter().map(|c| c / 4.0).collect();
    let truth = rms(&carrier_alone);

    let mix = render.mix();
    let measured = carrier_rms(&mix, FS);
    assert!((db(measured) - db(truth)).abs() < 0.25, "truth {truth}, measured {measured}");

    // One second of silence on each side must not change the measurement.
    let padded = pad(mix, FS as usize, FS as usize);
    let measured_padded = carrier_rms(&padded, FS);
    assert!((db(measured_padded) - db(truth)).abs() < 0.25, "padded: {measured_padded}");
}

fn energy_after(response: &[f64], start: usize) -> f64 {
    response[start..].iter().map(|s| s * s).sum()
}

/// RT60 of an impulse response from the backward integral of its energy (Schroeder
/// integration) with a straight line fitted between -5 and -35 dB, where 0 dB is the
/// energy after sample `start`.
fn schroeder_rt60(response: &[f64], start: usize, sample_rate: u32) -> f64 {
    let mut remaining = energy_after(response, start);
    let total = remaining;
    let (mut sum_t, mut sum_d, mut sum_tt, mut sum_td, mut count) = (0.0, 0.0, 0.0, 0.0, 0.0);
    for (n, sample) in response.iter().enumerate().skip(start) {
        let level = 10.0 * (remaining / total).log10();
        if (-35.0..=-5.0).contains(&level) {
            let t = n as f64 / sample_rate as f64;
            sum_t += t;
            sum_d += level;
            sum_tt += t * t;
            sum_td += t * level;
            count += 1.0;
        }
        remaining -= sample * sample;
    }
    let slope = (count * sum_td - sum_t * sum_d) / (count * sum_tt - sum_t * sum_t);
    -60.0 / slope
}

#[test]
fn small_profile_has_reflections_at_the_right_times_and_levels() {
    for sample_rate in [48_000u32, 192_000] {
        let response = impulse_response(sample_rate, ReverbProfile::Small);
        assert_eq!(response[0], 1.0);
        let duration = response.len() as f64 / sample_rate as f64;
        assert!(duration < 0.010, "small profile has no tail, length {duration} s");

        // The energy near each reflection, in a window a little wider than the sinc
        // kernel, must match its gain. The kernel's energy is 1 at any delay.
        for (delay_ms, gain_db) in [(1.07, -6.0), (2.33, -10.0), (3.71, -13.0), (5.29, -16.0), (7.93, -20.0)] {
            let centre = delay_ms / 1000.0 * sample_rate as f64;
            let half_width = 0.0002 * sample_rate as f64;
            let window_start = (centre - half_width) as usize;
            let window_end = ((centre + half_width) as usize).min(response.len());
            let energy: f64 = response[window_start..window_end].iter().map(|s| s * s).sum();
            let level = 10.0 * energy.log10();
            assert!(
                (level - gain_db).abs() < 0.6,
                "{sample_rate} Hz, {delay_ms} ms: wanted {gain_db} dB, measured {level} dB"
            );
        }
    }
}

#[test]
fn medium_profile_tail_has_an_rt60_of_about_150_ms() {
    for sample_rate in [48_000u32, 192_000] {
        let response = impulse_response(sample_rate, ReverbProfile::Medium);
        // Start after the last reflection, which is at about 30 ms.
        let start = (0.031 * sample_rate as f64) as usize;
        let rt60 = schroeder_rt60(&response, start, sample_rate);
        assert!((rt60 - 0.150).abs() < 0.015, "{sample_rate} Hz: RT60 {rt60} s");

        let tail_end = response.len() as f64 / sample_rate as f64;
        assert!(tail_end > 0.150, "response length {tail_end} s");
    }
}

#[test]
fn large_profile_tail_has_an_rt60_of_about_500_ms() {
    for sample_rate in [48_000u32, 192_000] {
        let response = impulse_response(sample_rate, ReverbProfile::Large);
        let start = (0.013 * sample_rate as f64) as usize;
        let rt60 = schroeder_rt60(&response, start, sample_rate);
        assert!((rt60 - 0.500).abs() < 0.05, "{sample_rate} Hz: RT60 {rt60} s");
    }
}

#[test]
fn tail_energy_is_as_documented_at_every_sample_rate() {
    // The tail energy relative to the direct sound, away from the reflections. Both
    // sample rates must agree so that a profile has the same effect at each.
    let tail_energy = |sample_rate: u32, profile, after_seconds: f64| -> f64 {
        let response = impulse_response(sample_rate, profile);
        energy_after(&response, (after_seconds * sample_rate as f64) as usize)
    };
    for (profile, after) in [(ReverbProfile::Medium, 0.040), (ReverbProfile::Large, 0.020)] {
        let at_48 = 10.0 * tail_energy(48_000, profile, after).log10();
        let at_192 = 10.0 * tail_energy(192_000, profile, after).log10();
        assert!((at_48 - at_192).abs() < 0.5, "{profile:?}: {at_48} dB and {at_192} dB");
    }
    // The medium tail is -9 dB from 10 ms onwards. From 40 ms onwards it has lost
    // 60 dB * (30 ms / 150 ms) = 12 dB more.
    let medium = 10.0 * tail_energy(48_000, ReverbProfile::Medium, 0.040).log10();
    assert!((medium - (-21.0)).abs() < 1.0, "medium tail after 40 ms: {medium} dB");
}

#[test]
fn reverb_of_a_short_burst_decays_at_the_profile_rate() {
    // A burst of 2 ms has a bandwidth of about 500 Hz, so a 20 ms window of the tail
    // holds about 20 independent values. A long sine would give a tail level with
    // much larger random swings, because it picks out a single frequency of the
    // noise in the impulse response.
    let burst = sine(18_000.0, 0.25, FS as usize / 500, FS);
    let burst_end = burst.len();
    let output = reverb(burst, FS, ReverbProfile::Large);

    // A decay of 60 dB per 500 ms gives 24 dB between two times 200 ms apart.
    let window = FS as usize / 50;
    let level_at = |seconds_after_end: f64| -> f64 {
        let start = burst_end + (seconds_after_end * FS as f64) as usize;
        db(rms(&output[start..start + window]))
    };
    let early = level_at(0.050);
    let late = level_at(0.250);
    assert!((early - late - 24.0).abs() < 6.0, "levels {early} dB and {late} dB");
}

#[test]
fn reverb_leaves_the_direct_sound_in_place_and_extends_the_output() {
    let burst = sine(18_000.0, 0.25, 4800, FS);
    let response_length = impulse_response(FS, ReverbProfile::Medium).len();
    let output = reverb(burst.clone(), FS, ReverbProfile::Medium);
    assert_eq!(output.len(), burst.len() + response_length - 1);
    // The first reflection is at 2.13 ms, which is sample 102, and the sinc kernel
    // starts 32 samples before that.
    for n in 0..64 {
        assert!((output[n] - burst[n]).abs() < 1e-3, "sample {n}");
    }
}

#[test]
fn rate_error_shifts_frequency_and_shortens_the_signal() {
    let ppm = 2000.0;
    let length = 48_000;
    for sample_rate in [48_000u32, 192_000] {
        let original = sine(18_000.0, 0.25, length, sample_rate);
        let output = rate_error_ppm(original, ppm);
        let speed = 1.0 + ppm * 1e-6;
        assert_eq!(output.len(), (length as f64 / speed).floor() as usize);

        let expected = sine(18_000.0 * speed, 0.25, output.len(), sample_rate);
        let residual: Vec<f64> = output.iter().zip(&expected).map(|(a, b)| a - b).collect();
        // Skip the ends, where the sinc kernel runs off the signal.
        let middle = &residual[100..residual.len() - 100];
        let error_db = db(rms(middle) / rms(&expected[100..expected.len() - 100]));
        assert!(error_db < -60.0, "{sample_rate} Hz: error {error_db} dB");
    }
}

#[test]
fn upsample_sinc_interpolates_a_48_khz_carrier_accurately() {
    let input = sine(18_000.0, 0.25, 4800, 48_000);
    let output = upsample_sinc(input.clone(), 4);
    assert_eq!(output.len(), 4 * 4800);
    for (k, value) in input.iter().enumerate() {
        assert!((output[4 * k] - value).abs() < 1e-9, "sample {k}");
    }
    let expected = sine(18_000.0, 0.25, output.len(), 192_000);
    let residual: Vec<f64> = output.iter().zip(&expected).map(|(a, b)| a - b).collect();
    let middle = &residual[400..residual.len() - 400];
    let error_db = db(rms(middle) / rms(&expected[400..expected.len() - 400]));
    assert!(error_db < -60.0, "error {error_db} dB");
}

#[test]
fn upsampled_carrier_has_images_only_with_zero_order_hold() {
    let size_at_48k = 1 << 14;
    let input = sine(18_000.0, 0.25, size_at_48k, 48_000);

    let held = zoh_upsample(input.clone(), 4);
    assert_eq!(held.len(), 4 * size_at_48k);
    assert_eq!(held[5], input[1]);
    let carrier = fft_band_rms(&held, 192_000, 17_000.0, 19_000.0);
    let image = fft_band_rms(&held, 192_000, 29_000.0, 31_000.0);
    // Repeating each sample 4 times is a 4-point moving average at 192 kHz, with the
    // magnitude response |sin(4 pi f / 192000) / (4 sin(pi f / 192000))|. That gives
    // 0.7957 at 18 kHz and 0.4900 at 30 kHz, which is a difference of -4.21 dB.
    assert!((db(image / carrier) - (-4.21)).abs() < 0.05, "image {} dB", db(image / carrier));

    let smooth = upsample_sinc(input, 4);
    let carrier = fft_band_rms(&smooth, 192_000, 17_000.0, 19_000.0);
    let image = fft_band_rms(&smooth, 192_000, 29_000.0, 31_000.0);
    assert!(db(image / carrier) < -60.0, "image {} dB", db(image / carrier));
}

#[test]
fn speaker_lowpass_is_3_db_down_at_cutoff() {
    let input = sine(10_000.0, 1.0, 9600, FS);
    let output = speaker_lowpass(input, FS, 10_000.0);
    let level = db(rms(&output[2000..]) * 2f64.sqrt());
    assert!((level - (-3.01)).abs() < 0.1, "{level} dB at cutoff");

    let low = speaker_lowpass(sine(500.0, 1.0, 9600, FS), FS, 10_000.0);
    assert!(db(rms(&low[2000..]) * 2f64.sqrt()).abs() < 0.1);
}

#[test]
fn speaker_nonlinearity_adds_the_expected_harmonics() {
    let size = 1 << 14;
    let frequency = 3000.0;
    let output = speaker_nonlinearity(sine(frequency, 0.5, size, FS), 0.1, 0.2);
    // For A sin(x): a2 A^2 / 2 at twice the frequency, a3 A^3 / 4 at three times.
    let second = fft_band_rms(&output, FS, 5900.0, 6100.0) * 2f64.sqrt();
    let third = fft_band_rms(&output, FS, 8900.0, 9100.0) * 2f64.sqrt();
    assert!((second - 0.1 * 0.25 / 2.0).abs() < 1e-4, "second harmonic {second}");
    assert!((third - 0.2 * 0.125 / 4.0).abs() < 1e-4, "third harmonic {third}");
}

#[test]
fn simple_effects_do_what_they_say() {
    assert_eq!(invert(vec![0.5, -0.25]), [-0.5, 0.25]);
    assert_eq!(clip(gain_db(vec![1.0, -1.0, 0.1], 20.0), 1.0), [1.0, -1.0, 1.0]);
    assert_eq!(dc_offset(vec![0.5], 0.25), [0.75]);
    assert_eq!(pad(vec![1.0], 2, 1), [0.0, 0.0, 1.0, 0.0]);
    let halved = gain_db(vec![1.0], -6.0206);
    assert!((halved[0] - 0.5).abs() < 1e-4);
    assert_eq!(to_f32(&[0.5]), [0.5f32]);
}

#[test]
fn hum_has_the_right_fundamental_and_harmonics() {
    // 68 cycles of the fundamental in the FFT length, about 49.8 Hz, so the
    // fundamental and its harmonics are exactly on FFT bins and do not leak.
    let size = 1 << 16;
    let frequency = 68.0 * FS as f64 / size as f64;
    let output = hum(vec![0.0; size], FS, frequency, -40.0);
    let fundamental = fft_band_rms(&output, FS, 45.0, 55.0) * 2f64.sqrt();
    let second = fft_band_rms(&output, FS, 95.0, 105.0) * 2f64.sqrt();
    assert!((db(fundamental) - (-40.0)).abs() < 0.2, "fundamental {} dB", db(fundamental));
    assert!((second / fundamental - 0.5).abs() < 0.02, "second harmonic ratio");
    let above_harmonics = fft_band_rms(&output, FS, 300.0, 20_000.0);
    assert!(above_harmonics < 1e-5, "{above_harmonics}");
}

#[test]
fn clicks_arrive_at_about_the_requested_rate() {
    let seconds = 20;
    let output = clicks(vec![0.0; FS as usize * seconds], FS, 50.0, -6.0, 3);
    let hits: Vec<f64> = output.iter().cloned().filter(|s| *s != 0.0).collect();
    let expected = 50 * seconds;
    assert!(hits.len().abs_diff(expected) < 100, "{} clicks, expected {expected}", hits.len());
    assert!(hits.iter().all(|s| (s.abs() - 0.5012).abs() < 0.001));
    assert!(hits.iter().any(|s| *s > 0.0) && hits.iter().any(|s| *s < 0.0));
    assert_eq!(output, clicks(vec![0.0; FS as usize * seconds], FS, 50.0, -6.0, 3));
}

#[test]
fn bat_calls_stay_between_40_and_80_khz() {
    let rate = 192_000;
    let output = bat_calls(vec![0.0; 1 << 18], rate, 20.0, -20.0, 5);
    let peak = output.iter().cloned().fold(0.0, |a: f64, b| a.max(b.abs()));
    assert!((db(peak) - (-20.0)).abs() < 0.5, "peak {} dB", db(peak));

    let in_band = fft_band_rms(&output, rate, 40_000.0, 80_000.0);
    let carrier_band = fft_band_rms(&output, rate, 17_000.0, 19_000.0);
    assert!((db(in_band) - db(rms(&output))).abs() < 0.2);
    assert!(db(carrier_band / in_band) < -40.0, "{} dB at 18 kHz", db(carrier_band / in_band));
}
