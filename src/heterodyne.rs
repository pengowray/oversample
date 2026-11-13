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
//!
//! When the text gives the detector's tuning frequency
//! ([`HeterodyneHint::tuned_hz`]), the note says how to read the file's
//! frequencies, and [`axis_call_suffix`] gives the frequency axis's hover text:
//! the two call frequencies a sound at each label came from.

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
    /// How to read the file's frequencies when the tuning is known, in the
    /// panel's normal text colour: it is a reading rule, not another warning.
    Plain,
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
    // With the tuning known, the call frequencies can be worked out (one of
    // two per sound), so "can't be corrected" is left out and a line saying
    // how to read the file follows.
    let consequence = || match hint.tuned_hz {
        Some(_) => warning("Frequencies are not the bat's call frequencies.".into()),
        None => {
            warning("Frequencies are not the bat's call frequencies and can't be corrected.".into())
        }
    };
    let reading = || {
        hint.tuned_hz.map(|tuned| NoteLine {
            text: tuned_line(tuned),
            role: LineRole::Plain,
        })
    };
    match hint.level {
        HeterodyneLevel::SaysHeterodyne => {
            let mut lines = vec![
                warning("Recorded from a heterodyne bat detector's audio output.".into()),
                evidence,
                consequence(),
            ];
            lines.extend(reading());
            lines
        }
        // The core gives a tuning only at level A, but a level B hint with
        // one would read the same way.
        HeterodyneLevel::HeterodyneOnlyDetector => {
            let detector = detector_name(hint).unwrap_or_else(|| hint.matched.clone());
            let mut lines = vec![
                warning("Recorded from a heterodyne bat detector's audio output.".into()),
                evidence,
                detail(format!("The {detector} is a heterodyne-only detector.")),
                consequence(),
            ];
            lines.extend(reading());
            lines
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
            match (hint.source, detector_name(hint)) {
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

/// The detector to name in the note. The core labels some detectors as a
/// group ("Magenta Bat4/Bat5", "Pettersson D230/D240x/D980/D1000X"); when the
/// quoted text names one model of the group, the note names that model
/// ("Magenta Bat5"), so it never lists models the recordist didn't mention.
/// A group label stays as it is when the text names none of its models.
fn detector_name(hint: &HeterodyneHint) -> Option<String> {
    let label = hint.detector?;
    let Some(slash) = label.find('/') else {
        return Some(label.to_string());
    };
    // "Magenta Bat4/Bat5" -> maker "Magenta", models ["Bat4", "Bat5"].
    let maker_end = label[..slash].rfind(' ')?;
    let maker = &label[..maker_end];
    let models: Vec<&str> = label[maker_end + 1..].split('/').collect();
    // Letters and digits only, lower case, so "Bat 5" matches "Bat5".
    let key = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_alphanumeric())
            .flat_map(char::to_lowercase)
            .collect()
    };
    // Read from the matched words on, so a model named earlier in the
    // excerpt doesn't count.
    let from = hint.excerpt.find(hint.matched.as_str()).unwrap_or(0);
    let text = key(&hint.excerpt[from..]);
    // The model named first; the longer name wins at the same position
    // ("D1000X" over "D1000").
    let model = models
        .iter()
        .filter_map(|m| {
            text.find(&key(m))
                .map(|at| (at, std::cmp::Reverse(m.len()), *m))
        })
        .min()
        .map(|(_, _, m)| m.to_string());
    // The text may name a shorter form of a model: "D240" for "D240x",
    // "D1000" for "D1000X". Then the note uses the text's own words.
    let short_form = || {
        let words = hint.matched.trim();
        let words_key = key(words);
        (!words_key.is_empty()
            && !words_key.starts_with(&key(maker))
            && models.iter().any(|m| key(m).starts_with(&words_key)))
        .then(|| words.to_string())
    };
    Some(match model.or_else(short_form) {
        Some(m) => format!("{maker} {m}"),
        None => label.to_string(),
    })
}

/// The note's line for a detector tuned to `tuned_hz`, with a worked example
/// at 10 kHz: "Detector tuned to 31.9 kHz: a sound at 10 kHz in this file came
/// from a call at either 21.9 or 41.9 kHz." 10 kHz is also a frequency axis
/// label, so its hover text shows the same numbers. A detector tuned to 10 kHz
/// or lower uses 5 kHz, so the lower call frequency stays above 0.
pub fn tuned_line(tuned_hz: u32) -> String {
    let tuned = i64::from(tuned_hz);
    let example = if tuned <= 10_000 { 5_000 } else { 10_000 };
    // Two decimals, so a tuning given as "31,95 kHz" is shown as written.
    let khz = |hz: i64| format_khz(hz, 2);
    format!(
        "Detector tuned to {} kHz: a sound at {} kHz in this file came from a call at \
         either {} or {} kHz.",
        khz(tuned),
        khz(example),
        khz(tuned - example),
        khz(tuned + example),
    )
}

/// Longest frequency axis hover label, in characters, that keeps its " kHz".
/// "10 → 21.9 or 41.9 kHz" is 21 characters; a longer label, such as
/// "15 → 97.5 or 127.5", drops the unit so it stays about as wide.
const AXIS_LABEL_MAX_CHARS: usize = 21;

/// The hover text after a frequency axis label when the file was recorded
/// from a heterodyne detector tuned to `tuned_hz`: the call frequencies that a
/// sound at the label's frequency (`freq_hz`) came from. Tuned to 31.9 kHz,
/// the 10 kHz label gets " → 21.9 or 41.9 kHz". The lower call frequency is
/// left out when it is 0 or less ("35 → 66.9 kHz"), and the unit is left out
/// when the whole label would be longer than [`AXIS_LABEL_MAX_CHARS`]. The
/// label itself is [`freq_marker_label`], which the renderer draws first.
///
/// [`freq_marker_label`]: crate::canvas::colors::freq_marker_label
pub fn axis_call_suffix(freq_hz: f64, tuned_hz: u32) -> String {
    let freq = freq_hz.round() as i64;
    let tuned = i64::from(tuned_hz);
    let khz = |hz: i64| format_khz(hz, 1);
    // A value that shows as 0 kHz is left out with the negative ones.
    let lower = (tuned - freq >= 50).then(|| tuned - freq);
    let upper = tuned + freq;
    let values = match lower {
        Some(lower) if lower != upper => format!("{} or {}", khz(lower), khz(upper)),
        _ => khz(upper),
    };
    let label = crate::canvas::colors::freq_marker_label(freq_hz);
    let with_unit = format!(" \u{2192} {values} kHz");
    if label.chars().count() + with_unit.chars().count() <= AXIS_LABEL_MAX_CHARS {
        with_unit
    } else {
        format!(" \u{2192} {values}")
    }
}

/// A positive frequency in Hz as kHz, rounded to at most `decimals` (1 or 2)
/// decimal places, with no trailing zeros: 21_900 gives "21.9", 40_000 gives
/// "40" and, with 2 decimals, 31_950 gives "31.95".
fn format_khz(hz: i64, decimals: u32) -> String {
    let step = 10_i64.pow(3 - decimals);
    let units = (hz.max(0) + step / 2) / step;
    let per_khz = 10_i64.pow(decimals);
    let (whole, frac) = (units / per_khz, units % per_khz);
    if frac == 0 {
        return whole.to_string();
    }
    let frac = format!("{frac:0width$}", width = decimals as usize);
    format!("{whole}.{}", frac.trim_end_matches('0'))
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

    use LineRole::{Detail, Plain, Warning};

    #[test]
    fn level_a_lines() {
        let hint = file_hint(
            &rows(&[("Remarks", "Recorded with a handheld hetrodyne detector")]),
            None,
            44_100,
        )
        .unwrap();
        assert_eq!(hint.level, HeterodyneLevel::SaysHeterodyne);
        assert_eq!(hint.tuned_hz, None);
        assert!(shows_badge(&hint));
        assert_eq!(
            texts(&hint),
            [
                "Recorded from a heterodyne bat detector's audio output.",
                "Remarks: \"Recorded with a handheld hetrodyne detector\"",
                "Frequencies are not the bat's call frequencies and can't be corrected.",
            ]
        );
        assert_eq!(roles(&hint), [Warning, Detail, Warning]);
        assert_eq!(tooltip(&hint), texts(&hint).join("\n"));
    }

    #[test]
    fn level_a_lines_with_the_tuning() {
        let hint = file_hint(
            &rows(&[("Remarks", "handheld hetrodyne detector set at 31,9 kHz")]),
            None,
            44_100,
        )
        .unwrap();
        assert_eq!(hint.tuned_hz, Some(31_900));
        assert_eq!(
            texts(&hint),
            [
                "Recorded from a heterodyne bat detector's audio output.",
                "Remarks: \"handheld hetrodyne detector set at 31,9 kHz\"",
                "Frequencies are not the bat's call frequencies.",
                "Detector tuned to 31.9 kHz: a sound at 10 kHz in this file came from a call \
                 at either 21.9 or 41.9 kHz.",
            ]
        );
        assert_eq!(roles(&hint), [Warning, Detail, Warning, Plain]);
        assert_eq!(tooltip(&hint), texts(&hint).join("\n"));
    }

    #[test]
    fn level_b_lines_with_a_tuning_keep_the_detector_line() {
        // The core gives a tuning only at level A; this hint is built by hand.
        let hint = HeterodyneHint {
            level: HeterodyneLevel::HeterodyneOnlyDetector,
            source: HeterodyneSource::HeterodyneOnlyDetector,
            detector: Some("Magenta Bat4/Bat5"),
            field: TextField::Remarks,
            excerpt: "Recording with Magenta Bat5".into(),
            matched: "Magenta Bat5".into(),
            tuned_hz: Some(45_000),
        };
        assert_eq!(
            texts(&hint),
            [
                "Recorded from a heterodyne bat detector's audio output.",
                "Remarks: \"Recording with Magenta Bat5\"",
                "The Magenta Bat5 is a heterodyne-only detector.",
                "Frequencies are not the bat's call frequencies.",
                "Detector tuned to 45 kHz: a sound at 10 kHz in this file came from a call \
                 at either 35 or 55 kHz.",
            ]
        );
        assert_eq!(roles(&hint), [Warning, Detail, Detail, Warning, Plain]);
    }

    #[test]
    fn tuned_line_examples() {
        assert_eq!(
            tuned_line(23_500),
            "Detector tuned to 23.5 kHz: a sound at 10 kHz in this file came from a call \
             at either 13.5 or 33.5 kHz."
        );
        // Two decimals, as the remark gave them.
        assert_eq!(
            tuned_line(31_950),
            "Detector tuned to 31.95 kHz: a sound at 10 kHz in this file came from a call \
             at either 21.95 or 41.95 kHz."
        );
        // Tuned to 10 kHz or lower: a 5 kHz example keeps both calls above 0.
        assert_eq!(
            tuned_line(10_000),
            "Detector tuned to 10 kHz: a sound at 5 kHz in this file came from a call \
             at either 5 or 15 kHz."
        );
        assert!(tuned_line(10_500).contains("a sound at 10 kHz"));
    }

    #[test]
    fn axis_labels_give_the_call_frequencies() {
        let label = |freq_khz: f64, tuned_hz: u32| {
            let freq = freq_khz * 1000.0;
            format!(
                "{}{}",
                crate::canvas::colors::freq_marker_label(freq),
                axis_call_suffix(freq, tuned_hz)
            )
        };
        // Below the tuning: both call frequencies, lower first.
        assert_eq!(label(5.0, 31_900), "5 \u{2192} 26.9 or 36.9 kHz");
        assert_eq!(label(10.0, 31_900), "10 \u{2192} 21.9 or 41.9 kHz");
        assert_eq!(label(15.0, 31_900), "15 \u{2192} 16.9 or 46.9 kHz");
        assert_eq!(label(20.0, 31_900), "20 \u{2192} 11.9 or 51.9 kHz");
        assert_eq!(label(30.0, 31_900), "30 \u{2192} 1.9 or 61.9 kHz");
        assert_eq!(label(5.0, 45_000), "5 \u{2192} 40 or 50 kHz");
        // At or above the tuning: only the upper call frequency.
        assert_eq!(label(35.0, 31_900), "35 \u{2192} 66.9 kHz");
        assert_eq!(label(45.0, 45_000), "45 \u{2192} 90 kHz");
        assert_eq!(label(50.0, 45_000), "50 \u{2192} 95 kHz");
        // 0 kHz came from a call at the tuning frequency.
        assert_eq!(label(0.0, 31_900), "0 \u{2192} 31.9 kHz");
        // "10 → 21.9 or 41.9 kHz" (21 characters) keeps the unit; a longer
        // label drops it.
        assert_eq!(label(10.0, 31_900).chars().count(), AXIS_LABEL_MAX_CHARS);
        assert_eq!(label(15.0, 112_500), "15 \u{2192} 97.5 or 127.5");
        // One decimal at most: 31.95 kHz rounds to 32.
        assert_eq!(label(10.0, 31_950), "10 \u{2192} 22 or 42 kHz");
        // A lower call frequency that rounds to 0 is left out.
        assert_eq!(label(31.0, 31_040), "31 \u{2192} 62 kHz");
    }

    #[test]
    fn khz_formatting() {
        assert_eq!(format_khz(21_900, 1), "21.9");
        assert_eq!(format_khz(40_000, 1), "40");
        assert_eq!(format_khz(1_900, 1), "1.9");
        assert_eq!(format_khz(31_950, 1), "32");
        assert_eq!(format_khz(31_950, 2), "31.95");
        assert_eq!(format_khz(31_905, 2), "31.91");
        assert_eq!(format_khz(31_050, 2), "31.05");
        assert_eq!(format_khz(31_900, 2), "31.9");
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
                "The Magenta Bat5 is a heterodyne-only detector.",
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
            tuned_hz: None,
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
        assert!(hint.detector.is_some(), "a dual-mode detector label");
        assert_eq!(
            texts(&hint),
            [
                "Possibly recorded from a bat detector's output \
                 (heterodyne, time expansion or frequency division)."
                    .to_string(),
                "Device: \"Pettersson D980\"".to_string(),
                "The Pettersson D980 has a heterodyne mode and other modes.".to_string(),
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
            tuned_hz: None,
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
            "The Magenta Bat5 is a heterodyne-only detector."
        );
        assert_eq!(roles(&hint), [Detail; 4]);
    }

    #[test]
    fn the_note_names_the_model_the_text_names() {
        let line = |field: &str, value: &str, rate: u32| {
            let hint = file_hint(&rows(&[(field, value)]), None, rate).unwrap();
            texts(&hint)
                .into_iter()
                .find(|t| t.starts_with("The "))
                .unwrap()
        };
        assert_eq!(
            line("Device", "Pettersson D980", 44_100),
            "The Pettersson D980 has a heterodyne mode and other modes."
        );
        assert_eq!(
            line("Device", "Pettersson D1000X", 44_100),
            "The Pettersson D1000X has a heterodyne mode and other modes."
        );
        assert_eq!(
            line("Microphone", "Magenta Bat 4 …", 48_000),
            "The Magenta Bat4 is a heterodyne-only detector."
        );
        assert_eq!(
            line("Device", "SSF BAT3", 48_000),
            "The SSF BAT3 is a heterodyne-only detector."
        );
        // A shorter form of a model in the group: the text's own words.
        assert_eq!(
            line("Device", "Pettersson D240", 44_100),
            "The Pettersson D240 has a heterodyne mode and other modes."
        );
        assert_eq!(
            line("Device", "Pettersson D1000", 44_100),
            "The Pettersson D1000 has a heterodyne mode and other modes."
        );
        // A model number later in the remark, outside the quoted clause,
        // doesn't count.
        assert_eq!(
            line(
                "Remarks",
                "Recorded with a Magenta bat detector. Bat 5 m overhead",
                48_000
            ),
            "The Magenta Bat4/Bat5 is a heterodyne-only detector."
        );
        // No model of the group in the text: the group label stays.
        assert_eq!(
            line("Remarks", "Recorded with a Magenta bat detector", 48_000),
            "The Magenta Bat4/Bat5 is a heterodyne-only detector."
        );
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
