//! Cutting a stream of source audio into model windows.

use crate::resample::StreamResampler;
use oversample_ipc::classifier::{Method, WINDOW_SAMPLES};

/// One window of model-rate audio, ready for the backbone.
pub struct Window {
    pub index: u32,
    /// Start in the recording, in seconds.
    pub start_s: f64,
    /// End in the recording, in seconds. For the last window this is the end of
    /// the recording, not the end of its zero padding.
    pub end_s: f64,
    /// Exactly [`WINDOW_SAMPLES`] samples.
    pub samples: Vec<f32>,
}

/// Resamples source audio to the method's model rate and cuts it into windows
/// of [`WINDOW_SAMPLES`], without overlap. Window `k` starts at
/// `k * method.window_secs()` in the recording.
///
/// The last window is padded with zeros. It is dropped when it holds less than a
/// third of a window of audio (1 s for BirdNET, as BirdNET-Analyzer's
/// `SIG_MINLEN` does, and 0.1875 s for bats), unless it is the only window, so a
/// very short recording still gets one result.
pub struct Windower {
    method: Method,
    source_rate: u32,
    resampler: StreamResampler,
    buf: Vec<f32>,
    next_index: u32,
    source_samples: u64,
}

impl Windower {
    pub fn new(method: Method, source_rate: u32) -> Self {
        Windower {
            method,
            source_rate,
            resampler: StreamResampler::new(source_rate, method.model_rate()),
            buf: Vec::with_capacity(WINDOW_SAMPLES * 2),
            next_index: 0,
            source_samples: 0,
        }
    }

    /// Feed source-rate mono samples; returns the windows they completed.
    pub fn push(&mut self, samples: &[f32]) -> Vec<Window> {
        self.source_samples += samples.len() as u64;
        self.resampler.push(samples, &mut self.buf);
        self.take_full()
    }

    /// End of the audio: returns the remaining full windows and the padded last
    /// one, if it is kept.
    pub fn finish(&mut self) -> Vec<Window> {
        self.resampler.finish(&mut self.buf);
        let mut out = self.take_full();
        let left = self.buf.len();
        let min = WINDOW_SAMPLES / 3;
        if left > 0 && (left >= min || self.next_index == 0) {
            let mut samples = std::mem::take(&mut self.buf);
            samples.resize(WINDOW_SAMPLES, 0.0);
            let start_s = self.next_index as f64 * self.method.window_secs();
            let end_s = self.duration_s().max(start_s);
            out.push(Window {
                index: self.next_index,
                start_s,
                end_s,
                samples,
            });
            self.next_index += 1;
        }
        self.buf.clear();
        out
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
        let mut at = 0;
        while self.buf.len() - at >= WINDOW_SAMPLES {
            let start_s = self.next_index as f64 * win_s;
            out.push(Window {
                index: self.next_index,
                start_s,
                end_s: start_s + win_s,
                samples: self.buf[at..at + WINDOW_SAMPLES].to_vec(),
            });
            self.next_index += 1;
            at += WINDOW_SAMPLES;
        }
        self.buf.drain(..at);
        out
    }
}
