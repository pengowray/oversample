//! Centralized, typed access to persisted browser `localStorage` settings.
//!
//! Previously every persisted preference was read/written inline with a
//! `web_sys::window().local_storage()...get_item("oversample_...")` dance and a
//! stringly-typed key literal repeated at each site (audit LOW: "scattered,
//! untyped localStorage persistence"). This module is the single registry of
//! keys plus typed get/set helpers, so a key typo is a compile error rather than
//! a silently-lost setting. The pure parse logic is unit-tested on the host; the
//! `web_sys` I/O wrappers are thin.

/// Every persisted `localStorage` key, in one place. Use these constants instead
/// of string literals.
pub mod keys {
    pub const GPS_ENABLED: &str = "oversample_gps_enabled";
    pub const HOME_WIFI: &str = "oversample_home_wifi";
    pub const DEVICE_MODEL: &str = "oversample_device_model";
    pub const BG_AUDIO_HINT_DISMISSED: &str = "oversample_bg_audio_hint_dismissed";
    pub const NOTIF_PERM_ASKED: &str = "oversample_notif_perm_asked";
    /// "true" after the notice about a hidden capture notification was closed.
    pub const NOTIF_HIDDEN_NOTICE_DISMISSED: &str = "oversample_notif_hidden_notice_dismissed";
    /// "false" after "Don't show again" on the card that suggests HFR when an
    /// ultrasonic recording plays at 1:1. Settings > Bats turns it back on.
    pub const HFR_NUDGE_ENABLED: &str = "oversample_hfr_nudge_enabled";
    /// "false" after "Don't show again" on the card that suggests Gain when
    /// playback is very quiet. Settings > Playback turns it back on.
    pub const GAIN_NUDGE_ENABLED: &str = "oversample_gain_nudge_enabled";
    pub const BAT_BOOK_MODE: &str = "oversample_bat_book_mode";
    pub const BAT_BOOK_REGION: &str = "oversample_bat_book_region";
    pub const BAT_BOOK_FAVOURITES: &str = "oversample_bat_book_favourites";
    pub const SHIELD_STYLE: &str = "oversample_shield_style";
    pub const SHOW_STATUS_BAR: &str = "oversample_show_status_bar";
    /// "Lowest bat frequency" in Hz: where bat calls start for call
    /// detection, the loudest-frequency line, ZC division and the default HFR
    /// band. Unset means `dsp::ultrasonic::DEFAULT_LOWEST_BAT_HZ`.
    pub const LOWEST_BAT_HZ: &str = "oversample_lowest_bat_hz";
    /// JSON map of mic device name -> auto-detected effective bit depth, so a
    /// device seen in a prior session is known ahead of time.
    pub const MIC_BIT_DEPTHS: &str = "oversample_mic_bit_depths";
    /// JSON map of mic device name -> manual bit-depth override (Auto if absent).
    pub const MIC_BIT_DEPTH_OVERRIDES: &str = "oversample_mic_bit_depth_overrides";
    /// The mic panel's Channels setting: "mono" or "stereo".
    pub const MIC_CHANNEL_MODE: &str = "oversample_mic_channel_mode";
    /// JSON map of mic device name -> the channels to record from it, for
    /// devices with more than 2 channels: 1-based, "5,6" for a pair or "5" for
    /// one, or "avg" for channels 1 and 2 averaged. A device with no entry
    /// records channels 1 and 2 in stereo. (Replaces the single
    /// "oversample_mic_usb_channels" setting, which is no longer read.)
    pub const MIC_CHANNEL_PICKS: &str = "oversample_mic_channel_picks";
    /// The Export panel's WAV bit depth: "same", "16" or "float".
    pub const EXPORT_WAV_DEPTH: &str = "oversample_export_wav_depth";
    /// The Export panel's "Normalize peak to -3 dBFS" checkbox.
    pub const EXPORT_NORMALIZE: &str = "oversample_export_normalize";
    /// The Noise menu's Denoised copy Strength: "normal" or "heavy".
    pub const DENOISE_STRENGTH: &str = "oversample_denoise_strength";
    /// Xeno-Canto API key saved in the browser (web build only; the desktop app
    /// keeps its key in a native config file, see `xc_lib::key_store`).
    pub const XC_WEB_API_KEY: &str = "oversample_xc_api_key";
    /// "true" when the user chose to explore Xeno-Canto through the Oversample
    /// proxy (no key of their own), even if a key is saved.
    pub const XC_USE_PROXY: &str = "oversample_xc_use_proxy";
    /// The language Explore Xeno-canto shows species names in: "en" for
    /// English only, or a language code such as "de". Unset means the
    /// browser's languages.
    pub const XC_NAMES_LANGUAGE: &str = "oversample_xc_names_language";
    /// The rows above the spectrogram picked in the Call detection tab: the
    /// species (with their method) and BirdNET's principal components, as
    /// JSON (`classifier::strips::to_stored`).
    pub const SCORE_STRIPS: &str = "oversample_score_strips";
    /// The Step beside the Analyse button in the Call detection tab, for
    /// every classifier: "window" (the model's whole window) or seconds
    /// ("1", "0.5") (`classifier::summary::StepPick`).
    pub const CLASSIFIER_STEP: &str = "oversample_classifier_step";
    /// Folds in the Call detection tab that are open, as a comma-separated
    /// list of fold names ("map,windows"). Folds start closed.
    pub const CALL_DETECTION_FOLDS: &str = "oversample_call_detection_folds";
    /// "true" when "Find bird boxes" is on in the Call detection tab's
    /// BirdNET+ V3.0 settings.
    pub const BIRD_BOXES: &str = "oversample_bird_boxes";
    /// Developer override for the proxy base URL (e.g. `http://localhost:8080`).
    /// No UI; set it from the browser console. Unset means the build default.
    pub const XC_API_BASE_OVERRIDE: &str = "oversample_xc_api_base";
    /// How much audio "Add before" and "Add after" fetch for an A2O clip, in
    /// seconds: one of `a2o::ADD_CHOICES_SECS`.
    pub const A2O_ADD_SECS: &str = "oversample_a2o_add_secs";
}

/// Load the persisted per-device effective-bit-depth map (device name -> bits).
pub fn get_mic_bit_depths() -> std::collections::HashMap<String, u16> {
    get_raw(keys::MIC_BIT_DEPTHS)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the per-device effective-bit-depth map.
pub fn set_mic_bit_depths(map: &std::collections::HashMap<String, u16>) {
    if let Ok(s) = serde_json::to_string(map) {
        set_raw(keys::MIC_BIT_DEPTHS, &s);
    }
}

/// Load the per-device manual bit-depth OVERRIDE map (device name -> forced bits;
/// absent = Auto). Distinct from the auto-detected map above.
pub fn get_mic_bit_depth_overrides() -> std::collections::HashMap<String, u16> {
    get_raw(keys::MIC_BIT_DEPTH_OVERRIDES)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the per-device manual bit-depth override map.
pub fn set_mic_bit_depth_overrides(map: &std::collections::HashMap<String, u16>) {
    if let Ok(s) = serde_json::to_string(map) {
        set_raw(keys::MIC_BIT_DEPTH_OVERRIDES, &s);
    }
}

/// Load the per-device Channels picks (device name -> pick as saved).
pub fn get_mic_channel_picks() -> std::collections::HashMap<String, String> {
    get_raw(keys::MIC_CHANNEL_PICKS)
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the per-device Channels picks.
pub fn set_mic_channel_picks(map: &std::collections::HashMap<String, String>) {
    if let Ok(s) = serde_json::to_string(map) {
        set_raw(keys::MIC_CHANNEL_PICKS, &s);
    }
}

/// Pure: interpret a stored value as a bool. `"true"`/`"false"` map literally;
/// anything else (including absent) falls back to `default`. Kept separate from
/// the I/O so it can be unit-tested on the host. This reproduces the two legacy
/// idioms exactly: `v == "true"` (default-off) and `v != "false"` (default-on).
fn parse_bool(raw: Option<&str>, default: bool) -> bool {
    match raw {
        Some("true") => true,
        Some("false") => false,
        _ => default,
    }
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Read a raw string setting (`None` if unset or storage unavailable).
pub fn get_raw(key: &str) -> Option<String> {
    local_storage()?.get_item(key).ok().flatten()
}

/// Write a raw string setting. Errors (storage full/unavailable) are ignored,
/// matching the previous best-effort behaviour.
pub fn set_raw(key: &str, value: &str) {
    if let Some(ls) = local_storage() {
        let _ = ls.set_item(key, value);
    }
}

/// Remove a setting. Errors are ignored, as in [`set_raw`].
pub fn remove_raw(key: &str) {
    if let Some(ls) = local_storage() {
        let _ = ls.remove_item(key);
    }
}

/// Read a bool setting, falling back to `default` when unset/unrecognised.
pub fn get_bool(key: &str, default: bool) -> bool {
    parse_bool(get_raw(key).as_deref(), default)
}

/// Write a bool setting as `"true"`/`"false"`.
pub fn set_bool(key: &str, value: bool) {
    set_raw(key, if value { "true" } else { "false" });
}

/// The saved rows above the spectrogram: species, principal components and
/// sound groups. None picked when unset or unreadable.
pub fn get_score_strips() -> crate::classifier::strips::StripPicks {
    get_raw(keys::SCORE_STRIPS)
        .map(|raw| crate::classifier::strips::from_stored(&raw))
        .unwrap_or_default()
}

pub fn set_score_strips(picks: &crate::classifier::strips::StripPicks) {
    set_raw(
        keys::SCORE_STRIPS,
        &crate::classifier::strips::to_stored(picks),
    );
}

/// The saved Step for classifier runs; the whole window when unset.
pub fn get_classifier_step() -> crate::classifier::summary::StepPick {
    crate::classifier::summary::StepPick::from_stored(get_raw(keys::CLASSIFIER_STEP).as_deref())
}

pub fn set_classifier_step(pick: crate::classifier::summary::StepPick) {
    set_raw(keys::CLASSIFIER_STEP, &pick.to_stored());
}

/// True when the Call detection tab's fold `name` was left open on this
/// device.
pub fn get_fold_open(name: &str) -> bool {
    get_raw(keys::CALL_DETECTION_FOLDS).is_some_and(|v| v.split(',').any(|n| n == name))
}

/// Remember whether the Call detection tab's fold `name` is open.
pub fn set_fold_open(name: &str, open: bool) {
    let raw = get_raw(keys::CALL_DETECTION_FOLDS).unwrap_or_default();
    let mut names: Vec<&str> = raw
        .split(',')
        .filter(|n| !n.is_empty() && *n != name)
        .collect();
    if open {
        names.push(name);
    }
    set_raw(keys::CALL_DETECTION_FOLDS, &names.join(","));
}

/// The saved "Lowest bat frequency" in Hz, or the default.
pub fn get_lowest_bat_hz() -> f32 {
    get_raw(keys::LOWEST_BAT_HZ)
        .and_then(|s| s.parse::<f32>().ok())
        .filter(|hz| hz.is_finite() && *hz > 0.0)
        .unwrap_or(crate::dsp::ultrasonic::DEFAULT_LOWEST_BAT_HZ)
}

pub fn set_lowest_bat_hz(hz: f32) {
    set_raw(keys::LOWEST_BAT_HZ, &hz.to_string());
}

#[cfg(test)]
mod tests {
    use super::parse_bool;

    #[test]
    fn parse_bool_matches_legacy_semantics() {
        // default-off keys (gps / bg-hint / notif): absent, garbage, or "false" -> false
        assert!(!parse_bool(None, false));
        assert!(!parse_bool(Some("false"), false));
        assert!(!parse_bool(Some("garbage"), false));
        assert!(parse_bool(Some("true"), false));
        // default-on key (device_model): absent or garbage -> true, explicit "false" -> false
        assert!(parse_bool(None, true));
        assert!(parse_bool(Some("garbage"), true));
        assert!(!parse_bool(Some("false"), true));
        assert!(parse_bool(Some("true"), true));
    }
}
