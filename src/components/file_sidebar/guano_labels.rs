//! Labels, headings and order for the GUANO rows in the Info panel.
//!
//! Standard GUANO fields keep their names (a few get a clearer label). Another
//! app's or vendor's fields, such as `PET|Profile REC LEN` or
//! `WA|Song Meter|Audio settings`, go under a heading named after the key's
//! prefix ("PET", "WA Song Meter"). Oversample's own fields go under one
//! "Oversample" heading with the labels in [`OVERSAMPLE_FIELDS`].

/// The heading for the fields Oversample writes, and the start of their keys.
pub(super) const OVERSAMPLE: &str = "Oversample";
const OVERSAMPLE_PREFIX: &str = "Oversample|";

/// Labels for the `Oversample|*` fields that recordings and exports contain
/// (written in `oversample-core/src/audio/guano.rs` and `src/audio/export.rs`).
/// The panel shows them in this order. A key that is not here still shows,
/// with the label from [`fallback_label`].
pub(super) const OVERSAMPLE_FIELDS: &[(&str, &str)] = &[
    ("Oversample|App|Version", "App version"),
    // "Desktop", "Android" or "Web".
    ("Oversample|App|Platform", "Platform"),
    // Android only, when "Include phone model" is on in the privacy settings.
    ("Oversample|Device|Make", "Phone make"),
    ("Oversample|Device|Model", "Phone model"),
    // How the app reads the mic: "ALSA", "WASAPI", "Oboe", "USB (UAC2)",
    // "Web Audio API".
    ("Oversample|Mic|Interface", "Audio API"),
    // A USB mic's name, or "Internal".
    ("Oversample|Mic|Name", "Mic name"),
    // The browser's name for the input device (web recordings only).
    ("Oversample|Mic|Audio Device", "Mic name (web)"),
    // Seconds of audio from before Record was pressed.
    ("Oversample|Audio|Preroll", "Pre-roll (s)"),
];

/// One GUANO field as the panel shows it.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct GuanoRow {
    pub label: String,
    /// The key as written in the file, shown when the pointer is on the label.
    pub key: String,
    pub value: String,
    /// Label and value side by side. Oversample's fields have short values.
    pub inline: bool,
}

/// Rows shown under one heading.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct GuanoSection {
    /// `None`: the panel's own heading ("Guano metadata"). `Some`: the app or
    /// vendor that the fields belong to ("Oversample", "PET", "WA Song Meter").
    pub vendor: Option<String>,
    pub rows: Vec<GuanoRow>,
}

/// The sections the panel shows for a file's GUANO fields. `labelled` is
/// false for metadata that is not GUANO (tags in a FLAC or MP3 file): those
/// keys show as they are, in one section.
pub(super) fn guano_sections(fields: &[(String, String)], labelled: bool) -> Vec<GuanoSection> {
    if !labelled {
        let rows = fields
            .iter()
            .map(|(k, v)| GuanoRow {
                label: k.clone(),
                key: k.clone(),
                value: v.clone(),
                inline: false,
            })
            .collect::<Vec<_>>();
        return if rows.is_empty() {
            Vec::new()
        } else {
            vec![GuanoSection { vendor: None, rows }]
        };
    }
    let mut sections: Vec<GuanoSection> = Vec::new();
    let mut oversample: Vec<GuanoRow> = Vec::new();
    for (k, v) in fields {
        if let Some(rest) = k.strip_prefix(OVERSAMPLE_PREFIX) {
            oversample.push(GuanoRow {
                label: oversample_label(k).map_or_else(|| fallback_label(rest), str::to_string),
                key: k.clone(),
                value: v.clone(),
                inline: true,
            });
            continue;
        }
        let (vendor, label) = categorize_guano_key(k);
        let row = GuanoRow {
            label,
            key: k.clone(),
            value: v.clone(),
            inline: false,
        };
        match sections.last_mut() {
            Some(s) if s.vendor == vendor => s.rows.push(row),
            _ => sections.push(GuanoSection {
                vendor,
                rows: vec![row],
            }),
        }
    }
    if !oversample.is_empty() {
        // App rows first, then phone, microphone and audio rows, whatever
        // order the file has them in. A key without a label goes after the
        // labelled keys of its group.
        oversample.sort_by_key(|r| oversample_rank(&r.key));
        // Last, where Oversample writes them. Placing them at their first
        // row would split the standard rows that an export writes after them
        // (Filter HP, Filter LP) under a second "Guano metadata" heading.
        sections.push(GuanoSection {
            vendor: Some(OVERSAMPLE.to_string()),
            rows: oversample,
        });
    }
    sections
}

/// The label in [`OVERSAMPLE_FIELDS`] for `key`.
pub(super) fn oversample_label(key: &str) -> Option<&'static str> {
    OVERSAMPLE_FIELDS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, label)| *label)
}

/// The label for an `Oversample|*` key without an entry in
/// [`OVERSAMPLE_FIELDS`]: the rest of the key with spaces for the `|`
/// separators ("Mic|Input Gain" shows as "Mic Input Gain").
fn fallback_label(rest: &str) -> String {
    let parts: Vec<&str> = rest
        .split('|')
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() {
        rest.to_string()
    } else {
        parts.join(" ")
    }
}

/// Sort position of an `Oversample|*` key: its group's first entry in
/// [`OVERSAMPLE_FIELDS`], then its own entry. Keys without an entry go after
/// their group's entries, and keys of an unknown group go last.
fn oversample_rank(key: &str) -> (usize, usize) {
    let group = |k: &str| {
        k.strip_prefix(OVERSAMPLE_PREFIX)
            .and_then(|rest| rest.split('|').next())
            .map(str::to_string)
    };
    let g = group(key);
    let group_rank = OVERSAMPLE_FIELDS
        .iter()
        .position(|(k, _)| group(k) == g)
        .unwrap_or(OVERSAMPLE_FIELDS.len());
    let own = OVERSAMPLE_FIELDS
        .iter()
        .position(|(k, _)| *k == key)
        .unwrap_or(usize::MAX);
    (group_rank, own)
}

/// Returns (vendor, label) for a GUANO field that is not Oversample's.
/// Known fields have no vendor. Another pipe-separated key gets its prefix as
/// the vendor (e.g. "BatGizmo App") and its last part as the label.
fn categorize_guano_key(key: &str) -> (Option<String>, String) {
    let known = match key {
        // Pipe-separated variants
        "Loc|Lat" => Some("Latitude"),
        "Loc|Lon" => Some("Longitude"),
        "Loc|Elev" => Some("Elevation"),
        "Filter|HP" => Some("High-pass Filter"),
        "Filter|LP" => Some("Low-pass Filter"),
        "Species|Auto" => Some("Species (Auto)"),
        "Species|Manual" => Some("Species (Manual)"),
        // Standard GUANO space-separated field names
        "Loc Position" => Some("GPS Position"),
        "Loc Elevation" => Some("Elevation"),
        "Filter HP" => Some("High-pass Filter (kHz)"),
        "Filter LP" => Some("Low-pass Filter (kHz)"),
        "Species Auto ID" => Some("Species (Auto)"),
        "Species Manual ID" => Some("Species (Manual)"),
        "Temperature Int" => Some("Internal Temp"),
        "Temperature Ext" => Some("External Temp"),
        "Model" => Some("Model"),
        "Serial" => Some("Serial"),
        "Microphone" => Some("Microphone"),
        // Common fields
        "TE" => Some("Time Expansion"),
        "Samplerate" => Some("Sample Rate"),
        "Length" => Some("Length"),
        _ => None,
    };
    if let Some(display) = known {
        return (None, display.into());
    }
    if let Some(pos) = key.rfind('|') {
        let prefix = &key[..pos];
        let short = &key[pos + 1..];
        // "GUANO|Version" is the format's own field.
        let vendor = (prefix != "GUANO").then(|| prefix.replace('|', " "));
        (vendor, short.into())
    } else {
        (None, key.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oversample_core::audio::guano::{build_recording_guano, RecordingGuanoExtra};

    fn fields(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    /// Every `"Oversample|..."` string in the files that write GUANO.
    fn written_keys() -> Vec<String> {
        let sources = [
            include_str!("../../../oversample-core/src/audio/guano.rs"),
            include_str!("../../audio/export.rs"),
        ];
        let open = format!("\"{OVERSAMPLE_PREFIX}");
        let mut keys = Vec::new();
        for src in sources {
            for (at, _) in src.match_indices(&open) {
                let rest = &src[at + 1..];
                let key = &rest[..rest.find('"').unwrap_or(rest.len())];
                // `k.starts_with("Oversample|")` is a prefix, not a key.
                if !key.ends_with('|') {
                    keys.push(key.to_string());
                }
            }
        }
        // Both files write the app version.
        assert!(
            keys.iter()
                .filter(|k| *k == "Oversample|App|Version")
                .count()
                >= 2,
            "{keys:?}"
        );
        keys
    }

    #[test]
    fn every_key_oversample_writes_has_a_label() {
        let mut keys = written_keys();
        // A recording with every optional field set, on each platform.
        let extra = RecordingGuanoExtra {
            mic_interface: Some("USB (UAC2)".into()),
            mic_name: Some("Echo Meter Touch 2 Pro".into()),
            mic_audio_device: Some("Default".into()),
            mic_make: Some("Wildlife Acoustics".into()),
            loc_position: Some((-37.8, 144.9)),
            loc_elevation: Some(31.0),
            loc_accuracy: Some(4.5),
            device_make: Some("samsung".into()),
            device_model: Some("SM-A556E".into()),
            preroll_secs: Some(1.5),
        };
        for (tauri, mobile) in [(true, true), (true, false), (false, false)] {
            let g = build_recording_guano(
                192_000,
                1.0,
                "a.wav",
                tauri,
                mobile,
                &extra,
                "2026-11-27T14:57:52+10:00",
                "0.5.316",
            );
            keys.extend(
                g.fields
                    .into_iter()
                    .map(|(k, _)| k)
                    .filter(|k| k.starts_with(OVERSAMPLE_PREFIX)),
            );
        }
        assert!(keys.len() >= OVERSAMPLE_FIELDS.len(), "found {keys:?}");
        let missing: Vec<_> = keys
            .iter()
            .filter(|k| oversample_label(k).is_none())
            .collect();
        assert!(
            missing.is_empty(),
            "add these keys to OVERSAMPLE_FIELDS with a label: {missing:?}"
        );
    }

    #[test]
    fn labels_are_unique() {
        for (i, (_, a)) in OVERSAMPLE_FIELDS.iter().enumerate() {
            for (_, b) in &OVERSAMPLE_FIELDS[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn oversample_fields_are_one_section_in_table_order() {
        // An export: Filter rows after the Oversample rows, which are out of
        // order, and one key that this version has no label for.
        let s = guano_sections(
            &fields(&[
                ("GUANO|Version", "1.0"),
                ("Original Filename", "batcap_20261127_145752.wav"),
                ("Oversample|App|Version", "0.5.316"),
                ("Oversample|Mic|Name", "Internal"),
                ("Oversample|Mic|Input Gain", "12 dB"),
                ("Oversample|Mic|Interface", "ALSA"),
                ("Oversample|App|Platform", "Desktop"),
                ("Oversample|Future|Thing", "x"),
                ("Oversample|Audio|Preroll", "0.500"),
                ("Filter HP", "15.0"),
            ]),
            true,
        );
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].vendor, None);
        let main: Vec<_> = s[0].rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            main,
            ["Version", "Original Filename", "High-pass Filter (kHz)"]
        );
        assert_eq!(s[1].vendor.as_deref(), Some("Oversample"));
        let rows: Vec<_> = s[1]
            .rows
            .iter()
            .map(|r| (r.label.as_str(), r.value.as_str(), r.inline))
            .collect();
        assert_eq!(
            rows,
            [
                ("App version", "0.5.316", true),
                ("Platform", "Desktop", true),
                ("Audio API", "ALSA", true),
                ("Mic name", "Internal", true),
                ("Mic Input Gain", "12 dB", true),
                ("Pre-roll (s)", "0.500", true),
                ("Future Thing", "x", true),
            ]
        );
        assert_eq!(s[1].rows[0].key, "Oversample|App|Version");
    }

    #[test]
    fn other_vendors_keep_their_headings() {
        let s = guano_sections(
            &fields(&[
                ("Make", "Pettersson"),
                ("PET|TIMEZONE", "+01"),
                ("PET|Profile REC LEN", "5.0"),
                ("WA|Song Meter|Audio settings", "{}"),
                ("Samplerate", "500000"),
            ]),
            true,
        );
        let heads: Vec<_> = s
            .iter()
            .map(|s| (s.vendor.as_deref(), s.rows.len()))
            .collect();
        assert_eq!(
            heads,
            [
                (None, 1),
                (Some("PET"), 2),
                (Some("WA Song Meter"), 1),
                (None, 1)
            ]
        );
        assert_eq!(s[1].rows[1].label, "Profile REC LEN");
        assert_eq!(s[3].rows[0].label, "Sample Rate");
        assert!(s.iter().flat_map(|s| &s.rows).all(|r| !r.inline));
    }

    #[test]
    fn tags_that_are_not_guano_keep_their_keys() {
        let s = guano_sections(&fields(&[("Oversample|App|Version", "0.5.316")]), false);
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].vendor, None);
        assert_eq!(s[0].rows[0].label, "Oversample|App|Version");
        assert!(guano_sections(&[], true).is_empty());
    }
}
