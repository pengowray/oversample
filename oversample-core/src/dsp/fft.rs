use crate::canvas::colors::{dbfs_to_greyscale, magnitude_to_db};
use crate::dsp::depth_range::max_display_range_db;
use crate::dsp::spectrum_level::{
    auto_range_db, fft_size_for_bins, full_scale_db, noise_floor_dbfs,
};
use crate::types::{AudioData, PreviewImage, SpectrogramColumn, SpectrogramData};
use realfft::RealFftPlanner;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

thread_local! {
    static FFT_PLANNER: RefCell<RealFftPlanner<f32>> = RefCell::new(RealFftPlanner::new());
    static HANN_CACHE: RefCell<HashMap<usize, Vec<f32>>> = RefCell::new(HashMap::new());
}

/// Plan (and cache) a forward real FFT of `size` via the shared thread-local
/// planner. Single source of truth — other DSP modules call this instead of
/// each keeping their own `RealFftPlanner`.
pub fn plan_fft_forward(size: usize) -> Arc<dyn realfft::RealToComplex<f32>> {
    FFT_PLANNER.with(|p| p.borrow_mut().plan_fft_forward(size))
}

/// Plan (and cache) an inverse real FFT of `size` via the shared planner.
pub fn plan_fft_inverse(size: usize) -> Arc<dyn realfft::ComplexToReal<f32>> {
    FFT_PLANNER.with(|p| p.borrow_mut().plan_fft_inverse(size))
}

/// Symmetric Hann window of `size`, cached. Single source of truth — other DSP
/// modules call this instead of each keeping their own identical copy + cache.
pub fn hann_window(size: usize) -> Vec<f32> {
    HANN_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .entry(size)
            .or_insert_with(|| {
                (0..size)
                    .map(|i| {
                        0.5 * (1.0
                            - (2.0 * std::f32::consts::PI * i as f32 / (size - 1) as f32).cos())
                    })
                    .collect()
            })
            .clone()
    })
}

/// Compute a spectrogram from audio data using a Short-Time Fourier Transform (STFT).
///
/// Uses a Hann window for spectral leakage reduction.
pub fn compute_spectrogram(audio: &AudioData, fft_size: usize, hop_size: usize) -> SpectrogramData {
    let fft = FFT_PLANNER.with(|p| p.borrow_mut().plan_fft_forward(fft_size));

    // Use the in-memory mono-mixed samples directly — allocating a fresh Vec
    // via read_region would duplicate up to a gigabyte for long audiobooks
    // that already fit in audio.samples (and OOM the WASM heap). For streaming
    // sources audio.samples is the head (~30s), which is all we need anyway.
    let samples: &[f32] = &audio.samples;

    let mut columns = Vec::new();

    let window = hann_window(fft_size);

    // Pre-allocate FFT buffers once and reuse across frames
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();

    let mut pos = 0;
    while pos + fft_size <= samples.len() {
        // Fill input in-place (no allocation per frame)
        for (inp, (&s, &w)) in input
            .iter_mut()
            .zip(samples[pos..pos + fft_size].iter().zip(window.iter()))
        {
            *inp = s * w;
        }

        fft.process(&mut input, &mut spectrum).expect("FFT failed");

        let magnitudes: Vec<f32> = spectrum.iter().map(|c| c.norm()).collect();

        let time_offset = pos as f64 / audio.sample_rate as f64;
        columns.push(SpectrogramColumn {
            magnitudes,
            time_offset,
        });

        pos += hop_size;
    }

    let freq_resolution = audio.sample_rate as f64 / fft_size as f64;
    let time_resolution = hop_size as f64 / audio.sample_rate as f64;
    let max_freq = audio.sample_rate as f64 / 2.0;

    let total_columns = columns.len();
    SpectrogramData {
        columns: Arc::new(columns),
        total_columns,
        freq_resolution,
        time_resolution,
        max_freq,
        sample_rate: audio.sample_rate,
    }
}

/// Compute a partial spectrogram — only columns `col_start .. col_start + col_count`.
///
/// Identical FFT parameters to `compute_spectrogram`.  Used for chunked async
/// computation so the browser stays responsive between chunks.
pub fn compute_spectrogram_partial(
    audio: &AudioData,
    fft_size: usize,
    hop_size: usize,
    col_start: usize,
    col_count: usize,
) -> Vec<SpectrogramColumn> {
    // Use audio.samples directly — same rationale as compute_spectrogram:
    // avoid allocating a duplicate Vec for multi-hour files.
    compute_stft_columns(
        &audio.samples,
        audio.sample_rate,
        fft_size,
        hop_size,
        col_start,
        col_count,
    )
}

/// Compute STFT columns directly from a sample slice.
///
/// Like `compute_spectrogram_partial` but works on a raw `&[f32]` slice instead
/// of `&AudioData`, avoiding the need to wrap live recording buffers in `Arc<Vec<f32>>`.
pub fn compute_stft_columns(
    samples: &[f32],
    sample_rate: u32,
    fft_size: usize,
    hop_size: usize,
    col_start: usize,
    col_count: usize,
) -> Vec<SpectrogramColumn> {
    if samples.len() < fft_size || col_count == 0 {
        return vec![];
    }

    let fft = FFT_PLANNER.with(|p| p.borrow_mut().plan_fft_forward(fft_size));
    let window = hann_window(fft_size);
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();

    let total_cols = (samples.len() - fft_size) / hop_size + 1;
    let col_end = (col_start + col_count).min(total_cols);

    let mut columns = Vec::with_capacity(col_end.saturating_sub(col_start));
    for col_i in col_start..col_end {
        let pos = col_i * hop_size;
        if pos + fft_size > samples.len() {
            break;
        }
        for (inp, (&s, &w)) in input
            .iter_mut()
            .zip(samples[pos..pos + fft_size].iter().zip(window.iter()))
        {
            *inp = s * w;
        }
        fft.process(&mut input, &mut spectrum).expect("FFT failed");
        let magnitudes: Vec<f32> = spectrum.iter().map(|c| c.norm()).collect();
        let time_offset = pos as f64 / sample_rate as f64;
        columns.push(SpectrogramColumn {
            magnitudes,
            time_offset,
        });
    }
    columns
}

/// Compute a fast low-resolution preview spectrogram as an RGBA pixel buffer.
/// Uses FFT=256 with a dynamic hop to produce roughly `target_width` columns.
/// `dc` is the recording's offset (`dc_offset`), removed from each frame; 0
/// keeps the samples as they are.
pub fn compute_preview(
    audio: &AudioData,
    target_width: u32,
    target_height: u32,
    dc: f32,
) -> PreviewImage {
    // For streaming files, only the head samples are in memory — don't try to
    // read the entire multi-GB file. audio.samples contains the loaded portion.
    let total = if audio.source.is_fully_loaded() {
        audio.source.total_samples() as usize
    } else {
        audio.samples.len()
    };
    if total < 256 {
        // Too short for even one FFT frame
        return PreviewImage {
            width: 1,
            height: 1,
            pixels: Arc::new(vec![0, 0, 0, 255]),
        };
    }

    let fft_size = 256;
    let hop = (total / target_width as usize).max(fft_size);
    // Copy out only the frames the preview uses (about `target_width` of
    // them), so the offset can come off without copying the whole recording.
    let samples: &[f32] = &audio.samples;
    let mut frames = Vec::new();
    let mut pos = 0;
    while pos + fft_size <= samples.len() {
        frames.extend_from_slice(&samples[pos..pos + fft_size]);
        pos += hop;
    }
    super::dc_offset::subtract_dc(&mut frames, dc);
    let columns = compute_stft_columns(
        &frames,
        audio.sample_rate,
        fft_size,
        fft_size,
        0,
        usize::MAX,
    );

    if columns.is_empty() {
        return PreviewImage {
            width: 1,
            height: 1,
            pixels: Arc::new(vec![0, 0, 0, 255]),
        };
    }

    let src_w = columns.len();
    let src_h = columns[0].magnitudes.len();
    let out_w = (src_w as u32).min(target_width);
    let out_h = (src_h as u32).min(target_height);

    // Same mapping as the main view at its default settings: dBFS, down to
    // the Auto range for this image's noise floor and the file's bit depth.
    let full_scale = full_scale_db(fft_size);
    let max_range = max_display_range_db(audio.metadata.bits_per_sample, audio.metadata.is_float);
    let range = auto_range_db(noise_floor_dbfs(&columns), max_range);

    let mut pixels = vec![0u8; (out_w * out_h * 4) as usize];

    for x in 0..out_w {
        let src_col = (x as usize * src_w) / out_w as usize;
        let col = &columns[src_col.min(src_w - 1)];
        for y in 0..out_h {
            // Map output row to source bin (row 0 = highest freq)
            let src_bin = src_h - 1 - ((y as usize * src_h) / out_h as usize).min(src_h - 1);
            let mag = col.magnitudes[src_bin];
            let grey = dbfs_to_greyscale(magnitude_to_db(mag) - full_scale, range);
            let idx = (y * out_w + x) as usize * 4;
            pixels[idx] = grey;
            pixels[idx + 1] = grey;
            pixels[idx + 2] = grey;
            pixels[idx + 3] = 255;
        }
    }

    PreviewImage {
        width: out_w,
        height: out_h,
        pixels: Arc::new(pixels),
    }
}

/// Compute a higher-resolution overview image by downsampling existing SpectrogramData.
/// Produces a ~1024×256 greyscale RGBA image (same format as PreviewImage),
/// mapped like [`compute_preview`]. `max_range_db` is the file's bit-depth
/// limit (`depth_range::max_display_range_db`).
pub fn compute_overview_from_spectrogram(
    data: &SpectrogramData,
    max_range_db: f32,
) -> Option<PreviewImage> {
    if data.columns.is_empty() {
        return None;
    }

    let src_w = data.columns.len();
    let src_h = data.columns[0].magnitudes.len();
    if src_h == 0 {
        return None;
    }

    let out_w = (src_w as u32).min(1024);
    let out_h = (src_h as u32).min(256);

    let full_scale = full_scale_db(fft_size_for_bins(src_h));
    let range = auto_range_db(noise_floor_dbfs(&data.columns), max_range_db);

    let mut pixels = vec![0u8; (out_w * out_h * 4) as usize];

    for x in 0..out_w {
        let src_col = (x as usize * src_w) / out_w as usize;
        let col = &data.columns[src_col.min(src_w - 1)];
        let col_len = col.magnitudes.len();
        if col_len == 0 {
            continue;
        }
        for y in 0..out_h {
            let src_bin = src_h - 1 - ((y as usize * src_h) / out_h as usize).min(src_h - 1);
            let mag = col.magnitudes[src_bin.min(col_len - 1)];
            let grey = dbfs_to_greyscale(magnitude_to_db(mag) - full_scale, range);
            let idx = (y * out_w + x) as usize * 4;
            pixels[idx] = grey;
            pixels[idx + 1] = grey;
            pixels[idx + 2] = grey;
            pixels[idx + 3] = 255;
        }
    }

    Some(PreviewImage {
        width: out_w,
        height: out_h,
        pixels: Arc::new(pixels),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::source::InMemorySource;
    use crate::types::{AudioData, AudioFormat, FileMetadata};

    fn test_audio(samples: Vec<f32>, sample_rate: u32) -> AudioData {
        let samples = Arc::new(samples);
        let source = Arc::new(InMemorySource {
            samples: samples.clone(),
            raw_samples: None,
            sample_rate,
            channels: 1,
        });
        AudioData {
            duration_secs: samples.len() as f64 / sample_rate as f64,
            samples,
            source,
            sample_rate,
            channels: 1,
            metadata: FileMetadata {
                file_size: 0,
                format: AudioFormat::Unknown,
                bits_per_sample: 32,
                is_float: true,
                guano: None,
                data_offset: None,
                data_size: None,
                zc_data: None,
                time_expansion: None,
                wav: None,
            },
        }
    }

    #[test]
    fn test_spectrogram_basic() {
        let sample_rate = 44100u32;
        let freq = 1000.0f64;
        let num_samples = 4096;

        let samples: Vec<f32> = (0..num_samples)
            .map(|i| {
                let t = i as f64 / sample_rate as f64;
                (2.0 * std::f64::consts::PI * freq * t).sin() as f32
            })
            .collect();

        let audio = test_audio(samples, sample_rate);

        let result = compute_spectrogram(&audio, 1024, 512);
        assert!(!result.columns.is_empty());
        assert_eq!(result.sample_rate, sample_rate);

        // The peak bin should be near 1000 Hz
        let col = &result.columns[1]; // skip first column (edge effects)
        let peak_bin = col
            .magnitudes
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        let peak_freq = peak_bin as f64 * result.freq_resolution;
        let error = (peak_freq - freq).abs();
        assert!(
            error < result.freq_resolution * 2.0,
            "Peak at {peak_freq} Hz, expected ~{freq} Hz"
        );
    }

    #[test]
    fn preview_drops_the_offset_row() {
        // Noise on top of a large offset.
        let mut seed = 1u32;
        let samples: Vec<f32> = (0..96_000)
            .map(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                0.05 + 0.01 * ((seed >> 8) as f32 / (1u32 << 24) as f32 - 0.5)
            })
            .collect();
        let dc = crate::dsp::dc_offset::mean_outside_silence(&samples);
        let audio = test_audio(samples, 48_000);
        // Mean grey of row `y` (row 0 is the top, the last row is bin 0).
        let row_grey = |p: &PreviewImage, y: u32| {
            (0..p.width)
                .map(|x| p.pixels[((y * p.width + x) * 4) as usize] as f32)
                .sum::<f32>()
                / p.width as f32
        };
        let kept = compute_preview(&audio, 256, 128, 0.0);
        let removed = compute_preview(&audio, 256, 128, dc);
        let bottom = kept.height - 1;
        let middle = kept.height / 2;
        assert!(row_grey(&kept, bottom) > row_grey(&kept, middle) + 60.0);
        assert!(row_grey(&removed, bottom) < row_grey(&removed, middle) + 20.0);
        // Rows away from the offset stay as they were (rounding aside).
        assert!((row_grey(&kept, middle) - row_grey(&removed, middle)).abs() < 1.0);
    }
}
