//! Warns when a recording is probably a heterodyne bat detector's audio
//! output, whose frequencies are not the bat's call frequencies.
//!
//! [`file_hint`] reads a loaded file's site metadata (Xeno-canto's remarks,
//! device, microphone and method, or Wikimedia Commons' description) and asks
//! [`oversample_core::heterodyne_hint::infer_heterodyne_hint`]. The result is
//! stored on the `LoadedFile` when the file is built, because the file list
//! rebuilds every row whenever any file changes.
//!
//! [`note_lines`] turns a hint into the lines shown in two places: the badge
//! tooltip (levels A and B) and the note under the Info panel's Sample rate row
//! (all levels).

use crate::types::TimeExpansion;
use oversample_core::heterodyne_hint::{infer_heterodyne_hint, HeterodyneSource, RecordingText};
pub use oversample_core::heterodyne_hint::{HeterodyneHint, HeterodyneLevel, TextField};

/// The site metadata label each text field is read from. The Info panel shows
/// these labels unchanged (`SourceSite::field_label` renames none of them), so
/// the evidence line names the field the way the panel does.
const FIELD_LABELS: [(TextField, &str); 5] = [
    (TextField::Remarks, "Remarks"),
    (TextField::Device, "Device"),
    (TextField::Microphone, "Microphone"),
    (TextField::Method, "Method"),
    (TextField::Description, "Description"),
];

/// The Info panel label of a text field.
pub fn field_label(field: TextField) -> &'static str {
    FIELD_LABELS
        .iter()
        .find(|(f, _)| *f == field)
        .map(|(_, label)| *label)
        .unwrap_or("Remarks")
}

/// The heterodyne hint for a file, from its site metadata rows.
///
/// - `fields`: the site metadata label/value rows (`SiteMetadata::fields`).
/// - `time_expansion`: the correction the app applied when it loaded the file.
///   A factor other than 1 gives `None`: the file is then known to be
///   time-expanded, and the correction already accounts for its frequencies.
/// - `sample_rate`: the file's rate as loaded. The rule needs the rate in the
///   file header, which is `time_expansion.stored_rate` when a correction was
///   applied and `sample_rate` otherwise.
pub fn file_hint(
    fields: &[(String, String)],
    time_expansion: Option<TimeExpansion>,
    sample_rate: u32,
) -> Option<HeterodyneHint> {
    if time_expansion.is_some_and(|te| te.factor != 1.0) {
        return None;
    }
    let header_rate = time_expansion.map_or(sample_rate, |te| te.stored_rate);
    let read = |field: TextField| -> Option<String> {
        let label = field_label(field);
        fields
            .iter()
            .find(|(key, _)| key == label)
            .map(|(_, value)| nfc(value))
    };
    let remarks = read(TextField::Remarks);
    let device = read(TextField::Device);
    let microphone = read(TextField::Microphone);
    let method = read(TextField::Method);
    let description = read(TextField::Description);
    let text = RecordingText {
        remarks: remarks.as_deref(),
        device: device.as_deref(),
        microphone: microphone.as_deref(),
        method: method.as_deref(),
        description: description.as_deref(),
    };
    infer_heterodyne_hint(&text, Some(header_rate))
}

/// The text in Unicode normalization form C, which the word lists in
/// `heterodyne_hint` need. ASCII text is already NFC.
///
/// Uses JavaScript's `String.prototype.normalize`. Native builds only run the
/// unit tests, which pass NFC text, so there the text is returned unchanged.
fn nfc(text: &str) -> String {
    if text.is_ascii() {
        return text.to_string();
    }
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::JsString::from(text).normalize("NFC").into()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        text.to_string()
    }
}

/// Whether the hint gets the "Heterodyne recording" badge: levels A and B.
/// Level C is shown in the Info panel only.
pub fn shows_badge(hint: &HeterodyneHint) -> bool {
    hint.level >= HeterodyneLevel::HeterodyneOnlyDetector
}

/// How a line of the note is styled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineRole {
    /// The finding and its consequence, in the warning colour.
    Warning,
    /// The evidence, muted.
    Detail,
}

/// One line of the heterodyne note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteLine {
    pub text: String,
    pub role: LineRole,
}

/// The lines that explain a hint. The badge tooltip joins them with newlines
/// and the Info panel shows them as a note.
pub fn note_lines(hint: &HeterodyneHint) -> Vec<NoteLine> {
    let warning = |text: String| NoteLine {
        text,
        role: LineRole::Warning,
    };
    let detail = |text: String| NoteLine {
        text,
        role: LineRole::Detail,
    };
    let evidence = detail(format!("{}: \"{}\"", field_label(hint.field), hint.excerpt));
    match hint.level {
        HeterodyneLevel::SaysHeterodyne => vec![
            warning("Recorded from a heterodyne bat detector's audio output.".into()),
            evidence,
            warning(
                "Frequencies are not the bat's call frequencies and can't be corrected.".into(),
            ),
        ],
        HeterodyneLevel::HeterodyneOnlyDetector => {
            let detector = hint.detector.unwrap_or(hint.matched.as_str());
            vec![
                warning("Recorded from a heterodyne bat detector's audio output.".into()),
                evidence,
                detail(format!("The {detector} is a heterodyne-only detector.")),
                warning(
                    "Frequencies are not the bat's call frequencies and can't be corrected.".into(),
                ),
            ]
        }
        // Level C is a possibility, not a finding, so every line is muted.
        HeterodyneLevel::MayBeHeterodyne => {
            let mut lines = vec![
                detail(
                    "Possibly recorded from a bat detector's output \
                     (heterodyne, time expansion or frequency division)."
                        .into(),
                ),
                evidence,
            ];
            // Level C also comes from a heterodyne-only detector named beside
            // time-expansion or full-spectrum wording, so the detector line
            // follows the rule that named it.
            match (hint.source, hint.detector) {
                (HeterodyneSource::DualModeDetector, Some(detector)) => {
                    lines.push(detail(format!(
                        "The {detector} has a heterodyne mode and other modes."
                    )));
                }
                (HeterodyneSource::HeterodyneOnlyDetector, Some(detector)) => {
                    lines.push(detail(format!(
                        "The {detector} is a heterodyne-only detector."
                    )));
                }
                _ => {}
            }
            lines.push(detail(
                "If so, frequencies are not the bat's call frequencies.".into(),
            ));
            lines
        }
    }
}

/// The badge tooltip: the note's lines, one per line.
pub fn tooltip(hint: &HeterodyneHint) -> String {
    note_lines(hint)
        .into_iter()
        .map(|line| line.text)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    fn texts(hint: &HeterodyneHint) -> Vec<String> {
        note_lines(hint).into_iter().map(|l| l.text).collect()
    }

    fn roles(hint: &HeterodyneHint) -> Vec<LineRole> {
        note_lines(hint).into_iter().map(|l| l.role).collect()
    }

    use LineRole::{Detail, Warning};

    #[test]
    fn level_a_lines() {
        let hint = file_hint(
            &rows(&[("Remarks", "handheld hetrodyne detector set at 31,9 kHz")]),
            None,
            44_100,
        )
        .unwrap();
        assert_eq!(hint.level, HeterodyneLevel::SaysHeterodyne);
        assert!(shows_badge(&hint));
        assert_eq!(
            texts(&hint),
            [
                "Recorded from a heterodyne bat detector's audio output.",
                "Remarks: \"handheld hetrodyne detector set at 31,9 kHz\"",
                "Frequencies are not the bat's call frequencies and can't be corrected.",
            ]
        );
        assert_eq!(roles(&hint), [Warning, Detail, Warning]);
        assert_eq!(tooltip(&hint), texts(&hint).join("\n"));
    }

    #[test]
    fn level_b_lines_name_the_detector() {
        let hint = file_hint(
            &rows(&[
                ("Method", "field recording"),
                ("Remarks", "Recording with Magenta Bat5"),
            ]),
            None,
            48_000,
        )
        .unwrap();
        assert_eq!(hint.level, HeterodyneLevel::HeterodyneOnlyDetector);
        assert!(shows_badge(&hint));
        assert_eq!(
            texts(&hint),
            [
                "Recorded from a heterodyne bat detector's audio output.",
                "Remarks: \"Recording with Magenta Bat5\"",
                "The Magenta Bat4/Bat5 is a heterodyne-only detector.",
                "Frequencies are not the bat's call frequencies and can't be corrected.",
            ]
        );
        assert_eq!(roles(&hint), [Warning, Detail, Detail, Warning]);
    }

    #[test]
    fn level_b_without_a_detector_label_names_the_matched_words() {
        let hint = HeterodyneHint {
            level: HeterodyneLevel::HeterodyneOnlyDetector,
            source: HeterodyneSource::HeterodyneOnlyDetector,
            detector: None,
            field: TextField::Device,
            excerpt: "Bat5 detector".into(),
            matched: "Bat5".into(),
        };
        assert_eq!(
            texts(&hint)[1..3],
            [
                "Device: \"Bat5 detector\"",
                "The Bat5 is a heterodyne-only detector."
            ]
        );
    }

    #[test]
    fn level_c_lines_are_all_muted() {
        let hint = file_hint(&rows(&[("Device", "Pettersson D980")]), None, 44_100).unwrap();
        assert_eq!(hint.level, HeterodyneLevel::MayBeHeterodyne);
        assert!(!shows_badge(&hint));
        let detector = hint.detector.expect("a dual-mode detector label");
        assert_eq!(
            texts(&hint),
            [
                "Possibly recorded from a bat detector's output \
                 (heterodyne, time expansion or frequency division)."
                    .to_string(),
                "Device: \"Pettersson D980\"".to_string(),
                format!("The {detector} has a heterodyne mode and other modes."),
                "If so, frequencies are not the bat's call frequencies.".to_string(),
            ]
        );
        assert_eq!(roles(&hint), [Detail; 4]);
    }

    #[test]
    fn level_c_without_a_detector_leaves_out_the_detector_line() {
        let hint = HeterodyneHint {
            level: HeterodyneLevel::MayBeHeterodyne,
            source: HeterodyneSource::DetectorOutput,
            detector: None,
            field: TextField::Remarks,
            excerpt: "detectors speaker with mobilphone".into(),
            matched: "speaker".into(),
        };
        assert_eq!(
            texts(&hint),
            [
                "Possibly recorded from a bat detector's output \
                 (heterodyne, time expansion or frequency division).",
                "Remarks: \"detectors speaker with mobilphone\"",
                "If so, frequencies are not the bat's call frequencies.",
            ]
        );
    }

    #[test]
    fn level_c_from_a_heterodyne_only_detector_does_not_call_it_dual_mode() {
        // Time-expansion wording lowers a heterodyne-only detector to level C.
        let hint = file_hint(
            &rows(&[("Remarks", "Recorded with a Magenta Bat5, time expanded 10x")]),
            None,
            44_100,
        )
        .unwrap();
        assert_eq!(hint.level, HeterodyneLevel::MayBeHeterodyne);
        assert_eq!(hint.source, HeterodyneSource::HeterodyneOnlyDetector);
        assert_eq!(
            texts(&hint)[2],
            "The Magenta Bat4/Bat5 is a heterodyne-only detector."
        );
        assert_eq!(roles(&hint), [Detail; 4]);
    }

    #[test]
    fn level_c_from_a_heterodyne_word_has_no_detector_line() {
        let hint = file_hint(
            &rows(&[("Remarks", "Heterodyne, time-expanded")]),
            None,
            44_100,
        )
        .unwrap();
        assert_eq!(hint.level, HeterodyneLevel::MayBeHeterodyne);
        assert_eq!(texts(&hint).len(), 3);
    }

    #[test]
    fn wikimedia_description_is_read() {
        let hint = file_hint(
            &rows(&[("Description", "Pipistrelle through a heterodyne detector")]),
            None,
            44_100,
        )
        .unwrap();
        assert_eq!(hint.field, TextField::Description);
        assert_eq!(
            texts(&hint)[1],
            "Description: \"Pipistrelle through a heterodyne detector\""
        );
    }

    #[test]
    fn no_hint_when_time_expansion_was_applied() {
        let fields = rows(&[("Remarks", "handheld hetrodyne detector set at 31,9 kHz")]);
        let te = TimeExpansion {
            factor: 10.0,
            stored_rate: 44_100,
        };
        assert_eq!(file_hint(&fields, Some(te), 441_000), None);
    }

    #[test]
    fn no_hint_above_96_khz_or_without_text() {
        let fields = rows(&[("Remarks", "Recording with Magenta Bat5")]);
        assert_eq!(file_hint(&fields, None, 192_000), None);
        assert_eq!(
            file_hint(&rows(&[("Remarks", "Recorded at dusk")]), None, 44_100),
            None
        );
        assert_eq!(file_hint(&[], None, 44_100), None);
    }

    #[test]
    fn other_labels_are_not_read() {
        // "Species" and "Location" are not text fields the rules read.
        let fields = rows(&[("Location", "Heterodyne Lane"), ("Species", "Bat5")]);
        assert_eq!(file_hint(&fields, None, 44_100), None);
    }
}
