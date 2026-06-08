// SPDX-License-Identifier: GPL-3.0-only OR MIT OR Apache-2.0
//! Thin adapter over the [`resonators`] crate — Alexandre François's Resonate
//! algorithm.
//!
//! The upstream crate implements the paper faithfully; this module only
//! reshapes its output into the project's [`SpectrogramColumn`] layout and
//! scales magnitudes to match STFT brightness so existing gain / floor_db
//! controls behave identically in Spectrogram and Resonators views.
//!
//! # Layout
//!
//! For compatibility with the existing spectrogram pipeline, we build a
//! linear-frequency bank of `num_bins = fft_size / 2 + 1` resonators covering
//! 0..Nyquist with `f_k = k · (sr/2) / (num_bins - 1)`. Downstream code
//! (row→freq mapping, tile blit, freq markers) needs no special cases.
//!
//! # References
//!
//! - Algorithm: <https://alexandrefrancois.org/Resonate/>
//! - C++ reference: <https://github.com/alexandrefrancois/noFFT>
//! - Rust reference (this crate): <https://github.com/jhartquist/resonators>

use crate::types::SpectrogramColumn;
use resonators::{ResonatorBank, ResonatorConfig, alpha_from_tau};

/// Frequency-bin spacing for a resonator bank.
///
/// Output always has `fft_size/2 + 1` rows (matching STFT so the rest of the
/// rendering pipeline stays linear); this enum only affects where the actual
/// resonators sit in frequency space. `Log` bins are resampled to the linear
/// output rows by nearest-bin-in-log-space mapping, which draws bat harmonics
/// as clean stripes while keeping the axis / overlay code unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ResonatorLayout {
    /// Evenly-spaced linear bins from 0 to Nyquist — same layout as the STFT.
    #[default]
    Linear,
    /// Log-spaced bins from `LOG_MIN_FREQ_HZ` to Nyquist. Gives more detail
    /// at low frequencies and concentrates bins where harmonic bat calls
    /// actually live.
    Log,
}

impl ResonatorLayout {
    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Log => "Log",
        }
    }

    pub const ALL: &'static [ResonatorLayout] = &[Self::Linear, Self::Log];
}

/// How each resonator's bandwidth (and thus its time/frequency tradeoff) is
/// chosen across the bank.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ResonatorAlphaMode {
    /// One global bandwidth (Hz) for every bin — uniform tradeoff at all
    /// frequencies. Q = f/bw rises with frequency, so high bins are needlessly
    /// slow. This is the original behavior the bandwidth slider is tuned for.
    #[default]
    ConstBandwidth,
    /// Constant Q: `bw_k = f_k / q`, so low bins are narrow (sharp in frequency)
    /// and high bins are wide (fast in time). Matches FM bat calls — sharp
    /// tonal/low structure, crisp tracking of fast high-frequency sweeps. The Q
    /// value is supplied separately (see `compute_resonator_columns`).
    ConstQ,
}

impl ResonatorAlphaMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::ConstBandwidth => "Const bandwidth",
            Self::ConstQ => "Const Q",
        }
    }

    pub const ALL: &'static [ResonatorAlphaMode] = &[Self::ConstBandwidth, Self::ConstQ];
}

/// Lowest frequency for log-spaced layouts. Below this the display shows the
/// lowest log bin's magnitude (no subsonic resonators).
pub const LOG_MIN_FREQ_HZ: f32 = 20.0;

/// Recommended warm-up samples for a given bandwidth.
///
/// Returns ≈5τ samples, where τ = 1/(2π·bandwidth) is the EMA time constant.
/// At 5τ the EMA has converged to within ~1% of steady state.
pub fn warmup_samples(sample_rate: u32, bandwidth_hz: f32) -> usize {
    let bw = bandwidth_hz.max(1.0);
    let tau_secs = 1.0 / (std::f32::consts::TAU * bw);
    (5.0 * tau_secs * sample_rate as f32).ceil().max(256.0) as usize
}

/// Compute resonator columns over a slice of audio samples.
///
/// Parameters mirror `dsp::fft::compute_stft_columns`:
/// - `fft_size` determines `num_bins = fft_size/2 + 1` (frequency resolution).
/// - `hop_size` is the output column interval in samples.
/// - `col_start`/`col_count` select which columns to emit (0-based, counted
///   from sample 0 of the input slice). A fresh bank is built per call, so
///   the caller should pre-pad with warm-up samples and pass `col_start` =
///   the warm-up column count.
///
/// `bandwidth_hz` sets the per-bin EMA bandwidth in `ConstBandwidth` mode
/// (uniform across all bins); smaller ⇒ sharper bins, slower tracking. In
/// `ConstQ` mode `bandwidth_hz` is ignored and each bin uses `bw_k = f_k / q`.
///
/// Output magnitudes are scaled by `fft_size * 0.5` to match the one-sided
/// STFT magnitude with Hann coherent gain, so existing brightness controls
/// work the same way in both views.
pub fn compute_resonator_columns(
    samples: &[f32],
    sample_rate: u32,
    fft_size: usize,
    hop_size: usize,
    col_start: usize,
    col_count: usize,
    bandwidth_hz: f32,
    alpha_mode: ResonatorAlphaMode,
    q: f32,
    layout: ResonatorLayout,
    freq_range: Option<(f32, f32)>,
) -> Vec<SpectrogramColumn> {
    let output_bins = fft_size / 2 + 1;
    if samples.is_empty() || output_bins == 0 || col_count == 0 || hop_size == 0 {
        return vec![];
    }

    // Tile path runs full-density: density is already baked into `fft_size`
    // (the adaptive FFT shrinks output_bins for loaded files).
    let setup = build_reso_setup(sample_rate, fft_size, bandwidth_hz, alpha_mode, q, layout, freq_range, 1.0);
    let mut bank = ResonatorBank::new(&setup.configs, sample_rate as f32);

    let col_end = col_start + col_count;
    let total_samples = samples.len();
    let mut out: Vec<SpectrogramColumn> = Vec::with_capacity(col_count);

    // Stream hop-by-hop instead of calling `bank.resonate()` — that method
    // allocates a `Vec<Complex32>` the size of (n_frames * n_bins) up front
    // (~1 MB per baseline tile) which we'd then discard. Processing one hop
    // at a time and reading magnitudes directly from bank state avoids the
    // intermediate buffer entirely.
    let mut pos = 0usize;
    for frame in 0..col_end {
        let next = pos + hop_size;
        if next > total_samples {
            break;
        }
        bank.process_samples(&samples[pos..next]);
        pos = next;

        if frame < col_start {
            continue;
        }

        // Library state reflects end of this hop.
        let time_offset = ((frame + 1) * hop_size) as f64 / sample_rate as f64;
        out.push(SpectrogramColumn { magnitudes: setup.read_mags(&bank), time_offset });
    }

    out
}

/// Shared resonator-bank setup (frequency layout, per-bin configs, log row-map,
/// magnitude scale) — built once and reused by both the one-shot
/// [`compute_resonator_columns`] and the persistent [`StreamingResonators`].
struct ResoSetup {
    configs: Vec<ResonatorConfig>,
    bank_bins: usize,
    row_to_bank: Option<Vec<usize>>,
    mag_scale: f32,
}

impl ResoSetup {
    /// Read the bank's current per-row magnitudes (scaled to STFT brightness),
    /// gathering through the log row-map when present.
    fn read_mags(&self, bank: &ResonatorBank) -> Vec<f32> {
        if let Some(map) = &self.row_to_bank {
            map.iter().map(|&k| bank.magnitude(k) * self.mag_scale).collect()
        } else {
            (0..self.bank_bins).map(|k| bank.magnitude(k) * self.mag_scale).collect()
        }
    }
}

/// `density` (0..1) scales the number of *resonators actually computed* without
/// changing `output_bins` (the display row count). At density 1.0 the bank has
/// one resonator per output row (Linear ⇒ identity, no gather). Below 1.0 the
/// bank is built with proportionally fewer resonators and a `row_to_bank` gather
/// stretches them across the fixed output rows — the live-capture lever: the
/// live waterfall's bin count is fixed (513), so we can't shrink the column, but
/// we can compute far fewer resonators (Quarter ⇒ ~4× cheaper).
fn build_reso_setup(
    sample_rate: u32,
    fft_size: usize,
    bandwidth_hz: f32,
    alpha_mode: ResonatorAlphaMode,
    q: f32,
    layout: ResonatorLayout,
    freq_range: Option<(f32, f32)>,
    density: f32,
) -> ResoSetup {
    let sr_f = sample_rate as f32;
    let nyq = sr_f * 0.5;
    let output_bins = fft_size / 2 + 1;
    // Number of resonators actually run. Fewer at low density; never more than
    // one per output row, never fewer than a small floor.
    let bank_bins = if density < 0.999 {
        ((output_bins as f32 * density).round() as usize).clamp(8, output_bins)
    } else {
        output_bins
    };

    // Default frequency range per layout. If the caller passes an explicit
    // range (e.g. viewport-zoom mode), use that instead — this is the key
    // resonator advantage over FFTs: we can concentrate all bins into the
    // user's current viewport for arbitrarily high vertical resolution.
    let (band_lo, band_hi) = freq_range
        .map(|(lo, hi)| (lo.max(0.01), hi.min(nyq).max(lo + 0.1)))
        .unwrap_or_else(|| match layout {
            ResonatorLayout::Linear => (0.01, nyq),
            ResonatorLayout::Log => (LOG_MIN_FREQ_HZ.max(0.01), nyq.max(LOG_MIN_FREQ_HZ * 2.0)),
        });

    // Build the `bank_bins` resonator frequencies spread across [band_lo,
    // band_hi] per layout.
    let bank_freqs: Vec<f32> = match layout {
        ResonatorLayout::Linear => {
            let denom = (bank_bins - 1).max(1) as f32;
            (0..bank_bins)
                .map(|k| (band_lo + k as f32 * (band_hi - band_lo) / denom).max(0.01))
                .collect()
        }
        ResonatorLayout::Log => {
            let min = band_lo.max(0.01);
            let max = band_hi.max(min * 2.0);
            if bank_bins == 1 {
                vec![min]
            } else {
                let ratio = (max / min).powf(1.0 / (bank_bins - 1) as f32);
                (0..bank_bins)
                    .map(|k| min * ratio.powi(k as i32))
                    .collect()
            }
        }
    };

    // Per-bin alpha. `alpha_from_tau(tau, sr) = 1 - exp(-dt/tau)` (the library's
    // "alpha large = fast response", the mirror of the prior scalar impl).
    // ConstBandwidth gives every bin the same bandwidth — the original behavior
    // the bandwidth slider is tuned against. ConstQ sets bw_k = f_k / q, so low
    // bins are sharp in frequency and high bins are fast in time (the per-bin
    // tradeoff suited to FM bat calls). The ConstQ 2 Hz floor keeps the lowest
    // bins' EMA warm-up bounded (those sub-bat bins are rarely the focus).
    // beta=1.0 disables the library's second-stage output EWMA (single-EWMA).
    let q = q.max(0.1);
    let configs: Vec<ResonatorConfig> = bank_freqs
        .iter()
        .map(|&f| {
            let bw = match alpha_mode {
                ResonatorAlphaMode::ConstBandwidth => bandwidth_hz.clamp(0.1, nyq * 0.99),
                ResonatorAlphaMode::ConstQ => (f / q).clamp(2.0, nyq * 0.99),
            };
            let tau = 1.0 / (std::f32::consts::TAU * bw);
            ResonatorConfig::new(f, alpha_from_tau(tau, sr_f), 1.0)
        })
        .collect();

    // Map each linear output row to a bank bin (a cheap gather in `read_mags`).
    // Identity for full-density Linear (no map); a linear gather for reduced
    // Linear; the log nearest-map for Log. The output row axis is linear over
    // [band_lo, band_hi] either way, matching the tile blit / freq markers.
    let row_to_bank: Option<Vec<usize>> = match layout {
        ResonatorLayout::Linear if bank_bins == output_bins => None,
        ResonatorLayout::Linear => {
            let denom_out = (output_bins - 1).max(1) as f32;
            let last = bank_bins - 1;
            Some(
                (0..output_bins)
                    .map(|r| ((r as f32 / denom_out) * last as f32).round() as usize)
                    .collect(),
            )
        }
        ResonatorLayout::Log => Some(build_log_row_map(output_bins, &bank_freqs, band_lo, band_hi)),
    };

    ResoSetup {
        configs,
        bank_bins,
        row_to_bank,
        mag_scale: (fft_size as f32) * 0.5,
    }
}

/// A persistent streaming resonator bank for live capture. Built once per
/// session/config, it processes each incoming sample exactly once and emits a
/// column per hop — avoiding the dominant cost of the live Resonators view,
/// where [`compute_resonator_columns`] otherwise re-creates and re-warms the
/// whole bank (≈5τ of samples) on every capture tick.
pub struct StreamingResonators {
    bank: ResonatorBank,
    setup: ResoSetup,
    hop_size: usize,
    sample_rate: u32,
}

impl StreamingResonators {
    /// `density` (0..1) computes proportionally fewer resonators while still
    /// emitting `fft_size/2+1` rows (the live waterfall's fixed bin count) — the
    /// live-capture perf lever. 1.0 = one resonator per row.
    pub fn new(
        sample_rate: u32,
        fft_size: usize,
        hop_size: usize,
        bandwidth_hz: f32,
        alpha_mode: ResonatorAlphaMode,
        q: f32,
        layout: ResonatorLayout,
        freq_range: Option<(f32, f32)>,
        density: f32,
    ) -> Self {
        let setup = build_reso_setup(sample_rate, fft_size, bandwidth_hz, alpha_mode, q, layout, freq_range, density);
        let bank = ResonatorBank::new(&setup.configs, sample_rate as f32);
        Self { bank, setup, hop_size, sample_rate }
    }

    /// Run `samples` through the bank without emitting columns — used once after
    /// a (re)build to converge the EMA state from recent buffer history so the
    /// first emitted columns are already settled.
    pub fn warm(&mut self, samples: &[f32]) {
        if !samples.is_empty() {
            self.bank.process_samples(samples);
        }
    }

    /// Feed a hop-aligned `samples` slice and emit one column per whole hop.
    /// `first_col` is the absolute column index of the first emitted column
    /// (only used to compute `time_offset`). Any trailing partial hop is
    /// ignored — callers pass whole-hop chunks.
    pub fn push_hops(&mut self, samples: &[f32], first_col: usize) -> Vec<SpectrogramColumn> {
        let n = samples.len() / self.hop_size;
        let mut out = Vec::with_capacity(n);
        for h in 0..n {
            let s = h * self.hop_size;
            self.bank.process_samples(&samples[s..s + self.hop_size]);
            let col = first_col + h;
            let time_offset = ((col + 1) * self.hop_size) as f64 / self.sample_rate as f64;
            out.push(SpectrogramColumn {
                magnitudes: self.setup.read_mags(&self.bank),
                time_offset,
            });
        }
        out
    }
}

/// For each linear output row (covering [band_lo, band_hi] uniformly), pick
/// the closest bank bin in log-frequency distance. Rows below the lowest
/// bank frequency use the lowest bank bin.
fn build_log_row_map(
    output_bins: usize,
    bank_freqs: &[f32],
    band_lo: f32,
    band_hi: f32,
) -> Vec<usize> {
    let denom = (output_bins - 1).max(1) as f32;
    let last_bank = bank_freqs.len() - 1;
    (0..output_bins)
        .map(|row| {
            let row_freq = (band_lo + row as f32 * (band_hi - band_lo) / denom).max(0.01);
            if row_freq <= bank_freqs[0] {
                return 0;
            }
            if row_freq >= bank_freqs[last_bank] {
                return last_bank;
            }
            let idx = bank_freqs
                .binary_search_by(|&f| {
                    f.partial_cmp(&row_freq).unwrap_or(std::cmp::Ordering::Equal)
                })
                .unwrap_or_else(|i| i)
                .min(last_bank);
            if idx == 0 {
                return 0;
            }
            let prev = bank_freqs[idx - 1];
            let curr = bank_freqs[idx];
            let dprev = (row_freq / prev).ln().abs();
            let dcurr = (row_freq / curr).ln().abs();
            if dprev <= dcurr { idx - 1 } else { idx }
        })
        .collect()
}

/// Result of a resonator bank benchmark run.
#[derive(Clone, Copy, Debug)]
pub struct BenchResult {
    /// Number of resonator bins in the bank.
    pub num_bins: usize,
    /// Number of input samples processed per iteration.
    pub samples_per_iter: usize,
    /// Number of iterations run.
    pub iterations: usize,
    /// Wall time for the hot loop, in milliseconds.
    pub elapsed_ms: f64,
}

/// Run a fixed-workload bench timing the resonators crate hot loop. Uses a
/// caller-supplied wall-clock `now_ms` (so this works in WASM via
/// `performance.now()` and natively via `Instant`).
///
/// `num_bins` controls the bank size, `samples_per_iter` the signal length
/// per call, `iterations` how many times the signal is fed through. For
/// meaningful timings, make `samples_per_iter * iterations` large enough
/// that the workload runs for at least a few tens of milliseconds.
///
/// Upstream `process_samples` is autovectorized by LLVM (WASM simd128, x86
/// SSE/AVX, aarch64 NEON) when the target feature is enabled at compile
/// time — there is no separate scalar entry point in the public API, so
/// this bench reports a single timing rather than a SIMD-vs-scalar ratio.
pub fn bench_resonator_bank<F: FnMut() -> f64>(
    num_bins: usize,
    samples_per_iter: usize,
    iterations: usize,
    bandwidth_hz: f32,
    sample_rate: u32,
    mut now_ms: F,
) -> BenchResult {
    use resonators::{ResonatorBank, ResonatorConfig, alpha_from_tau};

    // Match the production adapter: linear freq layout, single bandwidth,
    // beta=1.0 (disables the library's output EWMA).
    let sr_f = sample_rate as f32;
    let nyq = sr_f * 0.5;
    let tau = 1.0 / (std::f32::consts::TAU * bandwidth_hz.max(0.1));
    let alpha = alpha_from_tau(tau, sr_f);
    let denom = (num_bins - 1).max(1) as f32;
    let configs: Vec<ResonatorConfig> = (0..num_bins)
        .map(|k| {
            let f = (k as f32 * nyq / denom).max(0.01);
            ResonatorConfig::new(f, alpha, 1.0)
        })
        .collect();

    let signal: Vec<f32> = (0..samples_per_iter)
        .map(|i| {
            let t = i as f32 / sr_f;
            (std::f32::consts::TAU * 1000.0 * t).sin() * 0.5
        })
        .collect();

    let mut bank = ResonatorBank::new(&configs, sr_f);
    let t0 = now_ms();
    for _ in 0..iterations {
        bank.process_samples(&signal);
    }
    let elapsed_ms = now_ms() - t0;
    // Keep the result alive so the optimizer can't eliminate the loop.
    let _sink = bank.power(num_bins / 2);

    BenchResult {
        num_bins,
        samples_per_iter,
        iterations,
        elapsed_ms,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pure tone should produce a peak at the matching bin.
    #[test]
    fn peak_at_tone_frequency() {
        let sr = 48_000u32;
        let fft_size = 256;
        let hop = 64;
        let num_bins = fft_size / 2 + 1;

        // 6 kHz sine, 1 s long.
        let f = 6_000.0f32;
        let samples: Vec<f32> = (0..sr as usize)
            .map(|i| (std::f32::consts::TAU * f * i as f32 / sr as f32).sin())
            .collect();

        let cols = compute_resonator_columns(
            &samples, sr, fft_size, hop, 0, 100, 200.0,
            ResonatorAlphaMode::ConstBandwidth, 50.0, ResonatorLayout::Linear, None,
        );
        assert!(!cols.is_empty());

        // Look at a column well past warm-up.
        let mid = &cols[cols.len() - 1];
        let (peak_bin, _peak_val) = mid
            .magnitudes
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();

        let nyq = (sr as f32) / 2.0;
        let expected = (f / (nyq / (num_bins - 1) as f32)).round() as usize;
        assert!(
            (peak_bin as isize - expected as isize).abs() <= 1,
            "peak at bin {peak_bin}, expected {expected}"
        );
    }

    fn test_signal(sr: u32, n: usize) -> Vec<f32> {
        // Mix of two tones so multiple bins are exercised.
        (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                0.6 * (std::f32::consts::TAU * 12_000.0 * t).sin()
                    + 0.4 * (std::f32::consts::TAU * 30_000.0 * t).sin()
            })
            .collect()
    }

    fn assert_cols_eq(a: &[SpectrogramColumn], b: &[SpectrogramColumn]) {
        assert_eq!(a.len(), b.len(), "column count");
        for (i, (ca, cb)) in a.iter().zip(b.iter()).enumerate() {
            assert_eq!(ca.magnitudes.len(), cb.magnitudes.len(), "bin count col {i}");
            assert!(
                (ca.time_offset - cb.time_offset).abs() < 1e-9,
                "time_offset col {i}: {} vs {}", ca.time_offset, cb.time_offset
            );
            for (k, (x, y)) in ca.magnitudes.iter().zip(cb.magnitudes.iter()).enumerate() {
                assert!((x - y).abs() <= 1e-4, "mag col {i} bin {k}: {x} vs {y}");
            }
        }
    }

    /// Streaming a buffer from a fresh bank must match the one-shot path exactly.
    #[test]
    fn streaming_matches_oneshot() {
        let sr = 192_000u32;
        let (fft, hop, ncols) = (1024usize, 256usize, 60usize);
        for layout in [ResonatorLayout::Linear, ResonatorLayout::Log] {
            let samples = test_signal(sr, hop * ncols);
            let oneshot = compute_resonator_columns(
                &samples, sr, fft, hop, 0, ncols, 20.0,
                ResonatorAlphaMode::ConstBandwidth, 50.0, layout, None,
            );
            let mut s = StreamingResonators::new(
                sr, fft, hop, 20.0, ResonatorAlphaMode::ConstBandwidth, 50.0, layout, None, 1.0,
            );
            let streamed = s.push_hops(&samples[..ncols * hop], 0);
            assert_cols_eq(&oneshot, &streamed);
        }
    }

    /// Feeding in chunks (as the live loop does per tick) must match one feed —
    /// i.e. the bank state persists correctly across push_hops calls.
    #[test]
    fn streaming_incremental_matches_bulk() {
        let sr = 384_000u32;
        let (fft, hop, ncols) = (1024usize, 256usize, 80usize);
        let samples = test_signal(sr, hop * ncols);

        let mut bulk = StreamingResonators::new(
            sr, fft, hop, 20.0, ResonatorAlphaMode::ConstBandwidth, 50.0, ResonatorLayout::Linear, None, 1.0,
        );
        let all = bulk.push_hops(&samples[..ncols * hop], 0);

        let mut inc = StreamingResonators::new(
            sr, fft, hop, 20.0, ResonatorAlphaMode::ConstBandwidth, 50.0, ResonatorLayout::Linear, None, 1.0,
        );
        let mut chunked = Vec::new();
        // Uneven chunks to mimic variable per-tick column counts.
        for (start, len) in [(0usize, 17usize), (17, 31), (48, 32)] {
            chunked.extend(inc.push_hops(&samples[start * hop..(start + len) * hop], start));
        }
        assert_cols_eq(&all, &chunked);
    }

    /// ConstQ mode (the shipped per-bin-alpha path) must still localize a tone
    /// to the correct row — i.e. the per-bin bandwidth schedule doesn't disturb
    /// where energy lands, only how sharp/fast each bin is.
    #[test]
    fn const_q_localizes_tone() {
        let sr = 192_000u32;
        let (fft, hop, ncols) = (2048usize, 256usize, 200usize);
        let f = 30_000.0f32;
        let samples: Vec<f32> = (0..hop * ncols)
            .map(|i| (std::f32::consts::TAU * f * i as f32 / sr as f32).sin())
            .collect();
        let cols = compute_resonator_columns(
            &samples, sr, fft, hop, 0, ncols, 20.0,
            ResonatorAlphaMode::ConstQ, 200.0, ResonatorLayout::Linear, None,
        );
        let last = cols.last().expect("columns");
        assert_eq!(last.magnitudes.len(), fft / 2 + 1);
        let expected = (f * fft as f32 / sr as f32).round() as isize; // bin = f·fft/sr
        let (peak, _) = last
            .magnitudes
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .unwrap();
        assert!(
            (peak as isize - expected).abs() <= 1,
            "ConstQ peak bin {peak} != expected {expected}"
        );
    }

    /// Reduced density must keep the full output row count (the live waterfall's
    /// fixed bin count) while still localizing a tone to the right row.
    #[test]
    fn density_keeps_output_rows_and_peak() {
        let sr = 192_000u32;
        let (fft, hop, ncols) = (1024usize, 256usize, 80usize);
        let output_bins = fft / 2 + 1; // 513
        let f = 40_000.0f32;
        let samples: Vec<f32> = (0..hop * ncols)
            .map(|i| (std::f32::consts::TAU * f * i as f32 / sr as f32).sin())
            .collect();
        let nyq = sr as f32 / 2.0;
        let expected = (f / (nyq / (output_bins - 1) as f32)).round() as isize;
        for density in [0.5f32, 0.25] {
            let mut s = StreamingResonators::new(
                sr, fft, hop, 50.0, ResonatorAlphaMode::ConstBandwidth, 50.0, ResonatorLayout::Linear, None, density,
            );
            let cols = s.push_hops(&samples, 0);
            let last = cols.last().expect("columns");
            assert_eq!(
                last.magnitudes.len(),
                output_bins,
                "density {density} must still emit {output_bins} rows"
            );
            let (peak_row, _) = last
                .magnitudes
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
                .unwrap();
            // Fewer bank bins ⇒ coarser localization; allow a few output rows.
            let tol = (1.0 / density) as isize + 3;
            assert!(
                (peak_row as isize - expected).abs() <= tol,
                "density {density}: peak row {peak_row}, expected ~{expected} (tol {tol})"
            );
        }
    }
}
