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
