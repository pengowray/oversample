//! Microphone control: unified record/listen API.
//!
//! This module provides the public API for microphone recording and listening.
//! Backend-specific operations (Web Audio, cpal, USB) are delegated to
//! `mic_backend::ActiveBackend`. Finalization is handled by `live_recording`.

use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use crate::state::{AppState, GpsLocation, MicStrategy, MicBackend, MicAcquisitionState, MicPendingAction};
use crate::audio::mic_backend::{ActiveBackend, StopResult};
use crate::audio::live_recording::FinalizeParams;
use crate::tauri_bridge::{tauri_invoke, tauri_invoke_no_args, tauri_invoke_typed_no_args};

// ── GPS location acquisition ────────────────────────────────────────────

/// Query the current WiFi SSID from the native plugin.
/// Returns None if not connected or unavailable.
async fn get_wifi_ssid() -> Option<String> {
    let result: oversample_ipc::plugins::WifiSsidResult =
        tauri_invoke_typed_no_args("plugin:geolocation|getWifiSsid").await.ok()?;
    result.ssid
}

/// Fetch the Android device make/model from the native plugin.
/// Results are cached in state after first call (cached_device_make / cached_device_model).
async fn fetch_device_model(state: &AppState) {
    if state.cached_device_model.get_untracked().is_some() {
        return;
    }
    let Ok(result) = tauri_invoke_typed_no_args::<oversample_ipc::plugins::DeviceModelResult>(
        "plugin:geolocation|getDeviceModel",
    ).await else {
        return;
    };
    if !result.manufacturer.is_empty() {
        state.cached_device_make.set(Some(result.manufacturer));
    }
    if !result.model.is_empty() {
        state.cached_device_model.set(Some(result.model));
    }
}

/// Request a one-shot GPS fix from the native geolocation plugin.
/// Returns None if the plugin is unavailable, permission denied, or location times out.
async fn acquire_gps_location() -> Option<GpsLocation> {
    let result: oversample_ipc::plugins::GeolocationResult =
        tauri_invoke_typed_no_args("plugin:geolocation|getCurrentLocation").await.ok()?;
    // Error response (permission_denied, timeout, …) carries no coordinates.
    if result.error.is_some() {
        return None;
    }
    let latitude = result.latitude?;
    let longitude = result.longitude?;
    let elevation = if result.has_altitude.unwrap_or(false) {
        result.altitude
    } else {
        None
    };
    Some(GpsLocation { latitude, longitude, elevation, accuracy: result.accuracy })
}

// ── Tauri IPC query helpers ─────────────────────────────────────────────

/// Request Android RECORD_AUDIO runtime permission via Tauri plugin.
/// Returns true if granted, false if denied or not on Android.
pub async fn request_audio_permission_tauri(state: &AppState) -> bool {
    if !state.is_tauri {
        return true;
    }
    state.log_debug("info", "Requesting RECORD_AUDIO permission via plugin...");
    match tauri_invoke_typed_no_args::<oversample_ipc::plugins::PermissionGranted>(
        "plugin:usb-audio|requestAudioPermission",
    ).await {
        Ok(result) => {
            if result.granted {
                state.log_debug("info", "RECORD_AUDIO permission granted");
            } else {
                state.log_debug("error", "RECORD_AUDIO permission denied");
                state.show_error_toast("Microphone permission denied");
            }
            result.granted
        }
        Err(e) => {
            state.log_debug("warn", format!("requestAudioPermission failed (may not be Android): {}", e));
            true // Non-fatal on desktop Tauri
        }
    }
}

/// Query the default cpal input device's supported sample rates without opening the mic.
/// Updates `state.mic_supported_rates` with the result.
pub async fn query_cpal_supported_rates(state: &AppState) {
    if !state.is_tauri {
        return;
    }
    let result = match tauri_invoke_typed_no_args::<oversample_ipc::mic::DeviceListResult>("mic_list_devices").await {
        Ok(v) => v,
        Err(_) => return,
    };
    for dev in &result.devices {
        if !dev.is_default {
            continue;
        }
        let mut rates = std::collections::BTreeSet::new();
        for range in &dev.sample_rate_ranges {
            rates.insert(range.min);
            rates.insert(range.max);
            for &r in &[44100u32, 48000, 96000, 192000, 256000, 384000, 500000] {
                if r >= range.min && r <= range.max {
                    rates.insert(r);
                }
            }
        }
        let rates_vec: Vec<u32> = rates.into_iter().collect();
        if !rates_vec.is_empty() {
            state.mic_supported_rates.set(rates_vec);
        }
        break;
    }
}

/// Query mic info without opening the mic. Populates device name/type signals.
pub async fn query_mic_info(state: &AppState) {
    if !state.is_tauri {
        return;
    }
    let backend = state.mic_backend.get_untracked();

    match backend {
        Some(MicBackend::RawUsb) => {
            if let Ok(result) = tauri_invoke_typed_no_args::<oversample_ipc::plugins::UsbDeviceListResult>(
                "plugin:usb-audio|listUsbDevices",
            ).await {
                if let Some(dev) = result.devices.iter().find(|d| d.is_audio_device) {
                    state.mic_device_name.set(Some(dev.product_name.clone()));
                    state.mic_connection_type.set(Some("USB".to_string()));
                    state.mic_usb_connected.set(true);
                    return;
                }
            }
            state.mic_usb_connected.set(false);
        }
        Some(MicBackend::Cpal) | None => {
            if let Ok(result) = tauri_invoke_typed_no_args::<oversample_ipc::mic::DeviceListResult>("mic_list_devices").await {
                for dev in &result.devices {
                    if dev.is_default {
                        let n = dev.name.to_lowercase();
                        let conn = if n.contains("usb") {
                            "USB"
                        } else if n.contains("bluetooth") || n.contains("bt ") {
                            "Bluetooth"
                        } else {
                            "Internal"
                        };
                        state.mic_connection_type.set(Some(conn.to_string()));
                        state.mic_device_name.set(Some(dev.name.clone()));

                        let mut max_rate: u32 = 0;
                        let mut format_str: Option<String> = None;
                        for range in &dev.sample_rate_ranges {
                            if range.max > max_rate {
                                max_rate = range.max;
                                format_str = Some(range.format.clone());
                            }
                        }
                        if max_rate > 0 {
                            state.mic_sample_rate.set(max_rate);
                        }
                        if let Some(fmt) = format_str {
                            let bits: u16 = match fmt.as_str() {
                                "I16" => 16, "I24" => 24, "I32" => 32, "F32" => 32,
                                _ => 0,
                            };
                            if bits > 0 {
                                state.mic_bits_per_sample.set(bits);
                            }
                        }
                        break;
                    }
                }
            }
        }
        Some(MicBackend::Browser) => {}
    }

    // Also check for USB devices to update usb_connected status
    if let Ok(result) = tauri_invoke_typed_no_args::<oversample_ipc::plugins::UsbDeviceListResult>(
        "plugin:usb-audio|listUsbDevices",
    ).await {
        state.mic_usb_connected.set(result.devices.iter().any(|d| d.is_audio_device));
    }
}

/// Check for USB audio devices and update `mic_usb_connected` signal.
pub async fn check_usb_status(state: &AppState) {
    if let Ok(result) = tauri_invoke_typed_no_args::<oversample_ipc::plugins::UsbDeviceListResult>(
        "plugin:usb-audio|listUsbDevices",
    ).await {
        if let Some(dev) = result.devices.iter().find(|d| d.is_audio_device) {
            state.mic_usb_connected.set(true);
            state.show_info_toast(format!("USB mic: {}", dev.product_name));
            return;
        }
    }

    state.mic_usb_connected.set(false);
}

// ── Android foreground audio service ─────────────────────────────────────

/// Start (or update the notification of) the Android foreground audio service so
/// capture/monitoring survive the app being backgrounded. `mode` is "listening"
/// or "recording". No-op off mobile Tauri; best-effort (logged, not surfaced).
/// MUST be reached from a foreground user gesture — Android 14+ forbids starting
/// a microphone foreground service from the background.
async fn start_foreground_service(state: &AppState, mode: &str) {
    if !state.is_tauri || !state.is_mobile.get_untracked() {
        return;
    }
    let args = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&args, &JsValue::from_str("mode"), &JsValue::from_str(mode));
    if let Err(e) = tauri_invoke("plugin:audio-service|startForegroundAudio", &args.into()).await {
        state.log_debug("warn", format!("startForegroundAudio failed: {}", e));
    }
}

/// Stop the Android foreground audio service. No-op off mobile Tauri.
async fn stop_foreground_service(state: &AppState) {
    if !state.is_tauri || !state.is_mobile.get_untracked() {
        return;
    }
    if let Err(e) = tauri_invoke_no_args("plugin:audio-service|stopForegroundAudio").await {
        state.log_debug("warn", format!("stopForegroundAudio failed: {}", e));
    }
}

/// Persist that we've surfaced the notification-permission rationale so it never
/// re-prompts. Shared by the rationale modal and the no-op fast path below.
pub(crate) fn mark_notif_asked(state: &AppState) {
    state.notif_perm_asked.set(true);
    if let Some(ls) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = ls.set_item("oversample_notif_perm_asked", "true");
    }
}

/// Invoke the native POST_NOTIFICATIONS request (called after the in-app
/// rationale, so the OS prompt is never shown cold). No-op off mobile Tauri.
pub(crate) async fn request_notification_permission(state: &AppState) {
    if !state.is_tauri || !state.is_mobile.get_untracked() {
        return;
    }
    let _ = tauri_invoke_no_args("plugin:audio-service|requestNotificationPermission").await;
}

/// During mic setup on mobile Tauri, decide whether to surface the notification
/// rationale before the OS asks. We ask the plugin whether a runtime request is
/// needed (API 33+) and not yet granted; only then do we show the rationale.
/// Persists `notif_perm_asked` so this happens at most once. Best-effort.
async fn maybe_prompt_notifications(state: &AppState) {
    if !state.is_tauri || !state.is_mobile.get_untracked() {
        return;
    }
    if state.notif_perm_asked.get_untracked() {
        return;
    }
    let status = match tauri_invoke_typed_no_args::<oversample_ipc::plugins::NotificationPermissionStatus>(
        "plugin:audio-service|isNotificationPermissionGranted",
    ).await {
        Ok(v) => v,
        Err(_) => return, // plugin unavailable — leave the Listen-time path as-is
    };
    if !status.runtime_required || status.granted {
        // Older Android (no runtime permission) or already granted — nothing to
        // ask; record it so we don't re-check on every acquisition.
        mark_notif_asked(state);
        return;
    }
    state.show_notif_rationale.set(true);
}

// ── Backend resolution ──────────────────────────────────────────────────

/// Convert `state.mic_backend` to `ActiveBackend`.
fn resolve_active_backend(state: &AppState) -> Option<ActiveBackend> {
    state.mic_backend.get_untracked().map(ActiveBackend::from)
}

/// Open the appropriate mic backend based on a resolved MicBackend.
async fn open_backend(state: &AppState, backend: MicBackend) -> bool {
    ActiveBackend::from(backend).open(state).await
}

// ── Unified mic acquisition ─────────────────────────────────────────────

/// Unified mic acquisition. Called by both toggle_record and toggle_listen.
/// Returns the resolved MicBackend when the mic is ready, or None if the user
/// cancelled, permission was denied, or the mic failed to open.
pub async fn acquire_mic(state: &AppState, action: MicPendingAction) -> Option<MicBackend> {
    // If mic is already open and streaming, return current backend immediately
    if state.mic_acquisition_state.get_untracked() == MicAcquisitionState::Ready {
        if let Some(backend) = state.mic_backend.get_untracked() {
            let still_open = ActiveBackend::from(backend).is_open();
            if still_open {
                return Some(backend);
            }
            // Backend closed unexpectedly — fall through to re-acquire
            state.mic_acquisition_state.set(MicAcquisitionState::Idle);
        }
    }

    let strategy = state.mic_strategy.get_untracked();

    match strategy {
        MicStrategy::None => {
            state.log_debug("info", "acquire_mic: strategy=None, mic disabled");
            None
        }
        MicStrategy::Browser => {
            state.mic_acquisition_state.set(MicAcquisitionState::Acquiring);
            let t0 = js_sys::Date::now();
            if ActiveBackend::Browser.open(state).await {
                let elapsed = js_sys::Date::now() - t0;
                state.mic_permission_dialog_shown.set(elapsed > 1500.0);
                state.mic_backend.set(Some(MicBackend::Browser));
                state.mic_acquisition_state.set(MicAcquisitionState::Ready);
                let st = *state;
                wasm_bindgen_futures::spawn_local(async move { maybe_prompt_notifications(&st).await; });
                Some(MicBackend::Browser)
            } else {
                state.mic_acquisition_state.set(MicAcquisitionState::Failed);
                state.mic_strategy.set(MicStrategy::Ask);
                state.mic_backend.set(None);
                state.mic_device_info.set(None);
                state.mic_selected_device.set(None);
                state.status_message.set(Some("Browser mic failed. Please choose a microphone.".into()));
                None
            }
        }
        MicStrategy::Selected => {
            if let Some(backend) = state.mic_backend.get_untracked() {
                state.mic_acquisition_state.set(MicAcquisitionState::Acquiring);
                let t0 = js_sys::Date::now();
                if open_backend(state, backend).await {
                    let elapsed = js_sys::Date::now() - t0;
                    state.mic_permission_dialog_shown.set(elapsed > 1500.0);
                    state.mic_acquisition_state.set(MicAcquisitionState::Ready);
                    let st = *state;
                    wasm_bindgen_futures::spawn_local(async move { maybe_prompt_notifications(&st).await; });
                    return Some(backend);
                } else {
                    state.mic_strategy.set(MicStrategy::Ask);
                    state.mic_backend.set(None);
                    state.mic_device_info.set(None);
                    state.mic_selected_device.set(None);
                    state.mic_acquisition_state.set(MicAcquisitionState::Idle);
                    state.status_message.set(Some("Microphone failed. Please choose again.".into()));
                    return None;
                }
            }
            // No backend remembered despite Selected — fall back to Ask
            state.mic_strategy.set(MicStrategy::Ask);
            state.mic_pending_action.set(Some(action));
            state.mic_acquisition_state.set(MicAcquisitionState::AwaitingChoice);
            state.show_mic_chooser.set(true);
            None
        }
        MicStrategy::Ask => {
            state.mic_pending_action.set(Some(action));
            state.mic_acquisition_state.set(MicAcquisitionState::AwaitingChoice);
            state.show_mic_chooser.set(true);
            None
        }
    }
}

// ── Unified flows (private) ─────────────────────────────────────────────

/// Start recording with the given backend (mic already open).
async fn do_start_recording(state: &AppState, backend: ActiveBackend) {
    warn_if_te_for_live(state);
    let was_listening = state.mic_listening.get_untracked();
    let has_listen_file = was_listening && state.mic_live_file_idx.get_untracked().is_some();

    // Fetch device model on first recording (cached for future use)
    if state.is_tauri && state.is_mobile.get_untracked() && state.device_model_enabled.get_untracked() {
        let _ = fetch_device_model(state).await;
    }

    // Acquire GPS location if enabled (one-shot, non-blocking).
    // Skip if connected to a home WiFi network (geofencing).
    if state.gps_location_enabled.get_untracked() && state.is_tauri && state.is_mobile.get_untracked() {
        let on_home_wifi = if state.home_wifi_ssids.with_untracked(|list| !list.is_empty()) {
            get_wifi_ssid().await
                .map(|ssid| state.home_wifi_ssids.with_untracked(|list| list.contains(&ssid)))
                .unwrap_or(false)
        } else {
            false
        };
        if on_home_wifi {
            log::info!("On home WiFi — skipping location embedding");
            state.recording_location.set(None);
        } else {
            state.recording_location.set(acquire_gps_location().await);
        }
    } else {
        state.recording_location.set(None);
    }

    // Detect armed-doc reuse before any backend I/O so we can rename it to the
    // proper batcap_*.wav name BEFORE start_recording runs. The recovery
    // sidecar/.wav.part filename is built from the file at mic_live_file_idx
    // inside build_start_recording_args, so promoting after start_recording
    // would leave the recovery file with the armed "Live mic (HH:MM:SS)" name.
    let armed_idx = if !has_listen_file {
        // Fresh record (not converting an active listen): collapse stale empty
        // live placeholders and reuse the surviving one (armed doc or a stale
        // listen entry) instead of spawning a second recording file.
        let keep = state.mic_live_file_idx.get_untracked();
        prune_empty_live_placeholders(state, keep);
        state.mic_live_file_idx.get_untracked()
            .filter(|&idx| is_reusable_live_doc(state, idx))
    } else {
        None
    };
    if let Some(idx) = armed_idx {
        // Reset a stale listen entry's accumulated display state before turning
        // it into a recording (no-op for an already-armed doc).
        convert_listen_to_armed(state);
        promote_armed_to_recording(state, idx);
    }

    if !has_listen_file {
        // Fresh recording — clear buffer, tiles, and any stale pre-roll count
        backend.clear_buffer();
        crate::canvas::tile_cache::clear_all_caches();
        state.mic_preroll_samples.set(0);
        state.mic_listening.set(false);
    } else {
        // Listen→record: keep mic_listening=true during the await so the
        // processing loop doesn't exit (it checks `!recording && !listening`).
        // We'll clear it after mic_recording is set to true.
        crate::canvas::tile_cache::clear_all_caches();
        // Rename the listening file from its placeholder ("Listening") to the
        // proper batcap_*.wav name BEFORE start_recording — the recovery
        // sidecar/.wav.part and the Android MediaStore entry are both built
        // from `mic_live_file_idx`'s current name. With the placeholder, the
        // MediaStore entry ends up `DISPLAY_NAME=Listening` against MIME
        // `audio/wav`, which Android either rejects or saves invisibly.
        let sr = state.mic_sample_rate.get_untracked();
        rename_listen_to_recording(state, sr);
    }

    match backend.start_recording(state).await {
        Ok(()) => {
            // Reset frequency display so the waterfall shows the full mic range.
            state.min_display_freq.set(None);
            state.max_display_freq.set(None);
            state.mic_samples_recorded.set(0);
            state.mic_recording.set(true);
            // Now safe to clear listening — recording is active, loop won't exit.
            state.mic_listening.set(false);
            state.mic_recording_start_time.set(Some(js_sys::Date::now()));
            // Keep capturing if the app is backgrounded (Android foreground service).
            start_foreground_service(state, "recording").await;
            let sr = state.mic_sample_rate.get_untracked();

            let file_idx = if has_listen_file {
                // Convert the existing listening file into a recording file.
                // Don't respawn the processing loop or recreate the waterfall —
                // the listen loop is already running on the same buffer and its
                // exit condition (`!recording && !listening`) is no longer true
                // because we just set mic_recording=true.  Recreating the
                // waterfall would discard all accumulated columns and cause a
                // visible flash/glitch.
                convert_listen_to_recording(state, sr)
            } else if let Some(idx) = armed_idx {
                // Armed doc was renamed before start_recording above; just
                // start the live processing loop on it.
                set_live_recording_zoom(state, sr);
                spawn_live_processing_loop(*state, idx, sr);
                spawn_smooth_scroll_animation(*state);
                idx
            } else {
                let idx = start_live_recording(state, sr);
                spawn_live_processing_loop(*state, idx, sr);
                spawn_smooth_scroll_animation(*state);
                idx
            };
            log::info!("Recording started ({:?}, pre-roll={}, file_idx={})", backend, has_listen_file, file_idx);
        }
        Err(e) => {
            log::error!("start_recording failed: {}", e);
            state.status_message.set(Some(format!("Failed to start recording: {}", e)));
            // A shared-storage (Android MediaStore) entry + fd may have been
            // reserved before start failed; delete the orphaned pending row so
            // it doesn't linger as a 0-byte file. (The detached fd itself can
            // only be reclaimed by the OS — see SharedFdGuard for the native
            // stop-path leak fix.)
            if state.is_tauri {
                crate::audio::mic_backend::cancel_shared_entry().await;
            }
            // If we were listening, clean up the orphaned listen file
            if has_listen_file {
                state.mic_listening.set(false);
                cleanup_listen_file(state);
            }
            // If we promoted an armed doc to recording above, roll it back so
            // is_reusable_live_doc() recognizes it again on the next attempt.
            if let Some(idx) = armed_idx {
                revert_recording_to_armed(state, idx);
            }
        }
    }

    // Release the start-debounce gate so the next Record press is honoured.
    // Cleared on both Ok and Err paths — `mic_recording` is what really tells
    // the next press to stop instead of start, but the gate covers the gap
    // between user gesture and that flag flipping.
    state.mic_starting_recording.set(false);
}

/// Convert a Tauri recording result into FinalizeParams.  When pre-roll is
/// active, the Tauri backend's buffer only has samples from `is_recording=true`,
/// but the WASM-side `NATIVE_REC_BUFFER` has the full picture (pre-roll +
/// recording).  In that case we use the WASM buffer and force a WASM-side
/// re-save so the written WAV includes the pre-roll + cue marker.
fn tauri_result_to_params(rec: crate::audio::mic_backend::TauriRecordingResult, state: &AppState) -> FinalizeParams {
    let preroll = state.mic_preroll_samples.get_untracked();
    if preroll > 0 {
        // Use the full WASM buffer (which includes pre-roll) instead of the
        // Tauri-only samples. Clear saved_path to force a WASM-side re-encode.
        let full_samples = crate::audio::mic_backend::take_native_buffer();
        log::info!(
            "Pre-roll active: using WASM buffer ({} samples, {} pre-roll) instead of Tauri buffer ({} samples)",
            full_samples.len(), preroll, rec.samples.len(),
        );
        FinalizeParams {
            samples: full_samples,
            sample_rate: rec.sample_rate,
            bits_per_sample: rec.bits_per_sample,
            is_float: rec.is_float,
            saved_path: String::new(),
            file_size: None,
        }
    } else {
        FinalizeParams {
            samples: rec.samples,
            sample_rate: rec.sample_rate,
            bits_per_sample: rec.bits_per_sample,
            is_float: rec.is_float,
            saved_path: rec.saved_path,
            file_size: rec.file_size_bytes,
        }
    }
}

/// Dispatch a StopResult into finalize_recording or cleanup.
/// Shared by both `do_stop_recording` and `stop_all`.
fn handle_stop_result(result: StopResult, state: &AppState) {
    let bits_per_sample = state.mic_bits_per_sample.get_untracked();
    match result {
        StopResult::Samples { samples, sample_rate } => {
            finalize_recording(FinalizeParams {
                samples, sample_rate, bits_per_sample, is_float: false,
                saved_path: String::new(), file_size: None,
            }, *state);
        }
        StopResult::TauriResult(rec) => {
            finalize_recording(tauri_result_to_params(rec, state), *state);
        }
        StopResult::Empty => {
            log::warn!("No samples recorded");
            cleanup_failed_recording(state);
        }
        StopResult::Error(e) => {
            log::error!("stop_recording failed: {}", e);
            state.status_message.set(Some(format!("Recording failed: {}", e)));
            cleanup_failed_recording(state);
        }
    }
}

/// Stop recording and finalize.
async fn do_stop_recording(state: &AppState, backend: ActiveBackend) {
    let was_listening = state.mic_listening.get_untracked();
    state.mic_recording.set(false);
    state.mic_recording_start_time.set(None);
    state.mic_samples_recorded.set(0);

    let result = backend.stop_recording(state).await;
    handle_stop_result(result, state);

    if was_listening {
        // Listen overlay was active during the recording. finalize_recording
        // cleared mic_live_file_idx, which causes the processing loop to exit
        // and leaves listening "on" but with no live file or waterfall. Kick
        // off a fresh listen session to restore the live visualization.
        // (do_start_listening re-issues the foreground service as "listening".)
        do_start_listening(state, backend).await;
    } else {
        backend.maybe_close(state).await;
        stop_foreground_service(state).await;
    }
}

/// Start listening with the given backend (mic already open).
async fn do_start_listening(state: &AppState, backend: ActiveBackend) {
    warn_if_te_for_live(state);
    // Reset frequency display so the waterfall shows the full mic range
    // (not a zoomed range from a previously-open high-SR file).
    state.min_display_freq.set(None);
    state.max_display_freq.set(None);
    // Clear buffer and DSP state BEFORE enabling listening to prevent stale
    // audio from a previous listen session leaking into the new one. Also stop
    // any still-scheduled playback and reset the schedule cursor so a backlog
    // from a previous (possibly backgrounded) session can't carry over.
    backend.clear_buffer();
    backend.clear_dsp_state();
    crate::audio::mic_backend::stop_het_playback();
    // Set the frontend signal early so the chunk handler accepts data
    // as soon as the native side starts streaming.
    state.mic_listening.set(true);
    backend.set_listening(state, true).await;
    let sr = state.mic_sample_rate.get_untracked();
    // Clear tile caches so previous file's spectrogram doesn't flash
    crate::canvas::tile_cache::clear_all_caches();

    // Collapse any stale, empty live-mic placeholders so the file list never
    // accumulates more than one. Keep the current live entry (if any) as the
    // reuse candidate; prune fixes up mic_live_file_idx for any removals.
    let keep = state.mic_live_file_idx.get_untracked();
    prune_empty_live_placeholders(state, keep);
    // Reuse the existing live placeholder — an armed doc OR a stale listen
    // entry that wasn't cleanly converted — if one is present; otherwise
    // create a new transient listening file. The reuse path lets the user
    // pre-configure HFR settings before pressing Listen, without spawning a
    // second file.
    let reuse = state.mic_live_file_idx.get_untracked()
        .filter(|&idx| is_reusable_live_doc(state, idx));
    let file_idx = if let Some(idx) = reuse {
        // If it's a stale listen entry, reset its accumulated display state
        // (spectrogram/overview/duration) to a clean armed shape first — a
        // no-op for an already-armed doc.
        convert_listen_to_armed(state);
        promote_armed_to_listening(state, idx);
        idx
    } else {
        start_live_listening(state, sr)
    };

    spawn_live_processing_loop(*state, file_idx, sr);
    spawn_smooth_scroll_animation(*state);
    // Keep monitoring alive if the app is backgrounded (Android foreground service).
    start_foreground_service(state, "listening").await;
}

/// Stop listening. Leaves the live file in place as an empty "armed" doc
/// (mic stays open) so the user can adjust HFR / band and press Listen or
/// Record again without re-acquiring the mic or creating a new entry.
async fn do_stop_listening(state: &AppState, backend: ActiveBackend) {
    state.mic_listening.set(false);
    // Stop scheduled playback immediately so Stop is instant (no backlog tail).
    crate::audio::mic_backend::stop_het_playback();
    crate::canvas::live_waterfall::clear();
    convert_listen_to_armed(state);
    backend.clear_buffer();
    backend.set_listening(state, false).await;
    // Tear down the foreground service unless a recording is still running.
    if !state.mic_recording.get_untracked() {
        stop_foreground_service(state).await;
    }
    // Intentionally not calling backend.maybe_close — keep the mic warm
    // for the armed doc.
}

// ── Public API ──────────────────────────────────────────────────────────

/// Toggle live HET listening on/off.
pub async fn toggle_listen(state: &AppState) {
    // If a recording is in progress, treat listen as a pure overlay: flip
    // mic_listening + reset HET DSP state, but leave the recording buffer,
    // live file, and processing loop untouched. Otherwise calling
    // do_start_listening / do_stop_listening here would clear_buffer() (wiping
    // in-progress recording samples) and replace mic_live_file_idx with a new
    // "Listening" entry — corrupting the file list.
    if state.mic_recording.get_untracked() {
        if let Some(backend) = resolve_active_backend(state) {
            let enabling = !state.mic_listening.get_untracked();
            state.log_debug("info", format!(
                "toggle_listen: recording active, {} HET overlay only",
                if enabling { "enabling" } else { "disabling" },
            ));
            if enabling {
                // Reset HET state so we don't hear stale audio from a prior session
                backend.clear_dsp_state();
            } else {
                // Stop the listen overlay's scheduled playback immediately.
                crate::audio::mic_backend::stop_het_playback();
            }
            state.mic_listening.set(enabling);
            backend.set_listening(state, enabling).await;
        }
        return;
    }

    // If already listening, stop
    if state.mic_listening.get_untracked() {
        state.log_debug("info", "toggle_listen: stopping");
        if let Some(backend) = resolve_active_backend(state) {
            do_stop_listening(state, backend).await;
        } else {
            // Fallback: just clear signals
            state.mic_listening.set(false);
            crate::canvas::live_waterfall::clear();
            cleanup_listen_file(state);
        }
        return;
    }

    // Acquire mic (unified flow)
    let mic_backend = match acquire_mic(state, MicPendingAction::Listen).await {
        Some(b) => b,
        None => {
            state.log_debug("info", "toggle_listen: acquire_mic returned None (chooser shown or failed)");
            return;
        }
    };

    let backend = ActiveBackend::from(mic_backend);
    state.log_debug("info", format!("toggle_listen: backend={:?}, starting listen", backend));
    do_start_listening(state, backend).await;
}

/// Reset the DSP state on whichever mic backend is currently active. Use this
/// when a live-audio parameter change (mode switch, filter knob) would
/// otherwise leak stale buffer contents into the new mode (PS/PV overlap
/// buffers, HET filter delay lines, IIR warmup tail).
pub fn clear_live_dsp_state(state: &AppState) {
    if let Some(backend) = resolve_active_backend(state) {
        backend.clear_dsp_state();
    }
}

/// Show a one-shot toast if the user starts live audio while PlaybackMode is
/// TimeExpansion. TE relies on AudioContext sample-rate tricks that can't work
/// for an unbounded live stream — `process_live_audio` falls through to
/// passthrough, but the user should know it's doing nothing.
fn warn_if_te_for_live(state: &AppState) {
    if state.playback_mode.get_untracked() == crate::state::PlaybackMode::TimeExpansion {
        state.show_info_toast(
            "Time-expansion isn't applicable to live audio — playing back at 1:1.",
        );
    }
}

/// Open the mic and create an empty live document — but don't start listening
/// or recording. Lets the user adjust HFR mode/range/bandpass first, then
/// press Listen or Record on the armed doc. If a non-empty live doc already
/// exists, this is a no-op (we don't want to discard in-progress audio).
///
/// Triggered from the file panel's "+ New live recording" button.
pub async fn arm_live_doc(state: &AppState) {
    // If we already have a live doc that's currently streaming, refuse — don't
    // step on an in-progress listen or recording session.
    if state.mic_listening.get_untracked() || state.mic_recording.get_untracked() {
        state.show_error_toast("Already listening or recording.");
        return;
    }

    // Collapse stale empty live placeholders; keep the current one (if any)
    // as the reuse candidate.
    let keep = state.mic_live_file_idx.get_untracked();
    prune_empty_live_placeholders(state, keep);
    // If a reusable live placeholder already exists, reset it to the idle
    // "armed" shape (a stale listen entry gets cleared) and navigate to it
    // instead of making a second one — pressing +New repeatedly is idempotent.
    if let Some(idx) = state.mic_live_file_idx.get_untracked() {
        if is_reusable_live_doc(state, idx) {
            convert_listen_to_armed(state); // no-op unless it's a stale listen entry
            state.current_file_index.set(Some(idx));
            return;
        }
    }

    let mic_backend = match acquire_mic(state, MicPendingAction::Arm).await {
        Some(b) => b,
        None => {
            state.log_debug("info", "arm_live_doc: acquire_mic returned None");
            return;
        }
    };
    let backend = ActiveBackend::from(mic_backend);
    state.log_debug("info", format!("arm_live_doc: mic acquired ({:?})", backend));

    // Reset DSP state and tile caches so the empty doc starts clean.
    backend.clear_buffer();
    backend.clear_dsp_state();
    crate::canvas::tile_cache::clear_all_caches();

    let sr = state.mic_sample_rate.get_untracked().max(48_000);
    let _ = start_live_armed(state, sr);
}

/// Toggle recording on/off. When stopping, finalizes the recording.
pub async fn toggle_record(state: &AppState) {
    // If already recording, stop
    if state.mic_recording.get_untracked() {
        state.log_debug("info", "toggle_record: stopping");
        if let Some(backend) = resolve_active_backend(state) {
            do_stop_recording(state, backend).await;
        }
        return;
    }

    // Debounce: ignore the second tap of a rapid double-press. Without this
    // guard, a user impatient with the "invisible" acquire_mic / IPC phase
    // could fire two parallel start flows — each calling
    // `mic_start_recording` and `try_create_shared_fd`, leaving Android with
    // two MediaStore entries (one stuck IS_PENDING=1 → `.pending`, the other
    // saved as "…(1).wav").
    if state.mic_starting_recording.get_untracked() {
        state.log_debug("info", "toggle_record: ignored — start already in flight");
        state.show_info_toast("Recording is starting\u{2026}");
        return;
    }
    state.mic_starting_recording.set(true);

    // If already listening, the mic is ready — go straight to recording
    if state.mic_listening.get_untracked() {
        if let Some(backend) = resolve_active_backend(state) {
            state.log_debug("info", format!("toggle_record: already listening, starting immediate with {:?}", backend));
            do_start_recording(state, backend).await;
            return;
        }
    }

    // Acquire mic (unified flow)
    let mic_backend = match acquire_mic(state, MicPendingAction::Record).await {
        Some(b) => b,
        None => {
            state.log_debug("info", "toggle_record: acquire_mic returned None (chooser shown or failed)");
            state.mic_starting_recording.set(false);
            return;
        }
    };

    let backend = ActiveBackend::from(mic_backend);

    // If OS permission dialog was shown (detected by timing), skip our dialog
    if state.mic_permission_dialog_shown.get_untracked() {
        state.log_debug("info", format!("toggle_record: backend={:?}, permission dialog detected, starting immediately", backend));
        do_start_recording(state, backend).await;
    } else {
        // Show "Ready to record" dialog — user must confirm. The flag stays
        // set so a stray tap during the dialog can't kick off a second flow;
        // confirm_record_start / cancel_record_start clear it.
        state.log_debug("info", format!("toggle_record: backend={:?}, showing Ready to Record dialog", backend));
        state.record_ready_state.set(crate::state::RecordReadyState::AwaitingConfirmation);
    }
}

/// Toggle recording via long-press while listening.  Works even in ListenOnly mode.
/// Captures the current listen buffer as pre-roll and records the buffer length
/// so a WAV cue marker can be written at finalization time.
pub async fn toggle_record_with_preroll(state: &AppState) {
    // If already recording, just stop (same as toggle_record)
    if state.mic_recording.get_untracked() {
        if let Some(backend) = resolve_active_backend(state) {
            do_stop_recording(state, backend).await;
        }
        return;
    }

    // Debounce against rapid double-press (see `toggle_record` for why).
    if state.mic_starting_recording.get_untracked() {
        state.log_debug("info", "toggle_record_with_preroll: ignored — start already in flight");
        return;
    }
    state.mic_starting_recording.set(true);

    // Must be listening to have a pre-roll buffer
    if !state.mic_listening.get_untracked() {
        // Not listening — fall back to normal toggle_record. Clear our flag
        // first so toggle_record's own debounce check doesn't bounce.
        state.mic_starting_recording.set(false);
        toggle_record(state).await;
        return;
    }

    // Capture the current listen buffer length as pre-roll.
    // Compensate for audio accumulated during the hold gesture: the user's intent
    // is to capture the buffer state from when they *started* pressing, not when
    // the 400ms timeout fired.
    let raw_preroll = crate::audio::mic_backend::with_live_samples(state.is_tauri, |s| s.len());
    let gesture_start = state.mic_gesture_start_ms.get_untracked();
    state.mic_gesture_start_ms.set(None); // consume
    let sample_rate = state.mic_sample_rate.get_untracked();
    let preroll = if let Some(start_ms) = gesture_start {
        let gesture_ms = (js_sys::Date::now() - start_ms).max(0.0);
        let gesture_samples = ((gesture_ms / 1000.0) * sample_rate as f64).round() as usize;
        raw_preroll.saturating_sub(gesture_samples)
    } else {
        raw_preroll
    };
    // The listen buffer keeps ~2 s of headroom beyond the user-requested
    // duration (see trim logic in live_recording) specifically so that
    // gesture-time compensation above doesn't eat into what the user asked
    // for. Clamp back down to the requested duration here so long presses
    // don't produce *more* pre-roll than the setting promises.
    let requested_secs = state.mic_preroll_buffer_secs.get_untracked().max(2) as usize;
    let requested_samples = requested_secs.saturating_mul(sample_rate as usize);
    let preroll = preroll.min(requested_samples);
    state.mic_preroll_samples.set(preroll);

    if let Some(backend) = resolve_active_backend(state) {
        log::info!("Long-press record: capturing {} pre-roll samples (raw={}, gesture compensated)", preroll, raw_preroll);
        do_start_recording(state, backend).await;
    } else {
        // No backend to start with; do_start_recording won't run to clear the flag.
        state.mic_starting_recording.set(false);
    }
}

/// Called by the "Ready to record" dialog's OK button.
pub async fn confirm_record_start(state: &AppState) {
    state.record_ready_state.set(crate::state::RecordReadyState::None);
    if let Some(backend) = resolve_active_backend(state) {
        do_start_recording(state, backend).await;
    } else {
        // No backend resolvable; do_start_recording won't run to clear the flag.
        state.mic_starting_recording.set(false);
    }
}

/// Called by the "Ready to record" dialog's Cancel button.
pub fn cancel_record_start(state: &AppState) {
    state.record_ready_state.set(crate::state::RecordReadyState::None);
    state.mic_starting_recording.set(false);
}

/// Stop both listening and recording, close mic.
pub fn stop_all(state: &AppState) {
    // Defensive: clear the start-debounce gate in case stop_all is called
    // mid-acquisition (e.g. tab close, app teardown, error recovery).
    state.mic_starting_recording.set(false);

    // Silence any scheduled live playback synchronously so Stop is instant,
    // independent of the async close that follows.
    crate::audio::mic_backend::stop_het_playback();

    let backend = resolve_active_backend(state).or_else(|| {
        // Legacy: infer from what's open
        if ActiveBackend::RawUsb.is_open() {
            Some(ActiveBackend::RawUsb)
        } else if ActiveBackend::Cpal.is_open() {
            Some(ActiveBackend::Cpal)
        } else {
            None
        }
    });

    let state_copy = *state;

    match backend {
        Some(b) => {
            wasm_bindgen_futures::spawn_local(async move {
                if state_copy.mic_recording.get_untracked() {
                    state_copy.mic_recording.set(false);
                    state_copy.mic_recording_start_time.set(None);
                    state_copy.mic_samples_recorded.set(0);

                    let result = b.stop_recording(&state_copy).await;
                    handle_stop_result(result, &state_copy);
                }
                if state_copy.mic_listening.get_untracked() {
                    state_copy.mic_listening.set(false);
                    cleanup_listen_file(&state_copy);
                }
                crate::canvas::live_waterfall::clear();
                b.close(&state_copy).await;
                stop_foreground_service(&state_copy).await;
                state_copy.mic_acquisition_state.set(MicAcquisitionState::Idle);
            });
        }
        None => {
            // No backend known — just clear state
            cleanup_listen_file(state);
            state.mic_listening.set(false);
            state.mic_recording.set(false);
            state.mic_recording_start_time.set(None);
            wasm_bindgen_futures::spawn_local(async move {
                ActiveBackend::Browser.close(&state_copy).await;
                stop_foreground_service(&state_copy).await;
                state_copy.mic_acquisition_state.set(MicAcquisitionState::Idle);
            });
        }
    }
}

// Re-export from split modules
pub use crate::audio::wav_encoder::encode_wav;
pub(crate) use crate::audio::live_recording::{
    start_live_recording, start_live_listening, start_live_armed,
    is_reusable_live_doc, prune_empty_live_placeholders,
    promote_armed_to_listening, promote_armed_to_recording,
    revert_recording_to_armed, set_live_recording_zoom,
    cleanup_listen_file, convert_listen_to_armed, convert_listen_to_recording,
    rename_listen_to_recording,
    spawn_live_processing_loop,
    spawn_smooth_scroll_animation, finalize_recording,
    cleanup_failed_recording,
};
