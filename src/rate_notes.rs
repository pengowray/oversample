//! The notes under the Info panel's Sample rate row.
//!
//! A file can have up to two notes, in this order:
//!
//! 1. The time-expansion note, when the file's rate was corrected for time
//!    expansion: by the app as it loaded the file, or by the user.
//! 2. The heterodyne note ([`crate::heterodyne`]) or the species call-range
//!    note ([`crate::species_check`]), never both.
//!
//! A note can have one button ([`NoteAction`]), drawn after its lines: the
//! species note of a probably time-expanded file has one that reads the file
//! at ten times its rate, and the time-expansion note that replaces it has
//! one that goes back to the file's own rate ([`crate::te_correction`]).
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
    /// The note's button, drawn after the note's lines, so the claim and
    /// its evidence come before the action.
    pub action: Option<NoteAction>,
}

/// A button on a note that changes the rate the file is read at
/// ([`crate::te_correction::set_user_time_expansion`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteAction {
    /// On the species note that says "Probably time-expanded ×10": read the
    /// file at ten times the rate it states.
    CorrectTenfold,
    /// On the time-expansion note of a correction the user applied: read the
    /// file at the rate it states again.
    RemoveCorrection,
}

impl NoteAction {
    /// The button's label. Both use the words of the time-expansion note
    /// that a correction shows ("Corrected for ×10 time expansion"), so the
    /// button and its result read as one action.
    pub fn label(self) -> &'static str {
        match self {
            NoteAction::CorrectTenfold => "Correct for \u{d7}10 time expansion",
            NoteAction::RemoveCorrection => "Remove \u{d7}10 correction",
        }
    }

    /// The button's tooltip, for a file whose header states `stored_rate`.
    /// "Remove ×10 correction" has none: the note above it names the file's
    /// rate.
    pub fn tooltip(self, stored_rate: u32) -> Option<String> {
        match self {
            NoteAction::CorrectTenfold => Some(format!(
                "Sets the sample rate to {} until you close the file or reload \
                 the page. The audio file is not changed.",
                sample_rate_khz(stored_rate.saturating_mul(10))
            )),
            NoteAction::RemoveCorrection => None,
        }
    }

    /// The factor the button sets: ×10, or `None` for the file's own rate.
    pub fn factor(self) -> Option<f64> {
        match self {
            NoteAction::CorrectTenfold => Some(crate::te_correction::TENFOLD),
            NoteAction::RemoveCorrection => None,
        }
    }

    /// The action's name in the button's `data-action` attribute, which the
    /// Playwright specs use to find it.
    pub fn key(self) -> &'static str {
        match self {
            NoteAction::CorrectTenfold => "correct-tenfold",
            NoteAction::RemoveCorrection => "remove-correction",
        }
    }
}

/// The time-expansion note's line: "Corrected for ×10 time expansion. Sample
/// rate in the file header: 38.4 kHz." A bat detector stored the recording slowed down so it
/// can be heard, and its rate and duration were corrected: by the app when it
/// loaded the file, or by the user. The rate in the line is the one in the
/// file's header.
///
/// A correction the user applied lasts until the file is closed or the page is
/// reloaded, and the line says so: "Corrected for ×10 time expansion until you
/// close the file or reload the page. Sample rate in the file header:
/// 25.6 kHz." That also tells it apart from a correction made from
/// the file's metadata, which is made again each time the file is opened.
pub fn time_expansion_line(te: TimeExpansion) -> NoteLine {
    let until = match te.origin {
        TimeExpansionOrigin::User => " until you close the file or reload the page",
        TimeExpansionOrigin::File | TimeExpansionOrigin::Source => "",
    };
    NoteLine::muted(format!(
        "Corrected for {} time expansion{until}. Sample rate in the file header: {}.",
        crate::wikimedia::format_factor(te.factor),
        sample_rate_khz(te.stored_rate)
    ))
}

/// Whether the app can change the rate a file is read at
/// ([`crate::types::AudioData::can_set_time_expansion`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Correction {
    /// The file's samples are all in memory.
    Possible,
    /// The file is streamed from disk: a large file, or one opened when the
    /// files already open are large in total. The app says "Streaming large
    /// file" or "Streaming file to keep total open files under control" as
    /// it opens one.
    Streamed,
    /// Any other file: a zero-crossing (`.zc`) file.
    NotPossible,
}

impl Correction {
    pub fn of(audio: &crate::types::AudioData) -> Self {
        if audio.can_set_time_expansion() {
            Correction::Possible
        } else if crate::audio::streaming_source::is_streaming(audio.source.as_ref()) {
            Correction::Streamed
        } else {
            Correction::NotPossible
        }
    }
}

/// The species note's last line for a probably time-expanded file that the
/// app streams from disk, in place of the button.
pub const STREAMED_LINE: &str = "Streamed files can't be corrected for time expansion.";

/// The notes under a file's Sample rate row, in the order they are shown.
pub fn file_rate_notes(file: &LoadedFile) -> Vec<RateNote> {
    rate_notes(
        file.audio.metadata.time_expansion,
        file.heterodyne_hint.as_ref(),
        file.species_range.as_ref(),
        Correction::of(&file.audio),
    )
}

/// The notes for a file's time expansion, heterodyne hint and species
/// finding, in the order they are shown.
///
/// - `time_expansion`: the correction applied to the file's rate. It gives
///   the time-expansion note, which comes first. When the user applied it,
///   the note has the "Remove ×10 correction" button.
/// - `correction`: whether the app can change the rate the file is read at.
///   When it can, the species note of a file at a ×10 header rate
///   ([`crate::species_check::RangeCase::TenfoldHeaderRate`]) has the
///   "Correct for ×10 time expansion" button. For a streamed file, that
///   note ends with [`STREAMED_LINE`] instead.
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
    correction: Correction,
) -> Vec<RateNote> {
    let mut notes = Vec::new();
    if let Some(te) = time_expansion {
        notes.push(RateNote {
            kind: NoteKind::TimeExpansion,
            lines: vec![time_expansion_line(te)],
            action: (te.origin == TimeExpansionOrigin::User)
                .then_some(NoteAction::RemoveCorrection),
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
        (None, Some(finding)) => {
            let mut lines = crate::species_check::note_lines(finding);
            let tenfold = finding.case == crate::species_check::RangeCase::TenfoldHeaderRate;
            if tenfold && correction == Correction::Streamed {
                lines.push(NoteLine::muted(STREAMED_LINE));
            }
            notes.push(RateNote {
                kind: NoteKind::Species,
                lines,
                action: (tenfold && correction == Correction::Possible)
                    .then_some(NoteAction::CorrectTenfold),
            });
        }
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
        assert_eq!(rate_notes(None, None, None, Correction::Possible), []);
        let te = TimeExpansion {
            factor: 10.0,
            stored_rate: 38_400,
            origin: crate::types::TimeExpansionOrigin::File,
        };
        assert_eq!(
            rate_notes(Some(te), None, None, Correction::Possible),
            [RateNote {
                kind: NoteKind::TimeExpansion,
                lines: vec![NoteLine::muted(
                    "Corrected for \u{d7}10 time expansion. Sample rate in the file header: \
                     38.4\u{a0}kHz."
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
    fn a_probable_tenfold_file_gets_the_correct_button() {
        let species = case_t();
        let notes = rate_notes(None, None, Some(&species), Correction::Possible);
        assert_eq!(kinds(&notes), [NoteKind::Species]);
        assert_eq!(notes[0].action, Some(NoteAction::CorrectTenfold));
        // The button comes after the amber claim and the species evidence.
        assert_eq!(notes[0].lines, crate::species_check::note_lines(&species));
        assert_eq!(notes[0].lines[0].role, crate::note_line::LineRole::Warning);
        // A streamed file has no button, and its note says why.
        let notes = rate_notes(None, None, Some(&species), Correction::Streamed);
        assert_eq!(notes[0].action, None);
        let mut expected = crate::species_check::note_lines(&species);
        expected.push(NoteLine::muted(
            "Streamed files can't be corrected for time expansion.",
        ));
        assert_eq!(notes[0].lines, expected);
        // A zero-crossing file has neither.
        let notes = rate_notes(None, None, Some(&species), Correction::NotPossible);
        assert_eq!(notes[0].action, None);
        assert_eq!(notes[0].lines, crate::species_check::note_lines(&species));
    }

    #[test]
    fn only_case_t_gets_a_button() {
        // Case "Other": the factor is unknown.
        let other = finding("Pipistrellus kuhlii", ECHO, 48_000).unwrap();
        assert_eq!(other.case, crate::species_check::RangeCase::Other);
        let notes = rate_notes(None, None, Some(&other), Correction::Possible);
        assert_eq!(notes[0].action, None);
        // Nor the streamed line: there is nothing to correct.
        assert_eq!(
            rate_notes(None, None, Some(&other), Correction::Streamed),
            notes
        );
        // A heterodyne note never has one. At a x10 header rate only a
        // remark that says "heterodyne" gives a hint (level A), and its note
        // replaces the species note.
        let species = case_t();
        let hint_a = hint("Remarks", "hetrodyne detector set at 31,9 kHz", 25_600);
        assert_eq!(hint_a.level, HeterodyneLevel::SaysHeterodyne);
        let notes = rate_notes(None, Some(&hint_a), Some(&species), Correction::Possible);
        assert_eq!(kinds(&notes), [NoteKind::Heterodyne]);
        assert_eq!(notes[0].action, None);
        // A correction from the file's metadata or its source has none.
        for origin in [TimeExpansionOrigin::File, TimeExpansionOrigin::Source] {
            let te = TimeExpansion {
                factor: 10.0,
                stored_rate: 25_600,
                origin,
            };
            assert_eq!(
                rate_notes(Some(te), None, None, Correction::Possible)[0].action,
                None
            );
        }
    }

    #[test]
    fn a_user_correction_gets_the_remove_button() {
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
            rate_notes(Some(te), None, None, Correction::Possible),
            [RateNote {
                kind: NoteKind::TimeExpansion,
                lines: vec![NoteLine::muted(
                    "Corrected for \u{d7}10 time expansion until you close the file or \
                     reload the page. Sample rate in the file header: 25.6\u{a0}kHz."
                )],
                action: Some(NoteAction::RemoveCorrection),
            }]
        );
    }

    #[test]
    fn button_labels_and_tooltip() {
        assert_eq!(
            NoteAction::CorrectTenfold.label(),
            "Correct for \u{d7}10 time expansion"
        );
        assert_eq!(
            NoteAction::RemoveCorrection.label(),
            "Remove \u{d7}10 correction"
        );
        assert_eq!(
            NoteAction::CorrectTenfold.tooltip(38_400).as_deref(),
            Some(
                "Sets the sample rate to 384\u{a0}kHz until you close the file or reload \
                 the page. The audio file is not changed."
            )
        );
        assert_eq!(
            NoteAction::CorrectTenfold.tooltip(31_250).as_deref(),
            Some(
                "Sets the sample rate to 312.5\u{a0}kHz until you close the file or \
                 reload the page. The audio file is not changed."
            )
        );
        assert_eq!(NoteAction::RemoveCorrection.tooltip(38_400), None);
        assert_eq!(NoteAction::CorrectTenfold.factor(), Some(10.0));
        assert_eq!(NoteAction::RemoveCorrection.factor(), None);
    }

    #[test]
    fn heterodyne_levels_a_and_b_hide_the_species_note() {
        // XC924726: "Recording with Magenta Bat5" is level B.
        let species = finding("Rhinolophus pumilus", ECHO, 48_000).unwrap();
        let hint_b = hint("Remarks", "Recording with Magenta Bat5", 48_000);
        assert_eq!(hint_b.level, HeterodyneLevel::HeterodyneOnlyDetector);
        assert_eq!(
            rate_notes(None, Some(&hint_b), Some(&species), Correction::Possible),
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
            rate_notes(None, Some(&hint_a), Some(&species), Correction::Possible),
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
            rate_notes(None, Some(&hint_c), Some(&species), Correction::Possible),
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
            rate_notes(None, Some(&hint_c), None, Correction::Possible),
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
            rate_notes(None, None, Some(&species), Correction::Possible),
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
        let notes = rate_notes(Some(te), None, Some(&species), Correction::Possible);
        assert_eq!(kinds(&notes), [NoteKind::TimeExpansion, NoteKind::Species]);
        assert_eq!(
            notes[0].lines,
            [NoteLine::muted(
                "Corrected for \u{d7}2 time expansion. Sample rate in the file header: \
                 25.6\u{a0}kHz."
            )]
        );
    }

    #[test]
    fn every_khz_follows_a_no_break_space() {
        // XC1034529. The quote line is the recordist's words, with a no-break
        // space before "kHz" so the unit wraps with "31,9".
        let hint_a = hint("Remarks", "hetrodyne detector set at 31,9 kHz", 48_000);
        let notes = rate_notes(None, Some(&hint_a), None, Correction::Possible);
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
            let lines = rate_notes(None, Some(&hint), None, Correction::Possible)
                .remove(0)
                .lines;
            let quote = lines.iter().find(|l| l.role == LineRole::Quote).unwrap();
            assert!(quote.text.contains(quoted), "{:?}", quote.text);
        }
    }
}
