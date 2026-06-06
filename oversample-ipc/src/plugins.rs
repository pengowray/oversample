//! Wire types for the Android Kotlin Tauri plugins (geolocation, usb-audio,
//! audio-service, media-store).
//!
//! These are deserialized on the WASM frontend via `serde_wasm_bindgen`, which
//! is stricter than `Reflect::get`. The Kotlin side often returns *partial*
//! objects (error variants, optional fields), so fields here are `Option` /
//! `#[serde(default)]` to keep deserialization total. Field names match the
//! Kotlin `JSObject.put(...)` keys exactly (camelCase where the Kotlin uses it).

use serde::{Deserialize, Serialize};

// ── geolocation plugin ──────────────────────────────────────────────────

/// `plugin:geolocation|getDeviceModel`
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DeviceModelResult {
    #[serde(default)]
    pub manufacturer: String,
    #[serde(default)]
    pub model: String,
}

/// `plugin:geolocation|getWifiSsid` — `ssid` is null when unavailable / not on WiFi.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct WifiSsidResult {
    #[serde(default)]
    pub ssid: Option<String>,
}

/// `plugin:geolocation|getCurrentLocation` — either a fix or `{ error }`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct GeolocationResult {
    #[serde(default)]
    pub latitude: Option<f64>,
    #[serde(default)]
    pub longitude: Option<f64>,
    #[serde(default)]
    pub accuracy: Option<f64>,
    #[serde(default, rename = "hasAltitude")]
    pub has_altitude: Option<bool>,
    #[serde(default)]
    pub altitude: Option<f64>,
    /// Set instead of the coordinate fields on failure
    /// (permission_denied, no_location_service, location_disabled, timeout).
    #[serde(default)]
    pub error: Option<String>,
}

// ── audio-service plugin ────────────────────────────────────────────────

/// `plugin:audio-service|isNotificationPermissionGranted`. Both keys are always
/// present on the resolve path (the plugin rejects rather than resolving on
/// failure). `runtimeRequired` is true only on API 33+ where POST_NOTIFICATIONS
/// is a runtime permission.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationPermissionStatus {
    pub granted: bool,
    pub runtime_required: bool,
}

// ── usb-audio plugin ────────────────────────────────────────────────────

/// One entry from `plugin:usb-audio|listUsbDevices`. All keys are always
/// present (the Kotlin uses `?: "Unknown"` for nullable strings).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbDeviceEntry {
    pub device_name: String,
    pub vendor_id: i32,
    pub product_id: i32,
    pub product_name: String,
    pub manufacturer_name: String,
    pub device_class: i32,
    pub has_permission: bool,
    pub is_audio_device: bool,
}

/// `plugin:usb-audio|listUsbDevices`
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UsbDeviceListResult {
    pub devices: Vec<UsbDeviceEntry>,
}

/// `plugin:usb-audio|requestUsbPermission` result. `deviceName` is present on the
/// already-granted resolve but absent on the async-denied `{granted:false}`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbPermissionResult {
    pub granted: bool,
    #[serde(default)]
    pub device_name: Option<String>,
}

/// `plugin:usb-audio|openUsbDevice` result. Note: this command emits
/// `bitResolution` (NOT `bitDepth`) and does NOT emit `manufacturerName`
/// (source that from `listUsbDevices`). The `emt2*` fields appear only when
/// `isEmt2`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbOpenResult {
    pub fd: i32,
    pub endpoint_address: i32,
    pub max_packet_size: i32,
    pub sample_rate: i32,
    pub num_channels: i32,
    pub bit_resolution: i32,
    pub interface_number: i32,
    pub alternate_setting: i32,
    pub device_name: String,
    pub product_name: String,
    pub uac_version: i32,
    pub is_emt2: bool,
    #[serde(default)]
    pub emt2_oversized_packets: Option<bool>,
    #[serde(default)]
    pub reported_max_packet_size: Option<i32>,
}

/// Args for `plugin:usb-audio|requestUsbPermission` and `getUsbDeviceInfo`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbDeviceNameArgs {
    pub device_name: String,
}

/// Args for `plugin:usb-audio|openUsbDevice`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbOpenArgs {
    pub device_name: String,
    pub sample_rate: i32,
}

/// Args for the native `usb_start_stream` command (Rust, not a plugin). Tauri
/// maps these camelCase keys onto the snake_case command parameters.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbStartStreamArgs {
    pub fd: i32,
    pub endpoint_address: u32,
    pub max_packet_size: u32,
    pub sample_rate: u32,
    pub num_channels: u32,
    pub device_name: String,
    pub interface_number: u32,
    pub alternate_setting: u32,
    pub uac_version: u32,
}

/// Generic `{ granted }` result, e.g. `plugin:usb-audio|requestAudioPermission`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct PermissionGranted {
    #[serde(default)]
    pub granted: bool,
}

// USB hotplug is delivered by a native push to `window.__oversampleUsbHotplug`
// (UsbAudioPlugin → evaluateJavascript), not an IPC command, so there is no
// status-result type here. (Was `UsbStatusResult` for the removed checkUsbStatus
// poll — see lows #35.)

// ── media-store plugin ──────────────────────────────────────────────────
//
// NOTE: the byte payloads (`saveWavBytes.data`, `saveExportBytes.data`) are NOT
// modelled here — they are attached as a JS `Uint8Array` directly (serializing a
// Vec<u8> as a JSON number array would be huge/slow). Raw-bytes IPC is Phase 4.

/// Args for `plugin:media-store|createRecordingEntry`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CreateRecordingEntryArgs {
    pub filename: String,
}

/// Result of `plugin:media-store|createRecordingEntry`. `fd` is a raw POSIX file
/// descriptor (owned by Rust after detachFd); `uri` is the MediaStore row.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CreateRecordingEntryResult {
    pub fd: i32,
    pub uri: String,
}

/// Result of `plugin:media-store|saveWavBytes` / `saveExportBytes`. An empty
/// `path` means a permission-retry (pre-Q first run), not a real save.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SavePathResult {
    pub path: String,
}

/// Result of `plugin:media-store|cleanupPendingEntries`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CleanupResult {
    pub deleted: u32,
    pub skipped: bool,
}
