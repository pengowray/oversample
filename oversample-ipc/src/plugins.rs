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
/// `supported` is false when the device has no WiFi hardware.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WifiSsidResult {
    #[serde(default)]
    pub ssid: Option<String>,
    #[serde(default = "default_true")]
    pub supported: bool,
}

fn default_true() -> bool {
    true
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

/// Result of `plugin:audio-service|areCaptureNotificationsShown`: whether
/// Android shows the "Recording audio" / "Listening (live)" notification.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CaptureNotificationStatus {
    pub shown: bool,
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
    /// Bytes per sample in a packet (2, 3 or 4). 0 from a plugin that does not report it.
    #[serde(default)]
    pub subslot_size: i32,
    /// IEEE float samples rather than PCM. False from a plugin that does not report it.
    #[serde(default)]
    pub is_float: bool,
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
    /// Most bits per sample to stream (16, 24 or 32), or 0 for the deepest
    /// the device offers.
    pub max_bit_depth: i32,
    /// The input interface and alternate setting to stream, or -1 for the
    /// plugin's own choice (the deepest 2-channel setting, then 1, then wider).
    #[serde(default = "no_alt")]
    pub interface_number: i32,
    #[serde(default = "no_alt")]
    pub alternate_setting: i32,
}

fn no_alt() -> i32 {
    -1
}

/// One input endpoint in a `plugin:usb-audio|getUsbDeviceInfo` result: one
/// alternate setting of an audio streaming interface.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbEndpointEntry {
    /// bEndpointAddress, direction bit included.
    pub address: i32,
    /// Raw wMaxPacketSize, high-bandwidth bits included.
    pub max_packet_size: i32,
    pub channels: i32,
    pub bit_resolution: i32,
    #[serde(default)]
    pub subslot_size: i32,
    #[serde(default)]
    pub is_float: bool,
    pub sample_rate: i32,
    #[serde(default)]
    pub sample_rate_settable: bool,
    pub interface_number: i32,
    pub alternate_setting: i32,
    /// bInterval. 0 from a plugin that does not report it.
    #[serde(default)]
    pub interval: i32,
}

/// `plugin:usb-audio|getUsbDeviceInfo` result (the fields Oversample reads).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbDeviceInfoResult {
    pub device_name: String,
    #[serde(default)]
    pub product_name: String,
    pub uac_version: i32,
    #[serde(default)]
    pub sample_rates: Vec<i32>,
    #[serde(default)]
    pub endpoints: Vec<UsbEndpointEntry>,
    /// The name of each channel of the widest input setting, read from the
    /// device's `iChannelNames` strings. Empty when the device names none or
    /// a read failed.
    #[serde(default, deserialize_with = "or_default")]
    pub channel_names: Vec<String>,
    /// A TASCAM Portacapture X8's reply to vendor request 0x58, the input
    /// report (`oversample_core::audio::x8_report`), as a JSON array of the
    /// reply's bytes (0 to 255). Only from an X8 whose reply starts with "X8"
    /// and version 1, so absent on other devices and on stock firmware.
    #[serde(default, deserialize_with = "or_default")]
    pub x8_report: Option<Vec<u8>>,
}

/// `plugin:usb-audio|getUsbInputState` result: the channel names and the X8
/// input report read again, as in [`UsbDeviceInfoResult`]. `channel_names` is
/// empty and `x8_report` absent when a read failed.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbInputStateResult {
    #[serde(default, deserialize_with = "or_default")]
    pub channel_names: Vec<String>,
    #[serde(default, deserialize_with = "or_default")]
    pub x8_report: Option<Vec<u8>>,
}

/// Deserializes a field, or gives its default when the value has the wrong
/// type (a null, or a number out of range), so that one bad optional field
/// does not make the whole result fail to deserialize.
fn or_default<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OrJunk<T> {
        Value(T),
        Junk(serde::de::IgnoredAny),
    }
    Ok(match OrJunk::<T>::deserialize(d)? {
        OrJunk::Value(v) => v,
        OrJunk::Junk(_) => T::default(),
    })
}

/// Args for the native `usb_start_stream` command (Rust, not a plugin), sent
/// as its `args` parameter: `{ "args": { "fd": …, "endpointAddress": … } }`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbStartStreamArgs {
    pub fd: i32,
    pub endpoint_address: u32,
    pub max_packet_size: u32,
    pub sample_rate: u32,
    pub num_channels: u32,
    /// The channels to keep from each frame (0-based), 1 or 2 of them. Empty
    /// means channel 1 on a 1-channel device, otherwise channels 1 and 2.
    #[serde(default)]
    pub channels: Vec<u32>,
    /// Keep the 2 kept channels separate in recordings and `mic_pull_audio`
    /// (interleaved L/R) instead of a mono mix.
    pub stereo: bool,
    pub device_name: String,
    pub interface_number: u32,
    pub alternate_setting: u32,
    pub uac_version: u32,
    /// Bytes per sample in a packet: 2, 3 or 4. Recordings keep the same bit depth.
    pub subslot_size: u32,
    /// 4-byte samples are IEEE float, recorded as float WAV.
    pub is_float: bool,
}

/// The `usb_start_stream` invoke payload: the args under the command's
/// `args` parameter name.
#[derive(Clone, Debug, Serialize)]
pub struct UsbStartStreamPayload<'a> {
    pub args: &'a UsbStartStreamArgs,
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
// Exports are written through a file descriptor: `createExportEntry` returns
// one, Rust takes it over (`save_adopt_fd`) and the webview sends the bytes in
// parts (`save_write`).

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

/// Args for `plugin:media-store|createExportEntry`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateExportEntryArgs {
    pub filename: String,
    pub mime_type: String,
    /// Folder under shared storage, e.g. `Movies/Oversample/exports`.
    pub relative_path: String,
}

/// Result of `plugin:media-store|createExportEntry`. No `fd` means the pre-Q
/// storage permission was just asked for and nothing was created: the user
/// has to try again.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CreateExportEntryResult {
    #[serde(default)]
    pub fd: Option<i32>,
    #[serde(default)]
    pub uri: String,
    #[serde(default)]
    pub path: String,
}

/// Args for `plugin:media-store|finishExportEntry` / `cancelExportEntry`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportEntryArgs {
    pub uri: String,
    pub mime_type: String,
}

/// Result of `plugin:media-store|cleanupPendingEntries`.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CleanupResult {
    pub deleted: u32,
    pub skipped: bool,
}

/// Args for `plugin:media-store|openRecordingForRead`: open a MediaStore
/// `content://` recording for reading, for streaming display.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OpenRecordingForReadArgs {
    pub uri: String,
}

/// Result of `plugin:media-store|openRecordingForRead`: a raw POSIX fd that
/// the `read_fd_range` command reads byte ranges from.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct OpenRecordingForReadResult {
    pub fd: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_start_stream_payload_nests_camel_case_args() {
        let args = UsbStartStreamArgs {
            fd: 7,
            endpoint_address: 0x81,
            max_packet_size: 576,
            sample_rate: 192_000,
            num_channels: 2,
            channels: vec![],
            stereo: true,
            device_name: "X8".into(),
            interface_number: 1,
            alternate_setting: 2,
            uac_version: 2,
            subslot_size: 3,
            is_float: false,
        };
        let json = serde_json::to_value(UsbStartStreamPayload { args: &args }).unwrap();
        // Tauri passes the "args" object to the command's `args` parameter.
        assert_eq!(json["args"]["endpointAddress"], 0x81);
        assert_eq!(json["args"]["subslotSize"], 3);
        let back: UsbStartStreamArgs = serde_json::from_value(json["args"].clone()).unwrap();
        assert_eq!(back.max_packet_size, 576);
        assert_eq!(back.device_name, "X8");
        assert!(back.stereo && !back.is_float);
        assert!(back.channels.is_empty());
    }

    #[test]
    fn usb_open_args_default_to_the_plugin_s_own_alt_choice() {
        let args: UsbOpenArgs =
            serde_json::from_str(r#"{"deviceName":"x","sampleRate":0,"maxBitDepth":0}"#).unwrap();
        assert_eq!((args.interface_number, args.alternate_setting), (-1, -1));
    }

    #[test]
    fn usb_device_info_reads_the_plugin_s_endpoints() {
        let json = r#"{"deviceName":"/dev/bus/usb/001/002","productName":"X8","uacVersion":2,
            "sampleRates":[48000,192000],"fileDescriptor":5,
            "endpoints":[{"address":129,"maxPacketSize":800,"channels":8,"bitResolution":32,
            "subslotSize":4,"isFloat":true,"sampleRate":192000,"sampleRateSettable":true,
            "interfaceNumber":1,"alternateSetting":3,"interval":1}]}"#;
        let info: UsbDeviceInfoResult = serde_json::from_str(json).unwrap();
        assert_eq!(info.endpoints[0].channels, 8);
        assert!(info.endpoints[0].is_float);
        assert_eq!(info.sample_rates, vec![48_000, 192_000]);
        // An older plugin sends neither channel names nor a report.
        assert!(info.channel_names.is_empty());
        assert_eq!(info.x8_report, None);
    }

    #[test]
    fn usb_device_info_reads_channel_names_and_the_x8_report() {
        let json = r#"{"deviceName":"d","uacVersion":2,
            "channelNames":["Mix L","Mix R"],
            "x8Report":[88,56,1,28,200,255]}"#;
        let info: UsbDeviceInfoResult = serde_json::from_str(json).unwrap();
        assert_eq!(info.channel_names, vec!["Mix L", "Mix R"]);
        assert_eq!(info.x8_report, Some(vec![88, 56, 1, 28, 200, 255]));
    }

    #[test]
    fn a_bad_channel_names_or_report_field_is_dropped_not_fatal() {
        let json = r#"{"deviceName":"d","uacVersion":2,"sampleRates":[48000],
            "channelNames":null,"x8Report":[88,-56,300]}"#;
        let info: UsbDeviceInfoResult = serde_json::from_str(json).unwrap();
        assert!(info.channel_names.is_empty());
        assert_eq!(info.x8_report, None);
        assert_eq!(info.sample_rates, vec![48_000]);
    }
}
