//! Each recording's DC offset, removed from every 2D display of it.
//!
//! One constant per (file, channel view), measured once from the start of the
//! file (`dsp::dc_offset`) and subtracted from the raw samples of every
//! display read: Spectrogram tiles at every zoom level, Flow, Chromagram,
//! Resonators, the overview and the preview. Every tile of a file uses the
//! same number, so neighbouring tiles always agree, and a tile waits for the
//! number rather than drawing without it.
//!
//! Live listening and recordings made in the app are drawn with their offset,
//! as before: their tiles are built from columns computed while recording,
//! which still hold it. Playback, exports and analysis never use this.

use crate::audio::source::{AudioSource, ChannelView};
use crate::audio::streaming_source;
use crate::canvas::tile_cache::yield_to_browser;
use crate::state::LoadedFile;
use crate::types::{AudioData, AudioFormat};
use oversample_core::dsp::dc_offset::{mean_outside_silence, offset_window_len};
use std::cell::RefCell;
use std::collections::HashMap;

pub use oversample_core::dsp::dc_offset::subtract_dc;

/// Which file a display read is for, taken from the library in the same
/// look-up as its audio.
#[derive(Clone, Copy, Debug)]
pub struct DcTarget {
    file_id: u64,
    /// False for live and in-app recordings, whose offset stays.
    removes_offset: bool,
}

impl DcTarget {
    pub fn of(file: &LoadedFile) -> Self {
        // The format of a live or recorded entry stays Mic or Rec for the
        // entry's life, even after it is saved, so its tiles never switch.
        let in_app = matches!(
            file.audio.metadata.format,
            AudioFormat::Rec | AudioFormat::Mic
        );
        Self {
            file_id: file.id,
            removes_offset: !(in_app || file.is_recording || file.is_live_listen),
        }
    }
}

/// `Stereo` displays the mono mix, so it shares the mono mix's offset.
fn view_key(view: ChannelView) -> (u8, u32) {
    match view {
        ChannelView::Stereo | ChannelView::MonoMix => (0, 0),
        ChannelView::Channel(ch) => (1, ch),
        ChannelView::Difference => (2, 0),
    }
}

enum Entry {
    /// One read is measuring it; others wait.
    Measuring,
    Known(f32),
}

thread_local! {
    /// Keyed by (file id, view). File ids are never reused, so entries need
    /// no clearing when files close.
    static OFFSETS: RefCell<HashMap<(u64, (u8, u32)), Entry>> = RefCell::new(HashMap::new());
}

/// The offset of the head of `audio`'s mono mix, measured on the spot for the
/// load-time preview. [`seed_mono`] it once the file has an id, so the tiles
/// use the same number.
pub fn head_offset(audio: &AudioData) -> f32 {
    let len = offset_window_len(audio.sample_rate).min(audio.samples.len());
    mean_outside_silence(&audio.samples[..len])
}

/// Record the mono-mix offset measured at load ([`head_offset`]).
pub fn seed_mono(file_id: u64, dc: f32) {
    OFFSETS.with(|m| {
        m.borrow_mut()
            .insert((file_id, view_key(ChannelView::MonoMix)), Entry::Known(dc))
    });
}

/// The offset if it is known now, without measuring: 0 for files that keep
/// theirs, `None` while it is unmeasured.
pub fn known_offset(target: DcTarget, view: ChannelView) -> Option<f32> {
    if !target.removes_offset {
        return Some(0.0);
    }
    OFFSETS.with(
        |m| match m.borrow().get(&(target.file_id, view_key(view))) {
            Some(Entry::Known(dc)) => Some(*dc),
            _ => None,
        },
    )
}

/// Clears a `Measuring` entry if the measuring read is dropped before it
/// finishes, so the reads waiting on it measure for themselves.
struct MeasuringGuard {
    key: (u64, (u8, u32)),
    done: bool,
}

impl Drop for MeasuringGuard {
    fn drop(&mut self) {
        if !self.done {
            OFFSETS.with(|m| {
                let mut m = m.borrow_mut();
                if matches!(m.get(&self.key), Some(Entry::Measuring)) {
                    m.remove(&self.key);
                }
            });
        }
    }
}

/// The offset to subtract from `target`'s samples in `view`, measured on the
/// first call and remembered. `None` when the start of the file can't be read;
/// the caller then skips its tile, as for a failed read of its own.
pub async fn offset(target: DcTarget, source: &dyn AudioSource, view: ChannelView) -> Option<f32> {
    if !target.removes_offset {
        return Some(0.0);
    }
    let key = (target.file_id, view_key(view));
    loop {
        let state = OFFSETS.with(|m| match m.borrow().get(&key) {
            Some(Entry::Known(dc)) => Some(Some(*dc)),
            Some(Entry::Measuring) => Some(None),
            None => None,
        });
        match state {
            Some(Some(dc)) => return Some(dc),
            Some(None) => yield_to_browser().await,
            None => break,
        }
    }
    OFFSETS.with(|m| m.borrow_mut().insert(key, Entry::Measuring));
    let mut guard = MeasuringGuard { key, done: false };

    let len = offset_window_len(source.sample_rate()).min(source.total_samples() as usize);
    let samples = streaming_source::read_region_ready(source, view, 0, len).await;
    guard.done = true;
    match samples {
        Some(samples) => {
            let dc = mean_outside_silence(&samples);
            OFFSETS.with(|m| m.borrow_mut().insert(key, Entry::Known(dc)));
            Some(dc)
        }
        None => {
            OFFSETS.with(|m| m.borrow_mut().remove(&key));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_shares_the_mono_offset() {
        assert_eq!(
            view_key(ChannelView::Stereo),
            view_key(ChannelView::MonoMix)
        );
        assert_ne!(
            view_key(ChannelView::Channel(0)),
            view_key(ChannelView::Channel(1))
        );
        assert_ne!(
            view_key(ChannelView::Channel(0)),
            view_key(ChannelView::MonoMix)
        );
    }
}
