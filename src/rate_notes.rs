//! The notes under the Info panel's Sample rate row.
//!
//! A file can have up to two notes, in this order:
//!
//! 1. The time-expansion note, when the app corrected the file's rate for
//!    time expansion as it loaded the file.
//! 2. The heterodyne note ([`crate::heterodyne`]) or the species call-range
//!    note ([`crate::species_check`]), never both.
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
use crate::types::TimeExpansion;

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
}

/// The time-expansion note's line: "Corrected for ×10 time expansion; the
/// file says 38.4 kHz." A bat detector stored the recording slowed down so it
/// can be heard, and the app corrected its rate and duration when it loaded
/// the file. The rate in the line is the one in the file's header.
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
    )
}

/// The notes for a file's time expansion, heterodyne hint and species
/// finding, in the order they are shown.
///
/// - `time_expansion`: the correction the app applied when it loaded the
///   file. It gives the time-expansion note, which comes first.
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
) -> Vec<RateNote> {
    let mut notes = Vec::new();
    if let Some(te) = time_expansion {
        notes.push(RateNote {
            kind: NoteKind::TimeExpansion,
            lines: vec![time_expansion_line(te)],
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
            });
        }
        (None, Some(finding)) => notes.push(RateNote {
            kind: NoteKind::Species,
            lines: crate::species_check::note_lines(finding),
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
        assert_eq!(rate_notes(None, None, None), []);
        let te = TimeExpansion {
            factor: 10.0,
            stored_rate: 38_400,
            origin: crate::types::TimeExpansionOrigin::File,
        };
        assert_eq!(
            rate_notes(Some(te), None, None),
            [RateNote {
                kind: NoteKind::TimeExpansion,
                lines: vec![NoteLine::muted(
                    "Corrected for \u{d7}10 time expansion; the file says 38.4\u{a0}kHz."
                )],
            }]
        );
    }

    #[test]
    fn heterodyne_levels_a_and_b_hide_the_species_note() {
        // XC924726: "Recording with Magenta Bat5" is level B.
        let species = finding("Rhinolophus pumilus", ECHO, 48_000).unwrap();
        let hint_b = hint("Remarks", "Recording with Magenta Bat5", 48_000);
        assert_eq!(hint_b.level, HeterodyneLevel::HeterodyneOnlyDetector);
        assert_eq!(
            rate_notes(None, Some(&hint_b), Some(&species)),
            [RateNote {
                kind: NoteKind::Heterodyne,
                lines: crate::heterodyne::note_lines(&hint_b),
            }]
        );
        // XC1034529: level A.
        let species = finding("Eptesicus nilssonii", ECHO, 48_000).unwrap();
        let hint_a = hint("Remarks", "hetrodyne detector set at 31,9 kHz", 48_000);
        assert_eq!(hint_a.level, HeterodyneLevel::SaysHeterodyne);
        assert_eq!(
            rate_notes(None, Some(&hint_a), Some(&species)),
            [RateNote {
                kind: NoteKind::Heterodyne,
                lines: crate::heterodyne::note_lines(&hint_a),
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
            rate_notes(None, Some(&hint_c), Some(&species)),
            [RateNote {
                kind: NoteKind::Heterodyne,
                lines: expected,
            }]
        );
        // XC1062080: Myotis blythii (20-120 kHz) from a Pettersson D980 at
        // 44.1 kHz: level C only.
        assert_eq!(finding("Myotis blythii", ECHO, 44_100), None);
        let hint_c = hint("Device", "Pettersson D980", 44_100);
        assert_eq!(
            rate_notes(None, Some(&hint_c), None),
            [RateNote {
                kind: NoteKind::Heterodyne,
                lines: crate::heterodyne::note_lines(&hint_c),
            }]
        );
    }

    #[test]
    fn no_hint_gives_the_species_note() {
        let species = finding("Pipistrellus kuhlii", ECHO, 48_000).unwrap();
        assert_eq!(
            rate_notes(None, None, Some(&species)),
            [RateNote {
                kind: NoteKind::Species,
                lines: crate::species_check::note_lines(&species),
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
        let notes = rate_notes(Some(te), None, Some(&species));
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
        let notes = rate_notes(None, Some(&hint_a), None);
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
            let lines = rate_notes(None, Some(&hint), None).remove(0).lines;
            let quote = lines.iter().find(|l| l.role == LineRole::Quote).unwrap();
            assert!(quote.text.contains(quoted), "{:?}", quote.text);
        }
    }
}
