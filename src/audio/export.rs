//! WAV export: process audio regions through the DSP pipeline and download as WAV files.

use crate::state::store_fields::*;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::annotations::{Annotation, AnnotationKind, Region};
use crate::audio::playback::apply_gain;
use crate::audio::playback::snapshot_params;
use crate::audio::source::{AudioSource, ChannelView};
use crate::audio::streaming_playback::{
    apply_dsp_mode, apply_filters, fixed_gain_db, te_speed_ratio, PlaybackParams,
};
use crate::audio::wav_encoder::encode_wav_at;
use crate::dsp::agc::{AgcConfig, AgcProcessor};
use crate::state::{
    AppState, ExportFormat, GainMode, LoadedFile, PlaybackMode, Selection, WavExportDepth,
};
use crate::types::FileMetadata;

// Chunk size + filter warmup are shared with streaming playback so the offline
// and live DSP pipelines stay consistent. See audio::chunk_params.
use super::chunk_params::{CHUNK_SAMPLES, FILTER_WARMUP};

/// The bits per sample and float flag a WAV export of a file with `meta` is
/// written at, after the encoder's own rounding (an 8- or 12-bit file and a
/// lossy file are written as 16-bit).
pub(crate) fn wav_export_format(depth: WavExportDepth, meta: &FileMetadata) -> (u16, bool) {
    let (bits, is_float) = match depth {
        WavExportDepth::SameAsFile => (meta.pcm_bits().unwrap_or(16), meta.is_float),
        WavExportDepth::Pcm16 => (16, false),
        WavExportDepth::Float32 => (32, true),
    };
    let fmt = oversample_core::audio::wav::WavWriteFormat::for_f32_samples(1, 1, bits, is_float);
    (fmt.bits_per_sample, fmt.is_float)
}

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

/// Why [`process_region_ready`] stopped without the audio.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum RegionError {
    /// The file's audio at `at_secs` could not be loaded.
    NotLoaded { at_secs: f64 },
    /// `on_progress` returned false.
    Cancelled,
}

/// Processes a time range through the DSP pipeline a piece at a time, once
/// per channel view, so a long range never has to be held whole.
///
/// Each chunk is loaded before it is read, so a streamed file gives its real
/// samples (a plain `read_region` gives silence for audio that is not in the
/// cache).
pub(crate) struct RegionProcessor<'a> {
    source: &'a dyn AudioSource,
    views: &'a [ChannelView],
    sample_rate: u32,
    params: &'a PlaybackParams,
    plans: Vec<ChunkPlan>,
    /// The next chunk of `plans` to process.
    next: usize,
    chunk_overlap: usize,
    /// Per view: processed samples not handed out yet. Between chunks this
    /// is the crossfade tail that the next chunk is added to.
    pending: Vec<Vec<f32>>,
    gain: OutputGain,
}

impl<'a> RegionProcessor<'a> {
    pub(crate) fn new(
        source: &'a dyn AudioSource,
        views: &'a [ChannelView],
        sample_rate: u32,
        start_time: f64,
        end_time: f64,
        params: &'a PlaybackParams,
    ) -> Self {
        RegionProcessor {
            source,
            views,
            sample_rate,
            params,
            plans: chunk_plans(
                sample_rate,
                start_time,
                end_time,
                source.total_samples() as usize,
                params,
            ),
            next: 0,
            chunk_overlap: crate::audio::streaming_playback::chunk_overlap(params),
            pending: views.iter().map(|_| Vec::new()).collect(),
            gain: OutputGain::new(export_output_rate(sample_rate, params), params),
        }
    }

    /// Source frames in the range.
    pub(crate) fn frames(&self) -> usize {
        match (self.plans.first(), self.plans.last()) {
            (Some(first), Some(last)) => last.chunk_end - first.pos,
            _ => 0,
        }
    }

    /// The part of the range processed so far, 0 to 1.
    pub(crate) fn progress(&self) -> f64 {
        if self.plans.is_empty() {
            1.0
        } else {
            self.next as f64 / self.plans.len() as f64
        }
    }

    /// The next piece of finished audio: one buffer per view, each the same
    /// length, with the output gain applied. `None` after the last piece.
    pub(crate) async fn next_piece(&mut self) -> Result<Option<Vec<Vec<f32>>>, RegionError> {
        let Some(plan) = self.plans.get(self.next) else {
            return Ok(None);
        };
        let read = crate::audio::streaming_source::read_views_ready(
            self.source,
            self.views,
            plan.warmup_start as u64,
            plan.trailing_end - plan.warmup_start,
        )
        .await;
        let Some(raw_views) = read.views else {
            return Err(RegionError::NotLoaded {
                at_secs: plan.pos as f64 / self.sample_rate as f64,
            });
        };
        for (out, raw) in self.pending.iter_mut().zip(&raw_views) {
            add_chunk(
                out,
                raw,
                plan,
                self.sample_rate,
                self.params,
                self.chunk_overlap,
                self.next == 0,
            );
        }
        self.next += 1;
        // All but the crossfade tail is final. After the last chunk, all of it.
        let have = self.pending.iter().map(Vec::len).min().unwrap_or(0);
        let done = if self.next == self.plans.len() {
            have
        } else {
            have.saturating_sub(self.chunk_overlap)
        };
        let mut piece: Vec<Vec<f32>> = self
            .pending
            .iter_mut()
            .map(|p| {
                let rest = p.split_off(done);
                std::mem::replace(p, rest)
            })
            .collect();
        if self.next == self.plans.len() {
            self.pending.iter_mut().for_each(Vec::clear);
        }
        self.gain.apply(&mut piece);
        Ok(Some(piece))
    }
}

/// Process a time range through the DSP pipeline, once per channel view.
/// Returns one sample buffer per view, each the same length. See
/// [`RegionProcessor`].
///
/// The page gets to draw between chunks. `on_progress` is called after each
/// chunk with the part done, 0 to 1; returning false stops the work with
/// [`RegionError::Cancelled`].
pub(crate) async fn process_region_ready(
    source: &dyn AudioSource,
    views: &[ChannelView],
    sample_rate: u32,
    start_time: f64,
    end_time: f64,
    params: &PlaybackParams,
    mut on_progress: impl FnMut(f64) -> bool,
) -> Result<Vec<Vec<f32>>, RegionError> {
    let mut region = RegionProcessor::new(source, views, sample_rate, start_time, end_time, params);
    let frames = region.frames();
    let mut channels: Vec<Vec<f32>> = views.iter().map(|_| Vec::with_capacity(frames)).collect();
    while let Some(piece) = region.next_piece().await? {
        for (out, part) in channels.iter_mut().zip(&piece) {
            out.extend_from_slice(part);
        }
        if !on_progress(region.progress()) {
            return Err(RegionError::Cancelled);
        }
        crate::canvas::tile_cache::yield_to_browser().await;
    }
    Ok(channels)
}

/// The gain stage of playback, applied to processed audio a piece at a time:
/// none when gain is Off, the slider for Manual, the measured peak gain plus
/// the slider for Peak, and the slider then the AGC leveler for Adaptive. The
/// leveler carries on from one piece to the next.
struct OutputGain {
    gain_db: f64,
    agc: Option<AgcProcessor>,
}

impl OutputGain {
    fn new(output_rate: u32, params: &PlaybackParams) -> Self {
        OutputGain {
            gain_db: fixed_gain_db(params.gain_mode, params.gain_db, params.auto_peak_gain_db),
            agc: (params.gain_mode == GainMode::Adaptive)
                .then(|| AgcProcessor::new(AgcConfig::default(), output_rate)),
        }
    }

    fn apply(&mut self, channels: &mut [Vec<f32>]) {
        for ch in channels.iter_mut() {
            apply_gain(ch, self.gain_db);
        }
        if let Some(agc) = &mut self.agc {
            match channels {
                [left, right] => agc.process_stereo(left, right),
                _ => channels.iter_mut().for_each(|ch| agc.process(ch)),
            }
        }
    }
}

/// Scale `channels` together so their highest peak is `target_dbfs`. Silent
/// audio is left alone.
pub(crate) fn normalize_peak(channels: &mut [Vec<f32>], target_dbfs: f64) {
    let peak = channels
        .iter()
        .flatten()
        .fold(0.0f32, |p, s| p.max(s.abs()));
    if peak > 0.0 {
        apply_gain_linear(channels, 10f64.powf(target_dbfs / 20.0) as f32 / peak);
    }
}

fn apply_gain_linear(channels: &mut [Vec<f32>], factor: f32) {
    for s in channels.iter_mut().flatten() {
        *s *= factor;
    }
}

/// Peak level of a normalized export, in dBFS.
pub(crate) const NORMALIZE_DBFS: f64 = -3.0;

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

/// One chunk of a region, in source samples: `pos..chunk_end` is the audio
/// the chunk adds, and `warmup_start..trailing_end` is what is read for it.
struct ChunkPlan {
    pos: usize,
    chunk_end: usize,
    /// Start of the read: `pos` less the filter warmup.
    warmup_start: usize,
    /// In crossfade mode the chunk is kept past its nominal end by the
    /// overlap, up to here.
    keep_end: usize,
    /// End of the read. Anything after `keep_end` is cut off once processed.
    trailing_end: usize,
}

/// The chunks that cover `start_time..end_time` of a source with
/// `total_samples` samples, the same chunks as playback.
fn chunk_plans(
    sample_rate: u32,
    start_time: f64,
    end_time: f64,
    total_samples: usize,
    params: &PlaybackParams,
) -> Vec<ChunkPlan> {
    let start_sample = (start_time.max(0.0) * sample_rate as f64) as usize;
    let end_sample = ((end_time * sample_rate as f64) as usize).min(total_samples);
    let mut plans = Vec::new();
    let mut pos = start_sample;
    while pos < end_sample {
        let chunk_end = (pos + CHUNK_SAMPLES).min(end_sample);
        let (keep_end, trailing_end) =
            crate::audio::streaming_playback::chunk_tail(params, chunk_end, end_sample);
        plans.push(ChunkPlan {
            pos,
            chunk_end,
            warmup_start: pos.saturating_sub(FILTER_WARMUP),
            keep_end,
            trailing_end,
        });
        pos = chunk_end;
    }
    plans
}

/// Process one chunk's samples (`raw`, read from `plan.warmup_start` to
/// `plan.trailing_end`) and add them to `out`, with the same crossfade
/// between chunks as playback when `chunk_overlap` is above 0.
fn add_chunk(
    out: &mut Vec<f32>,
    raw: &[f32],
    plan: &ChunkPlan,
    sample_rate: u32,
    params: &PlaybackParams,
    chunk_overlap: usize,
    is_first_chunk: bool,
) {
    let warmup_len = plan.pos - plan.warmup_start;
    let trailing_len = plan.keep_end - plan.chunk_end;

    let filtered = apply_filters(raw, sample_rate, params);
    let mut processed = apply_dsp_mode(&filtered, sample_rate, params);
    processed.truncate(
        processed
            .len()
            .saturating_sub(plan.trailing_end - plan.keep_end),
    );

    if chunk_overlap > 0 {
        // Trim warmup but keep trailing overlap
        let mut chunk_samples = if warmup_len < processed.len() {
            processed[warmup_len..].to_vec()
        } else {
            processed.to_vec()
        };

        let core_len = plan.chunk_end - plan.pos;

        // Hann fade-in on leading overlap (skip for first chunk)
        if !is_first_chunk {
            let fade_in_len = chunk_overlap.min(core_len).min(chunk_samples.len());
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
            out.extend_from_slice(&chunk_samples);
        } else {
            // The overlap region is the last `chunk_overlap` samples of the
            // existing output, which fade out. Sum with this chunk's
            // fade-in region.
            let overlap = chunk_overlap.min(out.len()).min(chunk_samples.len());
            let out_start = out.len() - overlap;
            for i in 0..overlap {
                out[out_start + i] += chunk_samples[i];
            }
            // Append the non-overlapping tail
            if overlap < chunk_samples.len() {
                out.extend_from_slice(&chunk_samples[overlap..]);
            }
        }
    } else {
        // Standard mode: trim warmup and trailing
        let trim_end = processed.len().saturating_sub(trailing_len);
        let trimmed = if warmup_len < trim_end {
            &processed[warmup_len..trim_end]
        } else {
            &processed[..]
        };
        out.extend_from_slice(trimmed);
    }
}

/// Process one channel view of a time range through the DSP pipeline, with
/// plain reads: for sources that are wholly in memory.
#[cfg(test)]
fn process_region_view(
    source: &dyn AudioSource,
    view: ChannelView,
    sample_rate: u32,
    start_time: f64,
    end_time: f64,
    params: &PlaybackParams,
) -> Vec<f32> {
    let plans = chunk_plans(
        sample_rate,
        start_time,
        end_time,
        source.total_samples() as usize,
        params,
    );
    let chunk_overlap = crate::audio::streaming_playback::chunk_overlap(params);
    let mut out = Vec::new();
    for (i, plan) in plans.iter().enumerate() {
        let raw = source.read_region(
            view,
            plan.warmup_start as u64,
            plan.trailing_end - plan.warmup_start,
        );
        add_chunk(
            &mut out,
            &raw,
            plan,
            sample_rate,
            params,
            chunk_overlap,
            i == 0,
        );
    }
    out
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

/// Encode one processed region as a WAV file with GUANO metadata, at `bits`
/// bits per sample (32-bit float when `is_float`); see [`wav_export_format`].
/// Stereo when `channels` has two entries (see [`export_channel_views`]).
#[allow(clippy::too_many_arguments)]
fn encode_region(
    mut channels: Vec<Vec<f32>>,
    sample_rate: u32,
    params: &PlaybackParams,
    source_filename: &str,
    source_guano: Option<&crate::audio::guano::GuanoMetadata>,
    bits: u16,
    is_float: bool,
    normalize: bool,
) -> Vec<u8> {
    if normalize {
        normalize_peak(&mut channels, NORMALIZE_DBFS);
    }
    let frames = channels.first().map_or(0, Vec::len);
    let n_channels = channels.len().max(1) as u16;
    let output_rate = export_output_rate(sample_rate, params);

    let mut wav_data = encode_wav_at(
        &interleave(channels),
        n_channels,
        output_rate,
        bits,
        is_float,
    );

    let guano = build_export_guano(output_rate, frames, params, source_filename, source_guano);
    crate::audio::guano::append_guano_chunk(&mut wav_data, &guano.to_text());

    wav_data
}

/// Interleave equal-length channel buffers into L R L R ... order.
fn interleave(mut channels: Vec<Vec<f32>>) -> Vec<f32> {
    match channels.len() {
        0 => Vec::new(),
        1 => channels.remove(0),
        _ => {
            let frames = channels.iter().map(Vec::len).min().unwrap_or(0);
            let mut out = Vec::with_capacity(frames * channels.len());
            for i in 0..frames {
                for ch in &channels {
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
/// `_HET45k`, `_HET20k-comb4`, `_ZC8`.
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
        PlaybackMode::PitchShift => {
            let factor = params.ps_factor;
            if factor.is_bypass() {
                String::new()
            } else {
                let dir = if factor.is_up() { "up" } else { "" };
                format!("_PS{dir}{}", factor_tag(factor.magnitude()))
            }
        }
        PlaybackMode::Heterodyne => {
            let has_sel =
                params.has_selection && (params.sel_freq_low > 0.0 || params.sel_freq_high > 0.0);
            let khz = |hz: f64| factor_tag((hz / 100.0).round() / 10.0);
            // "-PS2": the HET output pitch shifted down by 2.
            let post = if params.het_ps_factor > 1.0 {
                format!("-PS{}", factor_tag(params.het_ps_factor))
            } else {
                String::new()
            };
            if params.het_comb_count <= 1 {
                let carrier = if has_sel {
                    (params.sel_freq_low + params.sel_freq_high) / 2.0
                } else {
                    params.het_freq
                };
                format!("_HET{}k{post}", khz(carrier))
            } else {
                let base = if has_sel {
                    params.sel_freq_low
                } else {
                    params.het_freq
                };
                format!("_HET{}k-comb{}{post}", khz(base), params.het_comb_count)
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
    match crate::native_save::save_to_shared_storage(data, filename, relative_path, mime).await {
        Ok(crate::native_save::SharedSave::Saved(path)) => {
            log::info!("Export saved to shared storage: {path}");
            ExportSave::Saved(path)
        }
        Ok(crate::native_save::SharedSave::NeedsRetry) => {
            // Pre-Q first run: storage permission was just granted and nothing
            // is written yet. Don't report this as a successful save.
            log::warn!("Export not saved (permission/retry) — needs another tap");
            ExportSave::Cancelled
        }
        Err(e) => {
            log::error!("Export save failed: {e}");
            ExportSave::Failed
        }
    }
}

/// Desktop: write export bytes through a native "Save As" dialog.
async fn save_export_via_dialog(data: &[u8], filename: &str) -> ExportSave {
    match crate::native_save::save_with_dialog(data, filename).await {
        Ok(Some(path)) => {
            log::info!("Export saved to: {path}");
            ExportSave::Saved(path)
        }
        Ok(None) => {
            log::info!("Export cancelled");
            ExportSave::Cancelled
        }
        Err(e) => {
            log::error!("Export save failed: {e}");
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
    pub source_label: &'static str, // "selection", "region", "regions" or "whole file"
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
        (1, "whole file", file_dur)
    } else {
        return None;
    };

    let mode = state.playback.mode().get();
    let estimated_duration_secs = source_duration.map(|d| match mode {
        PlaybackMode::TimeExpansion => d / te_speed_ratio(state.transform.te_factor().get()),
        _ => d,
    });

    Some(ExportInfo {
        count,
        source_label,
        estimated_duration_secs,
    })
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

/// One file an export writes: the name part after the base name (`_selection`,
/// a region label, ...), the time range, and the DSP settings to apply.
pub(crate) struct ExportJob {
    pub name_part: String,
    pub start: f64,
    pub end: f64,
    pub params: PlaybackParams,
}

/// The files a WAV export writes: one per selected region, else the current
/// selection, else the whole file.
pub(crate) fn export_jobs(state: &AppState, file: &LoadedFile) -> Vec<ExportJob> {
    let sample_rate = file.audio.sample_rate;
    let use_region_focus = state.annotations.export_use_region_focus().get_untracked();
    let regions = get_selected_regions(state);
    if !regions.is_empty() {
        return regions
            .iter()
            .enumerate()
            .map(|(i, (_annotation, region))| ExportJob {
                name_part: match region.label.as_deref() {
                    Some(l) if !l.is_empty() => format!("_{}", l.replace(' ', "_")),
                    _ if regions.len() > 1 => format!("_{}", i + 1),
                    _ => String::new(),
                },
                start: region.time_start,
                end: region.time_end,
                params: build_export_params(state, Some(region), use_region_focus, sample_rate),
            })
            .collect();
    }
    let params = build_export_params(state, None, false, sample_rate);
    match state.interaction.selection().get_untracked() {
        Some(sel) => vec![ExportJob {
            name_part: "_selection".to_string(),
            start: sel.time_start,
            end: sel.time_end,
            params,
        }],
        None => vec![ExportJob {
            name_part: "_export".to_string(),
            start: 0.0,
            end: file.audio.source.duration_secs(),
            params,
        }],
    }
}

/// File name for one WAV export job.
fn wav_file_name(file: &LoadedFile, views: &[ChannelView], job: &ExportJob) -> String {
    format!(
        "{}{}{}{}.wav",
        export_base_name(&file.name),
        job.name_part,
        transform_suffix(&job.params),
        channel_suffix(file.audio.source.as_ref(), views),
    )
}

/// File name for an MP4 export made with `params`. Video exports one range,
/// so the name has no region part.
pub(crate) fn video_file_name(
    file: &LoadedFile,
    views: &[ChannelView],
    params: &PlaybackParams,
) -> String {
    format!(
        "{}{}{}.mp4",
        export_base_name(&file.name),
        transform_suffix(params),
        channel_suffix(file.audio.source.as_ref(), views),
    )
}

/// The file names an export in `format` would save right now.
pub(crate) fn planned_file_names(state: &AppState, format: ExportFormat) -> Vec<String> {
    let Some(file) = state.library.files().with_untracked(|files| {
        state
            .library
            .current_index()
            .get_untracked()
            .and_then(|i| files.get(i).cloned())
    }) else {
        return Vec::new();
    };
    let views = export_channel_views(
        file.audio.source.as_ref(),
        state.viewmode.channel_view().get_untracked(),
    );
    let jobs = export_jobs(state, &file);
    match format {
        ExportFormat::Wav => jobs
            .iter()
            .map(|job| wav_file_name(&file, &views, job))
            .collect(),
        ExportFormat::Mp4 => jobs
            .first()
            .map(|job| vec![video_file_name(&file, &views, &job.params)])
            .unwrap_or_default(),
    }
}

/// The most processed audio one export holds in memory, as 32-bit float
/// samples over all its channels and files. The WAV bytes are made from these
/// and are held at the same time, so the page needs up to about three times
/// this while it exports.
pub(crate) const MAX_EXPORT_SAMPLE_BYTES: u64 = 512 * 1024 * 1024;

/// An export that is over [`MAX_EXPORT_SAMPLE_BYTES`]: its length and the
/// longest length that fits, both as the exported audio plays (the same
/// terms as [`ExportInfo::estimated_duration_secs`]).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TooLong {
    pub secs: f64,
    pub max_secs: f64,
}

/// `Some` when `frames` source frames over `channels` channels, written at
/// `output_rate`, are more than one export can hold.
fn too_long(frames: u64, channels: usize, output_rate: u32) -> Option<TooLong> {
    let max_frames = MAX_EXPORT_SAMPLE_BYTES / 4 / channels.max(1) as u64;
    (frames > max_frames).then(|| TooLong {
        secs: frames as f64 / output_rate.max(1) as f64,
        max_secs: max_frames as f64 / output_rate.max(1) as f64,
    })
}

/// `Some` when the export that `format` would make of `file` right now is too
/// long to make here. WAV counts every selected region; MP4 exports the
/// first one only.
pub(crate) fn export_too_long(
    state: &AppState,
    file: &LoadedFile,
    format: ExportFormat,
) -> Option<TooLong> {
    // A video's audio is encoded a piece at a time, so it has no limit.
    if format == ExportFormat::Mp4 || writes_long_exports_to_file(state) {
        return None;
    }
    over_memory_limit(state, file, format)
}

/// True where a WAV export too long for memory is written to a file a piece
/// at a time instead: the desktop app (through its "Save As" dialog), the
/// Android app (shared storage), and a browser with storage it can write to
/// that way (`crate::opfs_audio`).
fn writes_long_exports_to_file(state: &AppState) -> bool {
    state.is_tauri || crate::opfs_audio::supported()
}

/// `Some` when the export that `format` would make of `file` right now is
/// over [`MAX_EXPORT_SAMPLE_BYTES`].
fn over_memory_limit(state: &AppState, file: &LoadedFile, format: ExportFormat) -> Option<TooLong> {
    let source = file.audio.source.as_ref();
    let views = export_channel_views(source, state.viewmode.channel_view().get_untracked());
    let jobs = export_jobs(state, file);
    let counted = match format {
        ExportFormat::Wav => &jobs[..],
        ExportFormat::Mp4 => &jobs[..jobs.len().min(1)],
    };
    let rate = file.audio.sample_rate;
    let file_secs = source.duration_secs();
    let frames: f64 = counted
        .iter()
        .map(|job| (job.end.min(file_secs) - job.start.max(0.0)).max(0.0) * rate as f64)
        .sum();
    let output_rate = export_output_rate(rate, &counted.first()?.params);
    too_long(frames as u64, views.len(), output_rate)
}

/// The message for an export in `format` that is too long, shown under the
/// Export button and as the error if the export is started anyway. It names
/// what the export is made of, the same way [`export_jobs`] picks it.
pub(crate) fn too_long_text(state: &AppState, t: TooLong, format: ExportFormat) -> String {
    use crate::format_time::format_duration_compact as dur;
    let regions = match format {
        ExportFormat::Wav => get_selected_regions(state).len(),
        // A video is of the first region only.
        ExportFormat::Mp4 => get_selected_regions(state).len().min(1),
    };
    let (what, advice) = match regions {
        0 if state.interaction.selection().get_untracked().is_some() => {
            ("the selection is".to_string(), "Select a shorter part.")
        }
        0 => ("the whole file is".to_string(), "Select a shorter part."),
        1 => ("the region is".to_string(), "Select a shorter part."),
        n => (
            format!("the {n} regions total"),
            "Select fewer or shorter regions.",
        ),
    };
    format!(
        "Too long to export: {what} {}. The limit for this file is {}. {advice}",
        dur(t.secs),
        dur(t.max_secs),
    )
}

/// Show `msg` as a failed export in the Export panel for 10 seconds.
pub(crate) fn show_export_failed(state: &AppState, msg: &str) {
    let state = *state;
    let text = format!("Export failed: {msg}");
    log::error!("{text}");
    state.export.progress().set(None);
    state.export.status().set(Some(text.clone()));
    leptos::task::spawn_local(async move {
        crate::web_util::sleep_ms(10_000).await;
        if state.export.status().get_untracked().as_ref() == Some(&text) {
            state.export.status().set(None);
        }
    });
}

/// The message for audio that could not be read during an export.
pub(crate) fn not_loaded_text(at_secs: f64) -> String {
    format!(
        "couldn't read the audio at {} into the file",
        crate::format_time::format_duration_compact(at_secs)
    )
}

/// Export all selected regions (or current selection) as WAV files. Runs in
/// the background with the Export panel's progress bar and Cancel button.
pub fn export_selected(state: &AppState) {
    let state = *state;
    let file = match state.current_file() {
        Some(f) => f,
        None => return,
    };
    if state.export.progress().get_untracked().is_some() {
        return;
    }
    if let Some(t) = export_too_long(&state, &file, ExportFormat::Wav) {
        show_export_failed(&state, &too_long_text(&state, t, ExportFormat::Wav));
        return;
    }
    let views = export_channel_views(
        file.audio.source.as_ref(),
        state.viewmode.channel_view().get_untracked(),
    );
    let (bits, is_float) = wav_export_format(
        state.export.wav_depth().get_untracked(),
        &file.audio.metadata,
    );
    let normalize = state.export.normalize().get_untracked();
    // Everything the export depends on is read here, before the first await,
    // so changing the selection or the file while it runs has no effect.
    let jobs = export_jobs(&state, &file);
    let to_file = over_memory_limit(&state, &file, ExportFormat::Wav).is_some();

    state.export.cancel().set(false);
    state.export.progress().set(Some(0.0));
    state
        .export
        .status()
        .set(Some("Processing audio...".to_string()));

    leptos::task::spawn_local(async move {
        if to_file {
            export_to_files(state, &file, &views, &jobs, bits, is_float, normalize).await;
            return;
        }
        // Collect every region's WAV bytes first, then hand them to
        // save_exports as a batch so multi-region exports are saved
        // one-at-a-time (no stacked dialogs / racing permission prompts) and
        // reported with a single toast.
        let mut exports: Vec<(String, Vec<u8>)> = Vec::with_capacity(jobs.len());
        let n_jobs = jobs.len() as f64;
        for (i, job) in jobs.iter().enumerate() {
            let processed = process_region_ready(
                file.audio.source.as_ref(),
                &views,
                file.audio.sample_rate,
                job.start,
                job.end,
                &job.params,
                |done| {
                    let progress = (i as f64 + done) / n_jobs;
                    state.export.progress().set(Some(progress));
                    state.export.status().set(Some(format!(
                        "Processing audio... {}%",
                        (progress * 100.0) as u32
                    )));
                    !state.export.cancel().get_untracked()
                },
            )
            .await;
            let channels = match processed {
                Ok(channels) => channels,
                Err(RegionError::Cancelled) => {
                    state.export.progress().set(None);
                    state.export.status().set(None);
                    return;
                }
                Err(RegionError::NotLoaded { at_secs }) => {
                    show_export_failed(&state, &not_loaded_text(at_secs));
                    return;
                }
            };
            let wav_data = encode_region(
                channels,
                file.audio.sample_rate,
                &job.params,
                &file.name,
                file.audio.metadata.guano.as_ref(),
                bits,
                is_float,
                normalize,
            );
            exports.push((wav_file_name(&file, &views, job), wav_data));
        }
        state.export.progress().set(None);
        state.export.status().set(None);
        save_exports(&state, exports, false);
    });
}

thread_local! {
    /// Long exports still in the browser's storage. The browser downloads
    /// from the stored file, so it is deleted when the next long export
    /// starts, or by `crate::opfs_audio` after the tab closes.
    static STORED_EXPORTS: std::cell::RefCell<Vec<crate::opfs_audio::StoredFile>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Why a long export's file was not written. `None` is a cancel.
type FileExportError = Option<String>;

fn region_error(e: RegionError) -> FileExportError {
    match e {
        RegionError::Cancelled => None,
        RegionError::NotLoaded { at_secs } => Some(not_loaded_text(at_secs)),
    }
}

/// The file a long export is written to, a piece at a time.
enum ExportSink {
    /// A file in the browser's storage, downloaded once it is whole.
    Browser(crate::opfs_audio::Writer),
    /// Desktop app: the file the user picked in the "Save As" dialog.
    /// Android app: a new file in the shared `Recordings/Oversample/exports`.
    Native(crate::native_save::OpenSave),
}

/// A long export's finished file.
enum ExportedFile {
    Browser(web_sys::File, crate::opfs_audio::StoredFile),
    /// The path it was saved to.
    Native(String),
}

impl ExportSink {
    /// Opens the file for `name`, which needs `bytes` of room. `Err(None)`
    /// when the user cancels the "Save As" dialog, or when Android has just
    /// asked for the storage permission and the export has to be started
    /// again.
    async fn open(state: &AppState, name: &str, bytes: u64) -> Result<Self, FileExportError> {
        if state.is_tauri {
            let opened = if state.is_mobile_platform {
                crate::native_save::OpenSave::in_shared_storage(
                    name,
                    "Recordings/Oversample/exports",
                    "audio/wav",
                )
                .await
            } else {
                crate::native_save::OpenSave::with_dialog(name).await
            };
            return match opened {
                Ok(Some(save)) => Ok(ExportSink::Native(save)),
                Ok(None) => Err(None),
                Err(e) => Err(Some(e)),
            };
        }
        if let Some(free) = crate::opfs_audio::free_bytes().await {
            if free < bytes {
                return Err(Some(no_space_text(bytes, free)));
            }
        }
        crate::opfs_audio::Writer::create(name)
            .await
            .map(ExportSink::Browser)
            .map_err(Some)
    }

    async fn write(&mut self, bytes: &[u8]) -> Result<(), FileExportError> {
        match self {
            ExportSink::Browser(writer) => writer.write(&js_sys::Uint8Array::from(bytes)).await,
            ExportSink::Native(save) => save.write(bytes).await,
        }
        .map_err(Some)
    }

    async fn finish(self) -> Result<ExportedFile, FileExportError> {
        match self {
            ExportSink::Browser(writer) => writer
                .finish()
                .await
                .map(|(file, stored)| ExportedFile::Browser(file, stored)),
            ExportSink::Native(save) => save.close().await.map(ExportedFile::Native),
        }
        .map_err(Some)
    }

    async fn discard(self) {
        match self {
            ExportSink::Browser(writer) => writer.discard().await,
            ExportSink::Native(save) => save.discard().await,
        }
    }
}

/// [`export_selected`] for an export too long to hold in memory: each job is
/// written to a file a piece at a time. In the browser the file is in the
/// browser's storage and is downloaded once it is whole. In the desktop app
/// it is the file the user picks in a "Save As" dialog, asked before the
/// job's audio is processed.
async fn export_to_files(
    state: AppState,
    file: &LoadedFile,
    views: &[ChannelView],
    jobs: &[ExportJob],
    bits: u16,
    is_float: bool,
    normalize: bool,
) {
    for stored in STORED_EXPORTS.with(|s| std::mem::take(&mut *s.borrow_mut())) {
        crate::opfs_audio::delete(&stored).await;
    }
    let n_jobs = jobs.len() as f64;
    let mut downloads: Vec<(String, web_sys::File)> = Vec::new();
    let mut saved: Vec<String> = Vec::new();
    for (i, job) in jobs.iter().enumerate() {
        let name = wav_file_name(file, views, job);
        let result = export_job_to_file(
            &state,
            file,
            views,
            job,
            bits,
            is_float,
            normalize,
            &name,
            |done| {
                let progress = (i as f64 + done) / n_jobs;
                state.export.progress().set(Some(progress));
                state.export.status().set(Some(format!(
                    "Processing audio... {}%",
                    (progress * 100.0) as u32
                )));
                !state.export.cancel().get_untracked()
            },
        )
        .await;
        match result {
            Ok(ExportedFile::Browser(out, stored)) => {
                STORED_EXPORTS.with(|s| s.borrow_mut().push(stored));
                downloads.push((name, out));
            }
            Ok(ExportedFile::Native(path)) => saved.push(path),
            // A cancel, or a "Save As" dialog that was closed: the files
            // written so far are kept.
            Err(None) => break,
            Err(Some(message)) => {
                show_export_failed(&state, &message);
                return;
            }
        }
    }
    state.export.progress().set(None);
    state.export.status().set(None);
    for (name, out) in &downloads {
        trigger_blob_download(out, name);
    }
    match saved.as_slice() {
        [] => {}
        [path] => state.show_info_toast(format!("Exported: {}", short_path(path))),
        paths => state.show_info_toast(format!("Exported {}/{} files", paths.len(), jobs.len())),
    }
}

/// Processes one job and writes it as a WAV with GUANO metadata to a file.
/// With `normalize` the job is processed twice: once for its peak, once to
/// write it. `on_progress` gets the part done, 0 to 1, and returns false to
/// cancel.
#[allow(clippy::too_many_arguments)]
async fn export_job_to_file(
    state: &AppState,
    file: &LoadedFile,
    views: &[ChannelView],
    job: &ExportJob,
    bits: u16,
    is_float: bool,
    normalize: bool,
    name: &str,
    mut on_progress: impl FnMut(f64) -> bool,
) -> Result<ExportedFile, FileExportError> {
    use crate::audio::wav::{header_bytes, WavWriteFormat};
    let source = file.audio.source.as_ref();
    let rate = file.audio.sample_rate;
    let region = || RegionProcessor::new(source, views, rate, job.start, job.end, &job.params);
    let fmt = WavWriteFormat::for_f32_samples(
        export_output_rate(rate, &job.params),
        views.len().max(1) as u16,
        bits,
        is_float,
    );
    // Every mode gives one output sample for each source sample, so the
    // file's sizes are known before any audio is processed and its header
    // can be written first.
    let frames = region().frames();
    let data_bytes = frames as u64 * u64::from(fmt.block_align());
    let guano = build_export_guano(
        fmt.sample_rate,
        frames,
        &job.params,
        &file.name,
        file.audio.metadata.guano.as_ref(),
    );
    let after = chunks_after_data(data_bytes, &guano.to_text());
    let header = header_bytes(&fmt, data_bytes, after.len() as u64);

    // Asked first, so the desktop app's dialog comes straight after the click.
    let mut sink = ExportSink::open(
        state,
        name,
        header.len() as u64 + data_bytes + after.len() as u64,
    )
    .await?;

    let wrote = async {
        let passes = if normalize { 2.0 } else { 1.0 };
        let mut factor = 1.0f32;
        if normalize {
            let mut measured = region();
            let mut peak = 0.0f32;
            while let Some(piece) = measured.next_piece().await.map_err(region_error)? {
                peak = piece.iter().flatten().fold(peak, |p, s| p.max(s.abs()));
                if !on_progress(measured.progress() / passes) {
                    return Err(None);
                }
                crate::canvas::tile_cache::yield_to_browser().await;
            }
            if peak > 0.0 {
                factor = 10f64.powf(NORMALIZE_DBFS / 20.0) as f32 / peak;
            }
        }
        sink.write(&header).await?;
        write_samples(&mut sink, region(), &fmt, frames, factor, |done| {
            on_progress((passes - 1.0 + done) / passes)
        })
        .await?;
        sink.write(&after).await
    }
    .await;
    match wrote {
        Ok(()) => sink.finish().await,
        Err(e) => {
            sink.discard().await;
            Err(e)
        }
    }
}

/// Writes `region`'s audio, scaled by `factor`, as exactly `frames` frames
/// of `fmt` samples, the number the file's header says: audio past that is
/// left out, and silence makes up any shortfall.
async fn write_samples(
    sink: &mut ExportSink,
    mut region: RegionProcessor<'_>,
    fmt: &crate::audio::wav::WavWriteFormat,
    frames: usize,
    factor: f32,
    mut on_progress: impl FnMut(f64) -> bool,
) -> Result<(), FileExportError> {
    use crate::audio::wav::encode_f32_samples;
    let mut left = frames;
    while let Some(mut piece) = region.next_piece().await.map_err(region_error)? {
        if factor != 1.0 {
            apply_gain_linear(&mut piece, factor);
        }
        piece.iter_mut().for_each(|ch| ch.truncate(left));
        left -= piece.first().map_or(0, Vec::len);
        sink.write(&encode_f32_samples(&interleave(piece), fmt))
            .await?;
        if !on_progress(region.progress()) {
            return Err(None);
        }
        crate::canvas::tile_cache::yield_to_browser().await;
    }
    if left > 0 {
        let silence = vec![0.0f32; left * usize::from(fmt.channels)];
        sink.write(&encode_f32_samples(&silence, fmt)).await?;
    }
    Ok(())
}

/// What follows `data_bytes` of samples in an exported WAV: the data
/// chunk's pad byte when its length is odd, then the GUANO `guan` chunk.
fn chunks_after_data(data_bytes: u64, guano_text: &str) -> Vec<u8> {
    let text = guano_text.as_bytes();
    let mut out = Vec::with_capacity(text.len() + 10);
    if data_bytes % 2 == 1 {
        out.push(0);
    }
    out.extend_from_slice(b"guan");
    out.extend_from_slice(&(text.len() as u32).to_le_bytes());
    out.extend_from_slice(text);
    if text.len() % 2 == 1 {
        out.push(0);
    }
    out
}

/// The message for a long export that the browser's storage has no room for.
fn no_space_text(needed_bytes: u64, free_bytes: u64) -> String {
    format!(
        "not enough browser storage. The export needs {} MB. Free: {} MB.",
        needed_bytes.div_ceil(1_000_000),
        free_bytes / 1_000_000
    )
}

/// Have the browser download `blob` (a file in its storage) as `filename`,
/// without reading it into memory.
fn trigger_blob_download(blob: &web_sys::Blob, filename: &str) {
    let Ok(url) = web_sys::Url::create_object_url_with_blob(blob) else {
        log::error!("Failed to create object URL");
        return;
    };
    let document = web_sys::window().unwrap().document().unwrap();
    let a: web_sys::HtmlAnchorElement = document.create_element("a").unwrap().dyn_into().unwrap();
    a.set_href(&url);
    a.set_download(filename);
    a.set_attribute("style", "display:none").ok();
    document.body().unwrap().append_child(&a).ok();
    a.click();
    document.body().unwrap().remove_child(&a).ok();
    // The browser reads the file after the click, so the URL is kept a while.
    leptos::task::spawn_local(async move {
        crate::web_util::sleep_ms(120_000).await;
        web_sys::Url::revoke_object_url(&url).ok();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(bits_per_sample: u16, is_float: bool) -> FileMetadata {
        FileMetadata {
            bits_per_sample,
            is_float,
            ..Default::default()
        }
    }

    #[test]
    fn wav_export_depth_choices() {
        use WavExportDepth::*;
        assert_eq!(wav_export_format(SameAsFile, &meta(24, false)), (24, false));
        assert_eq!(wav_export_format(SameAsFile, &meta(32, true)), (32, true));
        // 12-bit and lossy files are written as 16-bit.
        assert_eq!(wav_export_format(SameAsFile, &meta(12, false)), (16, false));
        assert_eq!(
            wav_export_format(SameAsFile, &meta(FileMetadata::LOSSY_BITS, false)),
            (16, false)
        );
        assert_eq!(wav_export_format(Pcm16, &meta(24, false)), (16, false));
        assert_eq!(wav_export_format(Float32, &meta(16, false)), (32, true));
    }

    #[test]
    fn export_gain_follows_the_gain_mode() {
        use GainMode::*;
        assert_eq!(fixed_gain_db(Off, 6.0, 9.0), 0.0);
        assert_eq!(fixed_gain_db(Manual, 6.0, 9.0), 6.0);
        assert_eq!(fixed_gain_db(Adaptive, 6.0, 9.0), 6.0);
        assert_eq!(fixed_gain_db(AutoPeak, 6.0, 9.0), 15.0);
    }

    #[test]
    fn normalize_scales_all_channels_to_the_highest_peak() {
        let mut channels = vec![vec![0.1, -0.5], vec![0.25, 0.0]];
        normalize_peak(&mut channels, -6.0);
        let target = 10f32.powf(-6.0 / 20.0);
        assert!((channels[0][1] + target).abs() < 1e-6);
        assert!((channels[1][0] - target / 2.0).abs() < 1e-6);
        let mut silent = vec![vec![0.0; 4]];
        normalize_peak(&mut silent, -3.0);
        assert_eq!(silent, vec![vec![0.0; 4]]);
    }

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

    /// HET at 40 kHz turns a 48 kHz tone into 8 kHz; "Then pitch shift" ÷2
    /// brings that to 4 kHz, and the file name says so.
    #[test]
    fn het_output_can_be_pitch_shifted() {
        use crate::audio::streaming_playback::apply_dsp_mode;
        let rate = 192_000u32;
        let tone: Vec<f32> = (0..rate as usize / 2)
            .map(|i| (2.0 * std::f32::consts::PI * 48_000.0 * i as f32 / rate as f32).sin() * 0.5)
            .collect();
        let heard_hz = |post: f64| {
            let params = PlaybackParams {
                mode: PlaybackMode::Heterodyne,
                het_ps_factor: post,
                ..ps_params()
            };
            let out = apply_dsp_mode(&tone, rate, &params);
            // Frequency from the zero crossings of the middle half.
            let mid = &out[out.len() / 4..out.len() * 3 / 4];
            let crossings = mid
                .windows(2)
                .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
                .count();
            crossings as f64 / 2.0 / (mid.len() as f64 / rate as f64)
        };
        assert!((heard_hz(1.0) - 8_000.0).abs() < 200.0, "{}", heard_hz(1.0));
        assert!((heard_hz(2.0) - 4_000.0).abs() < 200.0, "{}", heard_hz(2.0));

        let named = |post: f64| {
            transform_suffix(&PlaybackParams {
                mode: PlaybackMode::Heterodyne,
                het_ps_factor: post,
                ..ps_params()
            })
        };
        assert_eq!(named(1.0), "_HET40k");
        assert_eq!(named(2.0), "_HET40k-PS2");
        assert_eq!(named(2.5), "_HET40k-PS2.5");
    }

    /// Playback parameters for pitch shift down by 10 with nothing else on.
    fn ps_params() -> PlaybackParams {
        use crate::dsp::pitch_shift::PitchFactor;
        PlaybackParams {
            mode: PlaybackMode::PitchShift,
            het_freq: 40_000.0,
            het_cutoff: 15_000.0,
            het_comb_count: 1,
            het_comb_spacing: 0.0,
            het_ps_factor: 1.0,
            te_factor: 10.0,
            ps_factor: PitchFactor::down(10.0),
            ps_shift_hz: 0.0,
            zc_factor: 8.0,
            zc_lowest_bat_hz: crate::dsp::ultrasonic::DEFAULT_LOWEST_BAT_HZ,
            gain_db: 0.0,
            gain_mode: GainMode::Off,
            auto_peak_gain_db: 0.0,
            live_gain_db: 0.0,
            filter_enabled: false,
            filter_freq_low: 0.0,
            filter_freq_high: 128_000.0,
            filter_db_below: 0.0,
            filter_db_selected: 0.0,
            filter_db_harmonics: 0.0,
            filter_db_above: 0.0,
            filter_band_mode: Default::default(),
            filter_quality: crate::state::FilterQuality::Fast,
            sel_freq_low: 0.0,
            sel_freq_high: 0.0,
            has_selection: false,
            notch_enabled: false,
            notch_bands: Vec::new(),
            noise_reduce_enabled: false,
            noise_reduce_strength: 0.0,
            noise_reduce_floor: None,
        }
    }

    fn mono_source(samples: Vec<f32>, sample_rate: u32) -> crate::audio::source::InMemorySource {
        crate::audio::source::InMemorySource {
            samples: std::sync::Arc::new(samples),
            raw_samples: None,
            sample_rate,
            channels: 1,
        }
    }

    /// The lowest level of `out` near each chunk join, in dB relative to its
    /// level over the whole signal: RMS over 128-sample blocks from 1024
    /// samples before each join to 1024 after it.
    fn worst_dip_at_joins_db(out: &[f32]) -> f32 {
        let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
        let whole = rms(&out[8192..out.len() - 8192]);
        let mut worst = f32::MAX;
        let mut join = CHUNK_SAMPLES;
        while join + 1024 < out.len() {
            for block in out[join - 1024..join + 1024].chunks_exact(128) {
                worst = worst.min(rms(block));
            }
            join += CHUNK_SAMPLES;
        }
        20.0 * (worst / whole).log10()
    }

    /// Playback and export shift 96 000-sample chunks one at a time and
    /// crossfade them over one grain. A steady tone that runs across the
    /// joins is the hardest case: each chunk lines its grains up on its own.
    /// The level at a join stays within 3 dB of the rest, at every tone
    /// tried, and the output has the input's length.
    #[test]
    fn pitch_shift_chunks_join_without_a_gap() {
        let rate = 256_000u32;
        let len = CHUNK_SAMPLES * 3 + 20_000;
        let params = ps_params();
        let mut worst_of_all = 0.0f32;
        for khz in [20.0f32, 33.7, 40.0, 47.3, 61.1, 80.0, 95.9] {
            let samples: Vec<f32> = (0..len)
                .map(|i| {
                    0.5 * (std::f32::consts::TAU * khz * 1000.0 * i as f32 / rate as f32).sin()
                })
                .collect();
            let source = mono_source(samples, rate);
            let out = process_region_view(
                &source,
                ChannelView::MonoMix,
                rate,
                0.0,
                len as f64 / rate as f64,
                &params,
            );
            assert_eq!(out.len(), len);
            let dip = worst_dip_at_joins_db(&out);
            println!("{khz} kHz tone at x10: worst level at a chunk join {dip:.2} dB");
            worst_of_all = worst_of_all.min(dip);
        }
        assert!(worst_of_all > -3.0, "worst dip {worst_of_all:.2} dB");
    }

    /// Writes what pitch shift at x10 gives through export's chunks, for
    /// listening: `PS_WAV_DIR=<folder> cargo test -p oversample --lib
    /// pitch_shift_chunked_wav -- --ignored`. The signal is 5 ms sweeps from
    /// 90 to 30 kHz every 60 ms, and a 40 kHz tone from 0.30 to 0.50 s that
    /// crosses the chunk join at 0.375 s. The WAV is at 51.2 kHz.
    #[test]
    #[ignore]
    fn pitch_shift_chunked_wav() {
        let Ok(dir) = std::env::var("PS_WAV_DIR") else {
            return;
        };
        let rate = 256_000u32;
        let len = rate as usize * 3;
        let (period, call) = (15_360usize, 1_280usize);
        let mut phase = 0.0f64;
        let samples: Vec<f32> = (0..len)
            .map(|i| {
                let k = i % period;
                let mut s = 0.0f64;
                if k < call {
                    let u = k as f64 / call as f64;
                    phase +=
                        std::f64::consts::TAU * 90_000.0 * (30.0f64 / 90.0).powf(u) / rate as f64;
                    s += 0.5 * (std::f64::consts::PI * u).sin() * phase.sin();
                } else {
                    phase = 0.0;
                }
                let t = i as f64 / rate as f64;
                if (0.30..0.50).contains(&t) {
                    s += 0.25 * (std::f64::consts::TAU * 40_000.0 * t).sin();
                }
                s as f32
            })
            .collect();
        let source = mono_source(samples, rate);
        let out = process_region_view(&source, ChannelView::MonoMix, rate, 0.0, 3.0, &ps_params());
        let out = crate::dsp::filters::decimate(&out, rate, 51_200);
        let out_rate = crate::dsp::filters::decimated_rate(rate, 51_200);
        let pcm: Vec<u8> = out
            .iter()
            .flat_map(|s| ((s.clamp(-1.0, 1.0) * 32767.0).round() as i16).to_le_bytes())
            .collect();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + pcm.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&out_rate.to_le_bytes());
        wav.extend_from_slice(&(out_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(pcm.len() as u32).to_le_bytes());
        wav.extend_from_slice(&pcm);
        let path = std::path::Path::new(&dir).join("ps-after-x10-chunked-as-played.wav");
        std::fs::write(&path, wav).unwrap();
        println!("wrote {}", path.display());
    }

    #[test]
    fn export_length_limit() {
        // 192 kHz stereo: 512 MB of float samples is 349.5 s.
        assert_eq!(too_long(192_000 * 300, 2, 192_000), None);
        let t = too_long(192_000 * 992, 2, 192_000).unwrap();
        assert!((t.secs - 992.0).abs() < 1e-6);
        assert!((t.max_secs - 349.5).abs() < 0.1, "{}", t.max_secs);
        // Time expansion x10 writes the same samples at a tenth of the rate.
        let te = too_long(192_000 * 992, 2, 19_200).unwrap();
        assert!((te.secs - 9920.0).abs() < 1e-6);
    }

    /// A streamed export's tail matches what the in-memory export appends.
    #[test]
    fn chunks_after_data_match_append_guano_chunk() {
        for (data_len, text) in [(8usize, "GUANO|Version: 1.0\n"), (7, "odd"), (6, "even")] {
            let fmt = crate::audio::wav::WavWriteFormat::for_f32_samples(48_000, 1, 16, false);
            let data = vec![1u8; data_len];
            let mut whole = crate::audio::wav::header_bytes(&fmt, data_len as u64, 0);
            whole.extend_from_slice(&data);
            crate::audio::guano::append_guano_chunk(&mut whole, text);

            let after = chunks_after_data(data_len as u64, text);
            let mut streamed =
                crate::audio::wav::header_bytes(&fmt, data_len as u64, after.len() as u64);
            streamed.extend_from_slice(&data);
            streamed.extend_from_slice(&after);
            assert_eq!(streamed, whole, "{data_len} bytes of data, {text:?}");
        }
    }

    #[test]
    fn interleave_stereo() {
        let ch = vec![vec![1.0, 2.0], vec![-1.0, -2.0]];
        assert_eq!(interleave(ch), vec![1.0, -1.0, 2.0, -2.0]);
    }
}
