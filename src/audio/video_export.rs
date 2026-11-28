//! MP4 video export: render spectrogram frames + DSP-processed audio into an MP4 file
//! using the WebCodecs API and mp4-muxer JS library.

use crate::state::store_fields::*;
use std::cell::RefCell;
use std::rc::Rc;

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use crate::audio::export::{
    build_export_params, export_channel_views, export_output_rate, get_selected_regions,
    not_loaded_text, save_export_bytes, show_export_failed, video_file_name, RegionError,
    RegionProcessor,
};
use crate::audio::webcodecs_bindings as wc;
use crate::canvas::spectrogram_renderer::{self, ColormapMode, SpectDisplaySettings, TileSource};
use crate::state::{AppState, AudioCodecOption, VideoCodec, VideoViewMode};
use crate::web_util::yield_now;

/// Frames per second for exported video.
const FPS: f64 = 30.0;

/// Default video bitrate (bits per second).
const DEFAULT_VIDEO_BITRATE: u32 = 2_000_000;

/// Default audio bitrate (bits per second).
const DEFAULT_AUDIO_BITRATE: u32 = 128_000;

/// Keyframe interval in frames.
const KEYFRAME_INTERVAL: u32 = 60;

/// Snapshot of all rendering parameters captured at export start.
struct RenderParams {
    file_idx: usize,
    total_cols: usize,
    time_res: f64,
    duration: f64,
    start_time: f64,
    end_time: f64,
    file_max_freq: f64,
    freq_crop_lo: f64,
    freq_crop_hi: f64,
    colormap: ColormapMode,
    display_settings: SpectDisplaySettings,
    min_freq: f64,
    max_freq: f64,
    canvas_w: u32,
    canvas_h: u32,
    shield_style: crate::state::ShieldStyle,
    /// The annotations to draw, already filtered. `None` draws none.
    annotations: Option<crate::annotations::AnnotationSet>,
}

/// Check if video export is available in this browser.
pub fn is_available() -> bool {
    wc::has_video_encoder() && wc::has_mp4_muxer()
}

/// Launch the async video export. Call from a button click handler.
pub fn start_export(state: &AppState) {
    let state = *state;
    leptos::task::spawn_local(async move {
        state.export.cancel().set(false);
        state.export.progress().set(Some(0.0));
        state.export.status().set(Some("Preparing...".to_string()));

        match export_video_impl(&state).await {
            Ok(()) => {
                state.export.progress().set(None);
                state.export.status().set(None);
                if state.export.cancel().get_untracked() {
                    log::info!("Video export cancelled");
                } else {
                    log::info!("Video export complete");
                }
            }
            // A cancel while the audio was read.
            Err(e) if e.is_null() => {
                state.export.progress().set(None);
                state.export.status().set(None);
            }
            Err(e) => {
                let msg = e.as_string().unwrap_or_else(|| format!("{e:?}"));
                show_export_failed(&state, &msg);
            }
        }
    });
}

async fn export_video_impl(state: &AppState) -> Result<(), JsValue> {
    log::info!("Video export: starting...");

    // Gather file info
    let file = state
        .current_file()
        .ok_or_else(|| JsValue::from_str("No file loaded"))?;
    let file_idx = state.library.current_index().get_untracked().unwrap();
    let sample_rate = file.audio.sample_rate;
    let source = &file.audio.source;

    // Determine time range (same logic as WAV export)
    let regions = get_selected_regions(state);
    let use_region_focus = state.annotations.export_use_region_focus().get_untracked();

    let (start_time, end_time) = if !regions.is_empty() {
        let r = &regions[0].1;
        (r.time_start, r.time_end)
    } else if let Some(sel) = state.interaction.selection().get_untracked() {
        (sel.time_start, sel.time_end)
    } else {
        (0.0, file.audio.source.duration_secs())
    };

    log::info!("Video export: time range {start_time:.2}s - {end_time:.2}s");

    if end_time <= start_time {
        return Err(JsValue::from_str("Invalid time range"));
    }

    // Video resolution
    let resolution = state.export.video_resolution().get_untracked();
    let canvas_w_hint = state
        .viewmode
        .spectrogram_canvas_width()
        .get_untracked()
        .max(320.0) as u32;
    let canvas_h_hint = 400u32; // reasonable default for spectrogram height
    let (vid_w, vid_h) = resolution.dimensions(canvas_w_hint, canvas_h_hint);
    // Ensure even dimensions (required by most codecs)
    let vid_w = ((vid_w + 1) & !1).max(2);
    let vid_h = ((vid_h + 1) & !1).max(2);

    // Codec
    let codec_str = match state.export.video_codec().get_untracked() {
        VideoCodec::H264 => wc::H264_CODEC,
        VideoCodec::Av1 => wc::AV1_CODEC,
    };

    log::info!("Video export: {vid_w}x{vid_h} codec={codec_str}");
    log::info!(
        "Video export: has_video_encoder={}, has_audio_encoder={}, has_mp4_muxer={}",
        wc::has_video_encoder(),
        wc::has_audio_encoder(),
        wc::has_mp4_muxer()
    );

    // Check codec support
    if !wc::is_video_config_supported(codec_str, vid_w, vid_h).await {
        return Err(JsValue::from_str(&format!(
            "Video codec {} not supported at {}x{}",
            codec_str, vid_w, vid_h
        )));
    }
    log::info!("Video export: codec supported");

    // Snapshot rendering parameters from current state
    let time_res = file.spectrogram.time_resolution;
    let file_max_freq = file.spectrogram.max_freq;
    let max_display_freq = state.view.max_display_freq().get_untracked();
    let min_display_freq = state.view.min_display_freq().get_untracked();
    let max_freq = max_display_freq.unwrap_or(file_max_freq).min(file_max_freq);
    let min_freq = min_display_freq.unwrap_or(0.0);
    // "Crop picture at 24 kHz": leave out what a 1:1 video's sound does not
    // reach. Ignored when the view starts above that.
    let max_freq = match crop_hz(state.playback.mode().get_untracked(), sample_rate) {
        Some(top) if state.export.video_crop_to_sound().get_untracked() && min_freq < top => {
            max_freq.min(top)
        }
        _ => max_freq,
    };
    let freq_crop_lo = min_freq / file_max_freq;
    let freq_crop_hi = (max_freq / file_max_freq).min(1.0);

    let hfr_enabled = state.viewmode.hfr_enabled().get_untracked();
    let colormap_pref = state.spect.colormap_preference().get_untracked();
    let hfr_colormap_pref = state.spect.hfr_colormap_preference().get_untracked();
    let band_ff_lo = state.filter.band_ff_freq_lo().get_untracked();
    let band_ff_hi = state.filter.band_ff_freq_hi().get_untracked();

    let colormap = if hfr_enabled && band_ff_hi > band_ff_lo {
        ColormapMode::HfrFocus {
            colormap: hfr_colormap_pref,
            band_ff_lo_frac: band_ff_lo / file_max_freq,
            band_ff_hi_frac: band_ff_hi / file_max_freq,
        }
    } else if hfr_enabled {
        ColormapMode::Uniform(hfr_colormap_pref)
    } else {
        ColormapMode::Uniform(colormap_pref)
    };

    let (spect_floor, spect_range) = untrack(|| state.spect_floor_range());
    let spect_gamma = state.spect.gamma().get_untracked();
    let spect_gain = state.spect.gain_db().get_untracked();

    // Reference level, the same way the spectrogram component does it: tiles
    // are drawn in dBFS, so 0 dB unless display auto-gain is on.
    let display_auto_gain = state.display.auto_gain().get_untracked();
    let total_cols = {
        let tc = file.spectrogram.total_columns;
        if tc > 0 {
            tc
        } else {
            file.spectrogram.columns.len()
        }
    };
    let ref_db = if display_auto_gain && total_cols > 0 {
        crate::canvas::spectral_store::get_max_magnitude_dbfs(file_idx).unwrap_or(0.0)
    } else {
        0.0
    };
    let display_boost = state.display.gain_boost().get_untracked();

    let display_settings = SpectDisplaySettings {
        floor_db: spect_floor,
        range_db: spect_range,
        gamma: spect_gamma,
        gain_db: spect_gain - ref_db + display_boost,
    };

    let render = RenderParams {
        file_idx,
        total_cols,
        time_res,
        duration: file.audio.source.duration_secs(),
        start_time,
        end_time,
        file_max_freq,
        freq_crop_lo,
        freq_crop_hi,
        colormap,
        display_settings,
        min_freq,
        max_freq,
        canvas_w: vid_w,
        canvas_h: vid_h,
        shield_style: state.viewmode.shield_style().get_untracked(),
        annotations: shown_annotations(state),
    };

    // ── Audio settings ───────────────────────────────────────────────────────
    let region = if !regions.is_empty() {
        Some(&regions[0].1)
    } else {
        None
    };
    let audio_params = build_export_params(state, region, use_region_focus, sample_rate);
    let views = export_channel_views(
        source.as_ref(),
        state.viewmode.channel_view().get_untracked(),
    );
    // The audio is processed a piece at a time once the encoders are set up
    // (see "Encode audio"), so a long video's audio is never held whole.
    let new_region = || {
        RegionProcessor::new(
            source.as_ref(),
            &views,
            sample_rate,
            start_time,
            end_time,
            &audio_params,
        )
    };
    let region_error = |e: RegionError| match e {
        RegionError::Cancelled => JsValue::NULL,
        RegionError::NotLoaded { at_secs } => JsValue::from_str(&not_loaded_text(at_secs)),
    };
    let output_rate = export_output_rate(sample_rate, &audio_params);
    // A standard rate: many players struggle with high rates.
    let final_rate = normalized_rate(output_rate);
    let n_channels = views.len().max(1) as u32;
    // Every mode gives one output sample for each source sample.
    let audio_duration = new_region().frames() as f64 / output_rate.max(1) as f64;

    // ── Create offscreen canvas ──────────────────────────────────────────────
    let document = web_sys::window().unwrap().document().unwrap();
    let canvas: HtmlCanvasElement = document.create_element("canvas")?.dyn_into()?;
    canvas.set_width(vid_w);
    canvas.set_height(vid_h);
    let ctx: CanvasRenderingContext2d = canvas.get_context("2d")?.unwrap().dyn_into()?;

    // ── Set up encoders and muxer ────────────────────────────────────────────
    state
        .export
        .status()
        .set(Some("Setting up encoders...".to_string()));
    yield_now().await;

    // Resolve audio codec: (webcodecs_codec_str, muxer_codec_str) or None
    let audio_codec_choice = state.export.video_audio_codec().get_untracked();
    let resolved_audio: Option<(&str, &str)> = match audio_codec_choice {
        AudioCodecOption::NoAudio => {
            log::info!("Video export: audio disabled by user");
            None
        }
        AudioCodecOption::Aac => {
            if !wc::has_audio_encoder() {
                return Err(JsValue::from_str(
                    "AudioEncoder API not available in this browser",
                ));
            }
            log::info!("Video export: checking AAC support at {}Hz...", final_rate);
            if !wc::is_audio_config_supported(wc::AAC_WEBCODECS_CODEC, final_rate, n_channels).await
            {
                return Err(JsValue::from_str(&format!(
                    "AAC audio not supported at {}Hz in this browser",
                    final_rate
                )));
            }
            Some((wc::AAC_WEBCODECS_CODEC, wc::AAC_MUXER_CODEC))
        }
        AudioCodecOption::Opus => {
            if !wc::has_audio_encoder() {
                return Err(JsValue::from_str(
                    "AudioEncoder API not available in this browser",
                ));
            }
            log::info!("Video export: checking Opus support at {}Hz...", final_rate);
            if !wc::is_audio_config_supported(wc::OPUS_WEBCODECS_CODEC, final_rate, n_channels)
                .await
            {
                return Err(JsValue::from_str(&format!(
                    "Opus audio not supported at {}Hz in this browser",
                    final_rate
                )));
            }
            Some((wc::OPUS_WEBCODECS_CODEC, wc::OPUS_MUXER_CODEC))
        }
        AudioCodecOption::Auto => {
            if !wc::has_audio_encoder() {
                return Err(JsValue::from_str(
                    "AudioEncoder API not available. Select 'No audio' to export video without audio."
                ));
            }
            // Try AAC first, then Opus
            log::info!(
                "Video export: auto-detecting audio codec at {}Hz...",
                final_rate
            );
            if wc::is_audio_config_supported(wc::AAC_WEBCODECS_CODEC, final_rate, n_channels).await
            {
                log::info!("Video export: using AAC");
                Some((wc::AAC_WEBCODECS_CODEC, wc::AAC_MUXER_CODEC))
            } else if wc::is_audio_config_supported(
                wc::OPUS_WEBCODECS_CODEC,
                final_rate,
                n_channels,
            )
            .await
            {
                log::info!("Video export: AAC not supported, using Opus");
                Some((wc::OPUS_WEBCODECS_CODEC, wc::OPUS_MUXER_CODEC))
            } else {
                return Err(JsValue::from_str(
                    "No supported audio codec found (tried AAC and Opus). Select 'No audio' to export video without audio."
                ));
            }
        }
    };
    log::info!("Video export: creating muxer target...");
    let target = wc::create_array_buffer_target()?;
    log::info!(
        "Video export: creating muxer (audio={:?})...",
        resolved_audio.map(|a| a.1)
    );
    let muxer = wc::create_muxer(
        &target,
        codec_str,
        vid_w,
        vid_h,
        resolved_audio.map(|(_, muxer_codec)| (muxer_codec, final_rate, n_channels)),
    )?;

    // Shared muxer reference for closures
    let muxer_rc = Rc::new(RefCell::new(muxer));
    let video_error: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    // Video encoder
    let muxer_v = muxer_rc.clone();
    let on_video_chunk = Closure::new(move |chunk: JsValue, meta: JsValue| {
        let m = muxer_v.borrow();
        let _ = wc::muxer_add_video_chunk(&m, &chunk, &meta);
    });
    let ve = video_error.clone();
    let on_video_error = Closure::new(move |err: JsValue| {
        let msg = format!("{:?}", err);
        log::error!("VideoEncoder error: {msg}");
        *ve.borrow_mut() = Some(msg);
    });
    log::info!("Video export: creating video encoder...");
    let video_encoder = wc::create_video_encoder(&on_video_chunk, &on_video_error)?;
    log::info!("Video export: configuring video encoder...");
    wc::configure_video_encoder(
        &video_encoder,
        codec_str,
        vid_w,
        vid_h,
        DEFAULT_VIDEO_BITRATE,
        FPS,
    )?;
    log::info!("Video export: video encoder configured");

    // Audio encoder (optional — AAC support varies by browser/platform)
    let audio_encoder;
    let _on_audio_chunk;
    let _on_audio_error;

    if let Some((webcodecs_codec, _)) = resolved_audio {
        log::info!("Video export: creating audio encoder ({webcodecs_codec})...");
        let muxer_a = muxer_rc.clone();
        let cb = Closure::new(move |chunk: JsValue, meta: JsValue| {
            let m = muxer_a.borrow();
            let _ = wc::muxer_add_audio_chunk(&m, &chunk, &meta);
        });
        let eb = Closure::new(move |err: JsValue| {
            let msg = format!("{:?}", err);
            log::error!("AudioEncoder error: {msg}");
        });
        let enc = wc::create_audio_encoder(&cb, &eb)?;
        wc::configure_audio_encoder(
            &enc,
            webcodecs_codec,
            final_rate,
            n_channels,
            DEFAULT_AUDIO_BITRATE,
        )?;
        audio_encoder = Some(enc);
        _on_audio_chunk = Some(cb);
        _on_audio_error = Some(eb);
    } else {
        audio_encoder = None;
        _on_audio_chunk = None;
        _on_audio_error = None;
    }

    // ── Encode audio ─────────────────────────────────────────────────────────
    if let Some(ref enc) = audio_encoder {
        let audio_status = |done: f64| {
            state.export.status().set(Some(format!(
                "Processing audio... {}%",
                (done * 100.0) as u32
            )));
        };
        audio_status(0.0);
        yield_now().await;

        // Normalize needs the peak before anything is encoded, so the audio
        // is processed twice.
        let normalize = state.export.normalize().get_untracked();
        let passes = if normalize { 2.0 } else { 1.0 };
        let mut factor = 1.0f32;
        if normalize {
            let mut measured = new_region();
            let mut peak = 0.0f32;
            while let Some(piece) = measured.next_piece().await.map_err(region_error)? {
                peak = piece.iter().flatten().fold(peak, |p, s| p.max(s.abs()));
                audio_status(measured.progress() / passes);
                if state.export.cancel().get_untracked() {
                    return Ok(());
                }
                yield_now().await;
            }
            if peak > 0.0 {
                factor = 10f64.powf(crate::audio::export::NORMALIZE_DBFS / 20.0) as f32 / peak;
            }
        }

        let encode_fn: js_sys::Function =
            js_sys::Reflect::get(enc, &"encode".into())?.dyn_into()?;
        // Frames sent to the encoder so far, at `final_rate`.
        let mut sent = 0usize;
        let mut send = |chunk: &[&[f32]]| -> Result<(), JsValue> {
            let timestamp_us = (sent as f64 / final_rate as f64 * 1_000_000.0) as i64;
            let audio_data = wc::create_audio_data(chunk, final_rate, timestamp_us)?;
            encode_fn.call1(enc, &audio_data)?;
            wc::close_audio_data(&audio_data)?;
            sent += chunk.first().map_or(0, |c| c.len());
            Ok(())
        };
        // Audio goes to the encoder 1024 frames at a time (AAC's frame
        // size). `waiting` is resampled audio not sent yet.
        const AUDIO_CHUNK: usize = 1024;
        let mut resampler = Resampler::new(output_rate, final_rate, n_channels as usize);
        let mut waiting: Vec<Vec<f32>> = vec![Vec::new(); n_channels as usize];
        let mut region = new_region();
        while let Some(mut piece) = region.next_piece().await.map_err(region_error)? {
            if factor != 1.0 {
                piece.iter_mut().flatten().for_each(|s| *s *= factor);
            }
            resampler.push(&piece, &mut waiting);
            let whole = waiting[0].len() / AUDIO_CHUNK * AUDIO_CHUNK;
            for at in (0..whole).step_by(AUDIO_CHUNK) {
                let chunk: Vec<&[f32]> =
                    waiting.iter().map(|ch| &ch[at..at + AUDIO_CHUNK]).collect();
                send(&chunk)?;
            }
            waiting.iter_mut().for_each(|ch| {
                ch.drain(..whole);
            });
            audio_status((passes - 1.0 + region.progress()) / passes);
            if state.export.cancel().get_untracked() {
                let _ = wc::close_encoder(enc);
                let _ = wc::close_encoder(&video_encoder);
                return Ok(());
            }
            // Let the encoder catch up, so its queue does not grow with the
            // length of the video.
            yield_now().await;
            while encode_queue_size(enc) > 64 {
                crate::web_util::sleep_ms(4).await;
            }
        }
        resampler.finish(&mut waiting);
        if !waiting[0].is_empty() {
            let chunk: Vec<&[f32]> = waiting.iter().map(|ch| &ch[..]).collect();
            send(&chunk)?;
        }

        log::info!("Video export: flushing audio encoder ({sent} frames)...");
        wc::flush_encoder(enc).await?;
        log::info!("Video export: audio encoding complete");
    } else {
        log::info!("Video export: skipping audio (no AudioEncoder available)");
    }

    // ── Encode video frames ──────────────────────────────────────────────────
    let total_frames = (audio_duration * FPS).ceil() as u32;
    log::info!(
        "Video export: encoding {total_frames} video frames ({audio_duration:.2}s at {FPS}fps)..."
    );
    state
        .export
        .status()
        .set(Some("Encoding video...".to_string()));

    let view_mode = state.export.video_view_mode().get_untracked();
    let export_duration = render.end_time - render.start_time;

    // In time expansion mode, the audio is longer than the original time range.
    // Map video (wall-clock) time back to spectrogram (original) time.
    let playback_speed = if audio_duration > 0.0 {
        export_duration / audio_duration
    } else {
        1.0
    };

    match view_mode {
        VideoViewMode::StaticPlayhead => {
            // Zoom to fit the entire export range in the viewport
            let visible_time = export_duration;
            let zoom = vid_w as f64 / (visible_time / render.time_res);
            let scroll = render.start_time;
            let scroll_col = scroll / render.time_res;

            // Render background spectrogram once onto a separate canvas
            let document = web_sys::window().unwrap().document().unwrap();
            let bg_canvas: HtmlCanvasElement = document.create_element("canvas")?.dyn_into()?;
            bg_canvas.set_width(vid_w);
            bg_canvas.set_height(vid_h);
            let bg_ctx: CanvasRenderingContext2d =
                bg_canvas.get_context("2d")?.unwrap().dyn_into()?;
            ensure_tiles(state, &render, scroll_col, zoom).await;
            render_frame(&bg_ctx, &render, scroll_col, zoom, visible_time, scroll);

            log::info!("Video export: static playhead mode, {total_frames} frames");

            for frame_idx in 0..total_frames {
                if state.export.cancel().get_untracked() {
                    break;
                }
                if let Some(ref e) = *video_error.borrow() {
                    return Err(JsValue::from_str(e));
                }

                // Blit cached background
                ctx.draw_image_with_html_canvas_element(&bg_canvas, 0.0, 0.0)
                    .map_err(|e| JsValue::from_str(&format!("draw_image failed: {:?}", e)))?;

                // Draw playhead line
                let t = frame_idx as f64 / FPS;
                let playhead_x = (t * playback_speed / export_duration) * vid_w as f64;
                ctx.set_stroke_style_str("#ffffff");
                ctx.set_line_width(2.0);
                ctx.begin_path();
                ctx.move_to(playhead_x, 0.0);
                ctx.line_to(playhead_x, vid_h as f64);
                ctx.stroke();

                // Encode frame (every frame is nearly identical → P-frames compress well)
                let timestamp_us = (t * 1_000_000.0) as i64;
                let frame = wc::create_video_frame(&canvas, timestamp_us)?;
                let is_key = frame_idx % KEYFRAME_INTERVAL == 0;
                wc::encode_video_frame(&video_encoder, &frame, is_key)?;
                wc::close_video_frame(&frame)?;

                let progress = (frame_idx + 1) as f64 / total_frames as f64;
                state.export.progress().set(Some(progress));
                if frame_idx % 5 == 0 {
                    state.export.status().set(Some(format!(
                        "Encoding video... {}%",
                        (progress * 100.0) as u32
                    )));
                    yield_now().await;
                }
            }
        }
        VideoViewMode::ScrollingView => {
            // Compute zoom proportional to current app zoom
            let current_zoom = state.view.zoom_level().get_untracked();
            let app_canvas_w = state.viewmode.spectrogram_canvas_width().get_untracked();
            let zoom = if app_canvas_w > 0.0 {
                current_zoom * (vid_w as f64 / app_canvas_w)
            } else {
                current_zoom
            };
            let visible_time = (vid_w as f64 / zoom) * render.time_res;
            // More frequent keyframes for scrolling (every pixel changes)
            let scrolling_keyframe_interval = 15u32;

            for frame_idx in 0..total_frames {
                if state.export.cancel().get_untracked() {
                    break;
                }
                if let Some(ref e) = *video_error.borrow() {
                    return Err(JsValue::from_str(e));
                }

                let t = frame_idx as f64 / FPS;
                let spect_t = t * playback_speed; // position in original timeline
                let scroll = (render.start_time + spect_t - visible_time * 0.25)
                    .max(0.0)
                    .min((render.duration - visible_time).max(0.0));
                let scroll_col = scroll / render.time_res;

                ensure_tiles(state, &render, scroll_col, zoom).await;
                render_frame(&ctx, &render, scroll_col, zoom, visible_time, scroll);

                // Draw playhead line
                let playhead_x =
                    ((render.start_time + spect_t - scroll) / visible_time) * vid_w as f64;
                if playhead_x >= 0.0 && playhead_x <= vid_w as f64 {
                    ctx.set_stroke_style_str("#ffffff");
                    ctx.set_line_width(2.0);
                    ctx.begin_path();
                    ctx.move_to(playhead_x, 0.0);
                    ctx.line_to(playhead_x, vid_h as f64);
                    ctx.stroke();
                }

                let timestamp_us = (t * 1_000_000.0) as i64;
                let frame = wc::create_video_frame(&canvas, timestamp_us)?;
                let is_key = frame_idx % scrolling_keyframe_interval == 0;
                wc::encode_video_frame(&video_encoder, &frame, is_key)?;
                wc::close_video_frame(&frame)?;

                let progress = (frame_idx + 1) as f64 / total_frames as f64;
                state.export.progress().set(Some(progress));
                if frame_idx % 5 == 0 {
                    state.export.status().set(Some(format!(
                        "Encoding video... {}%",
                        (progress * 100.0) as u32
                    )));
                    yield_now().await;
                }
            }
        }
    }

    // If cancelled, close encoders and return early (skip finalize/download)
    if state.export.cancel().get_untracked() {
        let _ = wc::close_encoder(&video_encoder);
        if let Some(ref enc) = audio_encoder {
            let _ = wc::close_encoder(enc);
        }
        return Ok(());
    }

    // Flush video encoder
    state.export.status().set(Some("Finalizing...".to_string()));
    yield_now().await;
    wc::flush_encoder(&video_encoder).await?;
    wc::close_encoder(&video_encoder)?;
    if let Some(ref enc) = audio_encoder {
        wc::close_encoder(enc)?;
    }

    // Finalize muxer and download
    log::info!("Video export: finalizing muxer...");
    let muxer = muxer_rc.borrow();
    let mp4_bytes = wc::muxer_finalize(&muxer, &target)?;
    log::info!("Video export: MP4 size = {} bytes", mp4_bytes.len());

    let filename = video_file_name(&file, &views, &audio_params);

    save_export_bytes(state, mp4_bytes, filename, true);
    Ok(())
}

/// The current file's annotations that the "Annotations" export option
/// shows, or `None` when it shows none.
fn shown_annotations(state: &AppState) -> Option<crate::annotations::AnnotationSet> {
    let choice = state.export.video_annotations().get_untracked();
    let id = state.current_file_id()?;
    let mut set = state
        .annotations
        .store()
        .with_untracked(|store| store.get(id).cloned())?;
    set.annotations.retain(|a| choice.shows(&a.tags));
    (!set.annotations.is_empty()).then_some(set)
}

/// Render a single spectrogram frame to the offscreen canvas.
fn render_frame(
    ctx: &CanvasRenderingContext2d,
    r: &RenderParams,
    scroll_col: f64,
    zoom: f64,
    visible_time: f64,
    scroll_offset: f64,
) {
    // Clear canvas
    ctx.set_fill_style_str("#000");
    ctx.fill_rect(0.0, 0.0, r.canvas_w as f64, r.canvas_h as f64);

    // Blit spectrogram tiles
    spectrogram_renderer::blit_tiles_viewport(
        ctx,
        r.canvas_w as f64,
        r.canvas_h as f64,
        r.file_idx,
        r.total_cols,
        scroll_col,
        zoom,
        r.freq_crop_lo,
        r.freq_crop_hi,
        spectrogram_renderer::TileRenderMode::Spectrogram(r.colormap),
        &r.display_settings,
        None, // freq_adjustments — skip for simplicity in video export
        None, // preview fallback — not needed, tiles should be cached
        scroll_offset,
        visible_time,
        r.duration,
        TileSource::Normal,
    );

    // Draw time markers
    crate::canvas::time_markers::draw_time_markers(
        ctx,
        scroll_offset,
        visible_time,
        r.canvas_w as f64,
        r.canvas_h as f64,
        r.duration,
        None,  // no clock time config
        false, // don't show clock time
        1.0,   // time_scale = 1.0 (normal)
    );

    // Draw frequency markers
    use crate::canvas::overlays::{FreqMarkerState, FreqShiftMode};
    let ms = FreqMarkerState {
        mouse_freq: None,
        mouse_in_label_area: false,
        label_hover_opacity: 1.0,
        file_max_freq: r.file_max_freq,
        axis_drag_lo: None,
        axis_drag_hi: None,
        band_ff_drag_active: false,
        band_ff_lo: r.min_freq,
        band_ff_hi: r.max_freq,
        band_ff_handles_active: false,
        shield_style: r.shield_style,
        tuned_hz: None,
        heard_caption: None,
    };
    crate::canvas::overlays::draw_freq_markers(
        ctx,
        r.min_freq,
        r.max_freq,
        r.canvas_h as f64,
        r.canvas_w as f64,
        FreqShiftMode::None,
        &ms,
        0.0,   // het_cutoff (not relevant for video)
        false, // labels on left
    );

    if let Some(set) = &r.annotations {
        spectrogram_renderer::draw_annotations(
            ctx,
            set,
            &[],  // none drawn as selected
            None, // no resize handles
            r.min_freq,
            r.max_freq,
            scroll_offset,
            r.time_res,
            zoom,
            r.canvas_w as f64,
            r.canvas_h as f64,
            false, // desktop label sizes
            false,
            0.0,
        );
    }
}

/// The top of a video's sound, in Hz, when the file's spectrogram goes above
/// it: half the video's 48 kHz audio rate, for a file above 48 kHz played
/// 1:1. `None` in the other modes, which bring the ultrasound down into the
/// sound, and for files that do not go higher.
pub fn crop_hz(mode: crate::state::PlaybackMode, sample_rate: u32) -> Option<f64> {
    (mode == crate::state::PlaybackMode::Normal && sample_rate > VIDEO_AUDIO_RATE)
        .then_some(VIDEO_AUDIO_RATE as f64 / 2.0)
}

/// The highest audio rate a video is given.
const VIDEO_AUDIO_RATE: u32 = 48_000;

/// Audio rate for the video: rates above 48 kHz drop to 48 kHz (high sample
/// rates often cause playback issues in video players).
fn normalized_rate(rate: u32) -> u32 {
    rate.min(VIDEO_AUDIO_RATE)
}

/// Frames waiting in an encoder's queue (0 if the browser does not say).
fn encode_queue_size(encoder: &JsValue) -> u32 {
    js_sys::Reflect::get(encoder, &"encodeQueueSize".into())
        .ok()
        .and_then(|v| v.as_f64())
        .map_or(0, |n| n as u32)
}

/// Waits until the spectrogram tiles that a frame at `scroll_col` and `zoom`
/// draws are computed. A frame draws only tiles that are ready, and a long
/// video passes over parts of the file that were never on screen. Gives up
/// after 20 s and lets the frame be drawn with what there is.
async fn ensure_tiles(state: &AppState, r: &RenderParams, scroll_col: f64, zoom: f64) {
    use crate::canvas::tile_cache::{self, TILE_COLS};
    let lod = tile_cache::select_lod(zoom);
    let ratio = tile_cache::lod_ratio(lod);
    let vis_start = scroll_col
        .max(0.0)
        .min((r.total_cols as f64 - 1.0).max(0.0));
    let vis_end = (vis_start + r.canvas_w as f64 / zoom).min(r.total_cols as f64);
    if vis_end <= vis_start {
        return;
    }
    let first = (vis_start * ratio / TILE_COLS as f64).floor() as usize;
    let last = ((vis_end * ratio - 0.001).max(0.0) / TILE_COLS as f64).floor() as usize;
    let ready = || (first..=last).all(|t| tile_cache::get_tile(r.file_idx, lod, t).is_some());
    let never_disposed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let started = js_sys::Date::now();
    while !ready() && js_sys::Date::now() - started < 20_000.0 {
        crate::canvas::tile_scheduler::schedule_normal_tiles(
            *state,
            r.file_idx,
            r.total_cols,
            scroll_col,
            zoom,
            r.canvas_w as f64,
            r.time_res,
            false,
            false,
            &never_disposed,
        );
        crate::web_util::sleep_ms(8).await;
    }
}

/// Resamples audio from `rate` to `target_rate` by linear interpolation, a
/// piece at a time. Going down in rate, sound above the new rate's range is
/// filtered out first (8th-order Butterworth low-pass at 45% of
/// `target_rate`), so ultrasound is not folded down into the audible range.
struct Resampler {
    /// Source samples per output sample. 1 passes the audio through.
    ratio: f64,
    /// One filter per channel when going down in rate.
    lowpass: Vec<crate::dsp::filters::Butterworth>,
    /// Output samples made so far, and source samples taken in. Output
    /// sample `n` falls at source sample `n * ratio`, counted from the start
    /// of the audio, so pieces of any size give the same samples.
    made: u64,
    taken: u64,
    /// The final (filtered) sample of the last piece, per channel.
    last: Vec<f32>,
}

impl Resampler {
    fn new(rate: u32, target_rate: u32, channels: usize) -> Self {
        Resampler {
            ratio: rate as f64 / target_rate.max(1) as f64,
            lowpass: if target_rate < rate {
                (0..channels)
                    .map(|_| {
                        crate::dsp::filters::Butterworth::lowpass(
                            8,
                            rate as f32,
                            0.45 * target_rate as f32,
                        )
                    })
                    .collect()
            } else {
                Vec::new()
            },
            made: 0,
            taken: 0,
            last: vec![0.0; channels],
        }
    }

    /// Adds the resampled `piece` (one buffer per channel) to `out`.
    fn push(&mut self, piece: &[Vec<f32>], out: &mut [Vec<f32>]) {
        let len = piece.first().map_or(0, Vec::len);
        if len == 0 {
            return;
        }
        if self.ratio == 1.0 {
            for (o, ch) in out.iter_mut().zip(piece) {
                o.extend_from_slice(ch);
            }
            return;
        }
        let mut made = self.made;
        for (c, (o, ch)) in out.iter_mut().zip(piece).enumerate() {
            let mut buf = ch.clone();
            if let Some(filter) = self.lowpass.get_mut(c) {
                filter.process_in_place(&mut buf);
            }
            let last = self.last[c];
            let at = |i: isize| if i < 0 { last } else { buf[i as usize] };
            // Each output needs the sample after it too, so the piece's
            // final sample waits for the next piece.
            // The position is from the start of this piece: negative when it
            // falls between the last piece's final sample and this one's first.
            let mut n = self.made;
            loop {
                let pos = n as f64 * self.ratio - self.taken as f64;
                if pos >= len as f64 - 1.0 {
                    break;
                }
                let idx = pos.floor();
                let frac = (pos - idx) as f32;
                let (s0, s1) = (at(idx as isize), at(idx as isize + 1));
                o.push(s0 + frac * (s1 - s0));
                n += 1;
            }
            made = n;
            self.last[c] = buf[len - 1];
        }
        self.made = made;
        self.taken += len as u64;
    }

    /// Adds the samples that were waiting for a next piece.
    fn finish(&mut self, out: &mut [Vec<f32>]) {
        if self.ratio == 1.0 {
            return;
        }
        while (self.made as f64 * self.ratio) < self.taken as f64 {
            for (o, last) in out.iter_mut().zip(&self.last) {
                o.push(*last);
            }
            self.made += 1;
        }
    }
}

/// [`Resampler`] over one whole channel.
#[cfg(test)]
fn resample_linear(samples: &[f32], rate: u32, target_rate: u32) -> Vec<f32> {
    let mut resampler = Resampler::new(rate, target_rate, 1);
    let mut out = vec![Vec::new()];
    resampler.push(&[samples.to_vec()], &mut out);
    resampler.finish(&mut out);
    out.remove(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(hz: f64, rate: u32) -> Vec<f32> {
        (0..rate as usize)
            .map(|i| (std::f64::consts::TAU * hz * i as f64 / rate as f64).sin() as f32)
            .collect()
    }
    fn peak_db(x: &[f32]) -> f64 {
        let peak = x[x.len() / 4..].iter().fold(0.0f32, |p, s| p.max(s.abs()));
        20.0 * (peak as f64).log10()
    }

    /// At 192 kHz a 50 kHz tone would fold down to 2 kHz at 48 kHz. It is
    /// filtered out, and a 5 kHz tone is kept.
    #[test]
    fn ultrasound_is_not_folded_into_the_video_audio() {
        let high = resample_linear(&tone(50_000.0, 192_000), 192_000, 48_000);
        assert_eq!(high.len(), 48_000);
        assert!(peak_db(&high) < -60.0, "{}", peak_db(&high));
        let low = resample_linear(&tone(5_000.0, 192_000), 192_000, 48_000);
        assert!(peak_db(&low) > -1.0, "{}", peak_db(&low));
    }

    /// Resampling in pieces gives the same samples as resampling the whole,
    /// at a whole ratio (192 to 48 kHz) and at one that is not (250 to 48).
    #[test]
    fn only_a_1_to_1_video_of_a_high_rate_file_can_be_cropped() {
        use crate::state::PlaybackMode::*;
        assert_eq!(crop_hz(Normal, 192_000), Some(24_000.0));
        assert_eq!(crop_hz(Normal, 48_000), None);
        assert_eq!(crop_hz(Heterodyne, 192_000), None);
        assert_eq!(crop_hz(TimeExpansion, 192_000), None);
    }

    #[test]
    fn resampling_in_pieces_matches_the_whole() {
        for rate in [192_000u32, 250_000, 44_100, 48_000] {
            let whole_in = tone(3_000.0, rate);
            let whole = resample_linear(&whole_in, rate, 48_000);
            let mut resampler = Resampler::new(rate, 48_000, 1);
            let mut out = vec![Vec::new()];
            for piece in whole_in.chunks(7_001) {
                resampler.push(&[piece.to_vec()], &mut out);
            }
            resampler.finish(&mut out);
            assert_eq!(out[0].len(), whole.len(), "{rate}");
            let worst = out[0]
                .iter()
                .zip(&whole)
                .fold(0.0f32, |w, (a, b)| w.max((a - b).abs()));
            assert!(worst < 1e-5, "{rate}: {worst}");
            let expected = (rate as f64 / (rate as f64 / 48_000.0)).round() as usize;
            assert!(
                whole.len().abs_diff(expected) <= 1,
                "{rate}: {}",
                whole.len()
            );
        }
    }
}
