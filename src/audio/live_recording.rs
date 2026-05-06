use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use crate::state::{AppState, FileSettings, LoadedFile};
use crate::audio::source::InMemorySource;
use crate::audio::mic_backend::{with_live_samples, with_live_samples_mut};
use crate::audio::wav_encoder::try_tauri_save;
use crate::types::{AudioData, FileMetadata, SpectrogramData};
use crate::dsp::fft::{compute_preview, compute_spectrogram_partial, compute_stft_columns};
use crate::dsp::resonators::{compute_resonator_columns, warmup_samples};
use crate::state::MainView;
use std::sync::Arc;

/// FFT and hop sizes for live waterfall rendering.
/// FFT=1024 gives 513 frequency bins for good resolution; hop=256 for smooth scrolling.
const LIVE_FFT: usize = 1024;
const LIVE_HOP: usize = 256;

/// Clean up the live recording file when finalization fails (empty samples,
/// command error, etc.).  If the file has no audio data and no preview,
/// removes it entirely and fixes `current_file_index`.  Otherwise marks it
/// as not-recording so the overview doesn't say "Recording…" forever.
pub(crate) fn cleanup_failed_recording(state: &AppState) {
    let live_idx = state.mic_live_file_idx.get_untracked();
    state.mic_live_file_idx.set(None);

    let Some(idx) = live_idx else { return };

    let is_empty = state.files.with_untracked(|files| {
        files.get(idx).map_or(true, |f| f.audio.samples.is_empty() && f.preview.is_none())
    });

    if is_empty {
        // Remove the phantom live file
        state.files.update(|files| {
            if idx < files.len() {
                files.remove(idx);
            }
        });
        // Adjust current_file_index after removal
        let len = state.files.with_untracked(|f| f.len());
        match state.current_file_index.get_untracked() {
            Some(ci) if ci == idx => {
                state.current_file_index.set(if len > 0 { Some(idx.min(len - 1)) } else { None });
            }
            Some(ci) if ci > idx => {
                state.current_file_index.set(Some(ci - 1));
            }
            _ => {}
        }
    } else {
        // File has partial data — keep it but stop the recording indicator
        state.files.update(|files| {
            if let Some(f) = files.get_mut(idx) {
                f.is_recording = false;
            }
        });
    }
}

/// Create a live LoadedFile at recording start for real-time visualization.
/// Returns the file index where the live file was inserted.
pub(crate) fn start_live_recording(state: &AppState, sample_rate: u32) -> usize {
    let now = js_sys::Date::new_0();
    let name = format!(
        "batcap_{:04}{:02}{:02}_{:02}{:02}{:02}.wav",
        now.get_full_year(),
        now.get_month() + 1,
        now.get_date(),
        now.get_hours(),
        now.get_minutes(),
        now.get_seconds(),
    );

    let samples: Arc<Vec<f32>> = Arc::new(Vec::new());
    let source = Arc::new(InMemorySource {
        samples: samples.clone(),
        raw_samples: None,
        sample_rate,
        channels: 1,
    });
    let audio = AudioData {
        samples,
        source,
        sample_rate,
        channels: 1,
        duration_secs: 0.0,
        metadata: FileMetadata {
            file_size: 0,
            format: "REC",
            bits_per_sample: state.mic_bits_per_sample.get_untracked(),
            is_float: false,
            guano: None,
            data_offset: None,
            data_size: None,
        },
    };

    let placeholder_spec = SpectrogramData {
        columns: Arc::new(Vec::new()),
        total_columns: 0,
        freq_resolution: sample_rate as f64 / LIVE_FFT as f64,
        time_resolution: LIVE_HOP as f64 / sample_rate as f64,
        max_freq: sample_rate as f64 / 2.0,
        sample_rate,
    };

    let mut file_index = 0;
    state.files.update(|files| {
        file_index = files.len();
        files.push(LoadedFile {
            name,
            audio,
            spectrogram: placeholder_spec,
            preview: None,
            overview_image: None,
            xc_metadata: None,
            xc_hashes: None,
            is_demo: false,
            is_recording: true,
            is_live_listen: false,
            settings: FileSettings::default(),
            add_order: file_index,
            last_modified_ms: None,
            identity: None,
            file_handle: None,
            cached_peak_db: None,
            cached_full_peak_db: None,
            read_only: false,
            had_sidecar: false,
            verify_outcome: crate::state::VerifyOutcome::Pending,
            all_hashes_verified: false,
            wav_markers: Vec::new(),
            loading_id: None,
            min_display_freq: None,
            max_display_freq: None,
        });
    });

    state.current_file_index.set(Some(file_index));
    state.mic_live_file_idx.set(Some(file_index));

    // Set zoom for comfortable live recording scroll speed.
    // Use hop=256 to match the actual hop size in spawn_live_processing_loop.
    let canvas_w = state.spectrogram_canvas_width.get_untracked();
    let live_time_res = LIVE_HOP as f64 / sample_rate as f64;
    state.zoom_level.set(crate::viewport::recording_zoom(canvas_w, live_time_res));
    state.scroll_offset.set(0.0);

    file_index
}

/// Create an empty "armed" live document — mic is open and ready, but no
/// streaming has started yet. Lets the user configure HFR mode/range/bandpass
/// before they press Listen or Record. The file persists in the list until
/// closed, listened-to, or recorded-into. Returns the file index.
pub(crate) fn start_live_armed(state: &AppState, sample_rate: u32) -> usize {
    let now = js_sys::Date::new_0();
    let name = format!(
        "Live mic ({:02}:{:02}:{:02})",
        now.get_hours(),
        now.get_minutes(),
        now.get_seconds(),
    );

    let samples: Arc<Vec<f32>> = Arc::new(Vec::new());
    let source = Arc::new(InMemorySource {
        samples: samples.clone(),
        raw_samples: None,
        sample_rate,
        channels: 1,
    });
    let audio = AudioData {
        samples,
        source,
        sample_rate,
        channels: 1,
        duration_secs: 0.0,
        metadata: FileMetadata {
            file_size: 0,
            format: "MIC",
            bits_per_sample: state.mic_bits_per_sample.get_untracked(),
            is_float: false,
            guano: None,
            data_offset: None,
            data_size: None,
        },
    };

    let placeholder_spec = SpectrogramData {
        columns: Arc::new(Vec::new()),
        total_columns: 0,
        freq_resolution: sample_rate as f64 / LIVE_FFT as f64,
        time_resolution: LIVE_HOP as f64 / sample_rate as f64,
        max_freq: sample_rate as f64 / 2.0,
        sample_rate,
    };

    let mut file_index = 0;
    state.files.update(|files| {
        file_index = files.len();
        files.push(LoadedFile {
            name,
            audio,
            spectrogram: placeholder_spec,
            preview: None,
            overview_image: None,
            xc_metadata: None,
            xc_hashes: None,
            is_demo: false,
            is_recording: false,
            is_live_listen: false,
            settings: FileSettings::default(),
            add_order: file_index,
            last_modified_ms: None,
            identity: None,
            file_handle: None,
            cached_peak_db: None,
            cached_full_peak_db: None,
            read_only: false,
            had_sidecar: false,
            verify_outcome: crate::state::VerifyOutcome::Pending,
            all_hashes_verified: false,
            wav_markers: Vec::new(),
            loading_id: None,
            min_display_freq: None,
            max_display_freq: None,
        });
    });

    state.current_file_index.set(Some(file_index));
    state.mic_live_file_idx.set(Some(file_index));
    // Reset display so the gutter immediately picks up the mic Nyquist.
    state.min_display_freq.set(None);
    state.max_display_freq.set(None);
    // Pre-set the live recording zoom + scroll origin so the user sees the
    // same viewport they'll get once audio actually starts streaming.
    set_live_recording_zoom(state, sample_rate);

    file_index
}

/// Apply the standard live-recording zoom (~recording_zoom) and reset
/// scroll_offset. Shared by start_live_recording, start_live_armed, and the
/// armed→record promotion path.
pub(crate) fn set_live_recording_zoom(state: &AppState, sample_rate: u32) {
    let canvas_w = state.spectrogram_canvas_width.get_untracked();
    let live_time_res = LIVE_HOP as f64 / sample_rate as f64;
    state.zoom_level.set(crate::viewport::recording_zoom(canvas_w, live_time_res));
    state.scroll_offset.set(0.0);
}

/// True when the file at `idx` looks like an armed-but-empty live doc — no
/// samples written, neither listening nor recording. Used to decide whether
/// Listen/Record should reuse it instead of creating a new file.
pub(crate) fn is_armed_live_doc(state: &AppState, idx: usize) -> bool {
    state.files.with_untracked(|files| {
        files.get(idx).map_or(false, |f| {
            !f.is_recording && !f.is_live_listen && f.audio.samples.is_empty()
        })
    })
}

/// Promote an armed live doc to a listening file. Sets the flags the live
/// processing loop and waveform overview key off without altering the file's
/// position or name. Use only on a file that satisfies `is_armed_live_doc`.
pub(crate) fn promote_armed_to_listening(state: &AppState, idx: usize) {
    state.files.update(|files| {
        if let Some(f) = files.get_mut(idx) {
            f.is_live_listen = true;
            // Reuse the recording display path for the live waveform/overview,
            // matching what start_live_listening sets.
            f.is_recording = true;
        }
    });
    state.current_file_index.set(Some(idx));
    let sr = state.files.with_untracked(|files| {
        files.get(idx).map(|f| f.audio.sample_rate).unwrap_or(48_000)
    });
    set_live_recording_zoom(state, sr);
}

/// Promote an armed live doc to a recording file. Renames it to the standard
/// recording filename and resets metadata so the recovery sidecar/wav-part
/// uses the right filename. Use only on a file that satisfies `is_armed_live_doc`.
///
/// MUST be called BEFORE `mic_backend::start_recording` so the recovery
/// sidecar/.wav.part filename (built by `build_start_recording_args` from the
/// file at `mic_live_file_idx`) picks up the new name.
pub(crate) fn promote_armed_to_recording(state: &AppState, idx: usize) {
    let now = js_sys::Date::new_0();
    let new_name = format!(
        "batcap_{:04}{:02}{:02}_{:02}{:02}{:02}.wav",
        now.get_full_year(),
        now.get_month() + 1,
        now.get_date(),
        now.get_hours(),
        now.get_minutes(),
        now.get_seconds(),
    );
    state.files.update(|files| {
        if let Some(f) = files.get_mut(idx) {
            f.name = new_name;
            f.is_recording = true;
            f.is_live_listen = false;
            f.audio.metadata.format = "REC";
        }
    });
    state.current_file_index.set(Some(idx));
}

/// Roll back a recording-promoted file to its armed state. Used when
/// `start_recording` fails after we've already renamed the armed file.
pub(crate) fn revert_recording_to_armed(state: &AppState, idx: usize) {
    let now = js_sys::Date::new_0();
    let armed_name = format!(
        "Live mic ({:02}:{:02}:{:02})",
        now.get_hours(),
        now.get_minutes(),
        now.get_seconds(),
    );
    state.files.update(|files| {
        if let Some(f) = files.get_mut(idx) {
            f.name = armed_name;
            f.is_recording = false;
            f.is_live_listen = false;
            f.audio.metadata.format = "MIC";
        }
    });
}

/// Create a transient listening file — appears in the file list, shows the
/// waterfall / waveform, and is auto-removed when listening stops.
/// Returns the file index.
pub(crate) fn start_live_listening(state: &AppState, sample_rate: u32) -> usize {
    let samples: Arc<Vec<f32>> = Arc::new(Vec::new());
    let source = Arc::new(InMemorySource {
        samples: samples.clone(),
        raw_samples: None,
        sample_rate,
        channels: 1,
    });
    let audio = AudioData {
        samples,
        source,
        sample_rate,
        channels: 1,
        duration_secs: 0.0,
        metadata: FileMetadata {
            file_size: 0,
            format: "MIC",
            bits_per_sample: 16,
            is_float: false,
            guano: None,
            data_offset: None,
            data_size: None,
        },
    };

    let placeholder_spec = SpectrogramData {
        columns: Arc::new(Vec::new()),
        total_columns: 0,
        freq_resolution: sample_rate as f64 / LIVE_FFT as f64,
        time_resolution: LIVE_HOP as f64 / sample_rate as f64,
        max_freq: sample_rate as f64 / 2.0,
        sample_rate,
    };

    let mut file_index = 0;
    state.files.update(|files| {
        file_index = files.len();
        files.push(LoadedFile {
            name: "Listening".to_string(),
            audio,
            spectrogram: placeholder_spec,
            preview: None,
            overview_image: None,
            xc_metadata: None,
            xc_hashes: None,
            is_demo: false,
            is_recording: true, // reuse recording display path for waveform/overview
            is_live_listen: true,
            settings: FileSettings::default(),
            add_order: file_index,
            last_modified_ms: None,
            identity: None,
            file_handle: None,
            cached_peak_db: None,
            cached_full_peak_db: None,
            read_only: false,
            had_sidecar: false,
            verify_outcome: crate::state::VerifyOutcome::Pending,
            all_hashes_verified: false,
            wav_markers: Vec::new(),
            loading_id: None,
            min_display_freq: None,
            max_display_freq: None,
        });
    });

    state.current_file_index.set(Some(file_index));
    state.mic_live_file_idx.set(Some(file_index));

    // Set zoom for comfortable waterfall viewing.
    // Use LIVE_HOP to match the actual hop size in spawn_live_processing_loop.
    let canvas_w = state.spectrogram_canvas_width.get_untracked();
    let live_time_res = LIVE_HOP as f64 / sample_rate as f64;
    state.zoom_level.set(crate::viewport::recording_zoom(canvas_w, live_time_res));
    state.scroll_offset.set(0.0);

    file_index
}

/// Remove the transient listening file and fix indices.
pub(crate) fn cleanup_listen_file(state: &AppState) {
    let live_idx = state.mic_live_file_idx.get_untracked();
    state.mic_live_file_idx.set(None);

    let Some(idx) = live_idx else { return };

    let is_listen = state.files.with_untracked(|files| {
        files.get(idx).map_or(false, |f| f.is_live_listen)
    });
    if !is_listen { return; }

    state.files.update(|files| {
        if idx < files.len() {
            files.remove(idx);
        }
    });

    // Fix current_file_index after removal
    let len = state.files.with_untracked(|f| f.len());
    match state.current_file_index.get_untracked() {
        Some(ci) if ci == idx => {
            state.current_file_index.set(if len > 0 { Some(idx.min(len - 1)) } else { None });
        }
        Some(ci) if ci > idx => {
            state.current_file_index.set(Some(ci - 1));
        }
        _ => {}
    }
}

/// Convert the listening file into a recording file (listen → record transition).
/// Returns the existing file index.  Does NOT clear the audio buffer so the
/// last ≤10 s of listened audio becomes pre-roll in the recording.
pub(crate) fn convert_listen_to_recording(state: &AppState, sample_rate: u32) -> usize {
    let file_index = state.mic_live_file_idx.get_untracked()
        .expect("convert_listen_to_recording: no live file");

    // When pre-roll is active, backdate the filename to reflect the actual
    // start of audio data (i.e. the beginning of the pre-roll buffer).
    let preroll = state.mic_preroll_samples.get_untracked();
    let preroll_ms = if preroll > 0 && sample_rate > 0 {
        (preroll as f64 / sample_rate as f64) * 1000.0
    } else {
        0.0
    };
    let ts = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(
        js_sys::Date::now() - preroll_ms,
    ));
    let name = format!(
        "batcap_{:04}{:02}{:02}_{:02}{:02}{:02}.wav",
        ts.get_full_year(),
        ts.get_month() + 1,
        ts.get_date(),
        ts.get_hours(),
        ts.get_minutes(),
        ts.get_seconds(),
    );

    state.files.update(|files| {
        if let Some(f) = files.get_mut(file_index) {
            f.name = name;
            f.is_live_listen = false;
            f.audio.metadata.format = "REC";
            f.audio.metadata.bits_per_sample = state.mic_bits_per_sample.get_untracked();
        }
    });

    state.current_file_index.set(Some(file_index));

    // Don't reset scroll_offset — the smooth scroll animation is already
    // running from the listen phase and will keep the view at the right edge.
    // Resetting to 0 causes a visible jump to the beginning.

    file_index
}

/// Spawns an async processing loop that incrementally computes STFT columns
/// and pushes them to the live waterfall for direct canvas rendering.
/// No tile cache or spectral store is used — the waterfall renders directly.
pub(crate) fn spawn_live_processing_loop(state: AppState, file_index: usize, sample_rate: u32) {
    use crate::canvas::live_waterfall;

    let (fft_size, hop_size) = (LIVE_FFT, LIVE_HOP);
    const PROCESS_INTERVAL_MS: i32 = 50;

    // Bump the generation counter so any previous processing loop will exit.
    let gen = state.mic_processing_gen.get_untracked().wrapping_add(1);
    state.mic_processing_gen.set(gen);

    // Initialize waterfall synchronously so the renderer sees it immediately
    // (before any async yield that could allow a spectrogram draw)
    live_waterfall::create(fft_size, hop_size, sample_rate);

    wasm_bindgen_futures::spawn_local(async move {
        let mut last_processed_col: usize = 0;
        let mut last_snapshot_len: usize = 0;
        let is_tauri = state.is_tauri;

        loop {
            let p = js_sys::Promise::new(&mut |resolve, _| {
                if let Some(w) = web_sys::window() {
                    let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(
                        &resolve, PROCESS_INTERVAL_MS,
                    );
                }
            });
            let _ = JsFuture::from(p).await;

            // A newer processing loop has started — this one is stale.
            if state.mic_processing_gen.get_untracked() != gen {
                log::info!("Processing loop superseded (gen {} vs {}), exiting",
                    gen, state.mic_processing_gen.get_untracked());
                break;
            }

            // Check if still recording/listening
            let is_recording = state.mic_recording.get_untracked();
            let is_listening = state.mic_listening.get_untracked();
            if !is_recording && !is_listening {
                break;
            }
            // Check file still valid
            if state.mic_live_file_idx.get_untracked() != Some(file_index) {
                break;
            }

            // Compute new FFT columns from the live buffer
            let (any_update, peak_normalized) = with_live_samples(is_tauri, |samples| -> (bool, f32) {
                if samples.len() < fft_size {
                    return (false, 0.0);
                }

                let total_possible_cols = (samples.len() - fft_size) / hop_size + 1;
                if total_possible_cols <= last_processed_col {
                    return (false, 0.0);
                }

                let new_col_count = total_possible_cols - last_processed_col;

                // Compute new spectral columns using the currently selected view.
                // Resonators are stateful: each tick we slice the buffer tail
                // and run a short warmup prefix so the EMA converges before we
                // emit the genuinely-new columns. This keeps per-tick cost
                // bounded regardless of total recording length.
                let new_cols = if state.main_view.get_untracked() == MainView::Resonators {
                    let bandwidth_hz = state.resonator_bandwidth_hz.get_untracked().max(1.0);
                    let layout = state.resonator_layout.get_untracked();
                    let freq_range = state
                        .resonator_viewport_range
                        .get_untracked()
                        .map(|(lo, hi)| (lo as f32, hi as f32));
                    let warmup = warmup_samples(sample_rate, bandwidth_hz);
                    let warmup_cols = warmup.div_ceil(hop_size);
                    let skip_cols = warmup_cols.min(last_processed_col);
                    let slice_start_col = last_processed_col - skip_cols;
                    let slice_start_sample = slice_start_col * hop_size;
                    if slice_start_sample >= samples.len() {
                        Vec::new()
                    } else {
                        let tail = &samples[slice_start_sample..];
                        compute_resonator_columns(
                            tail,
                            sample_rate,
                            fft_size,
                            hop_size,
                            skip_cols,
                            new_col_count,
                            bandwidth_hz,
                            layout,
                            freq_range,
                        )
                    }
                } else {
                    compute_stft_columns(
                        samples,
                        sample_rate,
                        fft_size,
                        hop_size,
                        last_processed_col,
                        new_col_count,
                    )
                };

                if new_cols.is_empty() {
                    return (false, 0.0);
                }

                // Push to waterfall for direct rendering
                live_waterfall::push_columns(&new_cols);

                // Update file metadata (recording OR listening with a live file)
                let has_live_file = state.mic_live_file_idx.get_untracked() == Some(file_index);
                if has_live_file {
                    let duration = samples.len() as f64 / sample_rate as f64;
                    state.files.update(|files| {
                        if let Some(f) = files.get_mut(file_index) {
                            f.spectrogram.total_columns = total_possible_cols;
                            f.audio.duration_secs = duration;
                        }
                    });
                }

                // Periodically snapshot for waveform rendering.
                //
                // A fixed ~1s interval is O(N²) in total copy cost and blew the
                // Android heap for multi-minute high-SR recordings. Instead we
                // scale the interval with the current buffer length so each
                // snapshot does at most ~25% new work — amortized O(N) total,
                // while still giving frequent updates at the start.
                if has_live_file {
                    let base = (sample_rate as usize).max(44100);
                    let adaptive = (samples.len() / 4).max(base);
                    let do_snapshot = last_snapshot_len == 0
                        || samples.len().saturating_sub(last_snapshot_len) >= adaptive;
                    if do_snapshot {
                        let snapshot = Arc::new(samples.to_vec());
                        state.files.update(|files| {
                            if let Some(f) = files.get_mut(file_index) {
                                f.audio.samples = snapshot;
                            }
                        });
                        last_snapshot_len = samples.len();
                    }
                }

                // Compute peak of recent samples for VU meter
                let vu_start = samples.len().saturating_sub(2048);
                let peak = samples[vu_start..]
                    .iter()
                    .fold(0.0f32, |max, &s| max.max(s.abs()));
                let peak_db = if peak > 0.0 { 20.0 * peak.log10() } else { -96.0 };
                let normalized = ((peak_db + 60.0) / 60.0).clamp(0.0, 1.0);

                last_processed_col = total_possible_cols;
                (true, normalized)
            });

            // Trim the WASM-side circular buffer.
            //
            // We do this during listen AND during recording (whenever the
            // Tauri side is streaming to disk, which is the default). During
            // recording the WASM buffer is purely for the live waterfall + a
            // recent-samples waveform strip — the full recording lives on
            // disk via the .wav.part file. Keeping the full recording in
            // WASM would blow the heap for long / high-SR captures.
            //
            // The two exceptions that DO need the full buffer in WASM RAM:
            //   - Pre-roll recording: WASM re-encodes the WAV with the
            //     pre-roll samples + cue marker, so everything must survive
            //     until stop.
            //   - To-memory mode: user explicitly opted out of disk writes.
            let to_memory = state.record_mode.get_untracked() == crate::state::RecordMode::ToMemory;
            let preroll_active = state.mic_preroll_samples.get_untracked() > 0;
            let wasm_is_authoritative = to_memory || preroll_active || !is_tauri;
            let should_trim = any_update
                && (is_listening || (is_recording && !wasm_is_authoritative));
            if should_trim {
                with_live_samples_mut(is_tauri, |samples| {
                    // Keep an extra 2 s of headroom beyond the user-requested
                    // pre-roll duration. `toggle_record_with_preroll` subtracts
                    // the long-press gesture time (typically 400–700 ms, but
                    // up to a few seconds if the user holds) from the buffer
                    // length to work out where the "press moment" was — if the
                    // buffer were exactly the requested length, the subtracted
                    // gesture-time would eat into the actual pre-roll the
                    // user asked for. With headroom the cap below ends up at
                    // exactly the user's setting.
                    const GESTURE_HEADROOM_SECS: u32 = 2;
                    let buf_secs = state
                        .mic_preroll_buffer_secs
                        .get_untracked()
                        .max(2)
                        .saturating_add(GESTURE_HEADROOM_SECS) as usize;
                    let max_samples = (sample_rate as usize) * buf_secs;
                    if samples.len() > max_samples {
                        let trim = samples.len() - max_samples;
                        samples.drain(..trim);
                        let trimmed_cols = trim / hop_size;
                        last_processed_col = last_processed_col.saturating_sub(trimmed_cols);
                        last_snapshot_len = last_snapshot_len.saturating_sub(trim);
                    }
                });
            }

            if any_update {
                state.mic_peak_level.set(peak_normalized);
                let total_cols = live_waterfall::total_columns();
                state.mic_live_data_cols.set(total_cols);

                // Trigger spectrogram redraw
                state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));

                // Set target scroll for waterfall effect
                if total_cols > 0 {
                    let time_res = hop_size as f64 / sample_rate as f64;
                    let recording_time = total_cols as f64 * time_res;
                    let canvas_w = state.spectrogram_canvas_width.get_untracked();
                    let zoom = state.zoom_level.get_untracked();
                    if zoom > 0.0 && canvas_w > 0.0 {
                        let visible_cols = canvas_w / zoom;
                        let visible_time = visible_cols * time_res;
                        let target_scroll = (recording_time - visible_time).max(0.0);
                        state.mic_recording_target_scroll.set(target_scroll);
                    }
                }
            } else if is_recording && last_processed_col == 0 {
                // No audio chunks have arrived yet — update file duration from
                // wall-clock time so the overview can show elapsed recording time
                // instead of static "Recording…" text.
                if let Some(start) = state.mic_recording_start_time.get_untracked() {
                    let elapsed = (js_sys::Date::now() - start) / 1000.0;
                    state.files.update(|files| {
                        if let Some(f) = files.get_mut(file_index) {
                            f.audio.duration_secs = elapsed;
                        }
                    });
                }
            }
        }

        // Processing loop exited — clean up
        state.mic_peak_level.set(0.0);
        if !state.mic_recording.get_untracked() {
            // Only clear waterfall when fully done (not when switching from listen to record)
            live_waterfall::clear();
        }
        // Note: do NOT clear mic_live_file_idx here — finalize_recording() is
        // responsible for that. Clearing it here causes a race: this loop exits
        // as soon as mic_recording is false, but the async stop command hasn't
        // returned yet, so finalize_recording sees mic_live_file_idx=None and
        // creates a duplicate file.
        state.mic_live_data_cols.set(0);
        state.mic_recording_target_scroll.set(0.0);
        state.mic_scroll_user_pan_until.set(0.0);
    });
}

/// Spawns a requestAnimationFrame loop that smoothly interpolates
/// `scroll_offset` toward `mic_recording_target_scroll` for waterfall scrolling.
/// Automatically stops when recording and listening both end.
pub(crate) fn spawn_smooth_scroll_animation(state: AppState) {
    use std::rc::Rc;
    use std::cell::RefCell;

    let cb: Rc<RefCell<Option<Closure<dyn FnMut()>>>> = Rc::new(RefCell::new(None));
    let cb_clone = cb.clone();

    *cb.borrow_mut() = Some(Closure::new(move || {
        if !state.mic_recording.get_untracked() && !state.mic_listening.get_untracked() {
            // Neither recording nor listening — exit the animation loop
            return;
        }
        // If the user is panning (or just released a pan within the grace
        // window), leave scroll_offset alone so they can look at earlier
        // material without fighting the auto-scroll.
        let pan_until = state.mic_scroll_user_pan_until.get_untracked();
        let suspended = pan_until > 0.0 && js_sys::Date::now() < pan_until;
        if !suspended {
            let target = state.mic_recording_target_scroll.get_untracked();
            let current = state.scroll_offset.get_untracked();
            let diff = target - current;
            if diff.abs() > 0.0001 {
                // Exponential ease: move 30% of remaining distance each frame (~60fps)
                let new_scroll = current + diff * 0.3;
                state.scroll_offset.set(new_scroll);
            }
        }
        // Re-register for next frame
        if let Some(w) = web_sys::window() {
            if let Some(ref c) = *cb_clone.borrow() {
                let _ = w.request_animation_frame(c.as_ref().unchecked_ref());
            }
        }
    }));

    // Start the animation loop
    if let Some(w) = web_sys::window() {
        if let Some(ref c) = *cb.borrow() {
            let _ = w.request_animation_frame(c.as_ref().unchecked_ref());
        }
    }

    // Prevent the closure from being dropped by leaking it.
    // It will self-terminate when recording stops (the callback checks mic_recording).
    std::mem::forget(cb);
}

/// Parameters for the unified recording finalization.
pub(crate) struct FinalizeParams {
    pub samples: Vec<f32>,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub is_float: bool,
    /// Path where native backend already saved the file (empty = needs WASM-side save).
    pub saved_path: String,
    /// Actual file size from native backend (overrides WASM-side estimate).
    pub file_size: Option<usize>,
}

/// Collected recording metadata (GUANO, markers, naming) built from AppState.
struct RecordingMeta {
    guano: crate::audio::guano::GuanoMetadata,
    wav_markers: Vec<crate::types::WavMarker>,
    preroll_samples: usize,
}

/// Read mic info, location, device model, and preroll from state to build
/// GUANO metadata and WAV cue markers. Pure state-read, no mutations.
fn build_recording_meta(
    state: &AppState,
    sample_rate: u32,
    duration_secs: f64,
    filename: &str,
) -> RecordingMeta {
    let mic_name = state.mic_device_name.get_untracked();
    let mic_manufacturer = state.mic_manufacturer.get_untracked();
    let conn_type = state.mic_connection_type.get_untracked();
    let loc = state.recording_location.get_untracked();
    let is_mobile = state.is_mobile.get_untracked();
    let (dev_make, dev_model) = if state.device_model_enabled.get_untracked() && is_mobile {
        (state.cached_device_make.get_untracked(), state.cached_device_model.get_untracked())
    } else {
        (None, None)
    };

    // Determine mic_name for GUANO: USB gets the device name, internal gets "Internal".
    // Web Audio API uses a separate "Audio Device" field instead of "Name".
    let is_usb = conn_type.as_deref().map(|c| c.contains("USB")).unwrap_or(false);
    let is_web_audio = conn_type.as_deref() == Some("Web Audio API");
    let (guano_mic_name, guano_mic_audio_device) = if is_web_audio {
        (None, mic_name.clone())
    } else if is_usb {
        (mic_name.clone(), None)
    } else if conn_type.is_some() {
        (Some("Internal".to_string()), None)
    } else {
        (mic_name.clone(), None)
    };

    let preroll = state.mic_preroll_samples.get_untracked();
    let preroll_secs = if preroll > 0 && sample_rate > 0 {
        Some(preroll as f64 / sample_rate as f64)
    } else {
        None
    };

    let guano_extra = crate::audio::guano::RecordingGuanoExtra {
        mic_interface: conn_type,
        mic_name: guano_mic_name,
        mic_audio_device: guano_mic_audio_device,
        mic_make: mic_manufacturer,
        loc_position: loc.as_ref().map(|l| (l.latitude, l.longitude)),
        loc_elevation: loc.as_ref().and_then(|l| l.elevation),
        loc_accuracy: loc.as_ref().and_then(|l| l.accuracy),
        device_make: dev_make,
        device_model: dev_model,
        preroll_secs,
    };
    let guano = crate::audio::guano::build_recording_guano(
        sample_rate, duration_secs, filename,
        state.is_tauri, is_mobile,
        &guano_extra,
        &crate::format_time::recording_timestamp(duration_secs),
        env!("CARGO_PKG_VERSION"),
    );

    let wav_markers = if preroll > 0 {
        vec![crate::types::WavMarker {
            id: 1,
            position: preroll as u64,
            label: Some("Recording start".to_string()),
            note: None,
        }]
    } else {
        Vec::new()
    };

    RecordingMeta { guano, wav_markers, preroll_samples: preroll }
}

/// Create or update the LoadedFile in state. Returns (file_index, filename).
fn update_or_create_file(
    state: AppState,
    live_idx: Option<usize>,
    audio: AudioData,
    preview: crate::types::PreviewImage,
    wav_markers: Vec<crate::types::WavMarker>,
    sample_rate: u32,
) -> (usize, String) {
    use crate::canvas::{spectral_store, tile_cache};

    let (file_index, name) = if let Some(idx) = live_idx {
        let name = state.files.with_untracked(|files| {
            files.get(idx).map(|f| f.name.clone()).unwrap_or_default()
        });

        tile_cache::clear_file(idx);
        spectral_store::clear_file(idx);

        state.files.update(|files| {
            if let Some(f) = files.get_mut(idx) {
                f.audio = audio;
                f.preview = Some(preview);
            }
        });

        (idx, name)
    } else {
        let name = generate_recording_name();
        let total_cols = if audio.samples.len() >= 2048 {
            (audio.samples.len() - 2048) / 512 + 1
        } else { 0 };
        let placeholder_spec = SpectrogramData {
            columns: Vec::new().into(),
            total_columns: total_cols,
            freq_resolution: sample_rate as f64 / 2048.0,
            time_resolution: 512.0 / sample_rate as f64,
            max_freq: sample_rate as f64 / 2.0,
            sample_rate,
        };

        let mut idx = 0;
        let name_clone = name.clone();
        state.files.update(|files| {
            idx = files.len();
            files.push(LoadedFile {
                name: name_clone,
                audio,
                spectrogram: placeholder_spec,
                preview: Some(preview),
                overview_image: None,
                xc_metadata: None,
                xc_hashes: None,
                is_demo: false,
                is_recording: true,
                is_live_listen: false,
                settings: FileSettings::default(),
                add_order: idx,
                last_modified_ms: None,
                identity: None,
                file_handle: None,
                cached_peak_db: None,
                cached_full_peak_db: None,
                read_only: false,
                had_sidecar: false,
                verify_outcome: crate::state::VerifyOutcome::Pending,
                all_hashes_verified: false,
                wav_markers: Vec::new(),
                loading_id: None,
                min_display_freq: None,
                max_display_freq: None,
            });
        });
        state.current_file_index.set(Some(idx));
        (idx, name)
    };

    // Store WAV markers (preroll cue point) on the file
    if !wav_markers.is_empty() {
        state.files.update(|files| {
            if let Some(f) = files.get_mut(file_index) {
                f.wav_markers = wav_markers;
            }
        });
    }

    (file_index, name)
}

/// Compute identity hashes and optionally save WAV bytes to disk.
/// `wav_bytes` are built once and reused for both hashing and saving.
fn persist_and_identify(
    state: AppState,
    file_index: usize,
    filename: String,
    wav_bytes: Vec<u8>,
    audio_data_size: u64,
    needs_save: bool,
    is_mobile: bool,
) {
    let exact_file_size = wav_bytes.len() as u64;

    // If we also need to save, clone the bytes before identity computation consumes them.
    let wav_bytes_for_save = if needs_save { Some(wav_bytes.clone()) } else { None };

    crate::file_identity::start_identity_computation(
        state, file_index, filename.clone(), exact_file_size, Some(wav_bytes),
        Some(44), Some(audio_data_size), None,
    );

    if let Some(wav_data) = wav_bytes_for_save {
        wasm_bindgen_futures::spawn_local(async move {
            if is_mobile {
                crate::audio::wav_encoder::save_wav_to_shared(&wav_data, &filename).await;
            } else if try_tauri_save(&wav_data, &filename).await.is_some() {
                // Desktop WASM save succeeded
            }
            state.files.update(|files| {
                if let Some(f) = files.get_mut(file_index) {
                    f.is_recording = false;
                }
            });
        });
    }
}

/// Unified recording finalization. Handles both browser (WASM-side save) and
/// native/Tauri (already-saved) recordings. Updates the live file in-place when
/// one exists, otherwise creates a new LoadedFile as fallback.
pub(crate) fn finalize_recording(params: FinalizeParams, state: AppState) {
    use crate::canvas::live_waterfall;

    let FinalizeParams { samples, sample_rate, bits_per_sample, is_float, saved_path, file_size } = params;

    let live_idx = state.mic_live_file_idx.get_untracked();

    // Streaming-to-disk mode: Tauri wrote the file during recording and
    // returned only metadata (no samples in RAM). Hand off to the streaming
    // loader so we only decode the head ~30 s for display and keep the rest
    // on disk. Avoids the multi-GB memory spike that the old "load all
    // samples into f.audio.samples" path would cause for long/high-SR
    // recordings.
    let has_native_path = !saved_path.is_empty() && !saved_path.starts_with("shared://");
    if samples.is_empty() && has_native_path {
        state.mic_live_file_idx.set(None);
        live_waterfall::clear();
        let path = saved_path.clone();
        let live_idx_for_async = live_idx;
        let fsize = file_size.unwrap_or(0) as u64;
        wasm_bindgen_futures::spawn_local(async move {
            if let Err(e) = finalize_streaming_tauri_recording(
                path, fsize, sample_rate, bits_per_sample, is_float, state, live_idx_for_async,
            ).await {
                log::error!("Streaming finalize failed: {}", e);
                if let Some(idx) = live_idx_for_async {
                    state.files.update(|files| {
                        if idx < files.len() { files.remove(idx); }
                    });
                }
                state.show_error_toast(format!("Recording save succeeded but load failed: {}", e));
            }
        });
        return;
    }
    // Shared storage (Android MediaStore): file was streamed directly into
    // the content-resolver fd and lives under a content:// URI we can't
    // read from here. Just drop the live placeholder; the user finds the
    // file through the system file manager.
    if samples.is_empty() && saved_path.starts_with("shared://") {
        state.mic_live_file_idx.set(None);
        live_waterfall::clear();
        if let Some(idx) = live_idx {
            state.files.update(|files| {
                if idx < files.len() { files.remove(idx); }
            });
        }
        state.show_info_toast("Recording saved to device storage");
        return;
    }

    state.mic_live_file_idx.set(None);

    if samples.is_empty() {
        log::warn!("Empty recording");
        if let Some(idx) = live_idx {
            state.files.update(|files| {
                if idx < files.len() { files.remove(idx); }
            });
        }
        return;
    }

    let duration_secs = samples.len() as f64 / sample_rate as f64;

    // ── Phase 1: Build metadata (GUANO + WAV markers) from state ────────
    let recording_name = live_idx
        .and_then(|idx| state.files.with_untracked(|f| f.get(idx).map(|f| f.name.clone())))
        .unwrap_or_else(generate_recording_name);
    let meta = build_recording_meta(&state, sample_rate, duration_secs, &recording_name);

    // ── Phase 2: Encode WAV bytes (single pass for size, hash, and save) ─
    let samples: Arc<Vec<f32>> = samples.into();
    let source = Arc::new(InMemorySource {
        samples: samples.clone(),
        raw_samples: None,
        sample_rate,
        channels: 1,
    });
    let wav_bytes = crate::audio::wav_encoder::encode_wav_complete(
        &samples, sample_rate, Some(&meta.guano), &meta.wav_markers,
    );
    let exact_file_size = file_size.unwrap_or(wav_bytes.len());
    let num_samples = samples.len() as u64;
    let audio_data_size = num_samples * (bits_per_sample as u64 / 8);

    let audio = AudioData {
        samples,
        source,
        sample_rate,
        channels: 1,
        duration_secs,
        metadata: FileMetadata {
            file_size: exact_file_size,
            format: "REC",
            bits_per_sample,
            is_float,
            guano: Some(meta.guano),
            data_offset: Some(44),
            data_size: Some(audio_data_size),
        },
    };

    let preview = compute_preview(&audio, 256, 128);
    let audio_for_stft = audio.clone();

    // ── Phase 3: Update or create the file in state ─────────────────────
    let (file_index, name_check) = update_or_create_file(
        state, live_idx, audio, preview, meta.wav_markers, sample_rate,
    );

    live_waterfall::clear();

    // Set file handle if native backend saved to internal storage
    let shared_saved = saved_path.starts_with("shared://");
    let native_saved = !saved_path.is_empty();
    if native_saved && !shared_saved {
        state.files.update(|files| {
            if let Some(f) = files.get_mut(file_index) {
                f.file_handle = Some(crate::audio::streaming_source::FileHandle::TauriPath(saved_path));
            }
        });
    }

    // Mark saved if native backend already persisted the file
    let record_mode = state.record_mode.get_untracked();
    let is_tauri = state.is_tauri;
    let is_mobile = state.is_mobile.get_untracked();
    let to_memory = record_mode == crate::state::RecordMode::ToMemory;

    if (native_saved || shared_saved) && !to_memory {
        state.files.update(|files| {
            if let Some(f) = files.get_mut(file_index) {
                f.is_recording = false;
            }
        });
    }

    // ── Phase 4: Hash computation + optional WAV save ───────────────────
    let needs_save = !to_memory && !shared_saved
        && if is_mobile { true } else { is_tauri && !native_saved };

    persist_and_identify(
        state, file_index, name_check.clone(), wav_bytes,
        audio_data_size, needs_save, is_mobile,
    );

    // ── Phase 5: Reset preroll + zoom + spectrogram ─────────────────────
    if meta.preroll_samples > 0 {
        state.mic_preroll_samples.set(0);
    }

    let canvas_w = state.spectrogram_canvas_width.get_untracked();
    let final_time_res = 512.0 / sample_rate as f64;
    state.zoom_level.set(crate::viewport::fit_zoom(canvas_w, final_time_res, duration_secs));
    state.scroll_offset.set(0.0);

    spawn_spectrogram_computation(audio_for_stft, name_check, file_index, state);
}

/// Async handoff for Tauri streaming-mode recordings. The `.wav` is already
/// on disk (written incrementally during recording). We read just the header
/// + first ~30 s via range reads, build a `StreamingWavSource`, and update
/// the live file entry so the rest of the recording is streamed on demand
/// via `read_file_range`. Peak memory = head window (~30 s of f32), not the
/// full recording.
async fn finalize_streaming_tauri_recording(
    path: String,
    file_size: u64,
    expected_sample_rate: u32,
    expected_bits_per_sample: u16,
    _expected_is_float: bool,
    state: AppState,
    live_idx: Option<usize>,
) -> Result<(), String> {
    use crate::audio::loader::parse_wav_header_with_file_size;
    use crate::audio::streaming_source::{FileHandle, StreamingWavSource};
    use crate::audio::source::DEFAULT_ANALYSIS_WINDOW_SECS;
    use crate::components::file_sidebar::streaming_load::{decode_head_pcm, scan_tail_for_guano};
    use crate::canvas::tile_cache;

    // Read first 64 KB for header parsing (covers fmt, optional fact, and
    // usually the data chunk start).
    let header_size = 65536u64.min(file_size);
    let header_bytes = crate::tauri_bridge::read_file_range(&path, 0, header_size).await?;
    let header = parse_wav_header_with_file_size(&header_bytes, Some(file_size))?;
    if header.sample_rate != expected_sample_rate {
        log::warn!(
            "recording sample rate mismatch: file says {}, expected {}",
            header.sample_rate, expected_sample_rate,
        );
    }

    // Decode the first ~30 s into mono f32 for fast display + analysis.
    let head_frames = ((DEFAULT_ANALYSIS_WINDOW_SECS * header.sample_rate as f64) as u64)
        .min(header.total_frames);
    let bytes_per_frame = header.channels as u64 * (header.bits_per_sample as u64 / 8);
    let head_byte_len = head_frames * bytes_per_frame;
    let head_pcm_bytes = crate::tauri_bridge::read_file_range(
        &path, header.data_offset, head_byte_len,
    ).await?;
    let head_interleaved = decode_head_pcm(
        &head_pcm_bytes,
        header.bits_per_sample,
        header.is_float,
        header.channels,
    );
    let channels = header.channels as usize;
    let (head_mono, head_raw) = if channels == 1 {
        (head_interleaved, None)
    } else {
        let mono: Vec<f32> = head_interleaved
            .chunks_exact(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect();
        (mono, Some(head_interleaved))
    };

    // GUANO chunk is typically after the data chunk in our writer — scan the
    // tail of the file for it.
    let mut guano = header.guano.clone();
    if guano.is_none() {
        let data_end = header.data_offset + header.data_size;
        if data_end < file_size {
            let tail_len = (file_size - data_end).min(65536);
            if let Ok(tail_bytes) = crate::tauri_bridge::read_file_range(&path, data_end, tail_len).await {
                guano = scan_tail_for_guano(&tail_bytes);
            }
        }
    }

    let source = Arc::new(StreamingWavSource::new(
        FileHandle::TauriPath(path.clone()),
        &header,
        head_mono.clone(),
        head_raw,
    ));

    let duration_secs = header.total_frames as f64 / header.sample_rate as f64;
    let samples_arc = Arc::new(head_mono);
    let audio = AudioData {
        samples: samples_arc,
        source,
        sample_rate: header.sample_rate,
        channels: header.channels as u32,
        duration_secs,
        metadata: crate::types::FileMetadata {
            file_size: file_size as usize,
            format: "REC",
            bits_per_sample: header.bits_per_sample,
            is_float: header.is_float,
            guano,
            data_offset: Some(header.data_offset),
            data_size: Some(header.data_size),
        },
    };
    let audio_for_stft = audio.clone();

    let preview = crate::dsp::fft::compute_preview(&audio, 256, 128);

    // Build the recording name from the saved filename.
    let name = path.rsplit(['/', '\\']).next().unwrap_or(&path).to_string();

    let (file_index, name_check) = update_or_create_file(
        state, live_idx, audio, preview, Vec::new(), header.sample_rate,
    );

    // Rename the live entry to match the on-disk filename + wire up the handle.
    state.files.update(|files| {
        if let Some(f) = files.get_mut(file_index) {
            f.name = name.clone();
            f.file_handle = Some(FileHandle::TauriPath(path.clone()));
            f.is_recording = false;
            f.audio.metadata.bits_per_sample = expected_bits_per_sample;
        }
    });

    let canvas_w = state.spectrogram_canvas_width.get_untracked();
    let final_time_res = 512.0 / header.sample_rate as f64;
    state.zoom_level.set(crate::viewport::fit_zoom(canvas_w, final_time_res, duration_secs));
    state.scroll_offset.set(0.0);
    if state.mic_preroll_samples.get_untracked() > 0 {
        state.mic_preroll_samples.set(0);
    }

    // Clear the provisional live-tile cache and kick off progressive tile
    // computation — the same path used by regular file loads.
    tile_cache::clear_file(file_index);
    spawn_spectrogram_computation(audio_for_stft, name_check, file_index, state);

    // Compute identity hash from the file on disk so the Name field in GUANO
    // and future sidecar resolution stay consistent.
    crate::file_identity::start_identity_computation(
        state, file_index, name, file_size, None,
        Some(44), Some(header.data_size), None,
    );

    Ok(())
}

fn generate_recording_name() -> String {
    let now = js_sys::Date::new_0();
    format!(
        "batcap_{:04}{:02}{:02}_{:02}{:02}{:02}.wav",
        now.get_full_year(),
        now.get_month() + 1,
        now.get_date(),
        now.get_hours(),
        now.get_minutes(),
        now.get_seconds(),
    )
}

/// Shared async spectrogram computation (used by both web and Tauri modes).
pub(crate) fn spawn_spectrogram_computation(
    audio: AudioData,
    name_check: String,
    file_index: usize,
    state: AppState,
) {
    wasm_bindgen_futures::spawn_local(async move {
        let yield_promise = js_sys::Promise::new(&mut |resolve, _| {
            if let Some(w) = web_sys::window() {
                let _ = w.set_timeout_with_callback(&resolve);
            }
        });
        JsFuture::from(yield_promise).await.ok();

        const FFT_SIZE: usize = 2048;
        const HOP_SIZE: usize = 512;
        const CHUNK_COLS: usize = 32;

        let total_cols = if audio.samples.len() >= FFT_SIZE {
            (audio.samples.len() - FFT_SIZE) / HOP_SIZE + 1
        } else {
            0
        };

        use crate::canvas::spectral_store;
        use crate::canvas::tile_cache::{self, TILE_COLS};

        // Initialise spectral store for progressive tile generation
        spectral_store::init(file_index, total_cols, FFT_SIZE);

        let n_tiles = total_cols.div_ceil(TILE_COLS);
        let mut tile_scheduled = vec![false; n_tiles];
        let mut chunk_start = 0;

        while chunk_start < total_cols {
            let still_present = state.files.get_untracked()
                .get(file_index)
                .map(|f| f.name == name_check)
                .unwrap_or(false);
            if !still_present {
                spectral_store::clear_file(file_index);
                return;
            }

            let chunk = compute_spectrogram_partial(
                &audio,
                FFT_SIZE,
                HOP_SIZE,
                chunk_start,
                CHUNK_COLS,
            );

            // Insert into spectral store for progressive tile generation
            spectral_store::insert_columns(file_index, chunk_start, &chunk);

            // Check if any tile is now complete and render it synchronously
            // (must be sync — async schedule_tile_from_store races with drain_columns below)
            let first_tile = chunk_start / TILE_COLS;
            let last_tile = ((chunk_start + chunk.len()).saturating_sub(1)) / TILE_COLS;
            let mut any_tile_rendered = false;
            let tile_end_idx = last_tile.min(n_tiles.saturating_sub(1));
            for (tile_idx, scheduled) in tile_scheduled.iter_mut().enumerate().take(tile_end_idx + 1).skip(first_tile) {
                if *scheduled { continue; }
                let tile_start = tile_idx * TILE_COLS;
                let tile_end = (tile_start + TILE_COLS).min(total_cols);
                if spectral_store::tile_complete(file_index, tile_start, tile_end) {
                    if tile_cache::render_tile_from_store_sync(file_index, tile_idx, FFT_SIZE) {
                        any_tile_rendered = true;
                    }
                    *scheduled = true;
                }
            }
            if any_tile_rendered {
                state.tile_ready_signal.update(|n| *n = n.wrapping_add(1));
            }

            chunk_start += CHUNK_COLS;

            let p = js_sys::Promise::new(&mut |resolve, _| {
                if let Some(w) = web_sys::window() {
                    let _ = w.set_timeout_with_callback(&resolve);
                }
            });
            JsFuture::from(p).await.ok();
        }

        // Drain store and assemble final SpectrogramData
        let final_columns = spectral_store::drain_columns(file_index)
            .unwrap_or_default();

        let freq_resolution = audio.sample_rate as f64 / FFT_SIZE as f64;
        let time_resolution = HOP_SIZE as f64 / audio.sample_rate as f64;
        let max_freq = audio.sample_rate as f64 / 2.0;

        let col_count = final_columns.len();
        let spectrogram = SpectrogramData {
            columns: final_columns.into(),
            total_columns: col_count,
            freq_resolution,
            time_resolution,
            max_freq,
            sample_rate: audio.sample_rate,
        };

        log::info!(
            "Recording spectrogram: {} columns, freq_res={:.1} Hz, time_res={:.4}s",
            spectrogram.columns.len(),
            spectrogram.freq_resolution,
            spectrogram.time_resolution
        );

        // Compute overview image for the recording
        let overview_img = crate::dsp::fft::compute_overview_from_spectrogram(&spectrogram);

        state.files.update(|files| {
            if let Some(f) = files.get_mut(file_index) {
                if f.name == name_check {
                    f.spectrogram = spectrogram;
                    f.overview_image = overview_img;
                }
            }
        });

        // Clear stale tiles (rendered with provisional max_magnitude) and
        // re-schedule with accurate final normalization.
        tile_cache::clear_file(file_index);
        let file_for_tiles = state.files.get_untracked().get(file_index).cloned();
        if let Some(file) = file_for_tiles {
            tile_cache::schedule_all_tiles(state, file, file_index);
        }

        state.tile_ready_signal.update(|n| *n += 1);
    });
}
