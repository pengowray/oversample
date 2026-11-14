//! The notes under the Info panel's Sample rate row.
//!
//! A file can have up to two notes, in this order:
//!
//! 1. The time-expansion note, when the app corrected the file's rate for
//!    time expansion as it loaded the file.
//! 2. The heterodyne note ([`crate::heterodyne`]) or the species call-range
//!    note ([`crate::species_check`]), never both.
//!
//! A note can have one button ([`NoteAction`]), drawn under its first line:
//! the species note of a probably time-expanded file has one that reads the
//! file at ten times its rate, and the time-expansion note that replaces it
//! has one that goes back to the file's own rate ([`crate::te_correction`]).
//!
//! [`file_rate_notes`] gives a file's notes. [`rate_notes`] is the same rule
//! on plain values, for unit tests. The `RateNotes` component
//! (`components/file_sidebar/rate_notes_view.rs`) draws them, and the
//! "Heterodyne recording" badge's tooltip is the heterodyne note's lines
//! ([`crate::note_line::lines_text`]). Each line of a note is a
//! [`NoteLine`], from [`crate::note_line`].

use crate::format_units::sample_rate_khz;
use crate::heterodyne::{HeterodyneHint, HeterodyneLevel};
use crate::note_line::NoteLine;
use crate::species_check::SpeciesRangeFinding;
use crate::state::LoadedFile;
use crate::types::{TimeExpansion, TimeExpansionOrigin};

/// Which note a [`RateNote`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteKind {
    TimeExpansion,
    Heterodyne,
    Species,
}

impl NoteKind {
    /// The kind's name in the note's `data-note` attribute, which
    /// `open_info_panel` and the Playwright specs use to find a note.
    pub fn key(self) -> &'static str {
        match self {
            NoteKind::TimeExpansion => "time-expansion",
            NoteKind::Heterodyne => "heterodyne",
            NoteKind::Species => "species",
        }
    }
}

/// One note under the Sample rate row. It has at least one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RateNote {
    pub kind: NoteKind,
    pub lines: Vec<NoteLine>,
    /// The note's button, drawn under its first line: the line the button
    /// acts on.
    pub action: Option<NoteAction>,
}

/// A button on a note that changes the rate the file is read at
/// ([`crate::te_correction::set_user_time_expansion`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteAction {
    /// On the species note's "Probably time-expanded ×10" line: read the
    /// file at ten times the rate it states.
    TreatAsTenfold,
    /// On the time-expansion note of a correction the user applied: read the
    /// file at the rate it states again.
    UseFileRate,
}

impl NoteAction {
    /// The button's label. Each repeats the words of the line above it, so
    /// the button reads as accepting or reversing what that line says.
    pub fn label(self) -> &'static str {
        match self {
            NoteAction::TreatAsTenfold => "Treat as time-expanded \u{d7}10",
            NoteAction::UseFileRate => "Use the file's sample rate",
        }
    }

    /// The button's tooltip, for a file whose header states `stored_rate`.
    /// "Use the file's sample rate" has none: the line above it names the
    /// file's rate.
    pub fn tooltip(self, stored_rate: u32) -> Option<String> {
        match self {
            NoteAction::TreatAsTenfold => Some(format!(
                "Uses {} as the sample rate. The file is not changed.",
                sample_rate_khz(stored_rate.saturating_mul(10))
            )),
            NoteAction::UseFileRate => None,
        }
    }

    /// The factor the button sets: ×10, or `None` for the file's own rate.
    pub fn factor(self) -> Option<f64> {
        match self {
            NoteAction::TreatAsTenfold => Some(crate::te_correction::TENFOLD),
            NoteAction::UseFileRate => None,
        }
    }

    /// The action's name in the button's `data-action` attribute, which the
    /// Playwright specs use to find it.
    pub fn key(self) -> &'static str {
        match self {
            NoteAction::TreatAsTenfold => "treat-as-tenfold",
            NoteAction::UseFileRate => "use-file-rate",
        }
    }
}

/// The time-expansion note's line: "Corrected for ×10 time expansion; the
/// file says 38.4 kHz." A bat detector stored the recording slowed down so it
/// can be heard, and its rate and duration were corrected: by the app when it
/// loaded the file, or by the user. The rate in the line is the one in the
/// file's header.
pub fn time_expansion_line(te: TimeExpansion) -> NoteLine {
    NoteLine::muted(format!(
        "Corrected for {} time expansion; the file says {}.",
        crate::wikimedia::format_factor(te.factor),
        sample_rate_khz(te.stored_rate)
    ))
}

/// The notes under a file's Sample rate row, in the order they are shown.
pub fn file_rate_notes(file: &LoadedFile) -> Vec<RateNote> {
    rate_notes(
        file.audio.metadata.time_expansion,
        file.heterodyne_hint.as_ref(),
        file.species_range.as_ref(),
        file.audio.can_set_time_expansion(),
    )
}

/// The notes for a file's time expansion, heterodyne hint and species
/// finding, in the order they are shown.
///
/// - `time_expansion`: the correction applied to the file's rate. It gives
///   the time-expansion note, which comes first. When the user applied it,
///   the note has the "Use the file's sample rate" button.
/// - `can_correct`: whether the app can change the rate the file is read at
///   ([`crate::types::AudioData::can_set_time_expansion`]): false for a
///   streamed file. When true, the species note of a file at a ×10 header
///   rate ([`crate::species_check::RangeCase::TenfoldHeaderRate`]) has the
///   "Treat as time-expanded ×10" button.
/// - Heterodyne levels A and B: the heterodyne note without the species
///   line. The note already says the frequencies are not the bat's.
/// - Heterodyne level C: the heterodyne note, then the species line, muted
///   like the rest of a level C note. The species line comes last so that
///   the note's "If so, ..." line still follows the "Possibly recorded from
///   ..." line it refers to.
/// - No hint: the species note, when there is a finding.
pub fn rate_notes(
    time_expansion: Option<TimeExpansion>,
    hint: Option<&HeterodyneHint>,
    finding: Option<&SpeciesRangeFinding>,
    can_correct: bool,
) -> Vec<RateNote> {
    let mut notes = Vec::new();
    if let Some(te) = time_expansion {
        notes.push(RateNote {
            kind: NoteKind::TimeExpansion,
            lines: vec![time_expansion_line(te)],
            action: (te.origin == TimeExpansionOrigin::User).then_some(NoteAction::UseFileRate),
        });
    }
    match (hint, finding) {
        (Some(hint), _) => {
            let mut lines = crate::heterodyne::note_lines(hint);
            if hint.level == HeterodyneLevel::MayBeHeterodyne {
                lines.extend(finding.map(crate::species_check::fact_line));
            }
            notes.push(RateNote {
                kind: NoteKind::Heterodyne,
                lines,
                action: None,
            });
        }
        (None, Some(finding)) => notes.push(RateNote {
            kind: NoteKind::Species,
            lines: crate::species_check::note_lines(finding),
            action: (can_correct
                && finding.case == crate::species_check::RangeCase::TenfoldHeaderRate)
                .then_some(NoteAction::TreatAsTenfold),
        }),
        (None, None) => {}
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bat_book::auto_resolve::find_species;
    use crate::heterodyne::file_hint;
    use crate::note_line::LineRole;

    const ECHO: Option<&str> = Some("echolocation");

    /// The species finding for a recording labelled `name`, looked up as the
    /// app does with no country.
    fn finding(name: &str, sound_type: Option<&str>, rate: u32) -> Option<SpeciesRangeFinding> {
        let species = find_species(name, None)?;
        crate::species_check::finding(name, species, sound_type, None, rate)
    }

    /// The heterodyne hint for a recording with one metadata field.
    fn hint(label: &str, value: &str, rate: u32) -> HeterodyneHint {
        file_hint(&[(label.to_string(), value.to_string())], None, rate).unwrap()
    }

    fn kinds(notes: &[RateNote]) -> Vec<NoteKind> {
        notes.iter().map(|n| n.kind).collect()
    }

    #[test]
    fn a_corrected_rate_says_what_the_file_holds() {
        assert_eq!(rate_notes(None, None, None, true), []);
        let te = TimeExpansion {
            factor: 10.0,
            stored_rate: 38_400,
            origin: crate::types::TimeExpansionOrigin::File,
        };
        assert_eq!(
            rate_notes(Some(te), None, None, true),
            [RateNote {
                kind: NoteKind::TimeExpansion,
                lines: vec![NoteLine::muted(
                    "Corrected for \u{d7}10 time expansion; the file says 38.4\u{a0}kHz."
                )],
                action: None,
            }]
        );
    }

    /// Case T: XC1001172, Eptesicus nilssonii (25-45 kHz) at 25.6 kHz.
    fn case_t() -> SpeciesRangeFinding {
        let f = finding("Eptesicus nilssonii", ECHO, 25_600).unwrap();
        assert_eq!(f.case, crate::species_check::RangeCase::TenfoldHeaderRate);
        f
    }

    #[test]
    fn a_probable_tenfold_file_gets_the_treat_as_button() {
        let species = case_t();
        let notes = rate_notes(None, None, Some(&species), true);
        assert_eq!(kinds(&notes), [NoteKind::Species]);
        assert_eq!(notes[0].action, Some(NoteAction::TreatAsTenfold));
        // The button sits under the amber line it acts on.
        assert_eq!(notes[0].lines[0].role, crate::note_line::LineRole::Warning);
        // A streamed file keeps the note without the button.
        let notes = rate_notes(None, None, Some(&species), false);
        assert_eq!(notes[0].action, None);
        assert_eq!(notes[0].lines, crate::species_check::note_lines(&species));
    }

    #[test]
    fn only_case_t_gets_a_button() {
        // Case "Other": the factor is unknown.
        let other = finding("Pipistrellus kuhlii", ECHO, 48_000).unwrap();
        assert_eq!(other.case, crate::species_check::RangeCase::Other);
        assert_eq!(rate_notes(None, None, Some(&other), true)[0].action, None);
        // A heterodyne note never has one. At a x10 header rate only a
        // remark that says "heterodyne" gives a hint (level A), and its note
        // replaces the species note.
        let species = case_t();
        let hint_a = hint("Remarks", "hetrodyne detector set at 31,9 kHz", 25_600);
        assert_eq!(hint_a.level, HeterodyneLevel::SaysHeterodyne);
        let notes = rate_notes(None, Some(&hint_a), Some(&species), true);
        assert_eq!(kinds(&notes), [NoteKind::Heterodyne]);
        assert_eq!(notes[0].action, None);
        // A correction from the file's metadata or its source has none.
        for origin in [TimeExpansionOrigin::File, TimeExpansionOrigin::Source] {
            let te = TimeExpansion {
                factor: 10.0,
                stored_rate: 25_600,
                origin,
            };
            assert_eq!(rate_notes(Some(te), None, None, true)[0].action, None);
        }
    }

    #[test]
    fn a_user_correction_gets_the_use_file_rate_button() {
        // After the user's x10 correction the file reaches the species'
        // calls, so the species note is gone and the time-expansion note,
        // now first under the Sample rate row, has the button.
        let te = TimeExpansion {
            factor: 10.0,
            stored_rate: 25_600,
            origin: TimeExpansionOrigin::User,
        };
        let species = find_species("Eptesicus nilssonii", None).unwrap();
        assert_eq!(
            crate::species_check::finding("Eptesicus nilssonii", species, ECHO, Some(te), 256_000),
            None
        );
        assert_eq!(
            rate_notes(Some(te), None, None, true),
            [RateNote {
                kind: NoteKind::TimeExpansion,
                lines: vec![NoteLine::muted(
                    "Corrected for \u{d7}10 time expansion; the file says 25.6\u{a0}kHz."
                )],
                action: Some(NoteAction::UseFileRate),
            }]
        );
    }

    #[test]
    fn button_labels_and_tooltip() {
        assert_eq!(
            NoteAction::TreatAsTenfold.label(),
            "Treat as time-expanded \u{d7}10"
        );
        assert_eq!(
            NoteAction::UseFileRate.label(),
            "Use the file's sample rate"
        );
        assert_eq!(
            NoteAction::TreatAsTenfold.tooltip(38_400).as_deref(),
            Some("Uses 384\u{a0}kHz as the sample rate. The file is not changed.")
        );
        assert_eq!(
            NoteAction::TreatAsTenfold.tooltip(31_250).as_deref(),
            Some("Uses 312.5\u{a0}kHz as the sample rate. The file is not changed.")
        );
        assert_eq!(NoteAction::UseFileRate.tooltip(38_400), None);
        assert_eq!(NoteAction::TreatAsTenfold.factor(), Some(10.0));
        assert_eq!(NoteAction::UseFileRate.factor(), None);
    }

    #[test]
    fn heterodyne_levels_a_and_b_hide_the_species_note() {
        // XC924726: "Recording with Magenta Bat5" is level B.
        let species = finding("Rhinolophus pumilus", ECHO, 48_000).unwrap();
        let hint_b = hint("Remarks", "Recording with Magenta Bat5", 48_000);
        assert_eq!(hint_b.level, HeterodyneLevel::HeterodyneOnlyDetector);
        assert_eq!(
            rate_notes(None, Some(&hint_b), Some(&species), true),
            [RateNote {
                kind: NoteKind::Heterodyne,
                lines: crate::heterodyne::note_lines(&hint_b),
                action: None,
            }]
        );
        // XC1034529: level A.
        let species = finding("Eptesicus nilssonii", ECHO, 48_000).unwrap();
        let hint_a = hint("Remarks", "hetrodyne detector set at 31,9 kHz", 48_000);
        assert_eq!(hint_a.level, HeterodyneLevel::SaysHeterodyne);
        assert_eq!(
            rate_notes(None, Some(&hint_a), Some(&species), true),
            [RateNote {
                kind: NoteKind::Heterodyne,
                lines: crate::heterodyne::note_lines(&hint_a),
                action: None,
            }]
        );
    }

    #[test]
    fn heterodyne_level_c_gets_the_species_line_last() {
        // XC933228.
        let species = finding(
            "Eptesicus nilssonii",
            Some("echolocation, feeding buzz"),
            44_100,
        )
        .unwrap();
        let hint_c = hint(
            "Remarks",
            "recorded sound from ultrasonic detectors spaker with mobilphone",
            44_100,
        );
        assert_eq!(hint_c.level, HeterodyneLevel::MayBeHeterodyne);
        let mut expected = crate::heterodyne::note_lines(&hint_c);
        expected.push(NoteLine::muted(
            "Eptesicus nilssonii: echolocation calls at 25\u{2013}\u{2060}45\u{a0}kHz, but this \
             file can only contain frequencies up to 22.1\u{a0}kHz.",
        ));
        assert_eq!(
            rate_notes(None, Some(&hint_c), Some(&species), true),
            [RateNote {
                kind: NoteKind::Heterodyne,
                lines: expected,
                action: None,
            }]
        );
        // XC1062080: Myotis blythii (20-120 kHz) from a Pettersson D980 at
        // 44.1 kHz: level C only.
        assert_eq!(finding("Myotis blythii", ECHO, 44_100), None);
        let hint_c = hint("Device", "Pettersson D980", 44_100);
        assert_eq!(
            rate_notes(None, Some(&hint_c), None, true),
            [RateNote {
                kind: NoteKind::Heterodyne,
                lines: crate::heterodyne::note_lines(&hint_c),
                action: None,
            }]
        );
    }

    #[test]
    fn no_hint_gives_the_species_note() {
        let species = finding("Pipistrellus kuhlii", ECHO, 48_000).unwrap();
        assert_eq!(
            rate_notes(None, None, Some(&species), true),
            [RateNote {
                kind: NoteKind::Species,
                lines: crate::species_check::note_lines(&species),
                action: None,
            }]
        );
    }

    #[test]
    fn the_time_expansion_note_comes_first() {
        // Corrected x2 from 25.6 kHz, and still too low for the species.
        let te = TimeExpansion {
            factor: 2.0,
            stored_rate: 25_600,
            origin: crate::types::TimeExpansionOrigin::File,
        };
        let species = find_species("Pipistrellus pygmaeus", None).unwrap();
        let species =
            crate::species_check::finding("Pipistrellus pygmaeus", species, ECHO, Some(te), 51_200)
                .unwrap();
        let notes = rate_notes(Some(te), None, Some(&species), true);
        assert_eq!(kinds(&notes), [NoteKind::TimeExpansion, NoteKind::Species]);
        assert_eq!(
            notes[0].lines,
            [NoteLine::muted(
                "Corrected for \u{d7}2 time expansion; the file says 25.6\u{a0}kHz."
            )]
        );
    }

    #[test]
    fn every_khz_follows_a_no_break_space() {
        // XC1034529. The quote line is the recordist's words, with a no-break
        // space before "kHz" so the unit wraps with "31,9".
        let hint_a = hint("Remarks", "hetrodyne detector set at 31,9 kHz", 48_000);
        let notes = rate_notes(None, Some(&hint_a), None, true);
        let quotes: Vec<&NoteLine> = notes[0]
            .lines
            .iter()
            .filter(|l| l.role == LineRole::Quote)
            .collect();
        assert_eq!(
            quotes,
            [&NoteLine::quote(
                "Remarks: \"hetrodyne detector set at 31,9\u{a0}kHz\""
            )]
        );
        for line in &notes[0].lines {
            assert!(!line.text.contains(" kHz"), "{:?}", line.text);
        }
        // The other spellings found in Xeno-canto remarks.
        for (remark, quoted) in [
            ("heterodyne detector set at 31.9 KHz", "31.9\u{a0}KHz"),
            ("heterodyne detector tuned to 45 k Hz", "45\u{a0}k\u{a0}Hz"),
        ] {
            let hint = hint("Remarks", remark, 48_000);
            let lines = rate_notes(None, Some(&hint), None, true).remove(0).lines;
            let quote = lines.iter().find(|l| l.role == LineRole::Quote).unwrap();
            assert!(quote.text.contains(quoted), "{:?}", quote.text);
        }
    }
}
