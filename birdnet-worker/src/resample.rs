//! Streaming band-limited resampling of mono audio, with the resampler's delay
//! removed so output sample `i` lines up with input time `i / out_rate`.

use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{
    Async, Fft, FixedAsync, FixedSync, Indexing, Resampler, SincInterpolationParameters,
    SincInterpolationType, WindowFunction,
};

/// Input frames per resampler call.
const CHUNK: usize = 4096;
/// Largest reduced rate (rate / gcd) the FFT resampler is used for. Above it the
/// FFT sizes get large, so an odd rate pair (e.g. 192001 Hz) uses sinc
/// interpolation instead.
const MAX_FFT_REDUCED: usize = 4000;

/// Resamples a stream of mono f32 pieces from `in_rate` to `out_rate`.
///
/// The total output is `ceil(n_in * out_rate / in_rate)` samples, aligned in
/// time with the input: the resampler's start-up delay is dropped from the
/// front and its tail is flushed by [`finish`](Self::finish).
pub struct StreamResampler {
    inner: Option<Box<dyn Resampler<f32>>>,
    in_rate: u32,
    out_rate: u32,
    /// Input not yet given to the resampler.
    pending: Vec<f32>,
    /// Output frames still to drop (the resampler's delay).
    skip: usize,
    in_total: u64,
    out_total: u64,
    scratch: Vec<f32>,
}

impl StreamResampler {
    pub fn new(in_rate: u32, out_rate: u32) -> Self {
        assert!(in_rate > 0 && out_rate > 0, "sample rates must be positive");
        let inner: Option<Box<dyn Resampler<f32>>> = if in_rate == out_rate {
            None
        } else {
            let g = gcd(in_rate as usize, out_rate as usize);
            let (a, b) = (in_rate as usize / g, out_rate as usize / g);
            if a <= MAX_FFT_REDUCED && b <= MAX_FFT_REDUCED {
                Some(Box::new(
                    Fft::<f32>::new(
                        in_rate as usize,
                        out_rate as usize,
                        CHUNK,
                        1,
                        FixedSync::Input,
                    )
                    .expect("FFT resampler for valid rates"),
                ))
            } else {
                let params = SincInterpolationParameters::new(128, WindowFunction::BlackmanHarris2)
                    .oversampling_factor(256)
                    .interpolation(SincInterpolationType::Linear);
                Some(Box::new(
                    Async::<f32>::new_sinc(
                        out_rate as f64 / in_rate as f64,
                        1.0,
                        &params,
                        CHUNK,
                        1,
                        FixedAsync::Input,
                    )
                    .expect("sinc resampler for valid rates"),
                ))
            }
        };
        let skip = inner.as_ref().map_or(0, |r| r.output_delay());
        let scratch = vec![0.0; inner.as_ref().map_or(0, |r| r.output_frames_max())];
        StreamResampler {
            inner,
            in_rate,
            out_rate,
            pending: Vec::new(),
            skip,
            in_total: 0,
            out_total: 0,
            scratch,
        }
    }

    /// Output samples the input so far will give once finished.
    fn expected_out(&self) -> u64 {
        (self.in_total as u128 * self.out_rate as u128).div_ceil(self.in_rate as u128) as u64
    }

    /// Resample `input` and append what is ready to `out`.
    pub fn push(&mut self, input: &[f32], out: &mut Vec<f32>) {
        self.in_total += input.len() as u64;
        let Some(r) = self.inner.as_mut() else {
            out.extend_from_slice(input);
            self.out_total += input.len() as u64;
            return;
        };
        self.pending.extend_from_slice(input);
        let mut at = 0;
        loop {
            let need = r.input_frames_next();
            if self.pending.len() - at < need {
                break;
            }
            let buf_in =
                InterleavedSlice::new(&self.pending[at..at + need], 1, need).expect("input slice");
            let (used, made) = run_once(r.as_mut(), &buf_in, &mut self.scratch, None);
            at += used;
            emit(
                &self.scratch[..made],
                &mut self.skip,
                &mut self.out_total,
                u64::MAX,
                out,
            );
        }
        self.pending.drain(..at);
    }

    /// Flush the resampler and append the rest of the output to `out`.
    pub fn finish(&mut self, out: &mut Vec<f32>) {
        let expected = self.expected_out();
        let Some(r) = self.inner.as_mut() else {
            return;
        };
        // The last partial chunk, then silence until the delayed tail is out.
        let mut first = true;
        let mut guard = 0;
        while self.out_total < expected {
            let need = r.input_frames_next();
            let n = if first {
                self.pending.len().min(need)
            } else {
                0
            };
            first = false;
            let mut padded = vec![0.0f32; need];
            padded[..n].copy_from_slice(&self.pending[..n]);
            let buf_in = InterleavedSlice::new(&padded[..], 1, need).expect("input slice");
            let idx = Indexing::new().partial_len(n);
            let (_, made) = run_once(r.as_mut(), &buf_in, &mut self.scratch, Some(&idx));
            emit(
                &self.scratch[..made],
                &mut self.skip,
                &mut self.out_total,
                expected,
                out,
            );
            guard += 1;
            if guard > 10_000 {
                break;
            }
        }
        self.pending.clear();
    }
}

fn run_once(
    r: &mut dyn Resampler<f32>,
    buf_in: &InterleavedSlice<&[f32]>,
    scratch: &mut [f32],
    idx: Option<&Indexing>,
) -> (usize, usize) {
    let frames = scratch.len();
    let mut buf_out = InterleavedSlice::new_mut(scratch, 1, frames).expect("output slice");
    r.process_into_buffer(buf_in, &mut buf_out, idx)
        .expect("resampler buffers are sized from its own limits")
}

/// Append `made` to `out`, first dropping `skip` frames of delay and never going
/// past `limit` frames in total.
fn emit(made: &[f32], skip: &mut usize, out_total: &mut u64, limit: u64, out: &mut Vec<f32>) {
    let drop = (*skip).min(made.len());
    *skip -= drop;
    let rest = &made[drop..];
    let room = limit.saturating_sub(*out_total).min(rest.len() as u64) as usize;
    out.extend_from_slice(&rest[..room]);
    *out_total += room as u64;
}

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Resample in uneven pieces, as the app sends them.
    fn resample(input: &[f32], in_rate: u32, out_rate: u32) -> Vec<f32> {
        let mut r = StreamResampler::new(in_rate, out_rate);
        let mut out = Vec::new();
        for piece in input.chunks(7_777) {
            r.push(piece, &mut out);
        }
        r.finish(&mut out);
        out
    }

    fn peak_index(v: &[f32]) -> usize {
        v.iter()
            .enumerate()
            .max_by(|a, b| a.1.abs().total_cmp(&b.1.abs()))
            .unwrap()
            .0
    }

    #[test]
    fn length_and_impulse_time() {
        // (in, out): FFT down, FFT up, odd-rate sinc, equal rates.
        for (in_rate, out_rate) in [
            (384_000u32, 48_000u32),
            (384_000, 256_000),
            (44_100, 48_000),
            (500_000, 256_000),
            (192_001, 256_000),
            (48_000, 48_000),
        ] {
            let n = in_rate as usize * 2 + 123;
            let mut input = vec![0.0f32; n];
            // A click at 1.25 s.
            let at = (1.25 * in_rate as f64) as usize;
            input[at] = 1.0;
            let out = resample(&input, in_rate, out_rate);
            let expected = (n as u64 * out_rate as u64).div_ceil(in_rate as u64) as usize;
            assert_eq!(out.len(), expected, "{in_rate} -> {out_rate}: length");
            let t = peak_index(&out) as f64 / out_rate as f64;
            assert!(
                (t - 1.25).abs() < 2.0 / out_rate as f64 + 1e-6,
                "{in_rate} -> {out_rate}: click at {t} s, expected 1.25 s"
            );
        }
    }

    #[test]
    fn sine_keeps_level() {
        // A 1 kHz tone keeps its amplitude through a 44.1 -> 48 kHz resample.
        let (in_rate, out_rate) = (44_100u32, 48_000u32);
        let input: Vec<f32> = (0..in_rate)
            .map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / in_rate as f32).sin())
            .collect();
        let out = resample(&input, in_rate, out_rate);
        let mid = &out[out.len() / 4..out.len() * 3 / 4];
        let peak = mid.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 1.0).abs() < 0.01, "peak {peak}");
    }

    #[test]
    fn removes_content_above_new_nyquist() {
        // 100 kHz in a 384 kHz file is above 24 kHz, so it must not alias into
        // the 48 kHz output.
        let (in_rate, out_rate) = (384_000u32, 48_000u32);
        let input: Vec<f32> = (0..in_rate)
            .map(|i| (2.0 * std::f32::consts::PI * 100_000.0 * i as f32 / in_rate as f32).sin())
            .collect();
        let out = resample(&input, in_rate, out_rate);
        let mid = &out[out.len() / 4..out.len() * 3 / 4];
        let rms = (mid.iter().map(|v| v * v).sum::<f32>() / mid.len() as f32).sqrt();
        assert!(rms < 0.01, "rms {rms}");
    }
}
