//! Reassignment view support: the per-bin noise floor that decides which bins
//! reassignment moves, measured once per file, FFT size and channel view.

use crate::audio::source::ChannelView;
use crate::audio::streaming_source;
use crate::canvas::tile_cache::yield_to_browser;
use crate::dsp::reassign::reassign_noise_floor;
use crate::types::AudioData;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// Stretches read across the file for its noise floor.
const FLOOR_READS: usize = 50;
/// Frames in each stretch, back to back. 50 × 8 = 400 frames, as in
/// `measure_current_noise_floor`.
const FRAMES_PER_READ: usize = 8;

struct FloorEntry {
    view: ChannelView,
    /// `None` while one tile is measuring it; other tiles wait.
    floor: Option<Rc<Vec<f32>>>,
}

thread_local! {
    /// Keyed by (file id, FFT size).
    static FLOORS: RefCell<HashMap<(u64, usize), FloorEntry>> = RefCell::new(HashMap::new());
}

/// Forget every measured floor, for when tiles are recomputed from scratch.
pub fn clear_noise_floors() {
    FLOORS.with(|m| m.borrow_mut().clear());
}

/// Per-bin noise floor of a file at `fft_size`, for `reassign` tiles. The
/// first call measures it from frames spread across the whole file; later
/// calls, and calls made while it is being measured, get the same floor.
/// Empty when the file has too little sound to measure.
pub async fn noise_floor(
    audio: &AudioData,
    file_id: u64,
    view: ChannelView,
    fft_size: usize,
) -> Rc<Vec<f32>> {
    let key = (file_id, fft_size);
    loop {
        let known = FLOORS.with(|m| {
            m.borrow()
                .get(&key)
                .filter(|e| e.view == view)
                .map(|e| e.floor.clone())
        });
        match known {
            Some(Some(floor)) => return floor,
            Some(None) => yield_to_browser().await,
            None => break,
        }
    }
    FLOORS.with(|m| m.borrow_mut().insert(key, FloorEntry { view, floor: None }));

    let total = audio.source.total_samples() as usize;
    let stretch = FRAMES_PER_READ * fft_size;
    let mut frames = Vec::with_capacity(FLOOR_READS * stretch);
    if total >= fft_size {
        let span = total.saturating_sub(stretch);
        for i in 0..FLOOR_READS {
            let start = span * i / (FLOOR_READS - 1).max(1);
            let read = streaming_source::read_region_ready(
                audio.source.as_ref(),
                view,
                start as u64,
                stretch.min(total),
            )
            .await;
            if let Some(samples) = read {
                let whole = samples.len() / fft_size * fft_size;
                frames.extend_from_slice(&samples[..whole]);
            }
        }
    }
    let floor = Rc::new(reassign_noise_floor(&frames, fft_size));
    FLOORS.with(|m| {
        m.borrow_mut().insert(
            key,
            FloorEntry {
                view,
                floor: Some(floor.clone()),
            },
        )
    });
    floor
}
