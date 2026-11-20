//! Window counts and times, for both methods and awkward sample rates.

use birdnet_worker::windows::Windower;
use oversample_ipc::classifier::{Method, WINDOW_SAMPLES};

/// Feed `secs` of silence with one click at `click_s`, in uneven pieces.
fn run(method: Method, rate: u32, secs: f64, click_s: Option<f64>) -> Vec<(u32, f64, f64, usize)> {
    let n = (secs * rate as f64).round() as usize;
    let mut audio = vec![0.0f32; n];
    if let Some(t) = click_s {
        audio[(t * rate as f64) as usize] = 1.0;
    }
    let mut w = Windower::new(method, rate);
    let mut out = Vec::new();
    for piece in audio.chunks(rate as usize / 3 + 17) {
        out.extend(w.push(piece));
    }
    out.extend(w.finish());
    out.into_iter()
        .map(|w| {
            assert_eq!(w.samples.len(), WINDOW_SAMPLES);
            let peak = w
                .samples
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
                .map(|(i, _)| i)
                .unwrap();
            (w.index, w.start_s, w.end_s, peak)
        })
        .collect()
}

#[test]
fn birdnet_windows_are_three_seconds() {
    let w = run(Method::BirdNet, 44_100, 10.5, None);
    let times: Vec<(f64, f64)> = w.iter().map(|w| (w.1, w.2)).collect();
    // The last 1.5 s is kept (at least 1 s) and ends at the end of the file.
    assert_eq!(times, vec![(0.0, 3.0), (3.0, 6.0), (6.0, 9.0), (9.0, 10.5)]);
}

#[test]
fn birdnet_drops_a_last_piece_under_one_second() {
    assert_eq!(run(Method::BirdNet, 48_000, 9.8, None).len(), 3);
    assert_eq!(run(Method::BirdNet, 48_000, 10.0, None).len(), 4);
}

#[test]
fn short_file_gets_one_window() {
    let w = run(Method::BirdNet, 22_050, 0.4, None);
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].1, 0.0);
    assert!((w[0].2 - 0.4).abs() < 1e-3);
}

#[test]
fn bat_windows_are_0_5625_seconds() {
    let w = run(Method::Bats, 384_000, 2.0, None);
    // 3 full windows, then 0.3125 s (at least 0.1875 s), kept.
    assert_eq!(w.len(), 4);
    assert_eq!(w[1].1, 0.5625);
    assert_eq!(w[2].2, 1.6875);
    assert_eq!(w[3].1, 1.6875);
    assert!((w[3].2 - 2.0).abs() < 1e-9);
}

#[test]
fn click_lands_at_its_time() {
    // BirdNET at 44.1 kHz: a click at 7.2 s is in window 2, 1.2 s in.
    let w = run(Method::BirdNet, 44_100, 10.0, Some(7.2));
    let win = &w[2];
    assert_eq!(win.0, 2);
    let at = win.3 as f64 / 48_000.0;
    assert!((at - 1.2).abs() < 1e-4, "click at {at} s into window 2");

    // Bats at 500 kHz: a click at 1.3 s is in window 2 (1.125 s to 1.6875 s).
    let w = run(Method::Bats, 500_000, 2.0, Some(1.3));
    let win = &w[2];
    let at = win.3 as f64 / 256_000.0;
    assert!((at - 0.175).abs() < 1e-4, "click at {at} s into window 2");
}

/// Start, end and first sample of each window of one pass, fed in uneven
/// pieces.
fn pass(method: Method, rate: u32, audio: &[f32], offsets: &[u32]) -> Vec<(f64, f64, f32)> {
    let mut w = Windower::with_offsets(method, rate, offsets);
    let mut out = Vec::new();
    for piece in audio.chunks(rate as usize / 3 + 17) {
        out.extend(w.push(piece));
    }
    out.extend(w.finish());
    out.into_iter()
        .map(|w| {
            assert_eq!(w.samples.len(), method.window_samples());
            (w.start_s, w.end_s, w.samples[0])
        })
        .collect()
}

/// Every pass of a run with `step` seconds between window starts.
fn passes(method: Method, rate: u32, secs: f64, step: f64) -> Vec<Vec<(f64, f64, f32)>> {
    use oversample_ipc::classifier::refine_passes;
    let audio = vec![0.0f32; (secs * rate as f64).round() as usize];
    let win = method.window_samples() as u32;
    let step = method.step_samples(step).unwrap_or(win);
    refine_passes(win, step)
        .iter()
        .map(|p| pass(method, rate, &audio, &p.offsets))
        .collect()
}

fn starts(windows: &[(f64, f64, f32)]) -> Vec<f64> {
    windows.iter().map(|w| w.0).collect()
}

#[test]
fn the_first_pass_is_the_windows_end_to_end() {
    for (method, rate, secs) in [
        (Method::BirdNet, 44_100, 10.5),
        (Method::BirdNet, 48_000, 9.8),
        (Method::BirdNet, 22_050, 0.4),
        (Method::BirdNet, 48_000, 12.0),
        (Method::Perch, 44_100, 17.3),
        (Method::BirdNetV3, 96_000, 7.0),
    ] {
        let n = (secs * rate as f64).round() as usize;
        let audio = vec![0.0f32; n];
        let mut w = Windower::new(method, rate);
        let mut plain = Vec::new();
        for piece in audio.chunks(rate as usize / 3 + 17) {
            plain.extend(w.push(piece));
        }
        plain.extend(w.finish());
        let plain: Vec<(f64, f64)> = plain.iter().map(|w| (w.start_s, w.end_s)).collect();
        for step in [1.0, 0.5] {
            let first: Vec<(f64, f64)> = passes(method, rate, secs, step)[0]
                .iter()
                .map(|w| (w.0, w.1))
                .collect();
            assert_eq!(first, plain, "{method:?} {rate} Hz {secs} s, step {step}");
        }
    }
}

#[test]
fn a_1_s_step_fills_in_whole_windows_inside_the_file() {
    // 10.5 s: windows end to end at 0, 3, 6 and 9 (that one 1.5 s long, padded),
    // then 1, 2, 4, 5 and 7, which end inside the file; 8 would end at 11 s.
    let p = passes(Method::BirdNet, 44_100, 10.5, 1.0);
    assert_eq!(p.len(), 2);
    assert_eq!(starts(&p[0]), vec![0.0, 3.0, 6.0, 9.0]);
    assert_eq!(starts(&p[1]), vec![1.0, 2.0, 4.0, 5.0, 7.0]);
    assert!(p[1].iter().all(|w| w.1 == w.0 + 3.0));
    assert!((p[0][3].1 - 10.5).abs() < 1e-9);
}

#[test]
fn a_half_second_step_goes_coarse_then_halves_then_the_rest() {
    let p = passes(Method::BirdNet, 48_000, 9.0, 0.5);
    assert_eq!(p.len(), 3);
    assert_eq!(starts(&p[0]), vec![0.0, 3.0, 6.0]);
    assert_eq!(starts(&p[1]), vec![1.5, 4.5]);
    assert_eq!(starts(&p[2]), vec![0.5, 1.0, 2.0, 2.5, 3.5, 4.0, 5.0, 5.5]);
    // Together: every 0.5 s from 0 to 6, once each.
    let mut all: Vec<f64> = p.iter().flat_map(|w| starts(w)).collect();
    all.sort_by(f64::total_cmp);
    assert_eq!(all, (0..=12).map(|k| k as f64 * 0.5).collect::<Vec<_>>());
}

#[test]
fn perch_passes() {
    let p = passes(Method::Perch, 32_000, 12.0, 0.5);
    assert_eq!(starts(&p[0]), vec![0.0, 5.0, 10.0]);
    assert_eq!(starts(&p[1]), vec![2.5]);
    // 0.5 to 7 s, less 2.5 and 5.
    assert_eq!(starts(&p[2]).len(), 12);
    let p = passes(Method::Perch, 32_000, 12.0, 1.0);
    assert_eq!(starts(&p[1]), vec![1.0, 2.0, 3.0, 4.0, 6.0, 7.0]);
}

#[test]
fn files_of_a_whole_number_of_seconds_keep_their_last_window() {
    // A window ending exactly at the end of the file is kept at any rate,
    // even if the resampler ends a few samples short.
    for rate in [44_100, 22_050, 48_000, 96_000, 11_025] {
        for (method, secs) in [
            (Method::BirdNet, 11.0),
            (Method::BirdNetV3, 8.0),
            (Method::Perch, 11.0),
        ] {
            let p = passes(method, rate, secs, 0.5);
            let last_fill = p[1..].iter().flat_map(|w| starts(w)).fold(0.0, f64::max);
            assert_eq!(
                last_fill,
                secs - method.window_secs(),
                "{method:?} at {rate} Hz"
            );
            assert!(p.iter().flatten().all(|w| w.1 <= secs + 1e-9));
        }
    }
}

#[test]
fn filled_in_windows_hold_the_audio_at_their_start() {
    // A ramp: each window's first sample tells where it was cut from.
    let rate = 48_000u32;
    let audio: Vec<f32> = (0..rate * 9).map(|i| i as f32 / rate as f32).collect();
    let w = pass(Method::BirdNet, rate, &audio, &[24_000, 72_000]);
    // 6.5 s would end at 9.5 s, past the end.
    assert_eq!(starts(&w), vec![0.5, 1.5, 3.5, 4.5]);
    for (start, _, first) in w {
        assert!((first as f64 - start).abs() < 1e-3, "{start}: {first}");
    }
}
