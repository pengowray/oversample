//! The Reassignment view: a line over the plain spectrogram at the loudest
//! frequency in each column, placed by the reassignment method
//! (`dsp::reassign::main_frequency_line`). This module keeps the line's tiles
//! and the per-bin noise floor they are measured against, and draws the line.

use crate::audio::source::ChannelView;
use crate::audio::streaming_source;
use crate::canvas::spectrogram_renderer::freq_to_y;
use crate::canvas::tile_cache::{
    at_spawn_limit, has_active_in_flight, is_current_file, tile_count_for_samples,
    yield_to_browser, TileKey, IN_FLIGHT_TIMEOUT_MS, LOD_CONFIGS, TILE_COLS,
};
use crate::dsp::reassign::{main_frequency_line, reassign_noise_floor, LineGrid};
use crate::state::store_fields::*;
use crate::state::AppState;
use crate::types::AudioData;
use leptos::prelude::*;
use oversample_core::canvas::tile_cache_core::in_flight_active_count;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen_futures::spawn_local;
use web_sys::CanvasRenderingContext2d;

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

// ── Main-frequency line tiles ────────────────────────────────────────────────

/// Line points for one tile, with what is needed to place them.
pub struct LineTile {
    pub points: Vec<crate::dsp::reassign::LinePoint>,
    pub fft_size: usize,
    pub hop_size: usize,
    pub sample_rate: u32,
}

/// Line tiles kept. Each holds at most 256 points (about 5 KB), so this caps
/// the cache at about 20 MB.
const MAX_LINE_TILES: usize = 4096;

thread_local! {
    static LINES: RefCell<HashMap<TileKey, LineTile>> = RefCell::new(HashMap::new());
}

pub fn has_line_tile(key: &TileKey) -> bool {
    LINES.with(|m| m.borrow().contains_key(key))
}

pub fn insert_line_tile(key: TileKey, tile: LineTile) {
    LINES.with(|m| {
        let mut m = m.borrow_mut();
        if m.len() >= MAX_LINE_TILES {
            // Drop the tiles of other zoom levels and files first; they are
            // not on screen.
            m.retain(|k, _| k.file_idx == key.file_idx && k.lod == key.lod);
            if m.len() >= MAX_LINE_TILES {
                m.clear();
            }
        }
        m.insert(key, tile);
    });
}

pub fn clear_line_tiles() {
    LINES.with(|m| m.borrow_mut().clear());
}

pub fn clear_line_tiles_for_file(file_idx: usize) {
    LINES.with(|m| m.borrow_mut().retain(|k, _| k.file_idx != file_idx));
}

/// Where the line is drawn, in the same terms as the spectrogram under it.
pub struct LineView {
    pub file_idx: usize,
    pub lod: u8,
    pub first_tile: usize,
    pub last_tile: usize,
    /// Time at the canvas's left edge, in seconds.
    pub scroll_s: f64,
    /// Canvas pixels per second.
    pub px_per_sec: f64,
    /// Frequency range on screen, in Hz.
    pub min_freq: f64,
    pub max_freq: f64,
    pub width: f64,
    pub height: f64,
}

/// Width of the line, in CSS pixels.
const LINE_WIDTH: f64 = 2.0;
/// Width of the dark edge drawn under it, so the line stands out on every
/// colour of every colour map.
const LINE_EDGE_WIDTH: f64 = 4.5;

/// Draw the main-frequency line over the spectrogram, from the line tiles
/// that are ready. Points are placed where the plain spectrogram draws the
/// same sound: column `c` of a tile at its pixel centre, bin `b` at the
/// centre of its row.
pub fn draw_main_frequency_line(ctx: &CanvasRenderingContext2d, view: &LineView) {
    let mut path: Vec<Option<(f64, f64)>> = Vec::new();
    LINES.with(|m| {
        let m = m.borrow();
        // The previous point's column, counted from the file's start, so a
        // join across a tile boundary can be checked.
        let mut last_col: Option<usize> = None;
        for tile_idx in view.first_tile..=view.last_tile {
            let key = TileKey {
                file_idx: view.file_idx,
                lod: view.lod,
                tile_idx,
            };
            let Some(tile) = m.get(&key) else {
                last_col = None;
                continue;
            };
            let sec_per_col = tile.hop_size as f64 / tile.sample_rate as f64;
            let n_bins = (tile.fft_size / 2 + 1) as f64;
            let nyquist = tile.sample_rate as f64 / 2.0;
            let tile_start_col = tile_idx * TILE_COLS;
            for p in &tile.points {
                let col = tile_start_col + p.col as usize;
                let joined = p.joins_previous && last_col.is_some_and(|c| c + 1 == col);
                if !joined {
                    path.push(None);
                }
                let t = (tile_start_col as f64 + p.time_col as f64 + 0.5) * sec_per_col;
                let f = (p.freq_bin as f64 + 0.5) / n_bins * nyquist;
                let x = (t - view.scroll_s) * view.px_per_sec;
                let y = freq_to_y(f, view.min_freq, view.max_freq, view.height);
                path.push(Some((x, y)));
                last_col = Some(col);
            }
        }
    });
    if path.is_empty() {
        return;
    }

    ctx.save();
    ctx.begin_path();
    ctx.rect(0.0, 0.0, view.width, view.height);
    ctx.clip();
    ctx.set_line_join("round");
    ctx.set_line_cap("round");
    ctx.begin_path();
    let mut pen_down = false;
    for point in &path {
        match point {
            None => pen_down = false,
            Some((x, y)) if pen_down => ctx.line_to(*x, *y),
            Some((x, y)) => {
                ctx.move_to(*x, *y);
                pen_down = true;
            }
        }
    }
    ctx.set_stroke_style_str("rgba(0, 0, 0, 0.75)");
    ctx.set_line_width(LINE_EDGE_WIDTH);
    ctx.stroke();
    ctx.set_stroke_style_str("#ffffff");
    ctx.set_line_width(LINE_WIDTH);
    ctx.stroke();
    ctx.restore();
}

// ── Scheduling ───────────────────────────────────────────────────────────────

thread_local! {
    /// Line tiles being computed, with when they started.
    static IN_FLIGHT: RefCell<HashMap<TileKey, f64>> = RefCell::new(HashMap::new());
    /// Bumped when the caches are cleared, so tiles still being computed
    /// discard their result.
    static GENERATION: RefCell<u64> = const { RefCell::new(0) };
}

/// Line tiles being computed now, for the shared limit on background tasks.
pub fn in_flight_count(now: f64) -> usize {
    IN_FLIGHT.with(|s| in_flight_active_count(&s.borrow(), now, IN_FLIGHT_TIMEOUT_MS))
}

fn request_still_active(key: &TileKey) -> bool {
    IN_FLIGHT.with(|s| has_active_in_flight(&mut s.borrow_mut(), key))
}

/// Forget every line tile and noise floor, and make line tiles still being
/// computed discard their result.
pub fn clear_all() {
    clear_line_tiles();
    clear_noise_floors();
    IN_FLIGHT.with(|s| s.borrow_mut().clear());
    GENERATION.with(|g| *g.borrow_mut() += 1);
}

pub fn clear_file(file_idx: usize) {
    clear_line_tiles_for_file(file_idx);
    IN_FLIGHT.with(|s| s.borrow_mut().retain(|k, _| k.file_idx != file_idx));
    GENERATION.with(|g| *g.borrow_mut() += 1);
}

/// Reassignment gate for 16-bit audio, in dB of unnormalized bin power.
const REASSIGN_GATE_16_BIT_DB: f32 = -60.0;

/// Schedule one tile of the main-frequency line for background computation.
///
/// Performs 3 FFTs per frame (standard, time-ramped, derivative-windowed) to
/// find where each column's loudest sound is (`main_frequency_line`).
pub fn schedule_line_tile(state: AppState, file_idx: usize, lod: u8, tile_idx: usize) {
    let key = TileKey {
        file_idx,
        lod,
        tile_idx,
    };
    if has_line_tile(&key) {
        return;
    }
    if IN_FLIGHT.with(|s| has_active_in_flight(&mut s.borrow_mut(), &key)) {
        return;
    }
    if at_spawn_limit() {
        return;
    }

    let total_samples = state.library.files().with_untracked(|files| {
        files
            .get(file_idx)
            .map(|f| f.audio.source.total_samples() as usize)
            .unwrap_or(0)
    });
    let max_tiles = tile_count_for_samples(total_samples, lod);
    if tile_idx >= max_tiles {
        return;
    }

    IN_FLIGHT.with(|s| s.borrow_mut().insert(key, js_sys::Date::now()));

    let gen = GENERATION.with(|g| *g.borrow());

    let config_hop = LOD_CONFIGS[lod as usize].hop_size;
    let actual_fft = state.spect.fft_mode().get_untracked().fft_for_lod(lod);

    spawn_local(async move {
        yield_to_browser().await;

        if !request_still_active(&key) {
            return;
        }

        // Extra yields: 3x FFT cost + expensive LODs (LOD3+)
        if lod >= 3 {
            yield_to_browser().await;
            if !request_still_active(&key) {
                return;
            }
        }
        yield_to_browser().await;

        if !request_still_active(&key) {
            return;
        }

        let is_current = is_current_file(&state, file_idx);
        if !is_current {
            for _ in 0..3 {
                yield_to_browser().await;
                if !request_still_active(&key) {
                    return;
                }
            }
        }

        let file = state
            .library
            .files()
            .with_untracked(|files| files.get(file_idx).map(|f| (f.id, f.audio.clone())));
        let Some((file_id, audio)) = file else {
            IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        };

        let cv = state.viewmode.channel_view().get_untracked();
        // Read extra frames either side: neighbouring frames move energy
        // into this tile's edge columns, and the edge columns are joined and
        // smoothed with the next tiles' points.
        let grid = LineGrid::new(actual_fft, config_hop, audio.sample_rate);
        let margin = grid.margin_cols();
        let tile_col_start = tile_idx * TILE_COLS;
        let lead_cols = margin.min(tile_col_start);
        let sample_start = (tile_col_start - lead_cols) * config_hop;
        let sample_len = (lead_cols + TILE_COLS + margin - 1) * config_hop + actual_fft;

        let read = streaming_source::read_region_ready(
            audio.source.as_ref(),
            cv,
            sample_start as u64,
            sample_len,
        )
        .await;

        if !request_still_active(&key) {
            return;
        }

        // Reads stop at the end of the file, so a tile starting past it reads empty.
        let Some(ch_samples) = read.filter(|v| !v.is_empty()) else {
            IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            return;
        };
        let samples = &ch_samples[..];

        // One floor per file and FFT size, measured from frames spread across
        // the whole file, so a call that fills a deep-zoom tile is still
        // measured against the quiet around it.
        let noise_floor = noise_floor(&audio, file_id, cv, actual_fft).await;
        if !request_still_active(&key) {
            return;
        }

        yield_to_browser().await;

        if !request_still_active(&key) {
            return;
        }

        // Bins below the gate are not moved. -60 dB (unnormalized power)
        // suits 16-bit audio; deeper formats move it down with their noise
        // floor so their quiet calls still get a line.
        let meta = &audio.metadata;
        let gate_db = REASSIGN_GATE_16_BIT_DB
            - oversample_core::dsp::depth_range::depth_extra_db(
                meta.bits_per_sample,
                meta.is_float,
            );
        let points =
            main_frequency_line(samples, lead_cols, TILE_COLS, grid, gate_db, &noise_floor);

        let current_gen = GENERATION.with(|g| *g.borrow());
        if current_gen != gen {
            IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
            state
                .viewmode
                .tile_ready_signal()
                .update(|n| *n = n.wrapping_add(1));
            return;
        }

        insert_line_tile(
            key,
            LineTile {
                points,
                fft_size: actual_fft,
                hop_size: config_hop,
                sample_rate: audio.sample_rate,
            },
        );
        IN_FLIGHT.with(|s| s.borrow_mut().remove(&key));
        state
            .viewmode
            .tile_ready_signal()
            .update(|n| *n = n.wrapping_add(1));
    });
}
