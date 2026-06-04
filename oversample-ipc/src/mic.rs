//! Microphone / audio-device IPC wire types.

use serde::{Deserialize, Serialize};

/// A contiguous supported configuration range for an input device.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SampleRateRange {
    pub min: u32,
    pub max: u32,
    pub channels: u16,
    /// cpal sample format tag, e.g. "I16", "I24", "I32", "F32".
    pub format: String,
}

/// One enumerated audio input device.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    pub is_default: bool,
    pub sample_rate_ranges: Vec<SampleRateRange>,
}

/// Result of the `mic_list_devices` command.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DeviceListResult {
    pub devices: Vec<DeviceInfo>,
    /// Audio host backend: "WASAPI", "Oboe", "CoreAudio", "ALSA", "JACK", etc.
    pub host_name: String,
}

// These mirror the native command structs (snake_case wire keys — Tauri's
// default serde naming). Result types for `mic_open` / `mic_stop_recording` /
// `mic_get_status` (shared by the USB record/stop variants too).

/// Result of `mic_open`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MicInfo {
    pub device_name: String,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub is_float: bool,
    pub format: String,
    pub supported_sample_rates: Vec<u32>,
    /// Audio host backend name ("Oboe", "WASAPI", "CoreAudio", "ALSA", "JACK", …).
    pub host_name: String,
}

/// Result of `mic_stop_recording` / `usb_stop_recording`. `samples_f32` is the
/// captured audio (can be large; raw-bytes transport is deferred to Phase 4).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RecordingResult {
    pub filename: String,
    pub saved_path: String,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub is_float: bool,
    pub duration_secs: f64,
    pub num_samples: usize,
    pub samples_f32: Vec<f32>,
    pub file_size_bytes: usize,
}

/// Result of `mic_get_status`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct MicStatus {
    pub is_open: bool,
    pub is_recording: bool,
    pub is_streaming: bool,
    pub samples_recorded: usize,
    pub sample_rate: u32,
}

// Arg structs (camelCase wire keys; Tauri maps them onto the snake_case command
// params). Optional fields are omitted from the wire when None, matching the
// previous "set only if present" behaviour.

/// Args for `mic_open` (cpal).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MicOpenArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_sample_rate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_bit_depth: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channels: Option<u16>,
}

/// Args for `mic_start_recording` / `usb_start_recording`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRecordingArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_fd: Option<i32>,
    pub enable_recovery: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connection_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mic_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mic_make: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_make: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    pub app_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loc_latitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loc_longitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loc_elevation: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loc_accuracy: Option<f64>,
}

/// Args for `mic_stop_recording` / `usb_stop_recording`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StopRecordingArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loc_latitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loc_longitude: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loc_elevation: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loc_accuracy: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_make: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_model: Option<String>,
    pub app_version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_native_save: Option<bool>,
}

/// Args for `mic_set_listening`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SetListeningArgs {
    pub listening: bool,
}

/// One entry from `mic_recover_recordings` — a crashed-session recording found on
/// disk and recoverable.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RecoveredRecording {
    pub path: String,
    pub filename: String,
    pub had_sidecar: bool,
    pub sample_count: u64,
    pub sample_rate: u32,
    pub duration_secs: f64,
    pub file_size_bytes: u64,
}
