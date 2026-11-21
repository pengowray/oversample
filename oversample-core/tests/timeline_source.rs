//! `TimelineAudioSource` and `ResampledSource`: clips placed one after
//! another, a pause between clips, and clips with different sample rates.

use oversample_core::audio::source::{
    AudioSource, ChannelView, InMemorySource, ResampledSource, TimelineAudioSource,
};
use std::sync::Arc;

fn tone(rate: u32, secs: f64, hz: f64, amplitude: f32) -> Arc<dyn AudioSource> {
    let n = (f64::from(rate) * secs).round() as usize;
    let samples: Vec<f32> = (0..n)
        .map(|i| {
            amplitude * (2.0 * std::f64::consts::PI * hz * i as f64 / f64::from(rate)).sin() as f32
        })
        .collect();
    Arc::new(InMemorySource {
        samples: Arc::new(samples),
        raw_samples: None,
        sample_rate: rate,
        channels: 1,
    })
}

fn rms(samples: &[f32]) -> f64 {
    (samples.iter().map(|&s| f64::from(s).powi(2)).sum::<f64>() / samples.len() as f64).sqrt()
}

/// How strongly `samples` holds a tone of `hz`: 1.0 for a full-scale sine.
fn tone_level(samples: &[f32], rate: u32, hz: f64) -> f64 {
    let (mut re, mut im) = (0.0, 0.0);
    for (i, &s) in samples.iter().enumerate() {
        let a = 2.0 * std::f64::consts::PI * hz * i as f64 / f64::from(rate);
        re += f64::from(s) * a.cos();
        im += f64::from(s) * a.sin();
    }
    2.0 * (re * re + im * im).sqrt() / samples.len() as f64
}

#[test]
fn clips_play_one_after_another_with_a_pause_between() {
    let rate = 48_000;
    // 1 s at 0.2, then 0.5 s of silence, then 1 s at 0.6, then 1 s at 0.4.
    let timeline = TimelineAudioSource::at_highest_rate(vec![
        (tone(rate, 1.0, 1000.0, 0.2), rate, 0.0, 1.0),
        (tone(rate, 1.0, 1000.0, 0.6), rate, 1.5, 1.0),
        (tone(rate, 1.0, 1000.0, 0.4), rate, 2.5, 1.0),
    ])
    .unwrap();
    assert_eq!(timeline.sample_rate(), rate);
    assert_eq!(timeline.total_samples(), 3 * 48_000 + 24_000);

    // One read across the end of the first clip, the pause and the start of
    // the second clip.
    let all = timeline.read_region(ChannelView::MonoMix, 0, 168_000);
    assert_eq!(all.len(), 168_000);
    let level = |from: usize, to: usize| rms(&all[from..to]) * std::f64::consts::SQRT_2;
    assert!((level(0, 48_000) - 0.2).abs() < 0.005);
    assert_eq!(level(48_000, 72_000), 0.0, "the pause is silent");
    assert!((level(72_000, 120_000) - 0.6).abs() < 0.005);
    assert!((level(120_000, 168_000) - 0.4).abs() < 0.005);

    // A read that starts inside the second clip.
    let part = timeline.read_region(ChannelView::MonoMix, 100_000, 1000);
    assert!((rms(&part) * std::f64::consts::SQRT_2 - 0.6).abs() < 0.02);
}

#[test]
fn a_clip_with_a_lower_rate_is_resampled_to_the_highest_rate() {
    // A 5 kHz tone at 44.1 kHz after a 20 kHz tone at 192 kHz.
    let timeline = TimelineAudioSource::at_highest_rate(vec![
        (tone(192_000, 1.0, 20_000.0, 0.5), 192_000, 0.0, 1.0),
        (tone(44_100, 1.0, 5_000.0, 0.5), 44_100, 1.0, 1.0),
    ])
    .unwrap();
    assert_eq!(timeline.sample_rate(), 192_000);
    assert_eq!(timeline.total_samples(), 384_000);

    let first = timeline.read_region(ChannelView::MonoMix, 48_000, 96_000);
    assert!((tone_level(&first, 192_000, 20_000.0) - 0.5).abs() < 0.01);

    // The second clip keeps its pitch, level and length.
    let second = timeline.read_region(ChannelView::MonoMix, 192_000 + 48_000, 96_000);
    assert!((tone_level(&second, 192_000, 5_000.0) - 0.5).abs() < 0.01);
    // Nothing is added above the 44.1 kHz file's own range.
    for hz in [39_100.0, 49_100.0, 83_200.0] {
        assert!(tone_level(&second, 192_000, hz) < 0.001, "image at {hz} Hz");
    }
}

#[test]
fn resampled_reads_in_pieces_match_one_read() {
    let source = ResampledSource::new(tone(44_100, 0.5, 3_000.0, 0.8), 44_100, 192_000);
    let whole = source.read_region(ChannelView::MonoMix, 0, source.total_samples() as usize);
    assert_eq!(whole.len() as u64, source.total_samples());
    let mut pieces = Vec::new();
    let mut at = 0u64;
    while at < source.total_samples() {
        let piece = source.read_region(ChannelView::MonoMix, at, 7_777);
        assert!(!piece.is_empty());
        at += piece.len() as u64;
        pieces.extend(piece);
    }
    assert_eq!(pieces.len(), whole.len());
    let worst = whole
        .iter()
        .zip(&pieces)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(worst < 1e-5, "largest difference {worst}");
    assert_eq!(
        source
            .read_region(ChannelView::MonoMix, source.total_samples(), 10)
            .len(),
        0
    );
}

#[test]
fn a_clip_read_at_a_corrected_rate_plays_at_that_rate() {
    // A x10 time-expanded recording: stored at 38.4 kHz, read at 384 kHz.
    // Its 2 kHz stored tone is a 20 kHz tone, and its 1 s is 0.1 s.
    let timeline = TimelineAudioSource::at_highest_rate(vec![
        (tone(38_400, 1.0, 2_000.0, 0.5), 384_000, 0.0, 0.1),
        (tone(192_000, 1.0, 30_000.0, 0.5), 192_000, 0.1, 1.0),
    ])
    .unwrap();
    assert_eq!(timeline.sample_rate(), 384_000);
    assert_eq!(timeline.total_samples(), 38_400 + 384_000);
    let first = timeline.read_region(ChannelView::MonoMix, 0, 38_400);
    assert!((tone_level(&first, 384_000, 20_000.0) - 0.5).abs() < 0.01);
    let second = timeline.read_region(ChannelView::MonoMix, 38_400 + 96_000, 96_000);
    assert!((tone_level(&second, 384_000, 30_000.0) - 0.5).abs() < 0.01);
}
