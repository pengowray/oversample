// SPDX-License-Identifier: GPL-3.0-only OR MIT OR Apache-2.0
//! Measures what the pitch shifters do to bat-like test signals.
//!
//! The pass/fail tests run by default. The report prints a table for each
//! shifter and is run by hand:
//!
//!   cargo test -p oversample-core --release --test pitch_shift_quality report -- --ignored --nocapture
//!
//! With `PS_WAV_DIR=<folder>` the report also writes the x10 output of each
//! shifter for the sweep signal as 16-bit WAV files at 51.2 kHz, to listen to.
//!
//! Three measures, each against the wanted result (every frequency divided by
//! the factor, timing unchanged):
//! - copies: power outside the wanted band, in dB relative to the power in it,
//!   split into under 20 kHz (audible) and 20 kHz and over (seen on the
//!   spectrogram, and audible after resampling folds it down);
//! - envelope: how much of each call's duration has output, and how far a
//!   steady tone's level dips;
//! - tracking: how far the output's frequency is from the input's divided by
//!   the factor, step by step through each call.

use oversample_core::dsp::heterodyne::heterodyne_mix;
use oversample_core::dsp::pitch_shift::{pitch_shift_realtime, PitchFactor};
use realfft::RealFftPlanner;
use std::f64::consts::PI;

const RATE: u32 = 256_000;
/// One call every 60 ms.
const CALL_PERIOD_S: f64 = 0.06;
const CALL_LEN_S: f64 = 0.005;
const F_HI: f64 = 90_000.0;
const F_LO: f64 = 30_000.0;

/// The sweep's frequency at `u` (0 to 1) through a call.
fn sweep_freq(u: f64) -> f64 {
    F_HI * (F_LO / F_HI).powf(u)
}

/// FM sweeps from 90 kHz down to 30 kHz, each 5 ms long with a half-sine
/// envelope, one every 60 ms. `harmonic` adds a second harmonic at that
/// linear level, and `noise` adds white noise at that linear level.
fn sweeps(rate: u32, seconds: f64, harmonic: f64, noise: f64) -> Vec<f32> {
    let n = (rate as f64 * seconds) as usize;
    let period = (CALL_PERIOD_S * rate as f64) as usize;
    let len = (CALL_LEN_S * rate as f64) as usize;
    let mut seed: u32 = 1234567;
    let mut phase = 0.0f64;
    (0..n)
        .map(|i| {
            seed = seed.wrapping_mul(1103515245).wrapping_add(12345) & 0x7fff_ffff;
            let mut s = (seed as f64 / 0x7fff_ffff as f64 * 2.0 - 1.0) * noise;
            let k = i % period;
            if k < len {
                let u = k as f64 / len as f64;
                phase += 2.0 * PI * sweep_freq(u) / rate as f64;
                let env = 0.5 * (PI * u).sin();
                s += env * (phase.sin() + harmonic * (2.0 * phase).sin());
            } else {
                phase = 0.0;
            }
            s as f32
        })
        .collect()
}

fn tone(rate: u32, seconds: f64, freq: f64) -> Vec<f32> {
    let n = (rate as f64 * seconds) as usize;
    (0..n)
        .map(|i| (0.5 * (2.0 * PI * freq * i as f64 / rate as f64).sin()) as f32)
        .collect()
}

/// Power spectrum averaged over Hann windows of `n` samples, half overlapped,
/// skipping `skip` samples at each end. Bin `k` is `k * rate / n` Hz.
fn power_spectrum(x: &[f32], n: usize, skip: usize) -> Vec<f64> {
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let mut buf = vec![0.0f32; n];
    let mut spec = fft.make_output_vec();
    let mut acc = vec![0.0f64; n / 2 + 1];
    let mut pos = skip;
    let mut frames = 0;
    while pos + n + skip <= x.len() {
        for i in 0..n {
            let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos();
            buf[i] = x[pos + i] * w as f32;
        }
        fft.process(&mut buf, &mut spec).unwrap();
        for (a, c) in acc.iter_mut().zip(&spec) {
            *a += (c.re * c.re + c.im * c.im) as f64;
        }
        pos += n / 2;
        frames += 1;
    }
    for a in &mut acc {
        *a /= frames.max(1) as f64;
    }
    acc
}

fn db(ratio: f64) -> f64 {
    10.0 * ratio.max(1e-30).log10()
}

/// Power outside `lo..hi` Hz relative to the power inside it, in dB:
/// (under 20 kHz, 20 kHz and over).
fn copies_db(x: &[f32], rate: u32, lo: f64, hi: f64) -> (f64, f64) {
    let n = 8192;
    let ps = power_spectrum(x, n, 4096);
    let hz = rate as f64 / n as f64;
    let (mut wanted, mut low, mut high) = (0.0, 0.0, 0.0);
    for (k, p) in ps.iter().enumerate() {
        let f = k as f64 * hz;
        if f >= lo && f <= hi {
            wanted += p;
        } else if f < 20_000.0 {
            low += p;
        } else {
            high += p;
        }
    }
    (db(low / wanted), db(high / wanted))
}

/// RMS of consecutive blocks of `block` samples.
fn block_rms(x: &[f32], block: usize) -> Vec<f64> {
    x.chunks_exact(block)
        .map(|c| (c.iter().map(|&s| (s as f64).powi(2)).sum::<f64>() / block as f64).sqrt())
        .collect()
}

/// For a steady tone: (mean level in dB relative to the input's, the deepest
/// dip below the mean in dB), over 0.5 ms blocks away from the ends.
fn tone_level(input: &[f32], output: &[f32], rate: u32) -> (f64, f64) {
    let block = (rate as f64 * 0.0005) as usize;
    let edge = 40;
    let i = block_rms(input, block);
    let o = block_rms(output, block);
    let i = &i[edge..i.len() - edge];
    let o = &o[edge..o.len() - edge];
    let mean_in = i.iter().sum::<f64>() / i.len() as f64;
    let mean_out = o.iter().sum::<f64>() / o.len() as f64;
    let min_out = o.iter().cloned().fold(f64::MAX, f64::min);
    (
        2.0 * db(mean_out / mean_in),
        2.0 * db(min_out / mean_out.max(1e-30)),
    )
}

/// The frequency of the strongest component of `x`, from a zero-padded FFT.
fn peak_freq(x: &[f32], rate: u32) -> f64 {
    let n = 8192;
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let mut buf = vec![0.0f32; n];
    for (i, &s) in x.iter().enumerate() {
        let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / x.len() as f64).cos();
        buf[i] = s * w as f32;
    }
    let mut spec = fft.make_output_vec();
    fft.process(&mut buf, &mut spec).unwrap();
    let k = spec
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.norm_sqr().total_cmp(&b.1.norm_sqr()))
        .map_or(0, |(k, _)| k);
    k as f64 * rate as f64 / n as f64
}

struct CallStats {
    /// Share of each call's duration where the output is within 6 dB of the
    /// input's level.
    covered: f64,
    /// Share of 0.5 ms steps through the calls whose output frequency is
    /// within 10% of the input's divided by the factor.
    on_track: f64,
    /// Output level in the 20 ms after each call, in dB relative to the
    /// call's own level: what is smeared past the call's end.
    after_db: f64,
}

/// Compares each call of the sweep signal with its shifted output.
/// `ratio` is output frequency / input frequency.
fn call_stats(input: &[f32], output: &[f32], rate: u32, ratio: f64) -> CallStats {
    let period = (CALL_PERIOD_S * rate as f64) as usize;
    let len = (CALL_LEN_S * rate as f64) as usize;
    let step = (rate as f64 * 0.0005) as usize;
    // Frequency is read from 2 ms of output centred on each step.
    let half = (rate as f64 * 0.001) as usize;
    let (mut steps, mut covered, mut on_track) = (0usize, 0usize, 0usize);
    let (mut call_pow, mut after_pow) = (0.0f64, 0.0f64);
    let calls = input.len() / period;
    // The first and last calls are left out: they are at the buffer's edges.
    for c in 1..calls.saturating_sub(1) {
        let start = c * period;
        let mut k = step;
        while k + step < len {
            let a = start + k - step / 2;
            let i_rms = block_rms(&input[a..a + step], step)[0];
            let o_rms = block_rms(&output[a..a + step], step)[0];
            steps += 1;
            if o_rms >= 0.5 * i_rms {
                covered += 1;
            }
            let centre = start + k;
            let got = peak_freq(&output[centre - half..centre + half], rate);
            let want = sweep_freq(k as f64 / len as f64) * ratio;
            if (got - want).abs() <= 0.1 * want {
                on_track += 1;
            }
            k += step;
        }
        call_pow += block_rms(&output[start..start + len], len)[0].powi(2);
        let tail = (rate as f64 * 0.02) as usize;
        after_pow += block_rms(&output[start + len..start + len + tail], tail)[0].powi(2);
    }
    CallStats {
        covered: covered as f64 / steps.max(1) as f64,
        on_track: on_track as f64 / steps.max(1) as f64,
        after_db: db(after_pow / call_pow.max(1e-30)),
    }
}

type Shifter = fn(&[f32], PitchFactor) -> Vec<f32>;

fn ps(x: &[f32], f: PitchFactor) -> Vec<f32> {
    pitch_shift_realtime(x, f)
}

/// The pitch shifter as it was before it was rewritten (v0.5.163), kept here
/// so the report gives before and after figures. It resampled by linear
/// interpolation and overlap-added 2048-sample windows half overlapped.
fn pitch_shift_before(samples: &[f32], factor: PitchFactor) -> Vec<f32> {
    if samples.is_empty() || factor.is_bypass() {
        return samples.to_vec();
    }
    let abs_factor = factor.magnitude();
    let shift_up = factor.is_up();
    let step = if shift_up {
        abs_factor
    } else {
        1.0 / abs_factor
    };
    let resampled_len = (samples.len() as f64 / step) as usize;
    let resampled: Vec<f32> = (0..resampled_len)
        .map(|i| {
            let src_pos = i as f64 * step;
            let idx = src_pos as usize;
            let frac = (src_pos - idx as f64) as f32;
            let s0 = samples[idx.min(samples.len() - 1)];
            let s1 = samples[(idx + 1).min(samples.len() - 1)];
            s0 + frac * (s1 - s0)
        })
        .collect();
    let window_size: usize = 2048;
    let synthesis_hop = window_size / 2;
    let analysis_hop = if shift_up {
        (synthesis_hop as f64 / abs_factor).max(1.0) as usize
    } else {
        (synthesis_hop as f64 * abs_factor) as usize
    };
    let out_len = samples.len();
    let mut output = vec![0.0f32; out_len];
    let mut window_sum = vec![0.0f32; out_len];
    let hann: Vec<f32> = (0..window_size)
        .map(|i| {
            let x = std::f32::consts::PI * i as f32 / window_size as f32;
            x.sin().powi(2)
        })
        .collect();
    let mut read_pos = 0usize;
    let mut write_pos = 0usize;
    while read_pos + window_size <= resampled.len() && write_pos + window_size <= out_len {
        for i in 0..window_size {
            output[write_pos + i] += resampled[read_pos + i] * hann[i];
            window_sum[write_pos + i] += hann[i];
        }
        read_pos += analysis_hop;
        write_pos += synthesis_hop;
    }
    for i in 0..out_len {
        if window_sum[i] > 0.001 {
            output[i] /= window_sum[i];
        }
    }
    output
}

fn ps_before(x: &[f32], f: PitchFactor) -> Vec<f32> {
    pitch_shift_before(x, f)
}

/// Pitch shift, then the output shift playback applies when "shift" is set:
/// a heterodyne mix down by 1 kHz with the low-pass playback uses.
fn ps_shifted(x: &[f32], f: PitchFactor) -> Vec<f32> {
    let cutoff = (128_000.0 / f.magnitude() + 5_000.0f64)
        .max(20_000.0)
        .min(RATE as f64 * 0.45);
    heterodyne_mix(&pitch_shift_realtime(x, f), RATE, 1_000.0, cutoff)
}

fn write_wav(path: &std::path::Path, samples: &[f32], rate: u32) {
    let mut bytes = Vec::with_capacity(44 + samples.len() * 2);
    let data_len = (samples.len() * 2) as u32;
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&rate.to_le_bytes());
    bytes.extend_from_slice(&(rate * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for &s in samples {
        let v = (s.clamp(-1.0, 1.0) * 32767.0).round() as i16;
        bytes.extend_from_slice(&v.to_le_bytes());
    }
    std::fs::write(path, bytes).unwrap();
}

fn report_for(name: &str, shift: Shifter, shift_hz: f64) {
    println!("\n== {name} ==");
    println!(
        "factor | sweep copies <20k / >=20k dB | covered | on track | after call dB | tone 40k: copies <20k / >=20k dB, level dB, dip dB | worst tone dip dB (20..100k)"
    );
    let sweep_in = sweeps(RATE, 1.0, 0.0, 0.0);
    for factor in [2.0, 4.0, 8.0, 10.0, 16.0] {
        let f = PitchFactor::down(factor);
        let out = shift(&sweep_in, f);
        let margin = 1_500.0;
        let (lo, hi) = (
            (F_LO / factor - shift_hz - margin).max(0.0),
            F_HI / factor - shift_hz + margin,
        );
        let (c_lo, c_hi) = copies_db(&out, RATE, lo, hi);
        // The frequency check reads the unshifted output only.
        let stats = call_stats(&sweep_in, &out, RATE, 1.0 / factor);
        let tone_in = tone(RATE, 0.5, 40_000.0);
        let tone_out = shift(&tone_in, f);
        let want = 40_000.0 / factor - shift_hz;
        let (t_lo, t_hi) = copies_db(&tone_out, RATE, want - 300.0, want + 300.0);
        let (level, dip) = tone_level(&tone_in, &tone_out, RATE);
        let mut worst_dip = 0.0f64;
        for khz in (20..=100).step_by(7) {
            let t_in = tone(RATE, 0.3, khz as f64 * 1000.0 + 137.0);
            let t_out = shift(&t_in, f);
            worst_dip = worst_dip.min(tone_level(&t_in, &t_out, RATE).1);
        }
        println!(
            "x{factor:<5} | {c_lo:6.1} / {c_hi:6.1} | {:5.0}% | {:5.0}% | {:6.1} | {t_lo:6.1} / {t_hi:6.1}, {level:5.1}, {dip:6.1} | {worst_dip:6.1}",
            stats.covered * 100.0,
            stats.on_track * 100.0,
            stats.after_db,
        );
    }
}

#[test]
#[ignore]
fn report() {
    report_for("pitch shift (PS) before the rewrite", ps_before, 0.0);
    report_for("pitch shift (PS), no output shift", ps, 0.0);
    // The output shift is a ring modulator: it leaves each frequency at
    // both f - 1 kHz and f + 1 kHz, 6 dB down each, which the copies and
    // on-track columns of this table show.
    report_for("pitch shift (PS) + 1 kHz output shift", ps_shifted, 1_000.0);

    // Other cases at one factor each.
    println!("\n== other cases ==");
    for (label, shift) in [("PS before", ps_before as Shifter), ("PS", ps as Shifter)] {
        // A sweep with a second harmonic 12 dB down, over noise 40 dB down.
        let x = sweeps(RATE, 1.0, 0.25, 0.005);
        let out = shift(&x, PitchFactor::down(10.0));
        let s = call_stats(&x, &out, RATE, 0.1);
        println!(
            "{label} x10 sweep + harmonic + noise: covered {:.0}%, on track {:.0}%, after call {:.1} dB",
            s.covered * 100.0,
            s.on_track * 100.0,
            s.after_db
        );
        // A 44.1 kHz file at x2: a 6 kHz tone.
        let t_in = tone(44_100, 1.0, 6_000.0);
        let t_out = shift(&t_in, PitchFactor::down(2.0));
        let (lo, hi) = copies_db(&t_out, 44_100, 2_700.0, 3_300.0);
        let (level, dip) = tone_level(&t_in, &t_out, 44_100);
        println!(
            "{label} x2 at 44.1 kHz, 6 kHz tone: copies {lo:.1} / {hi:.1} dB, level {level:.1} dB, dip {dip:.1} dB"
        );
        // Shift up x4: a 5 kHz tone with a 50 kHz tone that has nowhere to go
        // (200 kHz is over Nyquist) and must not fold back.
        let mut up_in = tone(RATE, 0.5, 5_000.0);
        for (a, b) in up_in.iter_mut().zip(tone(RATE, 0.5, 50_000.0)) {
            *a += b;
        }
        let up_out = shift(&up_in, PitchFactor::up(4.0));
        let (lo, hi) = copies_db(&up_out, RATE, 19_500.0, 20_500.0);
        println!("{label} up x4, 5 kHz + 50 kHz tones: other than 20 kHz {lo:.1} / {hi:.1} dB");
    }

    // Time for one playback chunk (96 000 samples plus warm-up and overlap).
    println!("\n== time per 108 288-sample chunk, native ==");
    let chunk = &sweeps(RATE, 1.0, 0.0, 0.005)[..96_000 + 4096 + 8192];
    for factor in [2.0, 10.0, 16.0, -4.0] {
        for (label, shift) in [("PS before", ps_before as Shifter), ("PS", ps as Shifter)] {
            let f = PitchFactor::from_signed(factor);
            let t = std::time::Instant::now();
            let reps = 20;
            for _ in 0..reps {
                std::hint::black_box(shift(std::hint::black_box(chunk), f));
            }
            println!(
                "{label} factor {factor}: {:.2} ms",
                t.elapsed().as_secs_f64() * 1000.0 / reps as f64
            );
        }
    }

    if let Ok(dir) = std::env::var("PS_WAV_DIR") {
        let dir = std::path::PathBuf::from(dir);
        std::fs::create_dir_all(&dir).unwrap();
        let x = sweeps(RATE, 3.0, 0.25, 0.005);
        let rate = oversample_core::dsp::filters::decimated_rate(RATE, 51_200);
        for (label, shift) in [
            ("ps-before", ps_before as Shifter),
            ("ps-after", ps as Shifter),
        ] {
            let out = shift(&x, PitchFactor::down(10.0));
            let out = oversample_core::dsp::filters::decimate(&out, RATE, 51_200);
            let path = dir.join(format!("{label}-x10-sweeps.wav"));
            write_wav(&path, &out, rate);
            println!("wrote {}", path.display());
        }
    }
}

// ── Pass/fail tests ──────────────────────────────────────────────────────────
// Pass levels are set from the report: each leaves a few dB over what the
// shifter measures, and each fails for the shifter as it was before.

/// A sweep comes out with no copies at other frequencies: everything outside
/// the wanted band is at least 40 dB under it. (Before: -15 to -33 dB.)
#[test]
fn a_shifted_sweep_has_no_copies() {
    let x = sweeps(RATE, 0.5, 0.0, 0.0);
    for factor in [2.0, 4.0, 10.0, 16.0] {
        let band = ((F_LO / factor - 1_500.0).max(0.0), F_HI / factor + 1_500.0);
        let out = pitch_shift_realtime(&x, PitchFactor::down(factor));
        let (lo, hi) = copies_db(&out, RATE, band.0, band.1);
        assert!(lo < -40.0 && hi < -40.0, "x{factor}: {lo:.1} / {hi:.1} dB");
        let before = pitch_shift_before(&x, PitchFactor::down(factor));
        let (lo, hi) = copies_db(&before, RATE, band.0, band.1);
        assert!(lo.max(hi) > -40.0, "the old shifter should fail this");
    }
}

/// Every part of each call is played, at the call's frequency divided by the
/// factor. (Before: 0 to 38% of a call had output, 0 to 25% on frequency.)
#[test]
fn a_shifted_call_has_no_gaps_and_keeps_its_shape() {
    let x = sweeps(RATE, 0.5, 0.25, 0.005);
    for factor in [2.0, 4.0, 10.0, 16.0] {
        let out = pitch_shift_realtime(&x, PitchFactor::down(factor));
        let s = call_stats(&x, &out, RATE, 1.0 / factor);
        assert!(s.covered >= 0.85, "x{factor}: covered {:.2}", s.covered);
        assert!(s.on_track >= 0.85, "x{factor}: on track {:.2}", s.on_track);
        assert!(s.after_db < -30.0, "x{factor}: after {:.1} dB", s.after_db);
    }
}

/// A steady tone keeps its level and has no dips, at any frequency.
/// (Before: dips of up to 11 dB, depending on the frequency.)
#[test]
fn a_shifted_tone_keeps_a_steady_level() {
    for factor in [2.0, 4.0, 10.0, 16.0] {
        for khz in (20..=100).step_by(7) {
            let x = tone(RATE, 0.2, khz as f64 * 1000.0 + 137.0);
            let out = pitch_shift_realtime(&x, PitchFactor::down(factor));
            let (level, dip) = tone_level(&x, &out, RATE);
            assert!(
                level.abs() < 0.5,
                "x{factor} {khz} kHz: level {level:.2} dB"
            );
            assert!(dip > -3.0, "x{factor} {khz} kHz: dip {dip:.2} dB");
        }
    }
}

/// Shifting up: a tone that would land over Nyquist is removed and does not
/// fold back. (Before: it folded back at full level.)
#[test]
fn shifting_up_does_not_fold_back() {
    let mut x = tone(RATE, 0.3, 5_000.0);
    for (a, b) in x.iter_mut().zip(tone(RATE, 0.3, 50_000.0)) {
        *a += b;
    }
    let out = pitch_shift_realtime(&x, PitchFactor::up(4.0));
    let (lo, hi) = copies_db(&out, RATE, 19_500.0, 20_500.0);
    assert!(lo < -40.0 && hi < -40.0, "{lo:.1} / {hi:.1} dB");
}

/// Magnitudes of an STFT (Hann, `n` samples, hop `hop`) of `x`, as power in
/// 2 kHz bands for each frame.
fn band_frames(x: &[f32], n: usize, hop: usize) -> Vec<Vec<f64>> {
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(n);
    let mut buf = vec![0.0f32; n];
    let mut spec = fft.make_output_vec();
    let per_band = (2_000.0 / (RATE as f64 / n as f64)).round() as usize;
    let mut frames = Vec::new();
    let mut pos = 0;
    while pos + n <= x.len() {
        for i in 0..n {
            let w = 0.5 - 0.5 * (2.0 * PI * i as f64 / n as f64).cos();
            buf[i] = x[pos + i] * w as f32;
        }
        fft.process(&mut buf, &mut spec).unwrap();
        frames.push(
            spec.chunks(per_band)
                .map(|c| c.iter().map(|v| v.norm_sqr() as f64).sum())
                .collect(),
        );
        pos += hop;
    }
    frames
}

/// A spectrogram tile is shifted on its own, with 8192 samples of the audio
/// before it and one grain after it. Its picture matches the picture of the
/// same samples cut from the whole recording shifted in one go, so tiles
/// join without a seam. The tile boundaries here fall inside calls.
#[test]
fn a_tile_shifted_alone_matches_the_whole_recording() {
    use oversample_core::dsp::pitch_shift::GRAIN;
    let x = sweeps(RATE, 1.0, 0.25, 0.005);
    let (fft, hop) = (1024, 256);
    let tile_len = 256 * hop + fft;
    let pre = 8192;
    for factor in [2.0, 10.0] {
        let f = PitchFactor::down(factor);
        let whole = pitch_shift_realtime(&x, f);
        // A call starts every 15 360 samples. These tiles start 384 and 896
        // samples into a call.
        for start in [15_360 + 384usize, 4 * 15_360 + 896] {
            let from = start - pre;
            let to = (start + tile_len + GRAIN).min(x.len());
            let piece = pitch_shift_realtime(&x[from..to], f);
            let tile = &piece[pre..pre + tile_len];
            let a = band_frames(tile, fft, hop);
            let b = band_frames(&whole[start..start + tile_len], fft, hop);
            let (mut diff, mut total) = (0.0, 0.0);
            for (fa, fb) in a.iter().zip(&b) {
                for (pa, pb) in fa.iter().zip(fb) {
                    diff += (pa - pb).abs();
                    total += pb;
                }
            }
            // The first and last columns are where a seam would be.
            let power = |frames: &[Vec<f64>], i: usize| frames[i].iter().sum::<f64>();
            for i in [0, 1, a.len() - 2, a.len() - 1] {
                let (pa, pb) = (power(&a, i), power(&b, i));
                let d = db(pa / pb.max(1e-30));
                assert!(
                    d.abs() < 1.5 || pb < 1e-6,
                    "x{factor} tile at {start}, column {i}: {d:.2} dB"
                );
            }
            println!("x{factor} tile at {start}: difference {:.3}", diff / total);
            assert!(
                diff / total < 0.15,
                "x{factor} tile at {start}: {}",
                diff / total
            );
        }
    }
}
