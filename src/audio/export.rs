//! WAV export: process audio regions through the DSP pipeline and download as WAV files.

use crate::state::store_fields::*;
use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue};

use crate::annotations::{Annotation, AnnotationKind, Region};
use crate::audio::playback::apply_gain;
use crate::audio::playback::snapshot_params;
use crate::audio::source::{AudioSource, ChannelView};
use crate::audio::streaming_playback::{
    apply_dsp_mode, apply_filters, te_speed_ratio, PlaybackParams, PV_MODE_BOOST_DB,
};
use crate::audio::wav_encoder::encode_wav_interleaved;
use crate::state::{AppState, PlaybackMode, Selection};

// Chunk size + filter warmup are shared with streaming playback so the offline
// and live DSP pipelines stay consistent. See audio::chunk_params.
use super::chunk_params::{CHUNK_SAMPLES, FILTER_WARMUP};

/// Overlap samples for PV/PS crossfade (same as streaming_playback::PV_HQ_OVERLAP).
const PV_HQ_OVERLAP: usize = 8192;

/// Build PlaybackParams for exporting a region.
/// When `use_region_focus` is true and the region has frequency bounds,
/// those bounds drive the selection-based bandpass/heterodyne.
pub(crate) fn build_export_params(
    state: &AppState,
    region: Option<&Region>,
    use_region_focus: bool,
    sample_rate: u32,
) -> PlaybackParams {
    let selection = if use_region_focus {
        region.and_then(|r| match (r.freq_low, r.freq_high) {
            (Some(lo), Some(hi)) => Some(Selection {
                time_start: r.time_start,
                time_end: r.time_end,
                freq_low: Some(lo),
                freq_high: Some(hi),
            }),
            _ => None,
        })
    } else {
        state.interaction.selection().get_untracked()
    };
    snapshot_params(state, selection, sample_rate)
}

/// The channel views an export reads, matching what playback sends to the
/// speakers: a 2+ channel source in the Stereo view gives `[L, R]`, anything
/// else gives the single view that playback uses.
pub(crate) fn export_channel_views(
    source: &dyn AudioSource,
    view: ChannelView,
) -> Vec<ChannelView> {
    if source.channel_count() >= 2 && view == ChannelView::Stereo {
        vec![ChannelView::Channel(0), ChannelView::Channel(1)]
    } else {
        vec![view]
    }
}

/// Process a time range through the DSP pipeline, once per channel view.
/// Returns one sample buffer per view, each the same length.
pub(crate) fn process_region(
    source: &dyn AudioSource,
    views: &[ChannelView],
    sample_rate: u32,
    start_time: f64,
    end_time: f64,
    params: &PlaybackParams,
) -> Vec<Vec<f32>> {
    let mut channels: Vec<Vec<f32>> = views
        .iter()
        .map(|&view| process_region_view(source, view, sample_rate, start_time, end_time, params))
        .collect();
    let frames = channels.iter().map(Vec::len).min().unwrap_or(0);
    for ch in &mut channels {
        ch.truncate(frames);
    }
    channels
}

/// Sample rate of the exported audio. Time expansion keeps every sample and
/// changes the rate instead (slower for TE > 1, faster for negative TE).
pub(crate) fn export_output_rate(sample_rate: u32, params: &PlaybackParams) -> u32 {
    match params.mode {
        PlaybackMode::TimeExpansion => {
            (sample_rate as f64 * te_speed_ratio(params.te_factor)) as u32
        }
        _ => sample_rate,
    }
}

/// Process one channel view of a time range through the DSP pipeline.
fn process_region_view(
    source: &dyn AudioSource,
    view: ChannelView,
    sample_rate: u32,
    start_time: f64,
    end_time: f64,
    params: &PlaybackParams,
) -> Vec<f32> {
    let start_sample = (start_time * sample_rate as f64) as usize;
    let end_sample =
        ((end_time * sample_rate as f64) as usize).min(source.total_samples() as usize);

    let crossfade_mode = params.pv_hq
        && matches!(
            params.mode,
            PlaybackMode::PhaseVocoder | PlaybackMode::PitchShift
        );

    let mut all_samples: Vec<f32> = Vec::new();
    let mut pos = start_sample;
    let mut is_first_chunk = true;

    while pos < end_sample {
        let chunk_end = (pos + CHUNK_SAMPLES).min(end_sample);
        let warmup_start = pos.saturating_sub(FILTER_WARMUP);
        let warmup_len = pos - warmup_start;

        // In crossfade mode, extend the read past the nominal end
        let trailing_end = if crossfade_mode {
            (chunk_end + PV_HQ_OVERLAP).min(end_sample)
        } else if matches!(
            params.mode,
            PlaybackMode::PitchShift | PlaybackMode::PhaseVocoder
        ) {
            (chunk_end + FILTER_WARMUP).min(end_sample)
        } else {
            chunk_end
        };
        let trailing_len = trailing_end - chunk_end;

        let chunk_with_warmup =
            source.read_region(view, warmup_start as u64, trailing_end - warmup_start);
        let filtered = apply_filters(&chunk_with_warmup, sample_rate, params);
        let processed = apply_dsp_mode(&filtered, sample_rate, params);

        if crossfade_mode {
            // Trim warmup but keep trailing overlap
            let trim_start = warmup_len;
            let mut chunk_samples = if trim_start < processed.len() {
                processed[trim_start..].to_vec()
            } else {
                processed.to_vec()
            };

            let core_len = chunk_end - pos;

            // Hann fade-in on leading overlap (skip for first chunk)
            if !is_first_chunk {
                let fade_in_len = PV_HQ_OVERLAP.min(core_len).min(chunk_samples.len());
                for (i, sample) in chunk_samples.iter_mut().enumerate().take(fade_in_len) {
                    let t = i as f32 / fade_in_len as f32;
                    let w = 0.5 * (1.0 - (std::f32::consts::PI * t).cos());
                    *sample *= w;
                }
            }

            // Hann fade-out on trailing overlap
            if trailing_len > 0 {
                let fade_out_start = chunk_samples.len().saturating_sub(trailing_len);
                let fade_out_len = chunk_samples.len() - fade_out_start;
                for i in 0..fade_out_len {
                    let t = i as f32 / fade_out_len as f32;
                    let w = 0.5 * (1.0 + (std::f32::consts::PI * t).cos());
                    chunk_samples[fade_out_start + i] *= w;
                }
            }

            // Overlap-add into output buffer
            if is_first_chunk {
                all_samples.extend_from_slice(&chunk_samples);
            } else {
                // The overlap region is the last PV_HQ_OVERLAP samples of the
                // existing output, which fade out. Sum with this chunk's
                // fade-in region.
                let overlap = PV_HQ_OVERLAP
                    .min(all_samples.len())
                    .min(chunk_samples.len());
                let out_start = all_samples.len() - overlap;
                for i in 0..overlap {
                    all_samples[out_start + i] += chunk_samples[i];
                }
                // Append the non-overlapping tail
                if overlap < chunk_samples.len() {
                    all_samples.extend_from_slice(&chunk_samples[overlap..]);
                }
            }
        } else {
            // Standard mode: trim warmup and trailing
            let trim_start = warmup_len;
            let trim_end = processed.len().saturating_sub(trailing_len);
            let trimmed = if trim_start < trim_end {
                &processed[trim_start..trim_end]
            } else {
                &processed[..]
            };
            all_samples.extend_from_slice(trimmed);
        }

        is_first_chunk = false;
        pos = chunk_end;
    }

    // Apply gain (including PV compensatory boost)
    let pv_boost = if matches!(params.mode, PlaybackMode::PhaseVocoder) {
        PV_MODE_BOOST_DB
    } else {
        0.0
    };
    let gain_db = params.gain_db + pv_boost;
    apply_gain(&mut all_samples, gain_db);

    all_samples
}

/// Trigger a browser download of raw bytes.
pub(crate) fn trigger_browser_download(data: &[u8], filename: &str) {
    let array = js_sys::Uint8Array::new_with_length(data.len() as u32);
    array.copy_from(data);

    let parts = js_sys::Array::new();
    parts.push(&array.buffer());

    let blob = match web_sys::Blob::new_with_u8_array_sequence(&parts) {
        Ok(b) => b,
        Err(e) => {
            log::error!("Failed to create Blob: {:?}", e);
            return;
        }
    };

    let url = match web_sys::Url::create_object_url_with_blob(&blob) {
        Ok(u) => u,
        Err(e) => {
            log::error!("Failed to create object URL: {:?}", e);
            return;
        }
    };

    let window = web_sys::window().unwrap();
    let document = window.document().unwrap();
    let a: web_sys::HtmlAnchorElement = document.create_element("a").unwrap().dyn_into().unwrap();
    a.set_href(&url);
    a.set_download(filename);
    a.set_attribute("style", "display:none").ok();
    document.body().unwrap().append_child(&a).ok();
    a.click();
    document.body().unwrap().remove_child(&a).ok();
    web_sys::Url::revoke_object_url(&url).ok();
}

/// Process one region and encode it as a WAV file with GUANO metadata.
/// Stereo when `views` has two entries (see [`export_channel_views`]).
#[allow(clippy::too_many_arguments)]
pub(crate) fn export_one_region(
    source: &dyn AudioSource,
    views: &[ChannelView],
    sample_rate: u32,
    start_time: f64,
    end_time: f64,
    params: &PlaybackParams,
    source_filename: &str,
    source_guano: Option<&crate::audio::guano::GuanoMetadata>,
) -> Vec<u8> {
    let channels = process_region(source, views, sample_rate, start_time, end_time, params);
    let frames = channels.first().map_or(0, Vec::len);
    let output_rate = export_output_rate(sample_rate, params);

    let mut wav_data = encode_wav_interleaved(
        &interleave(&channels),
        channels.len().max(1) as u16,
        output_rate,
    );

    let guano = build_export_guano(output_rate, frames, params, source_filename, source_guano);
    crate::audio::guano::append_guano_chunk(&mut wav_data, &guano.to_text());

    wav_data
}

/// Interleave equal-length channel buffers into L R L R ... order.
fn interleave(channels: &[Vec<f32>]) -> Vec<f32> {
    match channels {
        [] => Vec::new(),
        [mono] => mono.clone(),
        _ => {
            let frames = channels.iter().map(Vec::len).min().unwrap_or(0);
            let mut out = Vec::with_capacity(frames * channels.len());
            for i in 0..frames {
                for ch in channels {
                    out.push(ch[i]);
                }
            }
            out
        }
    }
}

/// File name without its audio extension (`bat.WAV` → `bat`).
pub(crate) fn export_base_name(name: &str) -> &str {
    const AUDIO_EXTS: [&str; 7] = ["wav", "w4v", "flac", "ogg", "mp3", "m4a", "m4b"];
    match name.rsplit_once('.') {
        Some((stem, ext)) if AUDIO_EXTS.iter().any(|e| ext.eq_ignore_ascii_case(e)) => stem,
        _ => name,
    }
}

/// Filename tag for a factor: whole numbers without decimals, others to 2 places
/// with trailing zeros dropped (`10` → `10`, `0.333` → `0.33`).
fn factor_tag(f: f64) -> String {
    if (f - f.round()).abs() < 1e-6 {
        format!("{}", f.round() as i64)
    } else {
        let s = format!("{f:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

/// Filename suffix naming the playback transform, so a time-expanded or
/// frequency-shifted export can't be mistaken for the original recording.
/// Empty when the audio is 1:1 (Normal mode, or a factor of 1).
///
/// `_TE10` (10x slower), `_TE0.1` (10x faster), `_PS10` / `_PSup10`,
/// `_PV10`, `_HET45k`, `_HET20k-comb4`, `_ZC8`.
pub(crate) fn transform_suffix(params: &PlaybackParams) -> String {
    match params.mode {
        PlaybackMode::Normal => String::new(),
        PlaybackMode::TimeExpansion => {
            let ratio = te_speed_ratio(params.te_factor);
            if (ratio - 1.0).abs() < 1e-9 {
                String::new()
            } else {
                format!("_TE{}", factor_tag(1.0 / ratio))
            }
        }
        PlaybackMode::PitchShift | PlaybackMode::PhaseVocoder => {
            let (tag, factor) = if params.mode == PlaybackMode::PitchShift {
                ("PS", params.ps_factor)
            } else {
                ("PV", params.pv_factor)
            };
            if factor.is_bypass() {
                String::new()
            } else {
                let dir = if factor.is_up() { "up" } else { "" };
                format!("_{tag}{dir}{}", factor_tag(factor.magnitude()))
            }
        }
        PlaybackMode::Heterodyne => {
            let has_sel =
                params.has_selection && (params.sel_freq_low > 0.0 || params.sel_freq_high > 0.0);
            let khz = |hz: f64| factor_tag((hz / 100.0).round() / 10.0);
            if params.het_comb_count <= 1 {
                let carrier = if has_sel {
                    (params.sel_freq_low + params.sel_freq_high) / 2.0
                } else {
                    params.het_freq
                };
                format!("_HET{}k", khz(carrier))
            } else {
                let base = if has_sel {
                    params.sel_freq_low
                } else {
                    params.het_freq
                };
                format!("_HET{}k-comb{}", khz(base), params.het_comb_count)
            }
        }
        PlaybackMode::ZeroCrossing => format!("_ZC{}", params.zc_factor as u32),
    }
}

/// Filename suffix for exporting one channel of a multi-channel file
/// (`_L`, `_R`, `_ch3`, `_mono`, `_L-R`). Empty for stereo or mono sources.
pub(crate) fn channel_suffix(source: &dyn AudioSource, views: &[ChannelView]) -> String {
    if source.channel_count() < 2 || views.len() != 1 {
        return String::new();
    }
    match views[0] {
        ChannelView::Stereo => String::new(),
        ChannelView::MonoMix => "_mono".to_string(),
        ChannelView::Channel(0) => "_L".to_string(),
        ChannelView::Channel(1) => "_R".to_string(),
        ChannelView::Channel(n) => format!("_ch{}", n + 1),
        ChannelView::Difference => "_L-R".to_string(),
    }
}

/// Outcome of saving one exported file.
enum ExportSave {
    Saved(String),
    Cancelled,
    Failed,
}

/// Save exported file bytes to the right place for the current platform.
///
/// - **Browser**: a blob download per file (a real browser honours them).
/// - **Android**: each file written into the shared MediaStore (visible in the
///   gallery / Files app) — WAV → `Recordings/Oversample/exports`,
///   MP4 → `Movies/Oversample/exports`. The Tauri WebView silently drops
///   `<a download>` blobs, which is why exports previously vanished.
/// - **Desktop**: a native "Save As" dialog per file.
///
/// On Tauri the saves run SEQUENTIALLY (one awaited at a time) so a first-run
/// Android permission prompt isn't raced by N concurrent invokes, and desktop
/// dialogs appear one after another instead of all stacked at once. Platform is
/// decided by `is_mobile_platform` (a stable startup flag), NOT the
/// viewport-width `is_mobile` signal — otherwise narrowing a desktop window
/// would wrongly try the Android-only MediaStore plugin.
pub(crate) fn save_exports(state: &AppState, items: Vec<(String, Vec<u8>)>, is_video: bool) {
    if items.is_empty() {
        return;
    }
    if !state.is_tauri {
        for (name, data) in &items {
            trigger_browser_download(data, name);
        }
        return;
    }

    let mobile = state.is_mobile_platform;
    let st = *state;
    wasm_bindgen_futures::spawn_local(async move {
        let (relative_path, mime) = if is_video {
            ("Movies/Oversample/exports", "video/mp4")
        } else {
            ("Recordings/Oversample/exports", "audio/wav")
        };
        let total = items.len();
        let mut saved = 0usize;
        let mut failed = 0usize;
        let mut last = String::new();
        for (name, data) in &items {
            let outcome = if mobile {
                save_export_to_media_store(data, name, relative_path, mime).await
            } else {
                save_export_via_dialog(data, name).await
            };
            match outcome {
                ExportSave::Saved(p) => {
                    saved += 1;
                    last = p;
                }
                ExportSave::Failed => {
                    failed += 1;
                }
                ExportSave::Cancelled => {}
            }
        }
        if saved > 0 {
            if total == 1 {
                st.show_info_toast(format!("Exported: {}", short_path(&last)));
            } else if mobile {
                st.show_info_toast(format!("Exported {saved}/{total} to {relative_path}"));
            } else {
                st.show_info_toast(format!("Exported {saved}/{total} files"));
            }
        } else if failed > 0 {
            st.show_error_toast("Export failed to save");
        }
        // saved == 0 && failed == 0 → everything cancelled: stay silent.
    });
}

/// Single-file convenience wrapper (used by the video export path).
pub(crate) fn save_export_bytes(state: &AppState, data: Vec<u8>, filename: String, is_video: bool) {
    save_exports(state, vec![(filename, data)], is_video);
}

/// Android: write export bytes into the shared MediaStore via the media-store
/// plugin (collection chosen by MIME type + relative path).
async fn save_export_to_media_store(
    data: &[u8],
    filename: &str,
    relative_path: &str,
    mime: &str,
) -> ExportSave {
    use crate::tauri_bridge::tauri_invoke_typed;
    let args = js_sys::Object::new();
    js_sys::Reflect::set(
        &args,
        &JsValue::from_str("filename"),
        &JsValue::from_str(filename),
    )
    .ok();
    js_sys::Reflect::set(
        &args,
        &JsValue::from_str("relativePath"),
        &JsValue::from_str(relative_path),
    )
    .ok();
    js_sys::Reflect::set(
        &args,
        &JsValue::from_str("mimeType"),
        &JsValue::from_str(mime),
    )
    .ok();
    let array = js_sys::Uint8Array::new_with_length(data.len() as u32);
    array.copy_from(data);
    js_sys::Reflect::set(&args, &JsValue::from_str("data"), &array).ok();

    match tauri_invoke_typed::<oversample_ipc::plugins::SavePathResult>(
        "plugin:media-store|saveExportBytes",
        &args.into(),
    )
    .await
    {
        Ok(result) => {
            if result.path.is_empty() {
                // e.g. pre-Q first run: storage permission was just granted and
                // nothing is written yet (the plugin asks the UI to retry). Don't
                // report this as a successful save.
                log::warn!("Export not saved (permission/retry) — needs another tap");
                ExportSave::Cancelled
            } else {
                log::info!("Export saved to shared storage: {}", result.path);
                ExportSave::Saved(result.path)
            }
        }
        Err(e) => {
            log::error!("Export save failed: {}", e);
            ExportSave::Failed
        }
    }
}

/// Desktop: write export bytes through a native "Save As" dialog.
async fn save_export_via_dialog(data: &[u8], filename: &str) -> ExportSave {
    use crate::tauri_bridge::tauri_invoke;
    let args = js_sys::Object::new();
    js_sys::Reflect::set(
        &args,
        &JsValue::from_str("filename"),
        &JsValue::from_str(filename),
    )
    .ok();
    let array = js_sys::Uint8Array::new_with_length(data.len() as u32);
    array.copy_from(data);
    js_sys::Reflect::set(&args, &JsValue::from_str("data"), &array).ok();

    match tauri_invoke("save_export_file", &args.into()).await {
        Ok(result) => {
            let path = result.as_string().unwrap_or_default();
            if path.is_empty() {
                log::info!("Export cancelled");
                ExportSave::Cancelled
            } else {
                log::info!("Export saved to: {}", path);
                ExportSave::Saved(path)
            }
        }
        Err(e) => {
            log::error!("Export save failed: {}", e);
            ExportSave::Failed
        }
    }
}

/// Trim a path/URI down to its trailing component for a compact toast.
fn short_path(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or(path).to_string()
}

/// Build GUANO metadata for an exported WAV file.
///
/// Field ordering: GUANO|Version first, then standard fields, then Oversample-specific.
/// For TE exports, Length is the "actual length" (WAV length / TE factor) per GUANO spec.
fn build_export_guano(
    output_rate: u32,
    frames: usize,
    params: &PlaybackParams,
    source_filename: &str,
    source_guano: Option<&crate::audio::guano::GuanoMetadata>,
) -> crate::audio::guano::GuanoMetadata {
    use crate::audio::guano::GuanoMetadata;
    let version = env!("CARGO_PKG_VERSION");
    let wav_duration_secs = frames as f64 / output_rate as f64;

    let mut g = GuanoMetadata::new();

    // ── GUANO namespace (must come first per spec) ──────────────────────
    g.add("GUANO|Version", "1.0");

    // ── Standard GUANO fields ───────────────────────────────────────────

    // Preserve source timestamp if available
    if let Some(src) = source_guano {
        for (k, v) in &src.fields {
            if k == "Timestamp" {
                g.add("Timestamp", v);
                break;
            }
        }
    }

    // TE factor and Length: per GUANO spec, Length is the "actual length"
    // which equals WAV length for direct recordings, but WAV length / TE
    // for time-expanded recordings.
    if let PlaybackMode::TimeExpansion = params.mode {
        // GUANO TE is the expansion factor: 10 for 10x slower, 0.1 for 10x faster.
        let te = 1.0 / te_speed_ratio(params.te_factor);
        let actual_length = wav_duration_secs / te;
        g.add("Length", &format!("{:.6}", actual_length));
        g.add("TE", &factor_tag(te));
    } else {
        g.add("Length", &format!("{:.6}", wav_duration_secs));
    }

    g.add("Samplerate", &output_rate.to_string());

    // Preserve Make/Model from source if present (recording hardware info)
    if let Some(src) = source_guano {
        for (k, v) in &src.fields {
            match k.as_str() {
                "Make" | "Model" => {
                    g.add(k, v);
                }
                _ => {}
            }
        }
    }

    g.add("Original Filename", source_filename);

    // Preserve location from source if present
    if let Some(src) = source_guano {
        for (k, v) in &src.fields {
            match k.as_str() {
                "Loc Position" | "Loc Elevation" | "Loc Accuracy" => {
                    g.add(k, v);
                }
                _ => {}
            }
        }
    }

    // ── Oversample-specific fields (after standard ones) ────────────────
    g.add("Oversample|App|Version", version);

    // Preserve Oversample mic/device fields from source
    if let Some(src) = source_guano {
        for (k, v) in &src.fields {
            if k.starts_with("Oversample|") && k != "Oversample|App|Version" {
                g.add(k, v);
            }
        }
    }

    // Export filter settings (these are actually applied to the audio)
    if params.filter_enabled {
        let hp_khz = params.filter_freq_low / 1000.0;
        let lp_khz = params.filter_freq_high / 1000.0;
        if hp_khz > 0.0 {
            g.add("Filter HP", &format!("{:.1}", hp_khz));
        }
        if lp_khz > 0.0 && lp_khz < output_rate as f64 / 2000.0 {
            g.add("Filter LP", &format!("{:.1}", lp_khz));
        }
    }

    g
}

/// Information about what will be exported, used for button text.
pub struct ExportInfo {
    pub count: usize,
    pub source_label: &'static str, // "selection" or "region" or "regions"
    pub mode_label: Option<String>, // e.g. "TE 10x", "HF", etc.
    pub estimated_duration_secs: Option<f64>,
}

/// Determine what will be exported and return info for the button label.
pub fn get_export_info(state: &AppState) -> Option<ExportInfo> {
    let selected_ids = state.annotations.selected_ids().get();
    let selection = state.interaction.selection().get();

    // Count selected annotations that are regions/segments (have time bounds)
    let (region_count, region_duration_sum) = if !selected_ids.is_empty() {
        if let Some(id) = state.current_file_id_tracked() {
            let store = state.annotations.store().get();
            if let Some(set) = store.get(id) {
                let mut count = 0usize;
                let mut dur = 0.0f64;
                for a in &set.annotations {
                    if selected_ids.contains(&a.id) {
                        if let AnnotationKind::Region(ref r) = a.kind {
                            count += 1;
                            dur += r.time_end - r.time_start;
                        }
                    }
                }
                (count, dur)
            } else {
                (0, 0.0)
            }
        } else {
            (0, 0.0)
        }
    } else {
        (0, 0.0)
    };

    let (count, source_label, source_duration) = if region_count > 0 {
        let label = if region_count == 1 {
            "region"
        } else {
            "regions"
        };
        (region_count, label, Some(region_duration_sum))
    } else if let Some(ref sel) = selection {
        (1, "selection", Some(sel.time_end - sel.time_start))
    } else if state.library.current_index().get().is_some() {
        let files = state.library.files().get();
        let file_dur = state
            .library
            .current_index()
            .get()
            .and_then(|idx| files.get(idx))
            .map(|f| f.audio.duration_secs);
        (1, "file", file_dur)
    } else {
        return None;
    };

    // Build mode label
    let mode = state.playback.mode().get();
    let hfr = state.viewmode.hfr_enabled().get();
    let mode_label = match mode {
        PlaybackMode::Normal if hfr => Some("HF".to_string()),
        PlaybackMode::TimeExpansion => {
            let te = state.transform.te_factor().get();
            Some(format!("TE {te}x"))
        }
        PlaybackMode::PitchShift => Some("pitch shift".to_string()),
        PlaybackMode::PhaseVocoder => Some("PV".to_string()),
        PlaybackMode::Heterodyne => Some("heterodyne".to_string()),
        PlaybackMode::ZeroCrossing => Some("ZC".to_string()),
        _ => None,
    };

    let estimated_duration_secs = source_duration.map(|d| match mode {
        PlaybackMode::TimeExpansion => d / te_speed_ratio(state.transform.te_factor().get()),
        _ => d,
    });

    Some(ExportInfo {
        count,
        source_label,
        mode_label,
        estimated_duration_secs,
    })
}

/// Format a duration in seconds to a human-readable string.
pub fn format_duration(secs: f64) -> String {
    if secs < 60.0 {
        format!("{:.1}s", secs)
    } else if secs < 3600.0 {
        let m = (secs / 60.0) as u32;
        let s = (secs % 60.0) as u32;
        format!("{}m {:02}s", m, s)
    } else {
        let h = (secs / 3600.0) as u32;
        let m = ((secs % 3600.0) / 60.0) as u32;
        let s = (secs % 60.0) as u32;
        format!("{}h {:02}m {:02}s", h, m, s)
    }
}

/// Get the list of selected Region annotations.
pub fn get_selected_regions(state: &AppState) -> Vec<(Annotation, Region)> {
    let selected_ids = state.annotations.selected_ids().get_untracked();
    if selected_ids.is_empty() {
        return Vec::new();
    }
    let id = match state.current_file_id() {
        Some(i) => i,
        None => return Vec::new(),
    };
    let store = state.annotations.store().get_untracked();
    let set = match store.get(id) {
        Some(s) => s,
        None => return Vec::new(),
    };
    set.annotations
        .iter()
        .filter(|a| selected_ids.contains(&a.id))
        .filter_map(|a| {
            if let AnnotationKind::Region(ref r) = a.kind {
                Some((a.clone(), r.clone()))
            } else {
                None
            }
        })
        .collect()
}

/// Export all selected regions (or current selection) as WAV files.
pub fn export_selected(state: &AppState) {
    let file = match state.current_file() {
        Some(f) => f,
        None => return,
    };
    let source = &file.audio.source;
    let sample_rate = file.audio.sample_rate;
    let use_region_focus = state.annotations.export_use_region_focus().get_untracked();
    let source_filename = &file.name;
    let source_guano = file.audio.metadata.guano.as_ref();
    let views = export_channel_views(
        source.as_ref(),
        state.viewmode.channel_view().get_untracked(),
    );
    let base_name = export_base_name(&file.name);
    let channel_tag = channel_suffix(source.as_ref(), &views);

    let regions = get_selected_regions(state);

    // (filename part, region time range, params) for each file to write.
    let mut jobs: Vec<(String, f64, f64, PlaybackParams)> = Vec::new();
    if !regions.is_empty() {
        for (i, (_annotation, region)) in regions.iter().enumerate() {
            let params = build_export_params(state, Some(region), use_region_focus, sample_rate);
            let suffix = match region.label.as_deref() {
                Some(l) if !l.is_empty() => format!("_{}", l.replace(' ', "_")),
                _ if regions.len() > 1 => format!("_{}", i + 1),
                _ => String::new(),
            };
            jobs.push((suffix, region.time_start, region.time_end, params));
        }
    } else if let Some(sel) = state.interaction.selection().get_untracked() {
        let params = build_export_params(state, None, false, sample_rate);
        jobs.push((
            "_selection".to_string(),
            sel.time_start,
            sel.time_end,
            params,
        ));
    } else {
        let params = build_export_params(state, None, false, sample_rate);
        let duration = file.audio.source.duration_secs();
        jobs.push(("_export".to_string(), 0.0, duration, params));
    }

    // Collect every region's WAV bytes first, then hand them to save_exports
    // as a batch so multi-region exports are saved one-at-a-time (no stacked
    // dialogs / racing permission prompts) and reported with a single toast.
    let exports: Vec<(String, Vec<u8>)> = jobs
        .into_iter()
        .map(|(suffix, start, end, params)| {
            let filename = format!(
                "{base_name}{suffix}{}{channel_tag}.wav",
                transform_suffix(&params)
            );
            let wav_data = export_one_region(
                source.as_ref(),
                &views,
                sample_rate,
                start,
                end,
                &params,
                source_filename,
                source_guano,
            );
            (filename, wav_data)
        })
        .collect();

    save_exports(state, exports, false);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_name_strips_audio_extensions_only() {
        assert_eq!(export_base_name("bat.WAV"), "bat");
        assert_eq!(export_base_name("a.b.flac"), "a.b");
        assert_eq!(export_base_name("notes.txt"), "notes.txt");
        assert_eq!(export_base_name("noext"), "noext");
    }

    #[test]
    fn factor_tag_formats() {
        assert_eq!(factor_tag(10.0), "10");
        assert_eq!(factor_tag(0.1), "0.1");
        assert_eq!(factor_tag(1.0 / 3.0), "0.33");
        assert_eq!(factor_tag(45.0), "45");
        assert_eq!(factor_tag(42.5), "42.5");
    }

    #[test]
    fn interleave_stereo() {
        let ch = vec![vec![1.0, 2.0], vec![-1.0, -2.0]];
        assert_eq!(interleave(&ch), vec![1.0, -1.0, 2.0, -2.0]);
    }
}
