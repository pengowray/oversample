//! Chromagram computation: maps STFT magnitude columns to 12 pitch classes,
//! each subdivided by octave.

/// Number of pitch classes (C, C#, D, ..., B).
pub const NUM_PITCH_CLASSES: usize = 12;

/// Maximum number of octaves the internal arrays can hold.
/// Octave 0 = C0 (16.35 Hz), octave 15 ≈ C15 (536 kHz).
pub const MAX_OCTAVES: usize = 16;

/// Default number of octaves for the "Musical" range (octaves 0–9).
pub const NUM_OCTAVES: usize = 10;

/// Result of mapping one STFT column to chromagram data.
pub struct ChromagramColumn {
    /// Total intensity per pitch class (sum across all octaves).
    pub pitch_classes: [f32; NUM_PITCH_CLASSES],
    /// Per-octave detail: `octave_detail[pitch_class][octave]`.
    /// Indexed up to `MAX_OCTAVES`; only entries within the active range are meaningful.
    pub octave_detail: [[f32; MAX_OCTAVES]; NUM_PITCH_CLASSES],
}

/// Pitch class names for labelling.
pub const PITCH_CLASS_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Convert an STFT magnitude column to a chromagram column.
///
/// `magnitudes`: FFT magnitude bins (index 0 = DC, index N = Nyquist).
/// `freq_resolution`: Hz per FFT bin (= sample_rate / fft_size).
/// `min_octave`: first octave to include (0 = C0).
/// `num_octaves`: how many octaves to include.
pub fn stft_to_chromagram(
    magnitudes: &[f32],
    freq_resolution: f64,
    min_octave: usize,
    num_octaves: usize,
) -> ChromagramColumn {
    let mut pitch_classes = [0.0f32; NUM_PITCH_CLASSES];
    let mut octave_detail = [[0.0f32; MAX_OCTAVES]; NUM_PITCH_CLASSES];

    let max_octave = min_octave + num_octaves; // exclusive

    // Frequency bounds for the active range
    // C(min_octave) ≈ 16.35 * 2^min_octave
    let min_freq = 16.35 * (1u64 << min_octave) as f64 * 0.95; // slight margin
    // B(max_octave-1) ≈ 15.89 * 2^max_octave (top of the last included octave)
    let max_freq = 15.89 * (1u64 << max_octave.min(20)) as f64 * 1.05;

    for (bin_idx, &mag) in magnitudes.iter().enumerate() {
        if bin_idx == 0 { continue; } // skip DC
        let freq = bin_idx as f64 * freq_resolution;
        if freq < min_freq { continue; }
        if freq > max_freq { break; } // bins are monotonically increasing

        // MIDI note number: 69 = A4 = 440 Hz
        let midi = 69.0 + 12.0 * (freq / 440.0).log2();
        if midi < 0.0 { continue; }

        let midi_rounded = midi.round() as usize;
        let pc = midi_rounded % 12;
        let octave = (midi_rounded / 12).saturating_sub(1);

        if octave < min_octave || octave >= max_octave { continue; }

        // Use energy (mag²) for better perceptual weighting
        let energy = mag * mag;
        pitch_classes[pc] += energy;
        // Store with absolute octave index (not relative)
        octave_detail[pc][octave] += energy;
    }

    ChromagramColumn { pitch_classes, octave_detail }
}

/// The total number of logical rows in a chromagram display.
pub fn chroma_rows(num_octaves: usize) -> usize {
    NUM_PITCH_CLASSES * num_octaves
}

/// Vertical render scale: each logical row is rendered as this many pixel rows
/// for smoother upscaling.
pub const CHROMA_RENDER_SCALE: usize = 3;

/// Actual pixel height of chromagram tiles for a given number of octaves.
pub fn chroma_pixel_height(num_octaves: usize) -> usize {
    chroma_rows(num_octaves) * CHROMA_RENDER_SCALE
}

/// Compute the global chromagram max (max_class, max_note) across a slice of
/// STFT columns.  Used to normalise all tiles to the same scale — analogous to
/// `global_max_magnitude` for the main spectrogram.
pub fn compute_chroma_max(
    stft_columns: &[crate::types::SpectrogramColumn],
    freq_resolution: f64,
    min_octave: usize,
    num_octaves: usize,
) -> (f32, f32) {
    let mut max_class = 0.0f32;
    let mut max_note = 0.0f32;
    let max_octave = min_octave + num_octaves;
    for col in stft_columns {
        let ch = stft_to_chromagram(&col.magnitudes, freq_resolution, min_octave, num_octaves);
        for &v in &ch.pitch_classes { max_class = max_class.max(v); }
        for octaves in &ch.octave_detail {
            for &v in &octaves[min_octave..max_octave.min(MAX_OCTAVES)] {
                max_note = max_note.max(v);
            }
        }
    }
    (max_class, max_note)
}

/// Pre-render a set of STFT columns as a chromagram tile.
///
/// `max_class` / `max_note` are the **global** normalisation maxima (from
/// `compute_chroma_max` over the entire file).  All tiles use the same values
/// so brightness is consistent across the full chromagram.
///
/// `min_octave` / `num_octaves` define the octave range to render.
///
/// Returns greyscale RGBA pixels where:
/// - Width = number of columns
/// - Height = `chroma_pixel_height(num_octaves)`
/// - Row 0 = top of highest octave, last row = bottom of lowest octave
///
/// Each pixel encodes two values packed into the RGB channels:
/// - R channel: overall pitch class intensity (0–255)
/// - G channel: specific note (octave) intensity (0–255)
/// - B channel: energy flow (128 = neutral, 0 = max decrease, 255 = max increase)
/// - A channel: 255
///
/// The 2D colormap is applied during blit (not baked in), so the chromagram
/// view can adjust color mapping without re-rendering tiles.
/// `gain_db`: boost in decibels applied before u8 quantization (0 = no boost).
///   Positive values amplify quiet signals; applied by lowering the effective
///   normalisation max so that `10^(gain_db/20)` more amplitude is visible.
pub fn pre_render_chromagram_columns(
    stft_columns: &[crate::types::SpectrogramColumn],
    freq_resolution: f64,
    max_class: f32,
    max_note: f32,
    min_octave: usize,
    num_octaves: usize,
    gain_db: f32,
) -> crate::types::PreRendered {
    // Compute all chromagram columns from the STFT, then hand off to the
    // shared pixel renderer (also used by the resonator path).
    let chromas: Vec<ChromagramColumn> = stft_columns.iter()
        .map(|col| stft_to_chromagram(&col.magnitudes, freq_resolution, min_octave, num_octaves))
        .collect();
    pre_render_chroma_from_columns(&chromas, max_class, max_note, min_octave, num_octaves, gain_db)
}

/// Render pre-computed chromagram columns to a greyscale RGBA tile.
///
/// This is the rendering half of [`pre_render_chromagram_columns`], split out
/// so the resonator chromagram path (which builds `ChromagramColumn`s directly
/// from a note-tuned resonator bank rather than from STFT bins) can reuse the
/// exact same pixel layout, flow encoding, gain, and normalisation.
///
/// See [`pre_render_chromagram_columns`] for the channel packing details.
pub fn pre_render_chroma_from_columns(
    chromas: &[ChromagramColumn],
    max_class: f32,
    max_note: f32,
    min_octave: usize,
    num_octaves: usize,
    gain_db: f32,
) -> crate::types::PreRendered {
    use crate::types::PreRendered;

    if chromas.is_empty() {
        return PreRendered { width: 0, height: 0, pixels: Vec::new(), db_data: Vec::new(), flow_shifts: Vec::new() };
    }

    let width = chromas.len();
    let height = chroma_pixel_height(num_octaves);
    let mut pixels = vec![0u8; width * height * 4];

    if max_class <= 0.0 || max_note <= 0.0 {
        return PreRendered { width: width as u32, height: height as u32, pixels, db_data: Vec::new(), flow_shifts: Vec::new() };
    }

    // Apply gain by lowering the effective normalisation max.
    // gain_db is in decibels; since we normalise energy (amplitude²) values,
    // each dB of amplitude gain halves the energy divisor by 10^(dB/10).
    let gain_factor = if gain_db.abs() > 0.01 {
        10.0f32.powf(gain_db / 10.0) // energy-domain gain
    } else {
        1.0
    };
    let eff_max_class = max_class / gain_factor;
    let eff_max_note = max_note / gain_factor;

    let max_octave = min_octave + num_octaves;

    // Render pixels with flow data in B channel
    for (col_idx, chroma) in chromas.iter().enumerate() {
        for pc in 0..NUM_PITCH_CLASSES {
            let class_norm = (chroma.pitch_classes[pc] / eff_max_class).sqrt().min(1.0);
            let class_byte = (class_norm * 255.0) as u8;

            for oct_abs in min_octave..max_octave.min(MAX_OCTAVES) {
                let oct_rel = oct_abs - min_octave; // relative index for row layout
                let note_norm = (chroma.octave_detail[pc][oct_abs] / eff_max_note).sqrt().min(1.0);
                let note_byte = (note_norm * 255.0) as u8;

                // B channel: energy flow between consecutive columns
                let flow_byte = if col_idx == 0 {
                    128u8 // neutral for first column
                } else {
                    let curr = chroma.octave_detail[pc][oct_abs];
                    let prev = chromas[col_idx - 1].octave_detail[pc][oct_abs];
                    let delta = (curr - prev) / max_note;
                    ((delta * 128.0) + 128.0).clamp(0.0, 255.0) as u8
                };

                // Row layout: pitch class 0 (C) at bottom, B at top
                // Within each pitch class: lowest displayed octave at bottom, highest at top
                let row_from_bottom = pc * num_octaves + oct_rel;
                for s in 0..CHROMA_RENDER_SCALE {
                    let y = height - 1 - (row_from_bottom * CHROMA_RENDER_SCALE + s);
                    let pixel_idx = (y * width + col_idx) * 4;
                    pixels[pixel_idx] = class_byte;       // R = pitch class intensity
                    pixels[pixel_idx + 1] = note_byte;    // G = note intensity
                    pixels[pixel_idx + 2] = flow_byte;    // B = energy flow
                    pixels[pixel_idx + 3] = 255;
                }
            }
        }
    }

    PreRendered { width: width as u32, height: height as u32, pixels, db_data: Vec::new(), flow_shifts: Vec::new() }
}

/// Default quality factor for the note-aligned resonator chromagram.
///
/// In this code's convention `bandwidth = f / q` is the resonator's −3 dB
/// half-width, so the −3 dB FWHM is `2·f/q`. q = 24 ⇒ FWHM ≈ f/12, i.e.
/// roughly one semitone wide at every frequency — matching the classic
/// "one bin per semitone" constant-Q chromagram. Lower q overlaps adjacent
/// notes (every broadband click then lights up all of them); higher q
/// narrows them but slows the EMA's time response at low frequencies.
pub const CHROMA_RESONATOR_Q: f32 = 24.0;

/// Cap on the per-resonator EMA time constant, in seconds.
///
/// Constant-Q would give τ = q/(2π·f), so at 16 Hz with q=24, τ would be
/// ~240 ms — long enough that a single broadband bat click sits in the
/// low-octave resonators for a quarter-second, dominating the per-pitch
/// class sums and washing the chromagram into uniform brightness across
/// all 12 notes. Capping τ at 30 ms keeps transient events visible as
/// transients; below the cap the low-frequency resonators trade pitch
/// selectivity for time response (which doesn't matter for bat work and
/// is only mildly relevant for sub-bass musical use).
pub const CHROMA_RESONATOR_TAU_MAX_SECS: f32 = 0.030;

/// Frequency in Hz of a chromagram note. `pc` = pitch class (0=C..11=B),
/// `octave` = chromagram octave (0 = C0 ≈ 16.35 Hz). Matches the
/// `(midi/12 - 1)` octave indexing used by [`stft_to_chromagram`].
fn note_freq_hz(pc: usize, octave: usize) -> f64 {
    let midi = ((octave + 1) * 12 + pc) as f64;
    440.0 * 2.0f64.powf((midi - 69.0) / 12.0)
}

/// Compute chromagram columns directly from a constant-Q resonator bank with
/// one resonator tuned to each note in the active octave range.
///
/// Unlike [`stft_to_chromagram`] — which re-bins linear FFT bins into notes,
/// giving coarse resolution at low frequencies (one bin spans several
/// semitones) and blurry attribution at high frequencies — every note gets a
/// dedicated resonator at its exact frequency, so pitch selectivity is
/// uniform across the whole range.
///
/// Parameters mirror [`crate::dsp::resonators::compute_resonator_columns`]:
/// a fresh bank is built per call, so the caller should pre-pad `samples`
/// with warm-up and pass `col_start` = warm-up column count.
///
/// `q` is the per-resonator quality factor (bandwidth = f/q). Use
/// [`CHROMA_RESONATOR_Q`] for the default.
pub fn compute_chroma_columns_resonators(
    samples: &[f32],
    sample_rate: u32,
    hop_size: usize,
    col_start: usize,
    col_count: usize,
    min_octave: usize,
    num_octaves: usize,
    q: f32,
) -> Vec<ChromagramColumn> {
    use resonators::{alpha_from_tau, ResonatorBank, ResonatorConfig};

    let empty_col = || ChromagramColumn {
        pitch_classes: [0.0; NUM_PITCH_CLASSES],
        octave_detail: [[0.0; MAX_OCTAVES]; NUM_PITCH_CLASSES],
    };

    if samples.is_empty() || col_count == 0 || hop_size == 0 {
        return Vec::new();
    }

    let sr_f = sample_rate as f32;
    let nyq = sr_f * 0.5;
    let q = q.max(0.5);
    let max_octave = (min_octave + num_octaves).min(MAX_OCTAVES);
    // Bandwidth floor that enforces the τ cap (see CHROMA_RESONATOR_TAU_MAX_SECS).
    let bw_floor = 1.0 / (std::f32::consts::TAU * CHROMA_RESONATOR_TAU_MAX_SECS);

    // One resonator per in-range note whose frequency is below Nyquist.
    // `bin_map[i] = (pc, octave)` tells the per-frame loop where bin i's
    // magnitude belongs in the output `ChromagramColumn`.
    let mut configs: Vec<ResonatorConfig> = Vec::new();
    let mut bin_map: Vec<(usize, usize)> = Vec::new();
    for octave in min_octave..max_octave {
        for pc in 0..NUM_PITCH_CLASSES {
            let f = note_freq_hz(pc, octave) as f32;
            if f <= 0.0 || f >= nyq * 0.999 { continue; }
            // Constant-Q at audible+ frequencies; widen below the τ cap so
            // low notes don't hold broadband click energy for hundreds of ms.
            let bw = (f / q).max(bw_floor).clamp(0.1, nyq * 0.99);
            let tau = 1.0 / (std::f32::consts::TAU * bw);
            let alpha = alpha_from_tau(tau, sr_f);
            // beta=1.0 disables the library's second-stage output EWMA to
            // match the single-EWMA response of the rest of the project.
            configs.push(ResonatorConfig::new(f, alpha, 1.0));
            bin_map.push((pc, octave));
        }
    }

    if configs.is_empty() {
        return (0..col_count).map(|_| empty_col()).collect();
    }

    let mut bank = ResonatorBank::new(&configs, sr_f);
    let col_end = col_start + col_count;
    let total_samples = samples.len();
    let mut out: Vec<ChromagramColumn> = Vec::with_capacity(col_count);

    // Stream hop-by-hop to avoid the upstream `resonate()` allocation, same
    // pattern as `compute_resonator_columns`.
    let mut pos = 0usize;
    for frame in 0..col_end {
        let next = pos + hop_size;
        if next > total_samples { break; }
        bank.process_samples(&samples[pos..next]);
        pos = next;
        if frame < col_start { continue; }

        let mut col = empty_col();
        for (k, &(pc, octave)) in bin_map.iter().enumerate() {
            let mag = bank.magnitude(k);
            let energy = mag * mag;
            col.pitch_classes[pc] += energy;
            col.octave_detail[pc][octave] += energy;
        }
        out.push(col);
    }

    out
}

/// Compute `(max_class, max_note)` over a slice of already-computed
/// chromagram columns. Used by the resonator chroma path for progressive
/// global-max tracking (the STFT path uses [`compute_chroma_max`] which
/// builds the columns from STFT magnitudes on the fly).
pub fn compute_chroma_max_from_columns(
    chromas: &[ChromagramColumn],
    min_octave: usize,
    num_octaves: usize,
) -> (f32, f32) {
    let mut max_class = 0.0f32;
    let mut max_note = 0.0f32;
    let max_octave = (min_octave + num_octaves).min(MAX_OCTAVES);
    for ch in chromas {
        for &v in &ch.pitch_classes { max_class = max_class.max(v); }
        for octaves in &ch.octave_detail {
            for &v in &octaves[min_octave..max_octave] {
                max_note = max_note.max(v);
            }
        }
    }
    (max_class, max_note)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a magnitude column where a single FFT bin is set, at the bin nearest
    /// `freq_hz`. Returns the full magnitudes vec.
    fn one_tone_magnitudes(freq_hz: f64, freq_resolution: f64, num_bins: usize, mag: f32) -> Vec<f32> {
        let mut mags = vec![0.0f32; num_bins];
        let bin = (freq_hz / freq_resolution).round() as usize;
        if bin < num_bins {
            mags[bin] = mag;
        }
        mags
    }

    #[test]
    fn chroma_rows_scales_with_octaves() {
        assert_eq!(chroma_rows(1), 12);
        assert_eq!(chroma_rows(10), 120);
        assert_eq!(chroma_rows(0), 0);
    }

    #[test]
    fn chroma_pixel_height_multiplies_by_render_scale() {
        assert_eq!(chroma_pixel_height(10), chroma_rows(10) * CHROMA_RENDER_SCALE);
    }

    #[test]
    fn a4_lands_in_pitch_class_a() {
        // A4 = 440 Hz. With 1 Hz/bin resolution we put a tone exactly on the bin.
        // Pitch class index for "A" is 9 (C=0..B=11).
        let mags = one_tone_magnitudes(440.0, 1.0, 1024, 1.0);
        let chroma = stft_to_chromagram(&mags, 1.0, 0, 10);
        let a_idx = PITCH_CLASS_NAMES.iter().position(|&n| n == "A").unwrap();
        // The "A" class should be the strongest of the 12.
        let strongest = chroma.pitch_classes
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap()
            .0;
        assert_eq!(strongest, a_idx, "440 Hz should land in pitch class A");
        // Energy should be magnitude² = 1.
        assert!((chroma.pitch_classes[a_idx] - 1.0).abs() < 1e-5);
        // Octave 4 should hold the energy.
        assert!((chroma.octave_detail[a_idx][4] - 1.0).abs() < 1e-5);
    }

    #[test]
    fn dc_bin_is_ignored() {
        // Magnitude at bin 0 (DC) must NOT contribute to any pitch class.
        let mut mags = vec![0.0f32; 1024];
        mags[0] = 100.0;
        let chroma = stft_to_chromagram(&mags, 1.0, 0, 10);
        assert!(chroma.pitch_classes.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn out_of_range_octaves_skipped() {
        // 440 Hz (A4) with the analysis range starting above octave 4 — no contribution.
        let mags = one_tone_magnitudes(440.0, 1.0, 1024, 1.0);
        let chroma = stft_to_chromagram(&mags, 1.0, 6, 4); // octaves 6..10
        let a_idx = PITCH_CLASS_NAMES.iter().position(|&n| n == "A").unwrap();
        assert_eq!(chroma.pitch_classes[a_idx], 0.0);
    }

    #[test]
    fn empty_magnitudes_returns_all_zeros() {
        let chroma = stft_to_chromagram(&[], 1.0, 0, 10);
        assert!(chroma.pitch_classes.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn resonator_chroma_peaks_at_tone_note() {
        // 440 Hz (A4) sine for 1 second through the note-aligned resonator
        // bank should put its peak energy in pitch class A, octave 4.
        let sr = 48_000u32;
        let f = 440.0f32;
        let samples: Vec<f32> = (0..sr as usize)
            .map(|i| (std::f32::consts::TAU * f * i as f32 / sr as f32).sin())
            .collect();
        let hop = 512;
        // Warm-up so the EMA has converged before we read columns.
        let warmup_cols = 64;
        let total_cols = warmup_cols + 16;
        let cols = compute_chroma_columns_resonators(
            &samples, sr, hop, warmup_cols, total_cols - warmup_cols,
            0, 10, CHROMA_RESONATOR_Q,
        );
        assert!(!cols.is_empty(), "should emit columns past warm-up");
        let last = cols.last().unwrap();
        let a_idx = PITCH_CLASS_NAMES.iter().position(|&n| n == "A").unwrap();
        let strongest = last.pitch_classes
            .iter().enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap()).unwrap().0;
        assert_eq!(strongest, a_idx, "440 Hz should land in pitch class A");
        // Among octaves of A, octave 4 should win.
        let a_oct = (0..MAX_OCTAVES)
            .max_by(|&i, &j| last.octave_detail[a_idx][i]
                .partial_cmp(&last.octave_detail[a_idx][j]).unwrap()).unwrap();
        assert_eq!(a_oct, 4, "440 Hz is A4");
    }

    #[test]
    fn chroma_max_from_columns_matches_stft_path() {
        // A single STFT-derived chromagram column passed through
        // `compute_chroma_max_from_columns` must equal what
        // `compute_chroma_max` produces from the same STFT input.
        use crate::types::SpectrogramColumn;
        let mags = one_tone_magnitudes(440.0, 1.0, 1024, 2.0);
        let col = SpectrogramColumn { magnitudes: mags, time_offset: 0.0 };
        let (stft_max_class, stft_max_note) =
            compute_chroma_max(std::slice::from_ref(&col), 1.0, 0, 10);
        let chroma = stft_to_chromagram(&col.magnitudes, 1.0, 0, 10);
        let (col_max_class, col_max_note) =
            compute_chroma_max_from_columns(std::slice::from_ref(&chroma), 0, 10);
        assert!((stft_max_class - col_max_class).abs() < 1e-6);
        assert!((stft_max_note - col_max_note).abs() < 1e-6);
    }

    #[test]
    fn pitch_class_names_well_formed() {
        assert_eq!(PITCH_CLASS_NAMES.len(), NUM_PITCH_CLASSES);
        assert_eq!(PITCH_CLASS_NAMES[0], "C");
        assert_eq!(PITCH_CLASS_NAMES[11], "B");
    }
}
