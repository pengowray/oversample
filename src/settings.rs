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
    pub const PROJECTS_ENABLED: &str = "oversample_projects_enabled";
    pub const BAT_BOOK_MODE: &str = "oversample_bat_book_mode";
    pub const BAT_BOOK_REGION: &str = "oversample_bat_book_region";
    pub const BAT_BOOK_FAVOURITES: &str = "oversample_bat_book_favourites";
    pub const SHIELD_STYLE: &str = "oversample_shield_style";
    pub const SHOW_STATUS_BAR: &str = "oversample_show_status_bar";
    /// JSON map of mic device name -> auto-detected effective bit depth, so a
    /// device seen in a prior session is known ahead of time.
    pub const MIC_BIT_DEPTHS: &str = "oversample_mic_bit_depths";
    /// JSON map of mic device name -> manual bit-depth override (Auto if absent).
    pub const MIC_BIT_DEPTH_OVERRIDES: &str = "oversample_mic_bit_depth_overrides";
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

/// Read a bool setting, falling back to `default` when unset/unrecognised.
pub fn get_bool(key: &str, default: bool) -> bool {
    parse_bool(get_raw(key).as_deref(), default)
}

/// Write a bool setting as `"true"`/`"false"`.
pub fn set_bool(key: &str, value: bool) {
    set_raw(key, if value { "true" } else { "false" });
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
