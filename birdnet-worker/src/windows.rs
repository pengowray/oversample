//! Cutting a stream of source audio into model windows.

use crate::resample::StreamResampler;
use oversample_ipc::classifier::Method;

/// One window of model-rate audio, ready for the model.
pub struct Window {
    /// Windows handed out before this one in the same pass.
    pub index: u32,
    /// Start in the recording, in seconds.
    pub start_s: f64,
    /// End in the recording, in seconds. For a zero-padded last window this
    /// is the end of the recording, not the end of its zero padding.
    pub end_s: f64,
    /// Exactly the method's [`Method::window_samples`] samples.
    pub samples: Vec<f32>,
}

/// Resamples source audio to the method's model rate and cuts it into windows
/// of [`Method::window_samples`].
///
/// [`Windower::new`] cuts the windows end to end: window `k` starts at
/// `k * method.window_secs()` in the recording. The last window is padded
/// with zeros. It is dropped when it holds less than a third of a window of
/// audio (1 s for BirdNET, as BirdNET-Analyzer's `SIG_MINLEN` does, 0.1875 s
/// for bats and 1.67 s for Perch), unless it is the only window, so a very
/// short recording still gets one result.
///
/// [`Windower::with_offsets`] cuts the windows of one later pass of a run
/// with a shorter step (`oversample_ipc::classifier::refine_passes`): those
/// starting at `k * window + offset`. These windows overlap the ones end to
/// end, and only those inside the audio are kept (one that misses the end by
/// at most [`TAIL_SLACK_S`] is padded and kept), so the one zero-padded
/// window of a run stays the last of the windows end to end.
pub struct Windower {
    method: Method,
    source_rate: u32,
    resampler: StreamResampler,
    /// Model-rate samples from `buf_at` on.
    buf: Vec<f32>,
    buf_at: u64,
    /// Offsets of the windows taken in each window length, sorted; `[0]` for
    /// the windows end to end.
    offsets: Vec<u64>,
    next_index: u32,
    source_samples: u64,
    /// Every model-rate sample made, for another model that hears the same
    /// audio (the bird box detector), when [`Windower::tap`] turned it on.
    tapped: Option<Vec<f32>>,
}

/// How far a window of a later pass may run past the end of the audio and
/// still be kept, in seconds: the resampler can end a few samples short of
/// the source's length.
pub const TAIL_SLACK_S: f64 = 0.01;

impl Windower {
    /// The windows end to end.
    pub fn new(method: Method, source_rate: u32) -> Self {
        Self::with_offsets(method, source_rate, &[])
    }

    /// The windows starting at `k * window + offset` model samples, for each
    /// of `offsets`. Offsets of a window or more are ignored; empty (or only
    /// 0) is the windows end to end.
    pub fn with_offsets(method: Method, source_rate: u32, offsets: &[u32]) -> Self {
        let win = method.window_samples() as u64;
        let mut offsets: Vec<u64> = offsets
            .iter()
            .map(|&o| o as u64)
            .filter(|&o| o < win)
            .collect();
        offsets.sort_unstable();
        offsets.dedup();
        if offsets.is_empty() {
            offsets.push(0);
        }
        Windower {
            method,
            source_rate,
            resampler: StreamResampler::new(source_rate, method.model_rate()),
            buf: Vec::with_capacity(method.window_samples() * 2),
            buf_at: 0,
            offsets,
            next_index: 0,
            source_samples: 0,
            tapped: None,
        }
    }

    /// Keep a copy of every model-rate sample from now on, for
    /// [`Windower::take_tapped`].
    pub fn tap(mut self) -> Self {
        self.tapped = Some(Vec::new());
        self
    }

    /// The model-rate samples made since the last call (empty unless
    /// [`Windower::tap`] turned the copy on).
    pub fn take_tapped(&mut self) -> Vec<f32> {
        self.tapped.as_mut().map(std::mem::take).unwrap_or_default()
    }

    fn copy_tapped(&mut self, from: usize) {
        if let Some(t) = self.tapped.as_mut() {
            t.extend_from_slice(&self.buf[from..]);
        }
    }

    /// True for the windows end to end.
    fn end_to_end(&self) -> bool {
        self.offsets == [0]
    }

    /// Start of window `i` of this pass, in model samples.
    fn start_of(&self, i: u32) -> u64 {
        let m = self.offsets.len() as u64;
        let i = i as u64;
        (i / m) * self.method.window_samples() as u64 + self.offsets[(i % m) as usize]
    }

    fn secs(&self, samples: u64) -> f64 {
        samples as f64 / self.method.model_rate() as f64
    }

    /// Feed source-rate mono samples; returns the windows they completed.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Window> {
        self.source_samples += samples.len() as u64;
        let from = self.buf.len();
        self.resampler.push(samples, &mut self.buf);
        self.copy_tapped(from);
        self.take_full()
    }

    /// End of the audio: returns the remaining full windows and, for the
    /// windows end to end, the padded last one if it is kept.
    pub fn finish(&mut self) -> Vec<Window> {
        let from = self.buf.len();
        self.resampler.finish(&mut self.buf);
        self.copy_tapped(from);
        let mut out = self.take_full();
        let win = self.method.window_samples();
        let end = self.buf_at + self.buf.len() as u64;
        if self.end_to_end() {
            let start = self.start_of(self.next_index);
            let left = end.saturating_sub(start) as usize;
            if left > 0 && (left >= win / 3 || self.next_index == 0) {
                out.push(self.padded(start));
            }
        } else {
            let slack = (TAIL_SLACK_S * self.method.model_rate() as f64) as u64;
            loop {
                let start = self.start_of(self.next_index);
                if start >= end || start + win as u64 > end + slack {
                    break;
                }
                out.push(self.padded(start));
            }
        }
        self.buf.clear();
        out
    }

    /// The window at `start` (inside the buffer), padded with zeros past the
    /// end of the audio. Ends at the end of the recording.
    fn padded(&mut self, start: u64) -> Window {
        let win = self.method.window_samples();
        let from = (start - self.buf_at) as usize;
        let mut samples = self.buf[from.min(self.buf.len())..].to_vec();
        samples.truncate(win);
        samples.resize(win, 0.0);
        let start_s = self.secs(start);
        let end_s = (start_s + self.method.window_secs()).min(self.duration_s().max(start_s));
        let w = Window {
            index: self.next_index,
            start_s,
            end_s,
            samples,
        };
        self.next_index += 1;
        w
    }

    /// Length of the audio fed so far, in seconds.
    pub fn duration_s(&self) -> f64 {
        self.source_samples as f64 / self.source_rate as f64
    }

    /// Windows handed out so far.
    pub fn windows(&self) -> u32 {
        self.next_index
    }

    fn take_full(&mut self) -> Vec<Window> {
        let mut out = Vec::new();
        let win_s = self.method.window_secs();
        let win = self.method.window_samples();
        let end = self.buf_at + self.buf.len() as u64;
        loop {
            let start = self.start_of(self.next_index);
            if start + win as u64 > end {
                break;
            }
            let from = (start - self.buf_at) as usize;
            let start_s = self.secs(start);
            out.push(Window {
                index: self.next_index,
                start_s,
                end_s: start_s + win_s,
                samples: self.buf[from..from + win].to_vec(),
            });
            self.next_index += 1;
        }
        // Keep the samples from the next window's start on.
        let keep_from = self.start_of(self.next_index).min(end);
        let drop = (keep_from - self.buf_at) as usize;
        self.buf.drain(..drop);
        self.buf_at = keep_from;
        out
    }
}
