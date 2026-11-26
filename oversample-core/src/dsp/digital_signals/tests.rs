use super::*;
use crate::dsp::call_detect::test_util::{add, faded_chirp, noise, sine, Butterworth};
use crate::dsp::filters::butterworth_bandpass;
use std::f64::consts::{PI, TAU};

const HISS: f32 = 0.003;

fn secs(rate: u32, s: f64) -> usize {
    (s * rate as f64) as usize
}

/// Deterministic numbers in [0, 1).
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        ((self.next() * n as f64) as usize).min(n - 1)
    }
}

/// A band-limited pulse wave (`duty` 0.5 is a square wave) with its
/// fundamental at amplitude `amp`, `len_s` long.
fn pulse_wave(rate: u32, f0: f64, duty: f64, amp: f32, len_s: f64) -> Vec<f32> {
    let n = secs(rate, len_s);
    let top = 0.45 * rate as f64;
    let c1 = (PI * duty).sin();
    let harmonics: Vec<(f64, f64)> = (1..)
        .map(|h| h as f64)
        .take_while(|h| h * f0 < top)
        .map(|h| (h, (h * PI * duty).sin() / h / c1))
        .filter(|(_, c)| c.abs() > 1e-6)
        .collect();
    (0..n)
        .map(|i| {
            let t = i as f64 / rate as f64;
            let v: f64 = harmonics
                .iter()
                .map(|&(h, c)| c * (TAU * h * f0 * t).cos())
                .sum();
            (amp as f64 * v) as f32
        })
        .collect()
}

/// A steady tone with harmonics at `levels[n - 2]` times the fundamental.
fn harmonic_tone(rate: u32, f0: f64, levels: &[f64], amp: f32, len_s: f64) -> Vec<f32> {
    let n = secs(rate, len_s);
    (0..n)
        .map(|i| {
            let t = i as f64 / rate as f64;
            let mut v = (TAU * f0 * t).sin();
            for (j, &a) in levels.iter().enumerate() {
                let h = (j + 2) as f64;
                if h * f0 < 0.45 * rate as f64 {
                    v += a * (TAU * h * f0 * t + 0.3 * h).sin();
                }
            }
            (amp as f64 * v) as f32
        })
        .collect()
}

/// Root-raised-cosine pulse at `t` symbols from its middle.
fn rrc_pulse(t: f64, alpha: f64) -> f64 {
    if t.abs() < 1e-9 {
        return 1.0 - alpha + 4.0 * alpha / PI;
    }
    let edge = 1.0 / (4.0 * alpha);
    if (t.abs() - edge).abs() < 1e-9 {
        return alpha / 2f64.sqrt()
            * ((1.0 + 2.0 / PI) * (PI / (4.0 * alpha)).sin()
                + (1.0 - 2.0 / PI) * (PI / (4.0 * alpha)).cos());
    }
    ((PI * t * (1.0 - alpha)).sin() + 4.0 * alpha * t * (PI * t * (1.0 + alpha)).cos())
        / (PI * t * (1.0 - (4.0 * alpha * t).powi(2)))
}

/// QPSK (`order` 4), 8-PSK (8) or square 16-QAM (16) on a carrier with
/// root-raised-cosine pulses, at RMS `amp`, `len_s` long.
fn qam(
    rate: u32,
    fc: f64,
    rs: f64,
    alpha: f64,
    order: usize,
    amp: f32,
    len_s: f64,
    seed: u64,
) -> Vec<f32> {
    let n = secs(rate, len_s);
    let span = 8i64;
    let n_sym = (len_s * rs) as i64 + 2 * span + 2;
    let mut rng = Rng(seed.max(1));
    let syms: Vec<(f64, f64)> = (0..n_sym)
        .map(|_| match order {
            2 => (if rng.below(2) == 0 { 1.0 } else { -1.0 }, 0.0),
            4 => {
                let k = rng.below(4) as f64;
                let a = PI / 4.0 + k * PI / 2.0;
                (a.cos(), a.sin())
            }
            8 => {
                let a = rng.below(8) as f64 * PI / 4.0;
                (a.cos(), a.sin())
            }
            _ => {
                let lv = [-3.0, -1.0, 1.0, 3.0];
                (lv[rng.below(4)] / 5f64.sqrt(), lv[rng.below(4)] / 5f64.sqrt())
            }
        })
        .collect();
    let mut x: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f64 / rate as f64;
            let u = t * rs;
            let k0 = u.floor() as i64;
            let (mut re, mut im) = (0.0, 0.0);
            for k in k0 - span..=k0 + span {
                let p = rrc_pulse(u - k as f64, alpha);
                let (a, b) = syms[(k + span).max(0) as usize];
                re += a * p;
                im += b * p;
            }
            let ph = TAU * fc * t;
            (re * ph.cos() - im * ph.sin()) as f32
        })
        .collect();
    let rms = (x.iter().map(|v| (v * v) as f64).sum::<f64>() / n as f64).sqrt() as f32;
    for v in &mut x {
        *v *= amp / rms;
    }
    x
}

/// Continuous-phase FSK over `tones`, a random tone per symbol.
fn fsk(rate: u32, tones: &[f64], baud: f64, amp: f32, len_s: f64, seed: u64) -> Vec<f32> {
    let n = secs(rate, len_s);
    let per = rate as f64 / baud;
    let mut rng = Rng(seed.max(1));
    let mut phase = 0.0f64;
    let mut tone = tones[0];
    let mut next = 0.0f64;
    (0..n)
        .map(|i| {
            if i as f64 >= next {
                tone = tones[rng.below(tones.len())];
                next += per;
            }
            phase += TAU * tone / rate as f64;
            // Short fades at the ends.
            let edge = (i.min(n - 1 - i) as f64 / (0.005 * rate as f64)).min(1.0);
            (amp as f64 * edge * phase.sin()) as f32
        })
        .collect()
}

fn hiss(rate: u32, len_s: f64) -> Vec<f32> {
    noise(HISS, secs(rate, len_s), 7)
}

/// Fades a piece in and out over `ms`.
fn fade(mut x: Vec<f32>, rate: u32, ms: f64) -> Vec<f32> {
    let r = ((ms / 1000.0) * rate as f64) as usize;
    let n = x.len();
    for i in 0..r.min(n / 2) {
        let w = (i as f32 / r as f32).min(1.0);
        x[i] *= w;
        x[n - 1 - i] *= w;
    }
    x
}

fn scene(rate: u32, len_s: f64, pieces: &[(f64, Vec<f32>)]) -> Vec<f32> {
    let mut x = hiss(rate, len_s);
    for (at, p) in pieces {
        add(&mut x, secs(rate, *at), p);
    }
    x
}

fn only(events: &[DigitalEvent], kind: DigitalKind) -> Vec<&DigitalEvent> {
    events.iter().filter(|e| e.kind == kind).collect()
}

fn close(a: f32, b: f32, frac: f32) -> bool {
    (a - b).abs() <= frac * b
}

#[test]
fn square_waves_and_pwm_are_found() {
    for (rate, f0, duty) in [
        (48_000, 1_000.0, 0.5),
        (48_000, 1_000.0, 0.25),
        (256_000, 25_000.0, 0.5),
        (384_000, 25_000.0, 0.5),
        (384_000, 25_000.0, 0.25),
        (384_000, 3_100.0, 0.5),
    ] {
        let sq = fade(pulse_wave(rate, f0, duty, 0.05, 2.0), rate, 5.0);
        let x = scene(rate, 4.0, &[(1.0, sq)]);
        let ev = scan_digital(&x, rate);
        let sq = only(&ev, DigitalKind::SquareWave);
        assert_eq!(sq.len(), 1, "{rate} Hz, {f0} Hz duty {duty}: {ev:?}");
        let e = sq[0];
        assert!(
            close(e.fundamental_hz.unwrap(), f0 as f32, 0.01),
            "{rate}: {e:?}"
        );
        assert!((e.start_s - 1.0).abs() < 0.1, "{rate}: {e:?}");
        assert!((e.end_s - 3.0).abs() < 0.1, "{rate}: {e:?}");
        assert!(e.hi_hz >= 3.0 * f0 as f32, "{rate}: {e:?}");
        assert!(only(&ev, DigitalKind::Modulated).is_empty(), "{rate}: {ev:?}");
    }
}

#[test]
fn qam_and_psk_carriers_are_found() {
    for (rate, fc, rs, order, want) in [
        (48_000, 12_000.0, 1_000.0, 16, "QAM"),
        (48_000, 12_000.0, 4_000.0, 4, "PSK"),
        (48_000, 12_000.0, 8_000.0, 16, "QAM"),
        (48_000, 12_000.0, 2_000.0, 2, "PSK"),
        (384_000, 40_000.0, 10_000.0, 16, "QAM"),
        (384_000, 40_000.0, 2_000.0, 4, "PSK"),
        (384_000, 40_000.0, 1_000.0, 4, "PSK"),
        (384_000, 40_000.0, 5_000.0, 16, "QAM"),
        (256_000, 60_000.0, 5_000.0, 8, "PSK"),
    ] {
        let sig = qam(rate, fc, rs, 0.35, order, 0.05, 2.5, 3);
        let x = scene(rate, 4.5, &[(1.0, fade(sig, rate, 5.0))]);
        let ev = scan_digital(&x, rate);
        let m = only(&ev, DigitalKind::Modulated);
        assert_eq!(m.len(), 1, "{rate} Hz, {order} at {rs} Bd: {ev:?}\n{}", explain(&x, rate));
        let e = m[0];
        assert!(close(e.carrier_hz.unwrap(), fc as f32, 0.02), "{rate}: {e:?}");
        let found = e.symbol_rate_hz.expect("symbol rate");
        assert!(close(found, rs as f32, 0.03), "{rate}: {e:?}");
        assert_eq!(e.modulation, Some(want), "{rate}: {e:?}\n{}", explain(&x, rate));
        assert!((e.start_s - 1.0).abs() < 0.1, "{rate}: {e:?}");
        assert!((e.end_s - 3.5).abs() < 0.1, "{rate}: {e:?}");
        assert!(e.lo_hz < (fc - 0.4 * rs) as f32 && e.hi_hz > (fc + 0.4 * rs) as f32);
        assert!(only(&ev, DigitalKind::SquareWave).is_empty(), "{rate}: {ev:?}");
    }
}

#[test]
fn qam_in_noise_is_found() {
    // About 12 dB over the noise in its band.
    for (rate, fc, rs, order) in [(48_000, 12_000.0, 2_000.0, 16), (384_000, 40_000.0, 4_000.0, 4)] {
        let band_share = 1.35 * rs / (rate as f64 / 2.0);
        let noise_amp = 0.05f32;
        let noise_power = (noise_amp * noise_amp / 3.0) as f64 * band_share;
        let amp = (noise_power * 10f64.powf(1.2)).sqrt() as f32;
        let mut x = noise(noise_amp, secs(rate, 4.5), 17);
        add(&mut x, secs(rate, 1.0), &fade(qam(rate, fc, rs, 0.35, order, amp, 2.5, 6), rate, 5.0));
        let ev = scan_digital(&x, rate);
        let m = only(&ev, DigitalKind::Modulated);
        assert_eq!(m.len(), 1, "{rate} Hz: {ev:?}\n{}", explain(&x, rate));
        let e = m[0];
        assert!(close(e.carrier_hz.unwrap(), fc as f32, 0.02), "{e:?}");
        assert!(close(e.symbol_rate_hz.expect("symbol rate"), rs as f32, 0.03), "{e:?}");
        assert!((e.start_s - 1.0).abs() < 0.15 && (e.end_s - 3.5).abs() < 0.15, "{e:?}");
    }
}

#[test]
fn fsk_burst_is_found() {
    for (rate, tones, baud) in [
        (48_000, vec![11_000.0, 11_800.0], 50.0),
        (384_000, vec![38_000.0, 39_000.0, 40_000.0, 41_000.0], 100.0),
    ] {
        let sig = fsk(rate, &tones, baud, 0.05, 1.5, 9);
        let x = scene(rate, 3.5, &[(1.2, sig)]);
        let ev = scan_digital(&x, rate);
        let m = only(&ev, DigitalKind::Modulated);
        assert_eq!(m.len(), 1, "{rate}: {ev:?}\n{}", explain(&x, rate));
        let e = m[0];
        assert_eq!(e.modulation, Some("FSK"), "{e:?}");
        let mid = 0.5 * (tones[0] + tones[tones.len() - 1]) as f32;
        assert!(close(e.carrier_hz.unwrap(), mid, 0.02), "{e:?}");
        assert!((e.start_s - 1.2).abs() < 0.1, "{e:?}");
        assert!((e.end_s - 2.7).abs() < 0.1, "{e:?}");
        let r = e.symbol_rate_hz.expect("symbol rate");
        assert!(close(r, baud as f32, 0.05), "{e:?}");
    }
}

/// Repeated bat calls: FM sweeps, or constant-frequency calls with a second
/// harmonic.
fn bat_calls(rate: u32, len_s: f64, cf: bool) -> Vec<f32> {
    let fs = rate as f32;
    let mut x = vec![0.0f32; secs(rate, len_s)];
    let mut t = 0.05;
    while t + 0.1 < len_s {
        let call = if cf {
            let n = secs(rate, 0.06);
            let mut c = sine(fs, 83_000.0, 0.3, n);
            add(&mut c, 0, &sine(fs, 166_000.0, 0.05, n));
            fade(c, rate, 3.0)
        } else {
            faded_chirp(fs, 90_000.0, 30_000.0, 0.3, secs(rate, 0.004))
        };
        add(&mut x, secs(rate, t), &call);
        t += if cf { 0.09 } else { 0.1 };
    }
    x
}

/// Cricket-like chirps: 3 pulses of a 4.5 kHz tone with weak harmonics, 3
/// chirps a second; or a continuous trill of 50 pulses a second.
fn cricket(rate: u32, len_s: f64, trill: bool) -> Vec<f32> {
    let mut x = vec![0.0f32; secs(rate, len_s)];
    let pulse = fade(
        harmonic_tone(rate, 4_500.0, &[0.05, 0.03], 0.2, if trill { 0.012 } else { 0.015 }),
        rate,
        2.0,
    );
    let mut t = 0.0;
    while t + 0.1 < len_s {
        if trill {
            add(&mut x, secs(rate, t), &pulse);
            t += 0.02;
        } else {
            for p in 0..3 {
                add(&mut x, secs(rate, t + p as f64 * 0.03), &pulse);
            }
            t += 0.33;
        }
    }
    x
}

fn brown(rate: u32, len_s: f64) -> Vec<f32> {
    let w = noise(1.0, secs(rate, len_s), 21);
    let mut acc = 0.0f32;
    let mut x: Vec<f32> = w
        .iter()
        .map(|&v| {
            acc = 0.999 * acc + v;
            acc
        })
        .collect();
    let peak = x.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    for v in &mut x {
        *v *= 0.3 / peak;
    }
    x
}

/// The scanner's trace for `x`, for failure messages.
fn explain(x: &[f32], rate: u32) -> String {
    let mut s = DigitalScanner::new(rate);
    s.set_trace(true);
    s.push(x);
    let t = s.take_trace();
    t.iter()
        .filter(|l| !l.contains("reason: \"shape\"") || std::env::var("TRACE_ALL").is_ok())
        .cloned()
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_quiet(name: &str, rate: u32, x: &[f32]) {
    let ev = scan_digital(x, rate);
    assert!(ev.is_empty(), "{name} at {rate} Hz: {ev:?}\n{}", explain(x, rate));
}

#[test]
fn tones_and_natural_harmonics_are_quiet() {
    for rate in [48_000, 256_000, 384_000] {
        let fs = rate as f32;
        let n = secs(rate, 3.0);
        let mut hum = sine(fs, 50.0, 0.1, n);
        add(&mut hum, 0, &sine(fs, 1_000.0, 0.05, n));
        add(&mut hum, 0, &hiss(rate, 3.0));
        assert_quiet("hum and a tone", rate, &hum);
        // Harmonics at 1/n^2, evens included, loud.
        let lv: Vec<f64> = (2..30).map(|h| 1.0 / (h * h) as f64).collect();
        let mut nat = harmonic_tone(rate, 700.0, &lv, 0.3, 3.0);
        add(&mut nat, 0, &hiss(rate, 3.0));
        assert_quiet("1/n^2 harmonics", rate, &nat);
        // A mildly clipped tone.
        let mut clip: Vec<f32> = sine(fs, 2_000.0, 0.5, n)
            .iter()
            .map(|v| v.clamp(-0.45, 0.45))
            .collect();
        add(&mut clip, 0, &hiss(rate, 3.0));
        assert_quiet("mildly clipped tone", rate, &clip);
    }
}

#[test]
fn mains_buzz_is_left_to_the_hum_finder() {
    // 50 Hz with odd harmonics at 1/n: below the lowest fundamental looked
    // for, and its harmonics too close together to be lines.
    let rate = 48_000;
    let lv: Vec<f64> = (2..60)
        .map(|h| if h % 2 == 1 { 1.0 / h as f64 } else { 0.0 })
        .collect();
    let mut buzz = harmonic_tone(rate, 50.0, &lv, 0.1, 3.0);
    add(&mut buzz, 0, &hiss(rate, 3.0));
    assert_quiet("mains buzz", rate, &buzz);
}

#[test]
fn animal_like_sounds_and_noise_are_quiet() {
    for rate in [256_000, 384_000] {
        let mut x = bat_calls(rate, 3.0, false);
        add(&mut x, 0, &hiss(rate, 3.0));
        assert_quiet("FM bat calls", rate, &x);
        let mut x = bat_calls(rate, 3.0, true);
        add(&mut x, 0, &hiss(rate, 3.0));
        assert_quiet("CF bat calls", rate, &x);
    }
    for rate in [48_000, 384_000] {
        for trill in [false, true] {
            let mut x = cricket(rate, 3.0, trill);
            add(&mut x, 0, &hiss(rate, 3.0));
            assert_quiet(if trill { "cricket trill" } else { "cricket chirps" }, rate, &x);
        }
        assert_quiet("white noise", rate, &noise(0.3, secs(rate, 3.0), 5));
        assert_quiet("brown noise", rate, &brown(rate, 3.0));
        // Steady band-limited noise, like a cicada or katydid chorus.
        let band = butterworth_bandpass(&noise(0.3, secs(rate, 3.0), 8), rate, 6_000.0, 9_000.0);
        let mut x = band.clone();
        add(&mut x, 0, &hiss(rate, 3.0));
        assert_quiet("band-pass noise", rate, &x);
        // The same noise pulsed 400 times a second.
        let mut x: Vec<f32> = band
            .iter()
            .enumerate()
            .map(|(i, &v)| {
                let t = i as f64 / rate as f64;
                let ph = (t * 400.0).fract();
                if ph < 0.4 {
                    v * (PI * ph / 0.4).sin() as f32
                } else {
                    0.0
                }
            })
            .collect();
        add(&mut x, 0, &hiss(rate, 3.0));
        assert_quiet("pulsed band-pass noise", rate, &x);
    }
}

#[test]
fn mixed_scene() {
    let rate = 384_000;
    let len = 6.0;
    let mut x = scene(
        rate,
        len,
        &[
            (0.5, fade(pulse_wave(rate, 5_000.0, 0.5, 0.03, 4.0), rate, 5.0)),
            (1.5, fade(qam(rate, 120_000.0, 8_000.0, 0.35, 16, 0.03, 3.0, 5), rate, 5.0)),
        ],
    );
    add(&mut x, 0, &bat_calls(rate, len, false));
    let ev = scan_digital(&x, rate);
    let sq = only(&ev, DigitalKind::SquareWave);
    let m = only(&ev, DigitalKind::Modulated);
    assert_eq!((sq.len(), m.len()), (1, 1), "{ev:?}");
    assert!(close(sq[0].fundamental_hz.unwrap(), 5_000.0, 0.01), "{ev:?}");
    assert!(close(m[0].carrier_hz.unwrap(), 120_000.0, 0.02), "{ev:?}");
    assert!((sq[0].start_s - 0.5).abs() < 0.1 && (sq[0].end_s - 4.5).abs() < 0.1, "{ev:?}");
    assert!((m[0].start_s - 1.5).abs() < 0.1 && (m[0].end_s - 4.5).abs() < 0.1, "{ev:?}");
}

#[test]
fn pieces_give_the_same_events() {
    let rate = 96_000;
    let x = scene(
        rate,
        5.0,
        &[
            (0.7, fade(pulse_wave(rate, 2_000.0, 0.3, 0.03, 3.0), rate, 5.0)),
            (1.3, fade(qam(rate, 30_000.0, 3_000.0, 0.35, 4, 0.03, 2.5, 2), rate, 5.0)),
        ],
    );
    let whole = scan_digital(&x, rate);
    assert_eq!(whole.len(), 2, "{whole:?}");
    let mut s = DigitalScanner::new(rate);
    for piece in x.chunks(7_777) {
        s.push(piece);
    }
    assert_eq!(s.finish(), whole);
}

#[test]
fn gap_keeps_times() {
    let rate = 48_000;
    let x = scene(rate, 3.0, &[(0.5, pulse_wave(rate, 1_500.0, 0.5, 0.05, 2.0))]);
    let mut s = DigitalScanner::new(rate);
    s.push(&x[..secs(rate, 0.8)]);
    s.gap(secs(rate, 10.0) as u64);
    s.push(&x[secs(rate, 0.8)..]);
    let ev = s.finish();
    // The part after the gap starts at 10.8 s; the part before it (0.3 s)
    // is too short to report.
    assert_eq!(ev.len(), 1, "{ev:?}");
    assert!((ev[0].start_s - 10.8).abs() < 0.05, "{ev:?}");
    assert!((ev[0].end_s - 12.5).abs() < 0.1, "{ev:?}");
}

#[test]
fn burst_across_a_block_edge_is_found() {
    // At 256 kHz a block is 1.02 s; a 0.6 s burst over the first edge.
    let rate = 256_000;
    let block_s = 32.0 * 8192.0 / rate as f64;
    for (start, sig) in [
        (block_s - 0.25, fade(pulse_wave(rate, 20_000.0, 0.5, 0.05, 0.6), rate, 3.0)),
        (block_s - 0.3, fade(qam(rate, 50_000.0, 5_000.0, 0.35, 4, 0.05, 0.6, 4), rate, 3.0)),
    ] {
        let x = scene(rate, 3.0, &[(start, sig)]);
        let ev = scan_digital(&x, rate);
        assert_eq!(ev.len(), 1, "{ev:?}\n{}", explain(&x, rate));
        assert!((ev[0].start_s - start).abs() < 0.1, "{ev:?}");
        assert!((ev[0].end_s - start - 0.6).abs() < 0.1, "{ev:?}");
    }
}

#[test]
fn loud_negatives_are_quiet() {
    let rate = 384_000;
    let mut x = bat_calls(rate, 3.0, true);
    for v in &mut x {
        *v *= 3.0;
    }
    add(&mut x, 0, &hiss(rate, 3.0));
    assert_quiet("loud CF bat calls", rate, &x);
    let mut x = cricket(rate, 3.0, true);
    for v in &mut x {
        *v *= 4.0;
    }
    assert_quiet("loud cricket trill", rate, &x);
    let lp = {
        let mut w = noise(0.8, secs(rate, 3.0), 3);
        Butterworth::lowpass(4, rate as f32, 2_000.0).process_in_place(&mut w);
        w
    };
    assert_quiet("loud low noise", rate, &lp);
}

#[test]
#[ignore = "prints feature traces for tuning"]
fn print_positive_traces() {
    for (rate, fc, rs, order, amp) in [
        (48_000, 12_000.0, 1_000.0, 16, 0.05),
        (48_000, 12_000.0, 4_000.0, 4, 0.05),
        (384_000, 40_000.0, 10_000.0, 16, 0.05),
        (384_000, 40_000.0, 2_000.0, 4, 0.05),
        (256_000, 60_000.0, 5_000.0, 8, 0.05),
        (384_000, 40_000.0, 2_000.0, 4, 0.01),
        (384_000, 40_000.0, 2_000.0, 16, 0.01),
        (48_000, 12_000.0, 2_000.0, 2, 0.02),
    ] {
        let sig = qam(rate, fc, rs, 0.35, order, amp, 2.5, 3);
        let x = scene(rate, 4.5, &[(1.0, fade(sig, rate, 5.0))]);
        println!("== {rate} {fc} {rs} {order} {amp}\n{}", explain(&x, rate));
    }
}
