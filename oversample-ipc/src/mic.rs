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
    /// Channels in each `mic_pull_audio` payload and in recordings: 2 means
    /// interleaved L/R, 1 means mono or a mono mix.
    pub channels: u16,
    /// Channels in the device's widest input setting. Above 2, `channel_pick`
    /// chooses which of them are recorded. 0 when not known.
    #[serde(default)]
    pub device_channels: u16,
}

/// Result of `mic_stop_recording` / `usb_stop_recording` (metadata only).
///
/// To-memory recordings keep the captured audio on the native side; when
/// `has_memory_samples` is set the frontend fetches it as raw f32 bytes via the
/// `mic_take_recorded_samples` command (an ArrayBuffer) rather than receiving it
/// inline as a JSON array of floats. Those samples are interleaved L/R when
/// `channels` is 2.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RecordingResult {
    pub filename: String,
    pub saved_path: String,
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub is_float: bool,
    pub duration_secs: f64,
    /// Length of the recording in frames (one sample per channel).
    pub num_samples: usize,
    pub channels: u16,
    /// True when the native side has stashed to-memory samples for pickup via
    /// `mic_take_recorded_samples`.
    pub has_memory_samples: bool,
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
    /// Effective bit depth auto-detected from the live i32 sample stream (unused
    /// LSBs imply the real depth, e.g. a 24-bit interface in a 32-bit container).
    /// `None` until enough signal has been seen, or for non-i32 streams.
    #[serde(default)]
    pub effective_bits: Option<u16>,
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
    /// For a device with more than 2 channels: the channels to record,
    /// 0-based, a pair (stereo) or one (mono). Empty or out of range means
    /// channels 1 and 2 in stereo. Ignored for devices with 1 or 2 channels,
    /// which follow `channels`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_pick: Option<Vec<u32>>,
    /// Record the picked pair averaged into one channel.
    #[serde(default)]
    pub channel_pick_averaged: bool,
}

/// Args for `mic_start_recording` / `usb_start_recording`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRecordingArgs {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shared_fd: Option<i32>,
    pub enable_recovery: bool,
    /// Per-device bit-depth override: force the recording to this effective depth
    /// (12/16/24/32) instead of the auto-detected one. `None` = Auto.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub force_bits: Option<u16>,
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
    /// Pre-roll length (samples) to seed from the native listening ring into the
    /// start of the recording, so a long-press capture includes the preceding
    /// audio written linearly to disk. `None`/0 = no pre-roll.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preroll_samples: Option<u32>,
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

/// Where new recordings are saved on the desktop. Result of
/// `recording_folder_get`, `recording_folder_choose` and
/// `recording_folder_reset`. On Android it is the app data folder's
/// `recordings/`, which is used only when shared storage is not.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RecordingFolder {
    /// The folder the next recording is saved in.
    pub path: String,
    /// `path` with the home folder written as `~` (not on Windows), for a
    /// one-line display.
    pub short_path: String,
    /// The folder used when the user has not chosen one: the Music folder's
    /// `Oversample`, or the app data folder's `recordings/` when there is no
    /// usable Music folder.
    pub default_path: String,
    /// The user chose a folder (it may not be usable now; see `unusable_custom`).
    pub is_custom: bool,
    /// The folder the user chose, when it can't be used now, so `path` is the
    /// default folder instead.
    pub unusable_custom: Option<String>,
    /// No usable Music folder: the default folder is the app data folder.
    pub music_missing: bool,
}
