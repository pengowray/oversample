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
