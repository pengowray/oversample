use crate::canvas::colors::{dbfs_to_greyscale, magnitude_to_db};
use crate::dsp::depth_range::max_display_range_db;
use crate::dsp::spectrum_level::{
    auto_range_db, fft_size_for_bins, full_scale_db, noise_floor_dbfs,
};
use crate::types::PreRendered;
use crate::types::{AudioData, PreviewImage, SpectrogramColumn, SpectrogramData};
use realfft::RealFftPlanner;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

thread_local! {
    static FFT_PLANNER: RefCell<RealFftPlanner<f32>> = RefCell::new(RealFftPlanner::new());
    static HANN_CACHE: RefCell<HashMap<usize, Vec<f32>>> = RefCell::new(HashMap::new());
    static THANN_CACHE: RefCell<HashMap<usize, Vec<f32>>> = RefCell::new(HashMap::new());
    static DHANN_CACHE: RefCell<HashMap<usize, Vec<f32>>> = RefCell::new(HashMap::new());
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

/// Time-ramped Hann window: `(m - center) * h[m]`.
/// Used for time reassignment (measures displacement from frame center).
fn t_hann_window(size: usize) -> Vec<f32> {
    THANN_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .entry(size)
            .or_insert_with(|| {
                let h = hann_window(size);
                let center = (size - 1) as f32 / 2.0;
                (0..size).map(|i| (i as f32 - center) * h[i]).collect()
            })
            .clone()
    })
}

/// Derivative of the Hann window: `h'[m] = -π/(N-1) * sin(2πm/(N-1))`.
/// Used for frequency reassignment.
fn dh_window(size: usize) -> Vec<f32> {
    DHANN_CACHE.with(|cache| {
        cache
            .borrow_mut()
            .entry(size)
            .or_insert_with(|| {
                let n_minus_1 = (size - 1) as f32;
                (0..size)
                    .map(|i| {
                        -std::f32::consts::PI / n_minus_1
                            * (2.0 * std::f32::consts::PI * i as f32 / n_minus_1).sin()
                    })
                    .collect()
            })
            .clone()
    })
}

/// How many columns a frame's energy can move in time under reassignment.
///
/// Reassignment keeps energy inside its own window, so it moves at most
/// `fft_size / 2` samples from the frame centre. A reassigned tile needs this
/// many extra frames on each side to collect the energy that its neighbours'
/// frames move into it.
pub fn reassign_margin_cols(fft_size: usize, hop_size: usize) -> usize {
    (fft_size / 2).div_ceil(hop_size.max(1))
}

/// Reassignment moves a bin only when its power is at least this far above
/// the bin's noise floor. Noise power goes 10 dB over its median in about
/// 1 frame in 1000, so noise stays where the plain spectrogram draws it
/// instead of turning into speckle.
pub const REASSIGN_ABOVE_FLOOR_DB: f32 = 10.0;

/// Per-bin noise floor for [`compute_reassigned_tile`]: the median Hann-window
/// power of each bin over non-overlapping frames of `samples`.
///
/// The median ignores calls as long as they are in fewer than half the frames
/// at a given frequency. Returns an empty Vec (no floor) when `samples` holds
/// fewer than 4 frames.
pub fn reassign_noise_floor(samples: &[f32], fft_size: usize) -> Vec<f32> {
    let frame_count = samples.len() / fft_size.max(1);
    if fft_size == 0 || frame_count < 4 {
        return Vec::new();
    }
    let n_bins = fft_size / 2 + 1;
    let fft = plan_fft_forward(fft_size);
    let window = hann_window(fft_size);
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();

    // by_bin[k * frame_count + f] = power of bin k in frame f
    let mut by_bin = vec![0.0f32; n_bins * frame_count];
    for (f, frame) in samples.chunks_exact(fft_size).enumerate() {
        for (inp, (&s, &w)) in input.iter_mut().zip(frame.iter().zip(window.iter())) {
            *inp = s * w;
        }
        fft.process(&mut input, &mut spectrum).expect("FFT failed");
        for (k, c) in spectrum.iter().enumerate() {
            by_bin[k * frame_count + f] = c.norm_sqr();
        }
    }
    by_bin
        .chunks_exact_mut(frame_count)
        .map(|powers| {
            *powers
                .select_nth_unstable_by(frame_count / 2, f32::total_cmp)
                .1
        })
        .collect()
}

/// Compute a reassigned spectrogram tile from raw audio samples.
///
/// Performs 3 FFTs per frame (standard Hann, time-ramped, derivative-windowed),
/// then accumulates |X|² at corrected (time, frequency) positions using
/// nearest-neighbor assignment.
///
/// Columns follow the same convention as `compute_stft_columns`: column `c`
/// holds the frame that starts at `c * hop_size`, so a click at sample `s`
/// lands in column `(s - (fft_size - 1) / 2) / hop_size`.
///
/// `samples[0]` is the start of the frame for column `-lead_cols`. Pass up to
/// [`reassign_margin_cols`] frames before the tile (fewer at the start of the
/// file) and as many after it, so energy moved across the tile edges arrives.
/// Energy that lands outside the tile, or outside the frequency range, is
/// dropped.
///
/// A bin is moved only when its power is at least `gate_db` (absolute dB of
/// unnormalized power) and at least [`REASSIGN_ABOVE_FLOOR_DB`] over
/// `noise_floor[k]` (from [`reassign_noise_floor`]; empty means no floor).
/// Other bins stay where the plain spectrogram draws them. A moved bin leaves
/// `noise_floor[k]` of its power behind and moves the rest.
///
/// Returns a `PreRendered` with `db_data` in the same format as normal tiles
/// (row 0 = highest freq, absolute dB values).
pub fn compute_reassigned_tile(
    samples: &[f32],
    lead_cols: usize,
    col_count: usize,
    fft_size: usize,
    hop_size: usize,
    gate_db: f32,
    noise_floor: &[f32],
) -> PreRendered {
    let n_bins = fft_size / 2 + 1;

    if samples.len() < fft_size || col_count == 0 || hop_size == 0 {
        return PreRendered {
            width: 0,
            height: 0,
            pixels: Vec::new(),
            db_data: Vec::new(),
            flow_shifts: Vec::new(),
        };
    }

    let fft = FFT_PLANNER.with(|p| p.borrow_mut().plan_fft_forward(fft_size));
    let win_h = hann_window(fft_size);
    let win_th = t_hann_window(fft_size);
    let win_dh = dh_window(fft_size);

    // Reusable FFT buffers
    let mut input = fft.make_input_vec();
    let mut spec_h = fft.make_output_vec();
    let mut spec_th = fft.make_output_vec();
    let mut spec_dh = fft.make_output_vec();

    // Accumulation grid (f64 for precision)
    let grid_size = col_count * n_bins;
    let mut accum = vec![0.0f64; grid_size];

    let gate_power = 10.0f32.powf(gate_db / 10.0);
    let above_floor = 10.0f32.powf(REASSIGN_ABOVE_FLOOR_DB / 10.0);
    let floor_gate: Vec<f32> = noise_floor.iter().map(|&p| p * above_floor).collect();
    let two_pi = 2.0 * std::f64::consts::PI;
    let fft_over_two_pi = fft_size as f64 / two_pi;
    let hop = hop_size as f64;
    let max_shift = fft_size as f64 / 2.0;

    // Frames whose energy can reach the tile: `margin` either side of it.
    let margin = reassign_margin_cols(fft_size, hop_size);
    let available_frames = (samples.len() - fft_size) / hop_size + 1;
    let frame_count = available_frames.min(lead_cols + col_count + margin);

    // Add power to the nearest cell; energy that lands outside the tile is dropped.
    let mut add = |(t, f): (f64, f64), power: f32| {
        let (t, f) = (t.round(), f.round());
        if t >= 0.0 && t < col_count as f64 && f >= 0.0 && f < n_bins as f64 {
            accum[f as usize * col_count + t as usize] += power as f64;
        }
    };

    for frame_i in 0..frame_count {
        let pos = frame_i * hop_size;
        let frame = &samples[pos..pos + fft_size];
        // This frame's column within the tile (negative in the lead).
        let frame_col = frame_i as f64 - lead_cols as f64;

        // FFT with standard Hann window
        for (inp, (&s, &w)) in input.iter_mut().zip(frame.iter().zip(win_h.iter())) {
            *inp = s * w;
        }
        fft.process(&mut input, &mut spec_h).expect("FFT failed");

        // FFT with time-ramped Hann window
        for (inp, (&s, &w)) in input.iter_mut().zip(frame.iter().zip(win_th.iter())) {
            *inp = s * w;
        }
        fft.process(&mut input, &mut spec_th).expect("FFT failed");

        // FFT with derivative Hann window
        for (inp, (&s, &w)) in input.iter_mut().zip(frame.iter().zip(win_dh.iter())) {
            *inp = s * w;
        }
        fft.process(&mut input, &mut spec_dh).expect("FFT failed");

        for k in 0..n_bins {
            let xh = spec_h[k];
            let power = xh.norm_sqr();

            let strong = power >= gate_power && floor_gate.get(k).is_none_or(|&gate| power >= gate);
            let moved = if strong {
                use realfft::num_complex::Complex;
                let xh64 = Complex::<f64>::new(xh.re as f64, xh.im as f64);
                let xth64 = Complex::<f64>::new(spec_th[k].re as f64, spec_th[k].im as f64);
                let xdh64 = Complex::<f64>::new(spec_dh[k].re as f64, spec_dh[k].im as f64);

                // Time offset from the frame centre, in samples. The ramp is
                // `(m - centre) * h[m]`, so a click at `m0` gives exactly
                // `m0 - centre`. Larger offsets come from near-zero X_h and
                // point outside the window, so those bins are not moved.
                let dt = (xth64 / xh64).re;

                // Corrected frequency bin. `win_dh` is the negated derivative
                // of the Hann window, so this adds:
                // f_hat = k + (N / 2π) Im(X_dh / X_h).
                let f_hat = k as f64 + fft_over_two_pi * (xdh64 / xh64).im;

                (dt.abs() <= max_shift).then_some((frame_col + dt / hop, f_hat))
            } else {
                None
            };
            let in_place = (frame_col, k as f64);
            match moved {
                Some(to) => {
                    // The bin's noise share stays put and only the excess
                    // moves, so a call doesn't leave a hole darker than the
                    // noise around it.
                    let left = noise_floor.get(k).map_or(0.0, |&n| n.min(power));
                    add(in_place, left);
                    add(to, power - left);
                }
                None => add(in_place, power),
            }
        }
    }

    // Convert accumulation grid to dB, flipped vertically (row 0 = highest freq)
    let width = col_count as u32;
    let height = n_bins as u32;
    let mut db_data = vec![f32::NEG_INFINITY; grid_size];

    for bin in 0..n_bins {
        let y = n_bins - 1 - bin; // flip: row 0 = highest freq
        for col in 0..col_count {
            let val = accum[bin * col_count + col];
            if val > 0.0 {
                db_data[y * col_count + col] = (10.0 * val.log10()) as f32;
            }
        }
    }

    PreRendered {
        width,
        height,
        pixels: Vec::new(),
        db_data,
        flow_shifts: Vec::new(),
    }
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
pub fn compute_preview(audio: &AudioData, target_width: u32, target_height: u32) -> PreviewImage {
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
    let spec = compute_spectrogram(audio, fft_size, hop);

    if spec.columns.is_empty() {
        return PreviewImage {
            width: 1,
            height: 1,
            pixels: Arc::new(vec![0, 0, 0, 255]),
        };
    }

    let src_w = spec.columns.len();
    let src_h = spec.columns[0].magnitudes.len();
    let out_w = (src_w as u32).min(target_width);
    let out_h = (src_h as u32).min(target_height);

    // Same mapping as the main view at its default settings: dBFS, down to
    // the Auto range for this image's noise floor and the file's bit depth.
    let full_scale = full_scale_db(fft_size);
    let max_range = max_display_range_db(audio.metadata.bits_per_sample, audio.metadata.is_float);
    let range = auto_range_db(noise_floor_dbfs(&spec.columns), max_range);

    let mut pixels = vec![0u8; (out_w * out_h * 4) as usize];

    for x in 0..out_w {
        let src_col = (x as usize * src_w) / out_w as usize;
        let col = &spec.columns[src_col.min(src_w - 1)];
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
    use crate::types::{AudioData, FileMetadata};

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
                format: "test",
                bits_per_sample: 32,
                is_float: true,
                guano: None,
                data_offset: None,
                data_size: None,
                zc_data: None,
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

    /// Power per (column, bin) from a reassigned tile, as `cells[col][bin]`.
    fn reassigned_power(tile: &PreRendered) -> Vec<Vec<f64>> {
        let (w, h) = (tile.width as usize, tile.height as usize);
        let mut cells = vec![vec![0.0f64; h]; w];
        for y in 0..h {
            let bin = h - 1 - y;
            for (col, cell) in cells.iter_mut().enumerate() {
                let db = tile.db_data[y * w + col];
                if db.is_finite() {
                    cell[bin] = 10f64.powf(db as f64 / 10.0);
                }
            }
        }
        cells
    }

    #[test]
    fn reassigned_click_lands_in_its_column() {
        // fft much larger than hop, as in a deep zoom: the click is in 32
        // frames, and every one of them should move it to the same column.
        let (fft, hop, cols) = (1024, 32, 64);
        let lead = reassign_margin_cols(fft, hop);
        // Column c holds the frame starting at c * hop, so a click at sample
        // s belongs in column (s - 511.5) / 32 (counting from samples[0]).
        let expected_col = 30;
        let click_at = 511 + (lead + expected_col) * hop;
        let mut samples = vec![0.0f32; (lead + cols + lead) * hop + fft];
        samples[click_at] = 1.0;

        let tile = compute_reassigned_tile(&samples, lead, cols, fft, hop, -100.0, &[]);
        let cells = reassigned_power(&tile);
        let col_energy: Vec<f64> = cells.iter().map(|c| c.iter().sum()).collect();
        let total: f64 = col_energy.iter().sum();
        assert!(total > 0.0, "click produced no energy");
        let near: f64 = col_energy[expected_col - 1..=expected_col + 1].iter().sum();
        assert!(
            near / total > 0.99,
            "only {:.1}% of the click's energy is within 1 column of column {expected_col}",
            100.0 * near / total
        );
    }

    #[test]
    fn reassigned_tone_lands_in_its_bin() {
        // A steady tone between bins (20.3) should collapse onto bin 20.
        let (fft, hop, cols) = (256, 64, 32);
        let lead = reassign_margin_cols(fft, hop);
        let bin = 20.3f64;
        let n = (lead + cols + lead) * hop + fft;
        let samples: Vec<f32> = (0..n)
            .map(|i| (2.0 * std::f64::consts::PI * bin * i as f64 / fft as f64).sin() as f32)
            .collect();

        let tile = compute_reassigned_tile(&samples, lead, cols, fft, hop, -100.0, &[]);
        let cells = reassigned_power(&tile);
        for (col, cell) in cells.iter().enumerate() {
            let total: f64 = cell.iter().sum();
            assert!(
                cell[20] / total > 0.95,
                "column {col}: only {:.1}% of the tone's energy is in bin 20",
                100.0 * cell[20] / total
            );
        }
    }

    /// A bat-like downward sweep, 80 kHz to 20 kHz in 4 ms at 384 kHz, with a
    /// Hann envelope. It crosses 20 bins in one 512-sample window, so the
    /// plain spectrogram smears it.
    struct Sweep {
        start: usize,
    }

    impl Sweep {
        const SR: f64 = 384_000.0;
        const F0: f64 = 80_000.0;
        const F1: f64 = 20_000.0;
        const DUR: f64 = 0.004;

        fn len(&self) -> usize {
            (Self::DUR * Self::SR) as usize
        }

        fn rate(&self) -> f64 {
            (Self::F1 - Self::F0) / Self::DUR
        }

        fn add_to(&self, samples: &mut [f32], amp: f64) {
            let len = self.len();
            for i in 0..len {
                let t = i as f64 / Self::SR;
                let phase = 2.0 * std::f64::consts::PI * (Self::F0 * t + 0.5 * self.rate() * t * t);
                let env = 0.5 * (1.0 - (2.0 * std::f64::consts::PI * i as f64 / len as f64).cos());
                samples[self.start + i] += (amp * env * phase.sin()) as f32;
            }
        }

        /// Share of the tile's energy, within the sweep's columns, that lies
        /// within 2.5 bins of its frequency at each column's time. One column
        /// at hop 32 is 1.3 bins of sweep.
        fn share_on_track(&self, cells: &[Vec<f64>], lead: usize, fft: usize, hop: usize) -> f64 {
            let bin_hz = Self::SR / fft as f64;
            let centre = (fft as f64 - 1.0) / 2.0;
            let (mut total, mut on_track) = (0.0f64, 0.0f64);
            for (col, cell) in cells.iter().enumerate() {
                // The sample this column stands for, counted from samples[0].
                let s = (col + lead) as f64 * hop as f64 + centre;
                let into_call = s - self.start as f64;
                if into_call < 0.0 || into_call > self.len() as f64 {
                    continue;
                }
                let track_bin = (Self::F0 + self.rate() * into_call / Self::SR) / bin_hz;
                for (bin, &p) in cell.iter().enumerate() {
                    total += p;
                    if (bin as f64 - track_bin).abs() <= 2.5 {
                        on_track += p;
                    }
                }
            }
            assert!(total > 0.0, "the sweep produced no energy");
            on_track / total
        }
    }

    /// Repeatable white noise, uniform in [-amp, amp).
    fn white_noise(len: usize, amp: f32, seed: u64) -> Vec<f32> {
        let mut s = seed;
        (0..len)
            .map(|_| {
                s = s
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                amp * ((s >> 40) as f32 / (1u64 << 24) as f32 * 2.0 - 1.0)
            })
            .collect()
    }

    #[test]
    fn reassigned_chirp_follows_its_frequency_track() {
        let (fft, hop, cols) = (512, 32, 96);
        let lead = reassign_margin_cols(fft, hop);
        let sweep = Sweep {
            start: (lead + 20) * hop,
        };
        let mut samples = vec![0.0f32; (lead + cols + lead) * hop + fft];
        sweep.add_to(&mut samples, 1.0);

        let tile = compute_reassigned_tile(&samples, lead, cols, fft, hop, -60.0, &[]);
        let share = sweep.share_on_track(&reassigned_power(&tile), lead, fft, hop);
        assert!(
            share > 0.9,
            "only {:.1}% of the sweep's energy is within 2.5 bins of its track",
            100.0 * share
        );
    }

    #[test]
    fn reassigned_noise_stays_put_and_calls_still_sharpen() {
        // Noise under the floor gate keeps the plain spectrogram's place, so
        // it looks the same as with reassignment off instead of speckled. A
        // sweep far above the noise is still moved onto its track.
        let (fft, hop, cols) = (512, 32, 96);
        let lead = reassign_margin_cols(fft, hop);
        let floor = reassign_noise_floor(&white_noise(131_072, 0.01, 1), fft);
        let mut samples = white_noise((lead + cols + lead) * hop + fft, 0.01, 2);
        // Starts in column 60. Columns 0-39 hold only noise: their frames end
        // before it, and its energy moves to where it is, never earlier.
        let sweep = Sweep {
            start: (lead + 60) * hop,
        };
        sweep.add_to(&mut samples, 0.5);

        let tile = compute_reassigned_tile(&samples, lead, cols, fft, hop, -200.0, &floor);
        let cells = reassigned_power(&tile);
        let plain = compute_stft_columns(&samples[lead * hop..], 384_000, fft, hop, 0, cols);
        let (mut same, mut checked) = (0usize, 0usize);
        for col in 0..40 {
            for (bin, &mag) in plain[col].magnitudes.iter().enumerate() {
                let plain_db = 20.0 * (mag as f64).log10();
                let reassigned_db = 10.0 * cells[col][bin].log10();
                checked += 1;
                if (plain_db - reassigned_db).abs() < 0.01 {
                    same += 1;
                }
            }
        }
        assert!(
            same as f64 / checked as f64 > 0.99,
            "only {same} of {checked} noise cells match the plain spectrogram"
        );

        let share = sweep.share_on_track(&cells, lead, fft, hop);
        assert!(
            share > 0.9,
            "only {:.1}% of the sweep's energy is within 2.5 bins of its track",
            100.0 * share
        );
    }

    #[test]
    fn moved_bins_leave_their_noise_share() {
        // A tone far above the noise moves its main lobe onto one bin. The
        // bins it moved from keep the noise floor's power instead of going
        // darker than the noise around them.
        let (fft, hop, cols) = (256, 64, 32);
        let lead = reassign_margin_cols(fft, hop);
        let floor = reassign_noise_floor(&white_noise(65_536, 0.01, 4), fft);
        let mut samples = white_noise((lead + cols + lead) * hop + fft, 0.01, 5);
        for (i, s) in samples.iter_mut().enumerate() {
            *s += (2.0 * std::f64::consts::PI * 20.3 * i as f64 / fft as f64).sin() as f32;
        }

        let tile = compute_reassigned_tile(&samples, lead, cols, fft, hop, -200.0, &floor);
        let cells = reassigned_power(&tile);
        for (col, cell) in cells.iter().enumerate() {
            for bin in [18, 19, 21, 22] {
                assert!(
                    cell[bin] >= 0.99 * floor[bin] as f64,
                    "column {col}, bin {bin}: {:.2e} is under its noise floor {:.2e}",
                    cell[bin],
                    floor[bin]
                );
            }
            let lobe: f64 = cell[18..=22].iter().sum();
            assert!(
                cell[20] / lobe > 0.95,
                "column {col}: only {:.1}% of the tone is in bin 20",
                100.0 * cell[20] / lobe
            );
        }
    }

    #[test]
    fn noise_floor_ignores_occasional_calls() {
        // Calls in a few frames must not raise the floor.
        let fft = 512;
        let quiet = reassign_noise_floor(&white_noise(131_072, 0.01, 3), fft);
        let mut busy = white_noise(131_072, 0.01, 3);
        for start in (0..120_000).step_by(20_000) {
            Sweep { start }.add_to(&mut busy, 0.5);
        }
        let with_calls = reassign_noise_floor(&busy, fft);
        let worst_db = quiet
            .iter()
            .zip(&with_calls)
            .map(|(q, c)| 10.0 * (c / q).log10())
            .fold(0.0f32, f32::max);
        assert!(worst_db < 3.0, "calls raised the floor by {worst_db:.1} dB");
    }

    #[test]
    fn reassigned_tiles_join_without_a_seam() {
        // Two adjacent tiles, each with its own margins, must match one tile
        // twice as wide: energy moved across the join is kept, not lost or
        // piled onto the edge columns.
        let (fft, hop, cols) = (1024, 32, 48);
        let margin = reassign_margin_cols(fft, hop);
        let n = (margin + 2 * cols + margin) * hop + fft;
        let mut samples: Vec<f32> = (0..n)
            .map(|i| {
                let t = i as f64 / 384_000.0;
                let f = 30_000.0 + 2.0e7 * t; // slow upward sweep
                (0.3 * (2.0 * std::f64::consts::PI * f * t).sin()) as f32
            })
            .collect();
        // A click right on the join.
        samples[(margin + cols) * hop + 511] += 1.0;

        let wide = compute_reassigned_tile(&samples, margin, 2 * cols, fft, hop, -80.0, &[]);
        let left = compute_reassigned_tile(&samples, margin, cols, fft, hop, -80.0, &[]);
        let right =
            compute_reassigned_tile(&samples[cols * hop..], margin, cols, fft, hop, -80.0, &[]);

        let height = wide.height as usize;
        for y in 0..height {
            for col in 0..2 * cols {
                let joined = if col < cols {
                    left.db_data[y * cols + col]
                } else {
                    right.db_data[y * cols + col - cols]
                };
                let whole = wide.db_data[y * 2 * cols + col];
                assert!(
                    joined == whole || (joined - whole).abs() < 1e-3,
                    "row {y}, column {col}: two tiles give {joined} dB, one wide tile gives {whole} dB"
                );
            }
        }
    }
}
