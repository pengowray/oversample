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
