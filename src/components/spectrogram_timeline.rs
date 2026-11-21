//! Draws the main spectrogram for a timeline: several files on one time
//! axis. Each clip is drawn in its row with its own frequency range, with
//! dashed lines in the gaps between recordings, an outline where a channel's
//! file is not open, and a line between rows.

use crate::canvas::spectrogram_renderer::{self, ColormapMode, SpectDisplaySettings};
use crate::state::store_fields::*;
use crate::state::{AppState, LoadedFile, MainView};
use crate::timeline::TimelineView;
use leptos::prelude::*;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use wasm_bindgen::JsValue;
use web_sys::CanvasRenderingContext2d;

/// What the spectrogram's draw Effect passes to [`draw_timeline`].
pub(crate) struct TimelineDraw<'a> {
    pub state: AppState,
    pub timeline: &'a TimelineView,
    pub files: &'a [LoadedFile],
    /// Canvas width and height in px.
    pub display_w: f64,
    pub display_h: f64,
    /// Timeline time at the left edge of the view, in seconds.
    pub scroll: f64,
    pub zoom: f64,
    /// Seconds per column of the timeline's primary file.
    pub time_res: f64,
    /// Seconds shown across the view.
    pub visible_time: f64,
    /// The frequency range the view shows, in Hz.
    pub min_freq: f64,
    pub max_freq: f64,
    /// The top frequency of the primary file, in Hz.
    pub file_max_freq: f64,
    /// `min_freq` and `max_freq` as fractions of `file_max_freq`.
    pub freq_crop_lo: f64,
    pub freq_crop_hi: f64,
    pub waterfall_active: bool,
    pub decim_effective: u32,
    pub main_view: MainView,
    pub line_on: bool,
    pub is_playing: bool,
    pub colormap: ColormapMode,
    pub display_settings: &'a SpectDisplaySettings,
    pub freq_adjustments: Option<&'a [f32]>,
    pub disposed: &'a Arc<AtomicBool>,
}

/// Draws the timeline view. Returns true when any clip drew tiles.
pub(crate) fn draw_timeline(ctx: &CanvasRenderingContext2d, args: &TimelineDraw<'_>) -> bool {
    let px_per_sec = args.zoom / args.time_res;

    // Fill entire canvas with black first (covers gaps)
    ctx.set_fill_style_str("#000");
    ctx.fill_rect(0.0, 0.0, args.display_w, args.display_h);

    let any_drawn = draw_clips(ctx, args, px_per_sec);
    draw_gaps(ctx, args, px_per_sec);
    draw_missing(ctx, args, px_per_sec);
    draw_row_lines(ctx, args);
    any_drawn
}

/// Draws each visible clip in its row and schedules its missing tiles.
fn draw_clips(ctx: &CanvasRenderingContext2d, args: &TimelineDraw<'_>, px_per_sec: f64) -> bool {
    let TimelineDraw {
        state,
        timeline: tl,
        files,
        display_w,
        display_h,
        scroll,
        visible_time,
        min_freq,
        max_freq,
        file_max_freq,
        freq_crop_lo,
        freq_crop_hi,
        waterfall_active,
        decim_effective,
        main_view,
        line_on,
        is_playing,
        colormap,
        display_settings,
        freq_adjustments,
        disposed,
        ..
    } = *args;
    let visible_start = scroll;
    let visible_end = scroll + visible_time;

    let mut any_drawn = false;
    for seg in tl.segments_in_range(visible_start, visible_end) {
        let seg_file = match files.get(seg.file_index) {
            Some(f) => f,
            None => continue,
        };
        let seg_time_res = seg_file.spectrogram.time_resolution;
        let seg_total_cols = {
            let tc = seg_file.spectrogram.total_columns;
            if tc > 0 {
                tc
            } else {
                seg_file.spectrogram.columns.len()
            }
        };
        if seg_total_cols == 0 {
            continue;
        }

        // Canvas pixel range for this segment
        let seg_canvas_start = (seg.timeline_offset_secs - scroll) * px_per_sec;
        let seg_canvas_end = ((seg.timeline_offset_secs + seg.duration_secs) - scroll) * px_per_sec;
        let clip_left = seg_canvas_start.max(0.0);
        let clip_right = seg_canvas_end.min(display_w);
        if clip_left >= clip_right {
            continue;
        }

        // The row this clip is drawn in. One row is the
        // whole view.
        let (row_top, row_h) = crate::timeline::row_rect(seg.row, tl.row_count, display_h);

        // The part of this clip's own frequency range that
        // the view shows. A clip with a lower sample rate
        // than the timeline's highest ends below the top of
        // the view: it is drawn on the rows up to its own
        // top frequency and the rows above stay black.
        let seg_max_freq = seg_file.spectrogram.max_freq;
        let own_range = !waterfall_active
            && decim_effective == 0
            && seg_max_freq > 0.0
            && max_freq > min_freq
            && (seg_max_freq - file_max_freq).abs() > 1.0;
        let (freq_crop_lo, freq_crop_hi, seg_top) = if own_range {
            if min_freq >= seg_max_freq {
                continue;
            }
            let shown_top = max_freq.min(seg_max_freq);
            let top = row_h * (max_freq - shown_top) / (max_freq - min_freq);
            (min_freq / seg_max_freq, shown_top / seg_max_freq, top)
        } else {
            (freq_crop_lo, freq_crop_hi, 0.0)
        };
        let seg_h = row_h - seg_top;
        if seg_h < 1.0 {
            continue;
        }

        // Scroll offset within this file
        let file_scroll = (scroll - seg.timeline_offset_secs).max(0.0);
        let file_scroll_col = file_scroll / seg_time_res;

        ctx.save();
        ctx.begin_path();
        ctx.rect(clip_left, row_top, clip_right - clip_left, row_h);
        ctx.clip();

        // Translate to the visible start of this segment so the helper
        // can render against a segment-local viewport.
        let translate_x = clip_left;
        ctx.translate(translate_x, row_top + seg_top).unwrap_or(());

        let seg_visible_time = (clip_right - clip_left) / px_per_sec;
        let seg_px_per_sec = px_per_sec;
        let seg_zoom = seg_px_per_sec * seg_time_res;

        let resonators_on = main_view == MainView::Resonators;
        let tile_source = if resonators_on {
            spectrogram_renderer::TileSource::Resonators
        } else {
            spectrogram_renderer::TileSource::Normal
        };
        let xform_on = state.display.transform().get_untracked();
        let preview_ref = if xform_on || decim_effective > 0 || resonators_on {
            None
        } else {
            seg_file.preview.as_ref()
        };

        let drawn = spectrogram_renderer::blit_tiles_viewport(
            ctx,
            clip_right - clip_left,
            seg_h,
            seg.file_index,
            seg_total_cols,
            file_scroll_col,
            seg_zoom,
            freq_crop_lo,
            freq_crop_hi,
            spectrogram_renderer::TileRenderMode::Spectrogram(colormap),
            display_settings,
            freq_adjustments,
            preview_ref,
            file_scroll,
            seg_visible_time,
            seg.duration_secs,
            tile_source,
        );

        // Reassignment line for this segment, as in single-file view.
        if line_on && !resonators_on && !xform_on && decim_effective == 0 {
            if let Some(vg) = crate::canvas::tile_blit::ViewportGeometry::new(
                clip_right - clip_left,
                seg_h,
                seg_total_cols,
                file_scroll_col,
                seg_zoom,
                freq_crop_lo,
                freq_crop_hi,
            )
            .filter(|vg| vg.ideal_lod > 1)
            {
                let nyquist = seg_file.spectrogram.sample_rate as f64 / 2.0;
                crate::canvas::reassign_line::draw_main_frequency_line(
                    ctx,
                    &crate::canvas::reassign_line::LineView {
                        file_idx: seg.file_index,
                        lod: vg.ideal_lod,
                        first_tile: vg.first_tile,
                        last_tile: vg.last_tile,
                        scroll_s: file_scroll,
                        px_per_sec: seg_px_per_sec,
                        min_freq: freq_crop_lo * nyquist,
                        max_freq: freq_crop_hi * nyquist,
                        width: clip_right - clip_left,
                        height: seg_h,
                    },
                );
            }
        }

        // Schedule missing tiles for this segment
        if resonators_on {
            crate::canvas::tile_scheduler::schedule_resonator_tiles(
                state,
                seg.file_index,
                seg_total_cols,
                file_scroll_col,
                seg_zoom,
                clip_right - clip_left,
            );
        } else {
            crate::canvas::tile_scheduler::schedule_normal_tiles(
                state,
                seg.file_index,
                seg_total_cols,
                file_scroll_col,
                seg_zoom,
                clip_right - clip_left,
                seg_time_res,
                is_playing,
                // The line is left off with XForm or decimation.
                line_on && !xform_on && decim_effective == 0,
                disposed,
            );
        }

        ctx.restore();
        if drawn {
            any_drawn = true;
        }
    }
    any_drawn
}

/// Draws a dashed line in each gap between recordings.
fn draw_gaps(ctx: &CanvasRenderingContext2d, args: &TimelineDraw<'_>, px_per_sec: f64) {
    let TimelineDraw {
        timeline: tl,
        display_w,
        display_h,
        scroll,
        ..
    } = *args;

    // Draw gap indicators between recordings. A place left
    // empty by a channel that is not loaded is not a gap: it
    // gets its own message below.
    let spans = tl.clip_spans();
    for pair in spans.windows(2) {
        let seg_end = pair[0].1;
        let next_start = pair[1].0;
        if next_start > seg_end + 0.001 {
            // There's a gap
            let gap_x1 = (seg_end - scroll) * px_per_sec;
            let gap_x2 = (next_start - scroll) * px_per_sec;
            if gap_x2 > 0.0 && gap_x1 < display_w {
                let x1 = gap_x1.max(0.0);
                let x2 = gap_x2.min(display_w);
                // Draw subtle dashed line at gap center
                let mid = (x1 + x2) / 2.0;
                ctx.set_stroke_style_str("#333");
                ctx.set_line_width(1.0);
                let _ = ctx.set_line_dash(&js_sys::Array::of2(
                    &JsValue::from_f64(3.0),
                    &JsValue::from_f64(3.0),
                ));
                ctx.begin_path();
                ctx.move_to(mid, 0.0);
                ctx.line_to(mid, display_h);
                ctx.stroke();
                let _ = ctx.set_line_dash(&js_sys::Array::new());
            }
        }
    }
}

/// Outlines each place where the chosen channel's file is not open and
/// says which file it is.
fn draw_missing(ctx: &CanvasRenderingContext2d, args: &TimelineDraw<'_>, px_per_sec: f64) {
    let TimelineDraw {
        timeline: tl,
        display_w,
        display_h,
        scroll,
        ..
    } = *args;

    // A recording whose file for the chosen channel is not
    // open: outline its place and say which file is missing,
    // so the empty stretch does not read as a fault. "Open"
    // is the app's word for a file in the Files list ("Open
    // files"); "not loaded" read as a load that failed.
    if let Some(track) = tl.track.as_deref() {
        // "channel 2" inside a sentence; "MIX" stays as it is.
        let channel = crate::timeline::track_name(track).replace("Channel ", "channel ");
        for m in &tl.missing {
            let x1 = (m.timeline_offset_secs - scroll) * px_per_sec;
            let x2 = (m.timeline_offset_secs + m.duration_secs - scroll) * px_per_sec;
            if x2 <= 0.0 || x1 >= display_w {
                continue;
            }
            // The empty place is on its own row.
            let (row_top, h) = crate::timeline::row_rect(m.row, tl.row_count, display_h);
            ctx.save();
            let _ = ctx.translate(0.0, row_top);
            ctx.set_fill_style_str("rgba(255, 255, 255, 0.04)");
            ctx.fill_rect(x1, 0.0, x2 - x1, h);
            ctx.set_stroke_style_str("#444");
            ctx.set_line_width(1.0);
            let _ = ctx.set_line_dash(&js_sys::Array::of2(
                &JsValue::from_f64(4.0),
                &JsValue::from_f64(4.0),
            ));
            ctx.stroke_rect(x1 + 0.5, 0.5, x2 - x1 - 1.0, h - 1.0);
            let _ = ctx.set_line_dash(&js_sys::Array::new());

            // The full sentence when it fits the place,
            // two short lines when it does not.
            ctx.set_font("12px sans-serif");
            let room = x2 - x1 - 12.0;
            let fits = |text: &str| {
                ctx.measure_text(text)
                    .map(|tm| tm.width() <= room)
                    .unwrap_or(false)
            };
            let long = format!("No {channel} file is open for {}", m.take);
            let short = format!("No {channel}");
            let lines: Vec<&str> = if fits(&long) {
                vec![long.as_str()]
            } else if fits(&short) && fits("file open") {
                vec![short.as_str(), "file open"]
            } else {
                Vec::new()
            };
            // #9a9a9a on black is 7.4:1.
            ctx.set_fill_style_str("#9a9a9a");
            ctx.set_text_align("center");
            ctx.set_text_baseline("middle");
            let line_h = 16.0;
            let top = h / 2.0 - line_h * (lines.len() as f64 - 1.0) / 2.0;
            for (i, line) in lines.iter().enumerate() {
                let _ = ctx.fill_text(line, (x1 + x2) / 2.0, top + line_h * i as f64);
            }
            ctx.set_text_align("start");
            ctx.set_text_baseline("alphabetic");
            ctx.restore();
        }
    }
}

/// Draws a line between rows when the timeline has more than one row.
fn draw_row_lines(ctx: &CanvasRenderingContext2d, args: &TimelineDraw<'_>) {
    let tl = args.timeline;

    // A line between rows.
    if tl.row_count > 1 {
        ctx.set_fill_style_str("#555");
        for row in 1..tl.row_count {
            let (top, _) = crate::timeline::row_rect(row, tl.row_count, args.display_h);
            ctx.fill_rect(0.0, top.round(), args.display_w, 1.0);
        }
    }
}
