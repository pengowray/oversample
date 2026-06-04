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
