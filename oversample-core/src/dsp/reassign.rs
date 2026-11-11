//! Reassigned spectrograms: each bin's energy moved to the time and
//! frequency where it actually is, from two extra FFTs per frame (a
//! time-ramped and a derivative Hann window).

use crate::dsp::fft::{hann_window, plan_fft_forward};
use crate::types::PreRendered;
use std::cell::RefCell;
use std::collections::HashMap;

thread_local! {
    static THANN_CACHE: RefCell<HashMap<usize, Vec<f32>>> = RefCell::new(HashMap::new());
    static DHANN_CACHE: RefCell<HashMap<usize, Vec<f32>>> = RefCell::new(HashMap::new());
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

/// Fewest frames of sound that give a usable noise floor.
const MIN_FLOOR_FRAMES: usize = 4;

/// Per-bin noise floor for [`compute_reassigned_tile`]: the median Hann-window
/// power of each bin over the frames in `frames`, back to back, `fft_size`
/// samples each.
///
/// The median ignores calls as long as they are in fewer than half the frames
/// at a given frequency. Frames of digital silence (padding, cut-out
/// stretches) are skipped, or they would pull the floor to zero. Returns an
/// empty Vec (no floor) when fewer than 4 frames have sound.
pub fn reassign_noise_floor(frames: &[f32], fft_size: usize) -> Vec<f32> {
    if fft_size == 0 {
        return Vec::new();
    }
    let sounding: Vec<&[f32]> = frames
        .chunks_exact(fft_size)
        .filter(|frame| frame.iter().any(|&s| s != 0.0))
        .collect();
    let frame_count = sounding.len();
    if frame_count < MIN_FLOOR_FRAMES {
        return Vec::new();
    }
    let n_bins = fft_size / 2 + 1;
    let fft = plan_fft_forward(fft_size);
    let window = hann_window(fft_size);
    let mut input = fft.make_input_vec();
    let mut spectrum = fft.make_output_vec();

    // by_bin[k * frame_count + f] = power of bin k in frame f
    let mut by_bin = vec![0.0f32; n_bins * frame_count];
    for (f, frame) in sounding.into_iter().enumerate() {
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

    let fft = plan_fft_forward(fft_size);
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

/// A column gets a point on the main-frequency line only when its strongest
/// reassigned energy is at least this far above the noise floor. White noise
/// almost never reaches it.
pub const LINE_ABOVE_FLOOR_DB: f32 = 15.0;

/// Two neighbouring points are joined only when the plain spectrogram has
/// sound at least this far above the floor...
const JOIN_ABOVE_FLOOR_DB: f32 = 6.0;
/// ...across at least this share of the frequencies between them, in one
/// column or the other. A sweep fills the gap; a jump from a call to an
/// unrelated sound in another band does not.
const JOIN_COVER: f32 = 0.8;

/// Bins either side of a column's strongest cell that are averaged into its
/// point: the Hann window's main lobe.
const LINE_LOBE_BINS: usize = 2;

/// One point of the main-frequency line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinePoint {
    /// Tile column the point belongs to.
    pub col: u16,
    /// Where the energy is in time, in columns from the tile's start, in the
    /// convention of `compute_stft_columns` (column `c` is the frame that
    /// starts at `c * hop_size`). Can be a little outside `col`.
    pub time_col: f32,
    /// Where the energy is in frequency, in FFT bins.
    pub freq_bin: f32,
    /// How far the column's strongest cell is above the noise floor, in dB.
    pub level_db: f32,
    /// Drawn joined to the point in column `col - 1`, which can be the last
    /// column of the previous tile.
    pub joins_previous: bool,
}

/// The main-frequency line for one tile: for each column, where its loudest
/// sound is, placed by reassignment so it sits on the call's actual track
/// between bins and between columns.
///
/// Bins at least [`REASSIGN_ABOVE_FLOOR_DB`] over `noise_floor` (and at least
/// `gate_db`) are moved to their reassigned time and frequency, keeping only
/// their power above the floor. Each column's point is the power-weighted
/// position of the strongest moved cell and its neighbours, if that cell is
/// [`LINE_ABOVE_FLOOR_DB`] over the floor. Neighbouring points are joined when
/// the plain spectrogram has sound across the frequencies between them, and a
/// point's frequency is the median of itself and the points joined to it.
/// Points joined to nothing are left out.
///
/// `samples` starts at the frame for column `-lead_cols`, as for
/// [`compute_reassigned_tile`]; pass [`reassign_margin_cols`] + 1 frames
/// either side so the first and last columns know their neighbours. Empty
/// without a noise floor.
pub fn main_frequency_line(
    samples: &[f32],
    lead_cols: usize,
    col_count: usize,
    fft_size: usize,
    hop_size: usize,
    gate_db: f32,
    noise_floor: &[f32],
) -> Vec<LinePoint> {
    let n_bins = fft_size / 2 + 1;
    if samples.len() < fft_size || col_count == 0 || hop_size == 0 || noise_floor.len() != n_bins {
        return Vec::new();
    }

    let fft = plan_fft_forward(fft_size);
    let win_h = hann_window(fft_size);
    let win_th = t_hann_window(fft_size);
    let win_dh = dh_window(fft_size);
    let mut input = fft.make_input_vec();
    let mut spec_h = fft.make_output_vec();
    let mut spec_th = fft.make_output_vec();
    let mut spec_dh = fft.make_output_vec();

    let db = |d: f32| 10.0f32.powf(d / 10.0);
    let gate_power = db(gate_db);
    let move_gate: Vec<f32> = noise_floor
        .iter()
        .map(|&p| (p * db(REASSIGN_ABOVE_FLOOR_DB)).max(gate_power))
        .collect();
    let fft_over_two_pi = fft_size as f64 / (2.0 * std::f64::consts::PI);
    let hop = hop_size as f64;
    let max_shift = fft_size as f64 / 2.0;

    // Columns -1 ..= col_count, so the tile's first and last columns can be
    // joined and smoothed with their neighbours in the next tiles. Row r of
    // these grids is column r - 1.
    let rows = col_count + 2;
    let mut power = vec![0.0f64; rows * n_bins];
    let mut sum_t = vec![0.0f64; rows * n_bins];
    let mut sum_f = vec![0.0f64; rows * n_bins];
    let mut plain: Vec<Option<Vec<f32>>> = vec![None; rows];

    let margin = reassign_margin_cols(fft_size, hop_size);
    let available_frames = (samples.len() - fft_size) / hop_size + 1;
    let frame_count = available_frames.min(lead_cols + col_count + 1 + margin);

    for frame_i in 0..frame_count {
        let frame = &samples[frame_i * hop_size..frame_i * hop_size + fft_size];
        let frame_col = frame_i as f64 - lead_cols as f64;
        for (spec, win) in [
            (&mut spec_h, &win_h),
            (&mut spec_th, &win_th),
            (&mut spec_dh, &win_dh),
        ] {
            for (inp, (&s, &w)) in input.iter_mut().zip(frame.iter().zip(win.iter())) {
                *inp = s * w;
            }
            fft.process(&mut input, spec).expect("FFT failed");
        }

        let row = frame_i as isize - lead_cols as isize + 1;
        if (0..rows as isize).contains(&row) {
            plain[row as usize] = Some(spec_h.iter().map(|c| c.norm_sqr()).collect());
        }

        for k in 0..n_bins {
            let p = spec_h[k].norm_sqr();
            if p < move_gate[k] {
                continue;
            }
            use realfft::num_complex::Complex;
            let xh = Complex::<f64>::new(spec_h[k].re as f64, spec_h[k].im as f64);
            let xth = Complex::<f64>::new(spec_th[k].re as f64, spec_th[k].im as f64);
            let xdh = Complex::<f64>::new(spec_dh[k].re as f64, spec_dh[k].im as f64);
            let dt = (xth / xh).re;
            // NaN (from X_h = 0) fails this too.
            let inside_window = dt.abs() <= max_shift;
            if !inside_window {
                continue;
            }
            let t = frame_col + dt / hop;
            let f = k as f64 + fft_over_two_pi * (xdh / xh).im;
            let (r, b) = ((t.round() + 1.0), f.round());
            let in_grid = (0.0..rows as f64).contains(&r) && (0.0..n_bins as f64).contains(&b);
            if !in_grid {
                continue;
            }
            let cell = r as usize * n_bins + b as usize;
            let excess = (p - noise_floor[k]) as f64;
            power[cell] += excess;
            sum_t[cell] += excess * t;
            sum_f[cell] += excess * f;
        }
    }

    // One candidate point per column.
    let need = db(LINE_ABOVE_FLOOR_DB) as f64;
    let points: Vec<Option<(f64, f64, f32)>> = (0..rows)
        .map(|r| {
            let cells = &power[r * n_bins..(r + 1) * n_bins];
            let excess = |b: usize| cells[b] / noise_floor[b].max(f32::MIN_POSITIVE) as f64;
            let peak = (0..n_bins).max_by(|&a, &b| excess(a).total_cmp(&excess(b)))?;
            if excess(peak) < need {
                return None;
            }
            let lobe = peak.saturating_sub(LINE_LOBE_BINS)..(peak + LINE_LOBE_BINS + 1).min(n_bins);
            let at = r * n_bins;
            let w: f64 = cells[lobe.clone()].iter().sum();
            let t: f64 = sum_t[at + lobe.start..at + lobe.end].iter().sum();
            let f: f64 = sum_f[at + lobe.start..at + lobe.end].iter().sum();
            Some((t / w, f / w, 10.0 * excess(peak).log10() as f32))
        })
        .collect();

    // joins[r]: the point in row r joins the one in row r - 1.
    let join_gate: Vec<f32> = noise_floor
        .iter()
        .map(|&p| p * db(JOIN_ABOVE_FLOOR_DB))
        .collect();
    let joins: Vec<bool> = (0..rows)
        .map(|r| {
            let (Some(a), Some(b)) = (points[r], r.checked_sub(1).and_then(|q| points[q])) else {
                return false;
            };
            let (Some(pa), Some(pb)) = (&plain[r], &plain[r - 1]) else {
                return false;
            };
            let (lo, hi) = (a.1.min(b.1), a.1.max(b.1));
            let between = (lo.ceil().max(0.0) as usize)..((hi.floor() as usize + 1).min(n_bins));
            if between.is_empty() {
                return true;
            }
            let loud = between
                .clone()
                .filter(|&k| pa[k].max(pb[k]) >= join_gate[k])
                .count();
            loud as f32 >= JOIN_COVER * between.len() as f32
        })
        .collect();

    (1..=col_count)
        .filter_map(|r| {
            let (t, f, level_db) = points[r]?;
            let joins_next = r + 1 < rows && joins[r + 1];
            if !joins[r] && !joins_next {
                return None;
            }
            // Median of three removes one-column spikes inside a joined run.
            let freq = if joins[r] && joins_next {
                let (a, b) = (points[r - 1]?.1, points[r + 1]?.1);
                a.max(b).min(f).max(a.min(b))
            } else {
                f
            };
            Some(LinePoint {
                col: (r - 1) as u16,
                time_col: t as f32,
                freq_bin: freq as f32,
                level_db,
                joins_previous: joins[r],
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::fft::compute_stft_columns;

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
    fn noise_floor_skips_digital_silence() {
        // Padding in most of the frames must not pull the floor to zero.
        let fft = 512;
        let noise = white_noise(16 * fft, 0.01, 6);
        let mut padded = vec![0.0f32; 48 * fft];
        padded.extend_from_slice(&noise);
        let floor = reassign_noise_floor(&padded, fft);
        assert_eq!(floor, reassign_noise_floor(&noise, fft));
        assert!(floor.iter().all(|&p| p > 0.0));
        assert!(reassign_noise_floor(&vec![0.0f32; 64 * fft], fft).is_empty());
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

    /// A tile's samples with `lead` = margin + 1 frames either side, as the
    /// line needs, filled with quiet noise; and the noise's floor.
    fn line_tile(fft: usize, hop: usize, cols: usize, seed: u64) -> (usize, Vec<f32>, Vec<f32>) {
        let lead = reassign_margin_cols(fft, hop) + 1;
        let samples = white_noise((lead + cols + lead) * hop + fft, 0.001, seed);
        let floor = reassign_noise_floor(&white_noise(131_072, 0.001, seed + 100), fft);
        (lead, samples, floor)
    }

    #[test]
    fn line_follows_a_sweep_between_bins_and_columns() {
        let (fft, hop, cols) = (512, 32, 96);
        let (lead, mut samples, floor) = line_tile(fft, hop, cols, 10);
        let sweep = Sweep {
            start: (lead + 20) * hop,
        };
        sweep.add_to(&mut samples, 0.5);

        let line = main_frequency_line(&samples, lead, cols, fft, hop, -200.0, &floor);
        let bin_hz = Sweep::SR / fft as f64;
        let centre = (fft as f64 - 1.0) / 2.0;
        let on_track = line
            .iter()
            .filter(|p| {
                // The sample this point stands for, counted from samples[0].
                let s = (p.time_col as f64 + lead as f64) * hop as f64 + centre;
                let into_call = s - sweep.start as f64;
                let track = (Sweep::F0 + sweep.rate() * into_call / Sweep::SR) / bin_hz;
                (p.freq_bin as f64 - track).abs() <= 1.0
            })
            .count();
        let call_cols = sweep.len() / hop;
        assert!(
            line.len() >= call_cols * 3 / 4,
            "{} points for a call {call_cols} columns long",
            line.len()
        );
        assert!(
            on_track * 10 >= line.len() * 9,
            "only {on_track} of {} points are within a bin of the sweep",
            line.len()
        );
    }

    #[test]
    fn line_finds_a_tone_between_bins() {
        let (fft, hop, cols) = (256, 64, 32);
        let (lead, mut samples, floor) = line_tile(fft, hop, cols, 11);
        for (i, s) in samples.iter_mut().enumerate() {
            *s += (2.0 * std::f64::consts::PI * 20.3 * i as f64 / fft as f64).sin() as f32;
        }
        let line = main_frequency_line(&samples, lead, cols, fft, hop, -200.0, &floor);
        assert_eq!(line.len(), cols);
        for p in &line {
            assert!(
                (p.freq_bin - 20.3).abs() < 0.1,
                "column {}: {} bins",
                p.col,
                p.freq_bin
            );
            assert!(p.joins_previous, "column {} is not joined", p.col);
        }
    }

    #[test]
    fn no_line_in_white_noise() {
        let (fft, hop, cols) = (512, 32, 256);
        let lead = reassign_margin_cols(fft, hop) + 1;
        let samples = white_noise((lead + cols + lead) * hop + fft, 0.01, 12);
        let floor = reassign_noise_floor(&white_noise(131_072, 0.01, 13), fft);
        let line = main_frequency_line(&samples, lead, cols, fft, hop, -200.0, &floor);
        assert!(line.is_empty(), "{} points in noise", line.len());
    }

    #[test]
    fn line_does_not_join_unrelated_sounds() {
        // One tone hands over to another far above it, with nothing between
        // them: the line breaks at the handover instead of drawing a jump.
        let (fft, hop, cols) = (256, 64, 64);
        let (lead, mut samples, floor) = line_tile(fft, hop, cols, 14);
        let switch = (lead + 32) * hop;
        let fade = fft;
        for (i, s) in samples.iter_mut().enumerate() {
            let b = ((i as f64 - (switch - fade / 2) as f64) / fade as f64).clamp(0.0, 1.0);
            let ph = 2.0 * std::f64::consts::PI * i as f64 / fft as f64;
            *s += ((1.0 - b) * (20.3 * ph).sin() + b * (80.3 * ph).sin()) as f32;
        }
        let line = main_frequency_line(&samples, lead, cols, fft, hop, -200.0, &floor);
        let jumps: Vec<_> = line
            .windows(2)
            .filter(|w| w[1].col == w[0].col + 1 && w[1].joins_previous)
            .filter(|w| (w[1].freq_bin - w[0].freq_bin).abs() > 10.0)
            .collect();
        assert!(jumps.is_empty(), "joined across the gap: {jumps:?}");
        assert!(line.iter().any(|p| (p.freq_bin - 20.3).abs() < 0.5));
        assert!(line.iter().any(|p| (p.freq_bin - 80.3).abs() < 0.5));
    }

    #[test]
    fn line_tiles_join_without_a_seam() {
        // Two adjacent tiles give the same points as one tile twice as wide.
        let (fft, hop, cols) = (512, 32, 48);
        let lead = reassign_margin_cols(fft, hop) + 1;
        let mut samples = white_noise((lead + 2 * cols + lead) * hop + fft, 0.001, 15);
        Sweep {
            start: (lead + cols - 20) * hop,
        }
        .add_to(&mut samples, 0.5);
        let floor = reassign_noise_floor(&white_noise(131_072, 0.001, 16), fft);

        let wide = main_frequency_line(&samples, lead, 2 * cols, fft, hop, -200.0, &floor);
        let mut joined = main_frequency_line(&samples, lead, cols, fft, hop, -200.0, &floor);
        let right =
            main_frequency_line(&samples[cols * hop..], lead, cols, fft, hop, -200.0, &floor);
        joined.extend(right.into_iter().map(|p| LinePoint {
            col: p.col + cols as u16,
            time_col: p.time_col + cols as f32,
            ..p
        }));
        assert!(!wide.is_empty());
        assert_eq!(wide.len(), joined.len());
        for (a, b) in wide.iter().zip(&joined) {
            assert_eq!((a.col, a.joins_previous), (b.col, b.joins_previous));
            assert!(
                (a.time_col - b.time_col).abs() < 1e-3 && (a.freq_bin - b.freq_bin).abs() < 1e-3
            );
        }
    }
}
