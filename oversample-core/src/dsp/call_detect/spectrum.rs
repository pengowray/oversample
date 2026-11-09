//! Spectrogram columns from a sliding FFT.
//!
//! [`SpectrumAnalyzer`] keeps the last `N` samples, and every `hop` samples
//! applies a Hann window, runs a real FFT and converts each bin to power in
//! dB. The result, a [`Column`], feeds the call detector.
//!
//! Levels are relative to a full-scale sine: a sine of amplitude 1 centred on
//! a bin reads 0 dB in that bin. A DC offset of 1, and a signal alternating
//! between 1 and -1 every sample (the Nyquist frequency), also read 0 dB.
//!
//! At 192 kHz, `N = 512` gives 375 Hz bins and a 2.7 ms window. See
//! [`FrameConfig`](super::FrameConfig) for the size used at other rates.
//!
//! Ported from the X8 bat app's `bat-dsp` crate, where `N` was fixed at
//! compile time and the FFT was microfft. Here `N` is any even size chosen at
//! run time and the FFT is realfft. The original's `Column::to_pixels`
//! (waterfall drawing for the X8 screen) is left out: Oversample draws its own
//! spectrograms.

use std::f32::consts::TAU;
use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::RealToComplex;

/// Level of a bin with no signal (digital silence), in dB.
pub const MIN_DB: f32 = -200.0;

/// Power that reads [`MIN_DB`].
const MIN_POWER: f32 = 1e-20;

/// 10 · log10(2): dB per doubling of power.
const DB_PER_OCTAVE: f32 = 3.010_3;

/// Smallest FFT size the analyzer accepts.
pub const MIN_FFT_SIZE: usize = 16;

/// Power to dB: `10 · log10(power)`, within 0.002 dB, and [`MIN_DB`] for
/// powers at or below 1e-20 (including zero and NaN).
///
/// Uses the float's exponent and a cubic for the mantissa, which costs a few
/// multiplies instead of a `log10`. Kept exactly as in `bat-dsp`, so the port
/// makes the same threshold decisions as the original.
#[inline]
pub fn power_to_db(power: f32) -> f32 {
    let bits = power.max(MIN_POWER).to_bits();
    let exponent = (bits >> 23) as i32 - 127;
    let m = f32::from_bits((bits & 0x007f_ffff) | 0x3f80_0000);
    // log2(m) for m in [1, 2): near-minimax cubic, error under 6.4e-4.
    let log2_m = ((0.158_245_2 * m - 1.051_847_3) * m + 3.047_825_5) * m - 2.153_584_2;
    DB_PER_OCTAVE * (exponent as f32 + log2_m)
}

/// One spectrogram column: power per bin for one windowed frame.
#[derive(Clone, Copy, Debug)]
pub struct Column<'a> {
    /// Index of the sample at the centre of the frame, counted from the first
    /// sample pushed after construction or [`SpectrumAnalyzer::reset`].
    pub centre_sample: u64,
    /// Bin spacing in Hz: sample rate / N.
    pub bin_hz: f32,
    /// Power in dB for bins `0..=N / 2`. Bin `k` is centred on `k · bin_hz`.
    pub db: &'a [f32],
}

impl Column<'_> {
    /// Centre frequency of a bin, in Hz.
    pub fn bin_freq(&self, bin: usize) -> f32 {
        bin as f32 * self.bin_hz
    }

    /// The bin nearest to a frequency, clamped to `0..=N / 2`.
    pub fn freq_bin(&self, hz: f32) -> usize {
        nearest_bin(hz, self.bin_hz, self.db.len() - 1)
    }

    /// True when every bin reads [`MIN_DB`], which happens only when every
    /// sample in the frame is zero: digital silence, such as padding or a
    /// stretch cut out of a recording.
    pub fn is_silent(&self) -> bool {
        self.db.iter().all(|&v| v <= MIN_DB + 0.01)
    }
}

fn nearest_bin(hz: f32, bin_hz: f32, last: usize) -> usize {
    // `as usize` saturates: negative and NaN give 0.
    ((hz / bin_hz + 0.5) as usize).min(last)
}

/// Sliding-window spectrum analyzer with an `N`-point FFT.
#[derive(Clone)]
pub struct SpectrumAnalyzer {
    sample_rate: f32,
    n: usize,
    hop: usize,
    /// Periodic Hann window, scaled so a full-scale sine centred on a bin
    /// gives a magnitude of 1.
    window: Vec<f32>,
    /// The last `N` samples. Once full, `pos` is the oldest.
    ring: Vec<f32>,
    /// Where the next sample goes in `ring`.
    pos: usize,
    /// Samples still needed before the next column.
    until_column: usize,
    /// Samples pushed since construction or reset.
    pushed: u64,
    fft: Arc<dyn RealToComplex<f32>>,
    /// FFT input (the windowed frame).
    frame: Vec<f32>,
    spectrum: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    db: Vec<f32>,
}

impl std::fmt::Debug for SpectrumAnalyzer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpectrumAnalyzer")
            .field("sample_rate", &self.sample_rate)
            .field("n", &self.n)
            .field("hop", &self.hop)
            .field("pushed", &self.pushed)
            .finish()
    }
}

impl SpectrumAnalyzer {
    /// Analyzer with an `fft_size`-point FFT and the default hop of
    /// `fft_size / 4` samples.
    ///
    /// # Panics
    /// If `fft_size` is odd or smaller than [`MIN_FFT_SIZE`].
    pub fn new(sample_rate: f32, fft_size: usize) -> Self {
        assert!(
            fft_size >= MIN_FFT_SIZE && fft_size % 2 == 0,
            "FFT size must be even and at least {MIN_FFT_SIZE}, not {fft_size}"
        );
        let n = fft_size;
        let mut window: Vec<f32> = (0..n)
            .map(|i| 0.5 - 0.5 * (TAU * i as f32 / n as f32).cos())
            .collect();
        let sum: f32 = window.iter().sum();
        // A sine of amplitude A centred on a bin gives |X| = A · sum / 2.
        let scale = 2.0 / sum;
        for w in &mut window {
            *w *= scale;
        }
        let fft = crate::dsp::fft::plan_fft_forward(n);
        let scratch = fft.make_scratch_vec();
        let spectrum = fft.make_output_vec();
        Self {
            sample_rate,
            n,
            hop: n / 4,
            window,
            ring: vec![0.0; n],
            pos: 0,
            until_column: n,
            pushed: 0,
            fft,
            frame: vec![0.0; n],
            spectrum,
            scratch,
            db: vec![MIN_DB; n / 2 + 1],
        }
    }

    /// FFT size `N`.
    pub fn fft_size(&self) -> usize {
        self.n
    }

    /// Number of bins in each column: `N / 2 + 1`.
    pub fn bins(&self) -> usize {
        self.n / 2 + 1
    }

    pub fn sample_rate(&self) -> f32 {
        self.sample_rate
    }

    /// Samples between columns, clamped to `1..=N`. The column already being
    /// filled keeps the old spacing; the ones after it use the new one.
    pub fn set_hop(&mut self, hop: usize) {
        self.hop = hop.clamp(1, self.n);
    }

    pub fn hop(&self) -> usize {
        self.hop
    }

    /// Bin spacing in Hz: sample rate / N.
    pub fn bin_hz(&self) -> f32 {
        self.sample_rate / self.n as f32
    }

    /// Centre frequency of a bin, in Hz.
    pub fn bin_freq(&self, bin: usize) -> f32 {
        bin as f32 * self.bin_hz()
    }

    /// The bin nearest to a frequency, clamped to `0..=N / 2`.
    pub fn freq_bin(&self, hz: f32) -> usize {
        nearest_bin(hz, self.bin_hz(), self.n / 2)
    }

    /// Clear the sample history and restart the sample count at 0.
    pub fn reset(&mut self) {
        self.ring.fill(0.0);
        self.pos = 0;
        self.until_column = self.n;
        self.pushed = 0;
    }

    /// Jump over `samples` samples that will never be pushed, because input was
    /// lost or analysis is paused. The sample count carries on, so later columns
    /// keep their true times, and the window starts again, so the next column
    /// comes `N` samples from now and holds only samples pushed after this call.
    /// No column ever spans the jump, which would show as a wide-band click.
    ///
    /// [`reset`](Self::reset) does not suit this: it restarts the count at 0.
    pub fn skip(&mut self, samples: u64) {
        // The ring needs no clearing: N new samples overwrite it before the
        // next column.
        self.pos = 0;
        self.until_column = self.n;
        self.pushed += samples;
    }

    /// Add samples, in a block of any size. Calls `on_column` for each frame
    /// completed by these samples, in order, before returning.
    ///
    /// The first column comes once `N` samples have arrived, then one every
    /// `hop` samples.
    pub fn push(&mut self, mut input: &[f32], mut on_column: impl FnMut(&Column<'_>)) {
        while !input.is_empty() {
            let take = self.until_column.min(input.len());
            let (now, rest) = input.split_at(take);
            self.write_ring(now);
            self.pushed += take as u64;
            self.until_column -= take;
            input = rest;
            if self.until_column == 0 {
                self.until_column = self.hop;
                self.compute_column();
                on_column(&Column {
                    centre_sample: self.pushed - (self.n / 2) as u64,
                    bin_hz: self.bin_hz(),
                    db: &self.db,
                });
            }
        }
    }

    /// Copy up to `N` samples into the ring.
    fn write_ring(&mut self, x: &[f32]) {
        let n = self.n;
        let first = (n - self.pos).min(x.len());
        self.ring[self.pos..self.pos + first].copy_from_slice(&x[..first]);
        let wrapped = x.len() - first;
        self.ring[..wrapped].copy_from_slice(&x[first..]);
        self.pos = (self.pos + x.len()) % n;
    }

    fn compute_column(&mut self) {
        let n = self.n;
        // Oldest samples are ring[pos..], newest ring[..pos].
        let older = n - self.pos;
        let (newest, oldest) = self.ring.split_at(self.pos);
        let (w_old, w_new) = self.window.split_at(older);
        let (f_old, f_new) = self.frame.split_at_mut(older);
        for ((f, &x), &w) in f_old.iter_mut().zip(oldest).zip(w_old) {
            *f = x * w;
        }
        for ((f, &x), &w) in f_new.iter_mut().zip(newest).zip(w_new) {
            *f = x * w;
        }

        // The buffers are sized by the plan, so this cannot fail.
        let _ =
            self.fft
                .process_with_scratch(&mut self.frame, &mut self.spectrum, &mut self.scratch);

        let half = n / 2;
        let (dc, nyquist) = (self.spectrum[0].re, self.spectrum[half].re);
        // DC and Nyquist have no negative-frequency twin, so they get a
        // quarter of the power scale of the other bins.
        self.db[0] = power_to_db(0.25 * dc * dc);
        self.db[half] = power_to_db(0.25 * nyquist * nyquist);
        for (b, c) in self.db[1..half].iter_mut().zip(&self.spectrum[1..half]) {
            *b = power_to_db(c.re * c.re + c.im * c.im);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsp::call_detect::test_util::{chirp, sine};

    const FS: f32 = 192_000.0;

    /// Every column from `x`, pushed in blocks of `block`, as (centre, dB).
    fn columns(an: &mut SpectrumAnalyzer, x: &[f32], block: usize) -> Vec<(u64, Vec<f32>)> {
        let mut out = Vec::new();
        for b in x.chunks(block) {
            an.push(b, |c| out.push((c.centre_sample, c.db.to_vec())));
        }
        out
    }

    fn sine_on_bin_reads_0_db(n: usize) {
        let mut an = SpectrumAnalyzer::new(FS, n);
        let bin = n / 5;
        let x = sine(FS, an.bin_freq(bin), 1.0, 4 * n);
        let cols = columns(&mut an, &x, 1000);
        let (_, db) = cols.last().unwrap();
        assert_eq!(db.len(), n / 2 + 1);
        assert!(db[bin].abs() < 0.1, "N = {n}: {} dB on the bin", db[bin]);
        // Hann: neighbours are 6 dB down, and bins further out far lower.
        assert!(
            (db[bin + 1] + 6.02).abs() < 0.1,
            "N = {n}: {} dB next bin",
            db[bin + 1]
        );
        assert!(
            db[bin + 4] < -80.0,
            "N = {n}: {} dB 4 bins away",
            db[bin + 4]
        );
    }

    #[test]
    fn full_scale_sine_on_a_bin_reads_0_db_for_each_size() {
        // The original's three sizes, and sizes that are not powers of two
        // (250 kHz and 500 kHz recordings get 666 and 1334).
        for n in [256, 512, 1024, 666, 1334] {
            sine_on_bin_reads_0_db(n);
        }
    }

    #[test]
    fn dc_and_nyquist_land_in_the_end_bins() {
        let mut an = SpectrumAnalyzer::new(FS, 512);
        // Alternating +0.5 / -0.5: the Nyquist frequency at -6 dB.
        let x: Vec<f32> = (0..2048)
            .map(|i| if i % 2 == 0 { 0.5 } else { -0.5 })
            .collect();
        let cols = columns(&mut an, &x, 512);
        let (_, db) = cols.last().unwrap();
        assert!((db[256] + 6.02).abs() < 0.05, "Nyquist bin {} dB", db[256]);
        assert!(db[0] < -100.0, "DC bin {} dB", db[0]);

        let mut an = SpectrumAnalyzer::new(FS, 512);
        let cols = columns(&mut an, &[1.0; 2048], 512);
        let (_, db) = cols.last().unwrap();
        assert!(db[0].abs() < 0.05, "DC bin {} dB", db[0]);
        assert!(db[256] < -100.0, "Nyquist bin {} dB", db[256]);
    }

    #[test]
    fn silence_reads_min_db() {
        let mut an = SpectrumAnalyzer::new(FS, 256);
        let cols = columns(&mut an, &[0.0; 256], 256);
        assert!(cols[0].1.iter().all(|&v| (v - MIN_DB).abs() < 0.01));
    }

    #[test]
    fn only_all_zero_frames_are_silent() {
        let mut an = SpectrumAnalyzer::new(FS, 256);
        an.set_hop(256);
        let mut x = vec![0.0; 512];
        // One sample at the 16-bit step, in the second frame only.
        x[400] = 1.0 / 32_768.0;
        let mut silent = Vec::new();
        an.push(&x, |c| silent.push(c.is_silent()));
        assert_eq!(silent, [true, false]);
    }

    #[test]
    fn columns_come_every_hop_with_frame_centres() {
        let mut an = SpectrumAnalyzer::new(FS, 512);
        assert_eq!(an.hop(), 128);
        let cols = columns(&mut an, &[0.0; 10_000], 333);
        assert_eq!(cols.len(), (10_000 - 512) / 128 + 1);
        for (i, (centre, _)) in cols.iter().enumerate() {
            assert_eq!(*centre, 256 + 128 * i as u64);
        }

        an.reset();
        an.set_hop(300);
        let cols = columns(&mut an, &[0.0; 2_000], 777);
        let centres: Vec<u64> = cols.iter().map(|c| c.0).collect();
        assert_eq!(centres, [256, 556, 856, 1156, 1456]);
    }

    #[test]
    fn skip_restarts_the_window_and_keeps_the_sample_count() {
        let mut an = SpectrumAnalyzer::new(FS, 512);
        an.set_hop(192);
        // 700 samples of a tone: the columns at 512 and 704 are not both due.
        let tone = sine(FS, 48_000.0, 0.5, 700);
        let before = columns(&mut an, &tone, 100);
        assert_eq!(before.iter().map(|c| c.0).collect::<Vec<_>>(), [256]);

        an.skip(10_000);
        // Silence after the jump: 511 samples make no column, the 512th does,
        // and it holds nothing from the tone before the jump.
        let cols = columns(&mut an, &[0.0; 511], 50);
        assert!(cols.is_empty());
        let cols = columns(&mut an, &[0.0; 1 + 192], 50);
        assert_eq!(
            cols.iter().map(|c| c.0).collect::<Vec<_>>(),
            [700 + 10_000 + 256, 700 + 10_000 + 256 + 192]
        );
        assert!(
            cols[0].1.iter().all(|&v| (v - MIN_DB).abs() < 0.01),
            "the tone is still in the window"
        );
    }

    #[test]
    fn block_size_does_not_change_the_columns() {
        let x = chirp(FS, 80_000.0, 30_000.0, 0.5, 20_000);
        let a = columns(&mut SpectrumAnalyzer::new(FS, 512), &x, 20_000);
        let b = columns(&mut SpectrumAnalyzer::new(FS, 512), &x, 37);
        let c = columns(&mut SpectrumAnalyzer::new(FS, 512), &x, 1);
        assert_eq!(a, b);
        assert_eq!(a, c);
    }

    #[test]
    fn fast_db_matches_log10_within_0_01_db() {
        let mut worst = 0.0f32;
        let mut p = 1e-19f32;
        while p < 1e6 {
            let exact = 10.0 * p.log10();
            worst = worst.max((power_to_db(p) - exact).abs());
            p *= 1.0137;
        }
        assert!(worst < 0.01, "worst error {worst} dB");
        assert_eq!(power_to_db(0.0), power_to_db(MIN_POWER));
        assert!((power_to_db(f32::NAN) - MIN_DB).abs() < 0.01);
    }

    #[test]
    fn bin_and_frequency_round_trip() {
        let an = SpectrumAnalyzer::new(FS, 1024);
        assert_eq!(an.bin_hz(), 187.5);
        for bin in [0, 1, 200, 512] {
            assert_eq!(an.freq_bin(an.bin_freq(bin)), bin);
        }
        assert_eq!(an.freq_bin(40_000.0), 213);
        assert_eq!(an.freq_bin(-5.0), 0);
        assert_eq!(an.freq_bin(200_000.0), 512);
    }
}
