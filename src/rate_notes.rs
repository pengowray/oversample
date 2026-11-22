//! The notes under the Info panel's Sample rate row.
//!
//! A file can have up to two notes, in this order:
//!
//! 1. The time-expansion note, when the file's rate was corrected for time
//!    expansion (by the app as it opened the file, or by the user), or when
//!    the user removed the correction the file was opened with.
//! 2. The heterodyne note ([`crate::heterodyne`]) or the species call-range
//!    note ([`crate::species_check`]), never both.
//!
//! A note can have one button ([`NoteAction`]), drawn after its lines: the
//! species note of a probably time-expanded file has one that reads the file
//! at ten times its rate, the time-expansion note has one that goes back to
//! the rate in the file header, and the note shown after that has one that
//! makes the correction the file was opened with again
//! ([`crate::te_correction`]).
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
use crate::wikimedia::format_factor;

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
#[derive(Clone, Debug, PartialEq)]
pub struct RateNote {
    pub kind: NoteKind,
    pub lines: Vec<NoteLine>,
    /// The note's button, drawn after the note's lines, so the claim and
    /// its evidence come before the action.
    pub action: Option<NoteAction>,
}

/// A button on a note that changes the rate the file is read at
/// ([`crate::te_correction::set_time_expansion`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NoteAction {
    /// On the species note that says "Probably time-expanded ×10": read the
    /// file at ten times the rate it states.
    CorrectTenfold,
    /// On the time-expansion note: read the file at the rate its header
    /// states. Holds the correction it removes: the user's, or the one the
    /// file was opened with.
    RemoveCorrection(TimeExpansion),
    /// On the time-expansion note after the user removed the correction the
    /// file was opened with: make that correction again. Holds it.
    ApplyCorrection(TimeExpansion),
}

impl NoteAction {
    /// The button's label. Each uses the words of the time-expansion note
    /// that a correction shows ("Corrected for ×10 time expansion"), so the
    /// button and its result read as one action.
    pub fn label(self) -> String {
        match self {
            NoteAction::CorrectTenfold => "Correct for \u{d7}10 time expansion".to_string(),
            NoteAction::RemoveCorrection(te) => {
                format!("Remove {} correction", format_factor(te.factor))
            }
            NoteAction::ApplyCorrection(te) => {
                format!("Correct for {} time expansion", format_factor(te.factor))
            }
        }
    }

    /// The button's tooltip, for a file whose header states `stored_rate`.
    ///
    /// Removing the user's correction has none: the note above it names the
    /// file's rate and says the correction lasts until the file is closed.
    /// Removing a correction the file was opened with has one, since the
    /// note does not say that the correction is made again when the file is
    /// opened, and the user may expect the button to change the file.
    ///
    /// `kept` says whether the change is saved ([`Kept`]).
    pub fn tooltip(self, stored_rate: u32, kept: Kept) -> Option<String> {
        let until_closed = |rate: u32| match kept {
            Kept::Saved => format!(
                "Sets the sample rate to {} and saves this choice in the recording's .batm \
                 file. The audio file is not changed.",
                sample_rate_khz(rate)
            ),
            Kept::UntilClosed => format!(
                "Sets the sample rate to {} until you close the file or reload the page. \
                 The audio file is not changed.",
                sample_rate_khz(rate)
            ),
        };
        match self {
            NoteAction::CorrectTenfold => Some(until_closed(stored_rate.saturating_mul(10))),
            NoteAction::RemoveCorrection(te) if te.origin == TimeExpansionOrigin::User => None,
            NoteAction::RemoveCorrection(te) => Some(until_closed(te.stored_rate)),
            NoteAction::ApplyCorrection(te) => Some(format!(
                "Sets the sample rate to {}.",
                sample_rate_khz(te.corrected_rate())
            )),
        }
    }

    /// The correction the button makes, as a factor and what gave it, or
    /// `None` for the rate in the file header. "Correct for ×N" on a removed
    /// correction makes it with the origin it was opened with, so the file
    /// is as it was opened.
    pub fn target(self) -> Option<(f64, TimeExpansionOrigin)> {
        match self {
            NoteAction::CorrectTenfold => {
                Some((crate::te_correction::TENFOLD, TimeExpansionOrigin::User))
            }
            NoteAction::RemoveCorrection(_) => None,
            NoteAction::ApplyCorrection(te) => Some((te.factor, te.origin)),
        }
    }

    /// The action's name in the button's `data-action` attribute, which the
    /// Playwright specs use to find it.
    pub fn key(self) -> &'static str {
        match self {
            NoteAction::CorrectTenfold => "correct-tenfold",
            NoteAction::RemoveCorrection(_) => "remove-correction",
            NoteAction::ApplyCorrection(_) => "apply-correction",
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
pub fn time_expansion_line(te: TimeExpansion, kept: Kept) -> NoteLine {
    let factor = format_factor(te.factor);
    let first = match (te.origin, kept) {
        (TimeExpansionOrigin::User, Kept::Saved) => user_factor_sentence(&factor),
        (TimeExpansionOrigin::User, Kept::UntilClosed) => format!(
            "Corrected for {factor} time expansion until you close the file or reload the page."
        ),
        (TimeExpansionOrigin::File | TimeExpansionOrigin::Source, _) => {
            format!("Corrected for {factor} time expansion.")
        }
    };
    NoteLine::muted(format!(
        "{first} Sample rate in the file header: {}.",
        sample_rate_khz(te.stored_rate)
    ))
}

/// Where the factor of the correction a file was opened with is stated, as
/// the note shown after the user removes that correction names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatedIn {
    /// GUANO's TE field, in the file.
    Guano,
    /// The Time Expansion field of a Wildlife Acoustics `wamd` chunk.
    WildlifeAcoustics,
    /// The file's metadata, when neither field above gives the factor.
    FileMetadata,
    /// The site the file came from, by its name: Wikimedia Commons, whose
    /// P14424 statement is shown as the site's "Time expansion" row.
    Site(String),
    /// A Find recordings result. The search service reads the factor from
    /// the file's GUANO, so this is only for a file whose own metadata does
    /// not give the same factor. The note names the result, not the site the
    /// recording is on: the site's own record may not state the factor.
    FindRecordings,
}

impl StatedIn {
    /// The words after "Time expansion" in the note: "in the file's GUANO
    /// metadata", "on Wikimedia Commons", "in the Find recordings result".
    fn phrase(&self) -> String {
        match self {
            StatedIn::Guano => "in the file's GUANO metadata".to_string(),
            StatedIn::WildlifeAcoustics => "in the file's Wildlife Acoustics metadata".to_string(),
            StatedIn::FileMetadata => "in the file's metadata".to_string(),
            StatedIn::Site(name) => format!("on {name}"),
            StatedIn::FindRecordings => "in the Find recordings result".to_string(),
        }
    }
}

/// The correction a file was opened with
/// ([`LoadedFile::opened_time_expansion`]) and where its factor is stated.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenedCorrection {
    pub te: TimeExpansion,
    pub stated_in: StatedIn,
}

/// The correction file `file` was opened with, if any.
pub fn opened_correction(file: &LoadedFile) -> Option<OpenedCorrection> {
    let te = file.opened_time_expansion?;
    let meta = &file.audio.metadata;
    let wamd = meta.wav.as_ref().and_then(|w| w.wamd.as_ref());
    let guano = crate::types::declared_time_expansion(meta.format, meta.guano.as_ref(), None);
    let wamd = crate::types::declared_time_expansion(meta.format, None, wamd);
    let site = file.site_metadata.as_ref().map(|m| {
        let states_factor = m
            .fields
            .iter()
            .any(|(label, _)| label == crate::wikimedia::TIME_EXPANSION_FIELD);
        (m.site.name(), states_factor)
    });
    Some(OpenedCorrection {
        te,
        stated_in: stated_in(te, guano, wamd, site),
    })
}

/// Where the factor of `te`, the correction a file was opened with, is
/// stated.
///
/// - `guano`, `wamd`: the factors that GUANO's TE field and a Wildlife
///   Acoustics `wamd` chunk in the file give, if any. The first that gives
///   `te`'s factor is named, GUANO first, as the loader reads them.
/// - `site`: the name of the site the file came from, and whether its
///   metadata has a "Time expansion" row (Wikimedia Commons' P14424
///   statement). A correction from the site that has no such row came from
///   a Find recordings result.
pub fn stated_in(
    te: TimeExpansion,
    guano: Option<f64>,
    wamd: Option<f64>,
    site: Option<(&str, bool)>,
) -> StatedIn {
    let gives_factor = |factor: Option<f64>| factor.is_some_and(|f| (f - te.factor).abs() < 1e-9);
    if gives_factor(guano) {
        StatedIn::Guano
    } else if gives_factor(wamd) {
        StatedIn::WildlifeAcoustics
    } else if te.origin == TimeExpansionOrigin::File {
        StatedIn::FileMetadata
    } else {
        match site {
            Some((name, true)) => StatedIn::Site(name.to_string()),
            _ => StatedIn::FindRecordings,
        }
    }
}

/// The time-expansion note's line after the user removed the correction
/// the file was opened with: "Correction for ×10 time expansion removed
/// until you close the file or reload the page. Time expansion in the
/// file's GUANO metadata: ×10." The first sentence uses the words of the
/// corrected note ("Corrected for ×10 time expansion") and of the "Remove
/// ×10 correction" button that was pressed, and starts with a word, not the
/// factor. The Sample rate row above shows the rate in the file header. A
/// no-break space keeps the factor on the line of the words it ends.
pub fn removed_line(opened: &OpenedCorrection, kept: Kept) -> NoteLine {
    let factor = format_factor(opened.te.factor);
    let first = match kept {
        Kept::Saved => removed_sentence(&factor),
        Kept::UntilClosed => format!(
            "Correction for {factor} time expansion removed until you close the file or \
             reload the page."
        ),
    };
    NoteLine::muted(format!(
        "{first} Time expansion {}:\u{a0}{factor}.",
        opened.stated_in.phrase()
    ))
}

/// Whether a change of the rate a file is read at is saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kept {
    /// Saved in the recording's .batm file and made again when the
    /// recording is opened ([`crate::annotations::SavedTimeExpansion`]).
    Saved,
    /// The file is view-only, so nothing about it is saved: the change
    /// lasts until the file is closed.
    UntilClosed,
}

impl Kept {
    pub fn of(file: &LoadedFile) -> Self {
        if file.read_only {
            Kept::UntilClosed
        } else {
            Kept::Saved
        }
    }
}

/// The first sentence of the time-expansion note for a factor the user set,
/// when it is saved ([`Kept::Saved`]). `factor` is written "×10".
fn user_factor_sentence(factor: &str) -> String {
    format!(
        "Corrected for {factor} time expansion. You set this factor, and it is saved in \
         the recording's .batm file."
    )
}

/// The first sentences of the note shown after the user removed the
/// correction the file was opened with, when that is saved.
fn removed_sentence(factor: &str) -> String {
    format!(
        "Correction for {factor} time expansion removed. This choice is saved in the \
         recording's .batm file."
    )
}

/// The label of the Info panel row with the factor field.
pub const FACTOR_LABEL: &str = "Time expansion";
/// The factor field's accessible name.
pub const FACTOR_ARIA: &str = "Time expansion factor";

/// The tooltip of the Time expansion row. `saved` is false for a view-only
/// file, whose factor is not saved.
pub fn factor_tooltip(saved: bool) -> String {
    let what = if saved {
        "This is the time expansion factor for the recording, and it is saved in the \
         recording's .batm file."
    } else {
        "This is the time expansion factor for the recording."
    };
    format!(
        "{what} Enter 1 to read the recording at the sample rate in the file header. \
         The audio file is not changed."
    )
}

/// Shown under the Time expansion row for a value that is not a number from
/// 0.01 to 1000.
pub const FACTOR_RANGE_ERROR: &str = "Factor must be a number from 0.01 to 1000.";

/// Shown under the Time expansion row for a factor (written "×500") that
/// gives a sample rate of `rate_hz`, over the 50 MHz the app reads.
fn factor_rate_error(factor: &str, rate_hz: u64) -> String {
    format!(
        "Sample rate too high: {factor} gives {}\u{a0}MHz. The maximum is 50\u{a0}MHz.",
        (rate_hz as f64 / 1e6).round()
    )
}

/// The factor as the Time expansion row's field shows it: "10", "2.5".
pub fn factor_text(factor: f64) -> String {
    format_factor(factor)
        .trim_start_matches('\u{d7}')
        .to_string()
}

/// The factor typed into the Time expansion row's field, for a file whose
/// header states `stored_rate`. 1 is the rate in the header. A leading "×"
/// or "x" and a decimal comma are accepted. `Err` is the message shown under
/// the row.
pub fn parse_factor(text: &str, stored_rate: u32) -> Result<f64, String> {
    let text = text
        .trim()
        .trim_start_matches(['\u{d7}', 'x', 'X'])
        .trim()
        .replace(',', ".");
    let factor = text
        .parse::<f64>()
        .ok()
        .filter(|f| f.is_finite() && (0.01..=crate::types::MAX_TIME_EXPANSION_FACTOR).contains(f))
        .ok_or_else(|| FACTOR_RANGE_ERROR.to_string())?;
    let rate = f64::from(stored_rate) * factor;
    if rate.round() > crate::types::MAX_CORRECTED_SAMPLE_RATE {
        return Err(factor_rate_error(
            &format_factor(factor),
            rate.round() as u64,
        ));
    }
    Ok(factor)
}

/// Whether the app can change the rate a file is read at
/// ([`crate::types::AudioData::can_set_time_expansion`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Correction {
    /// The file's samples are all in memory, or it is a WAV file streamed
    /// from disk.
    Possible,
    /// Any other file streamed from disk: a large FLAC, MP3, Ogg or M4A
    /// file, or one opened when the files already open are large in total.
    /// The app says "Streaming large file" or "Streaming file to keep total
    /// open files under control" as it opens one.
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
/// app streams from disk and can't read at another rate, in place of the
/// button.
pub const STREAMED_LINE: &str = "Streamed files can't be corrected for time expansion.";

/// The notes under a file's Sample rate row, in the order they are shown.
pub fn file_rate_notes(file: &LoadedFile) -> Vec<RateNote> {
    rate_notes(
        file.audio.metadata.time_expansion,
        opened_correction(file).as_ref(),
        file.heterodyne_hint.as_ref(),
        file.species_range.as_ref(),
        Correction::of(&file.audio),
        Kept::of(file),
    )
}

/// The notes for a file's time expansion, heterodyne hint and species
/// finding, in the order they are shown.
///
/// - `time_expansion`: the correction applied to the file's rate. It gives
///   the time-expansion note, which comes first, with the "Remove ×10
///   correction" button. A correction the file was opened with gets the
///   button only when the app can change the file's rate (`correction`).
/// - `opened`: the correction the file was opened with. When the file is
///   read at the rate in its header (`time_expansion` is `None`), the
///   time-expansion note says that correction was removed, with a button
///   that makes it again.
/// - `correction`: whether the app can change the rate the file is read at.
///   When it can, the species note of a file at a ×10 header rate
///   ([`crate::species_check::RangeCase::TenfoldHeaderRate`]) has the
///   "Correct for ×10 time expansion" button. For a streamed file, that note
///   ends with [`STREAMED_LINE`] instead.
/// - While a correction from the file's metadata is removed, the species
///   note is only the species line ([`crate::species_check::fact_line`]):
///   the time-expansion note above it gives the factor the metadata states,
///   with the button that applies it. The species note's other lines are
///   guesses from the header rate ("Probably time-expanded ×10", or the list
///   of bat detector outputs), which that statement replaces. With a factor
///   other than 10, the ×10 guess and its button would contradict it.
/// - Heterodyne levels A and B: the heterodyne note without the species
///   line. The note already says the frequencies are not the bat's.
/// - Heterodyne level C: the heterodyne note, then the species line, muted
///   like the rest of a level C note. The species line comes last so that
///   the note's "If so, ..." line still follows the "Possibly recorded from
///   ..." line it refers to.
/// - No hint: the species note, when there is a finding.
pub fn rate_notes(
    time_expansion: Option<TimeExpansion>,
    opened: Option<&OpenedCorrection>,
    hint: Option<&HeterodyneHint>,
    finding: Option<&SpeciesRangeFinding>,
    correction: Correction,
    kept: Kept,
) -> Vec<RateNote> {
    let mut notes = Vec::new();
    let removed = if time_expansion.is_none() {
        opened
    } else {
        None
    };
    if let Some(te) = time_expansion {
        let can_remove =
            te.origin == TimeExpansionOrigin::User || correction == Correction::Possible;
        notes.push(RateNote {
            kind: NoteKind::TimeExpansion,
            lines: vec![time_expansion_line(te, kept)],
            action: can_remove.then_some(NoteAction::RemoveCorrection(te)),
        });
    } else if let Some(opened) = removed {
        notes.push(RateNote {
            kind: NoteKind::TimeExpansion,
            lines: vec![removed_line(opened, kept)],
            action: (correction == Correction::Possible)
                .then_some(NoteAction::ApplyCorrection(opened.te)),
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
        (None, Some(finding)) if removed.is_some() => notes.push(RateNote {
            kind: NoteKind::Species,
            lines: vec![crate::species_check::fact_line(finding)],
            action: None,
        }),
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

    #[test]
    fn a_saved_user_factor_says_where_it_is_saved() {
        let user = TimeExpansion {
            factor: 10.0,
            stored_rate: 25_600,
            origin: TimeExpansionOrigin::User,
        };
        assert_eq!(
            time_expansion_line(user, Kept::Saved).text,
            "Corrected for \u{d7}10 time expansion. You set this factor, and it is saved in \
             the recording's .batm file. Sample rate in the file header: 25.6\u{a0}kHz."
        );
        // A factor from the file or its source reads the same either way.
        let guano = TimeExpansion {
            origin: TimeExpansionOrigin::File,
            ..user
        };
        assert_eq!(
            time_expansion_line(guano, Kept::Saved),
            time_expansion_line(guano, Kept::UntilClosed)
        );
        assert_eq!(
            removed_line(
                &OpenedCorrection {
                    te: guano,
                    stated_in: StatedIn::Guano
                },
                Kept::Saved
            )
            .text,
            "Correction for \u{d7}10 time expansion removed. This choice is saved in the \
             recording's .batm file. Time expansion in the file's GUANO metadata:\u{a0}\u{d7}10."
        );
        assert_eq!(
            NoteAction::CorrectTenfold
                .tooltip(25_600, Kept::Saved)
                .as_deref(),
            Some(
                "Sets the sample rate to 256\u{a0}kHz and saves this choice in the \
                 recording's .batm file. The audio file is not changed."
            )
        );
    }

    #[test]
    fn a_typed_factor_is_checked() {
        assert_eq!(parse_factor("20", 48_000), Ok(20.0));
        assert_eq!(parse_factor(" \u{d7}2,5 ", 48_000), Ok(2.5));
        assert_eq!(parse_factor("x256", 44_100), Ok(256.0));
        assert_eq!(parse_factor("1", 48_000), Ok(1.0));
        assert_eq!(factor_text(2.5), "2.5");
        assert_eq!(factor_text(256.0), "256");
        for bad in ["", "abc", "0", "-3", "1001", "inf"] {
            assert_eq!(
                parse_factor(bad, 48_000),
                Err(FACTOR_RANGE_ERROR.to_string())
            );
        }
        assert_eq!(
            parse_factor("500", 384_000),
            Err(
                "Sample rate too high: \u{d7}500 gives 192\u{a0}MHz. The maximum is \
                 50\u{a0}MHz."
                    .to_string()
            )
        );
    }

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
        assert_eq!(
            rate_notes(
                None,
                None,
                None,
                None,
                Correction::Possible,
                Kept::UntilClosed
            ),
            []
        );
        let te = TimeExpansion {
            factor: 10.0,
            stored_rate: 38_400,
            origin: crate::types::TimeExpansionOrigin::File,
        };
        assert_eq!(
            rate_notes(
                Some(te),
                None,
                None,
                None,
                Correction::Possible,
                Kept::UntilClosed
            ),
            [RateNote {
                kind: NoteKind::TimeExpansion,
                lines: vec![NoteLine::muted(
                    "Corrected for \u{d7}10 time expansion. Sample rate in the file header: \
                     38.4\u{a0}kHz."
                )],
                action: Some(NoteAction::RemoveCorrection(te)),
            }]
        );
        // A file whose rate can't be changed keeps its correction.
        for correction in [Correction::Streamed, Correction::NotPossible] {
            assert_eq!(
                rate_notes(Some(te), None, None, None, correction, Kept::UntilClosed)[0].action,
                None
            );
        }
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
        let notes = rate_notes(
            None,
            None,
            None,
            Some(&species),
            Correction::Possible,
            Kept::UntilClosed,
        );
        assert_eq!(kinds(&notes), [NoteKind::Species]);
        assert_eq!(notes[0].action, Some(NoteAction::CorrectTenfold));
        // The button comes after the amber claim and the species evidence.
        assert_eq!(notes[0].lines, crate::species_check::note_lines(&species));
        assert_eq!(notes[0].lines[0].role, crate::note_line::LineRole::Warning);
        // A streamed file has no button, and its note says why.
        let notes = rate_notes(
            None,
            None,
            None,
            Some(&species),
            Correction::Streamed,
            Kept::UntilClosed,
        );
        assert_eq!(notes[0].action, None);
        let mut expected = crate::species_check::note_lines(&species);
        expected.push(NoteLine::muted(
            "Streamed files can't be corrected for time expansion.",
        ));
        assert_eq!(notes[0].lines, expected);
        // A zero-crossing file has neither.
        let notes = rate_notes(
            None,
            None,
            None,
            Some(&species),
            Correction::NotPossible,
            Kept::UntilClosed,
        );
        assert_eq!(notes[0].action, None);
        assert_eq!(notes[0].lines, crate::species_check::note_lines(&species));
    }

    #[test]
    fn only_case_t_gets_a_button() {
        // Case "Other": the factor is unknown.
        let other = finding("Pipistrellus kuhlii", ECHO, 48_000).unwrap();
        assert_eq!(other.case, crate::species_check::RangeCase::Other);
        let notes = rate_notes(
            None,
            None,
            None,
            Some(&other),
            Correction::Possible,
            Kept::UntilClosed,
        );
        assert_eq!(notes[0].action, None);
        // Nor the streamed line: there is nothing to correct.
        assert_eq!(
            rate_notes(
                None,
                None,
                None,
                Some(&other),
                Correction::Streamed,
                Kept::UntilClosed
            ),
            notes
        );
        // A heterodyne note never has one. At a x10 header rate only a
        // remark that says "heterodyne" gives a hint (level A), and its note
        // replaces the species note.
        let species = case_t();
        let hint_a = hint("Remarks", "hetrodyne detector set at 31,9 kHz", 25_600);
        assert_eq!(hint_a.level, HeterodyneLevel::SaysHeterodyne);
        let notes = rate_notes(
            None,
            None,
            Some(&hint_a),
            Some(&species),
            Correction::Possible,
            Kept::UntilClosed,
        );
        assert_eq!(kinds(&notes), [NoteKind::Heterodyne]);
        assert_eq!(notes[0].action, None);
        // A correction from the file's metadata or its source gets the
        // Remove button, not the x10 one.
        for origin in [TimeExpansionOrigin::File, TimeExpansionOrigin::Source] {
            let te = TimeExpansion {
                factor: 10.0,
                stored_rate: 25_600,
                origin,
            };
            assert_eq!(
                rate_notes(
                    Some(te),
                    None,
                    None,
                    None,
                    Correction::Possible,
                    Kept::UntilClosed
                )[0]
                .action,
                Some(NoteAction::RemoveCorrection(te))
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
            rate_notes(
                Some(te),
                None,
                None,
                None,
                Correction::Possible,
                Kept::UntilClosed
            ),
            [RateNote {
                kind: NoteKind::TimeExpansion,
                lines: vec![NoteLine::muted(
                    "Corrected for \u{d7}10 time expansion until you close the file or \
                     reload the page. Sample rate in the file header: 25.6\u{a0}kHz."
                )],
                action: Some(NoteAction::RemoveCorrection(te)),
            }]
        );
    }

    #[test]
    fn button_labels_and_tooltip() {
        let user = TimeExpansion {
            factor: 10.0,
            stored_rate: 38_400,
            origin: TimeExpansionOrigin::User,
        };
        let guano = TimeExpansion {
            origin: TimeExpansionOrigin::File,
            ..user
        };
        let commons = TimeExpansion {
            factor: 2.5,
            stored_rate: 48_000,
            origin: TimeExpansionOrigin::Source,
        };
        assert_eq!(
            NoteAction::CorrectTenfold.label(),
            "Correct for \u{d7}10 time expansion"
        );
        assert_eq!(
            NoteAction::RemoveCorrection(user).label(),
            "Remove \u{d7}10 correction"
        );
        assert_eq!(
            NoteAction::RemoveCorrection(commons).label(),
            "Remove \u{d7}2.5 correction"
        );
        assert_eq!(
            NoteAction::ApplyCorrection(guano).label(),
            "Correct for \u{d7}10 time expansion"
        );
        assert_eq!(
            NoteAction::ApplyCorrection(commons).label(),
            "Correct for \u{d7}2.5 time expansion"
        );
        assert_eq!(
            NoteAction::CorrectTenfold
                .tooltip(38_400, Kept::UntilClosed)
                .as_deref(),
            Some(
                "Sets the sample rate to 384\u{a0}kHz until you close the file or reload \
                 the page. The audio file is not changed."
            )
        );
        assert_eq!(
            NoteAction::CorrectTenfold
                .tooltip(31_250, Kept::UntilClosed)
                .as_deref(),
            Some(
                "Sets the sample rate to 312.5\u{a0}kHz until you close the file or \
                 reload the page. The audio file is not changed."
            )
        );
        assert_eq!(
            NoteAction::RemoveCorrection(user).tooltip(38_400, Kept::UntilClosed),
            None
        );
        assert_eq!(
            NoteAction::RemoveCorrection(guano)
                .tooltip(38_400, Kept::UntilClosed)
                .as_deref(),
            Some(
                "Sets the sample rate to 38.4\u{a0}kHz until you close the file or reload \
                 the page. The audio file is not changed."
            )
        );
        assert_eq!(
            NoteAction::ApplyCorrection(guano)
                .tooltip(38_400, Kept::UntilClosed)
                .as_deref(),
            Some("Sets the sample rate to 384\u{a0}kHz.")
        );
        assert_eq!(
            NoteAction::ApplyCorrection(commons)
                .tooltip(48_000, Kept::UntilClosed)
                .as_deref(),
            Some("Sets the sample rate to 120\u{a0}kHz.")
        );
        assert_eq!(
            NoteAction::CorrectTenfold.target(),
            Some((10.0, TimeExpansionOrigin::User))
        );
        assert_eq!(NoteAction::RemoveCorrection(guano).target(), None);
        // Made again with the origin it was opened with.
        assert_eq!(
            NoteAction::ApplyCorrection(guano).target(),
            Some((10.0, TimeExpansionOrigin::File))
        );
        assert_eq!(
            NoteAction::ApplyCorrection(commons).target(),
            Some((2.5, TimeExpansionOrigin::Source))
        );
    }

    fn opened(stated_in: StatedIn) -> OpenedCorrection {
        let origin = match stated_in {
            StatedIn::Guano | StatedIn::WildlifeAcoustics | StatedIn::FileMetadata => {
                TimeExpansionOrigin::File
            }
            StatedIn::Site(_) | StatedIn::FindRecordings => TimeExpansionOrigin::Source,
        };
        OpenedCorrection {
            te: TimeExpansion {
                factor: 10.0,
                stored_rate: 38_400,
                origin,
            },
            stated_in,
        }
    }

    #[test]
    fn the_note_names_where_the_factor_is_stated() {
        let te = |factor: f64, origin| TimeExpansion {
            factor,
            stored_rate: 38_400,
            origin,
        };
        let file = te(10.0, TimeExpansionOrigin::File);
        let source = te(10.0, TimeExpansionOrigin::Source);
        let commons = Some(("Wikimedia Commons", true));
        let inat = Some(("iNaturalist", false));
        for (te, guano, wamd, site, expected) in [
            (file, Some(10.0), None, None, StatedIn::Guano),
            (file, None, Some(10.0), None, StatedIn::WildlifeAcoustics),
            // Both state it: GUANO, which the loader reads first.
            (file, Some(10.0), Some(10.0), None, StatedIn::Guano),
            // Only the wamd chunk gives the factor the file was opened with.
            (
                file,
                Some(8.0),
                Some(10.0),
                None,
                StatedIn::WildlifeAcoustics,
            ),
            (file, None, None, None, StatedIn::FileMetadata),
            (
                source,
                None,
                None,
                commons,
                StatedIn::Site("Wikimedia Commons".to_string()),
            ),
            (source, None, None, inat, StatedIn::FindRecordings),
            (source, None, None, None, StatedIn::FindRecordings),
            // A Find recordings result whose factor the file's GUANO gives too.
            (source, Some(10.0), None, inat, StatedIn::Guano),
            (
                te(2.5, TimeExpansionOrigin::Source),
                None,
                Some(10.0),
                commons,
                StatedIn::Site("Wikimedia Commons".to_string()),
            ),
        ] {
            assert_eq!(
                stated_in(te, guano, wamd, site),
                expected,
                "{te:?} {guano:?} {wamd:?} {site:?}"
            );
        }
    }

    #[test]
    fn a_removed_correction_says_what_the_metadata_states() {
        for (stated_in, phrase) in [
            (StatedIn::Guano, "in the file's GUANO metadata"),
            (
                StatedIn::WildlifeAcoustics,
                "in the file's Wildlife Acoustics metadata",
            ),
            (StatedIn::FileMetadata, "in the file's metadata"),
            (
                StatedIn::Site("Wikimedia Commons".into()),
                "on Wikimedia Commons",
            ),
            (StatedIn::FindRecordings, "in the Find recordings result"),
        ] {
            let o = opened(stated_in);
            assert_eq!(
                rate_notes(
                    None,
                    Some(&o),
                    None,
                    None,
                    Correction::Possible,
                    Kept::UntilClosed
                ),
                [RateNote {
                    kind: NoteKind::TimeExpansion,
                    lines: vec![NoteLine::muted(format!(
                        "Correction for \u{d7}10 time expansion removed until you close the \
                         file or reload the page. Time expansion {phrase}:\u{a0}\u{d7}10."
                    ))],
                    action: Some(NoteAction::ApplyCorrection(o.te)),
                }]
            );
        }
        // While the correction is on, the note is the usual one.
        let o = opened(StatedIn::Guano);
        assert_eq!(
            rate_notes(
                Some(o.te),
                Some(&o),
                None,
                None,
                Correction::Possible,
                Kept::UntilClosed
            ),
            rate_notes(
                Some(o.te),
                None,
                None,
                None,
                Correction::Possible,
                Kept::UntilClosed
            )
        );
    }

    #[test]
    fn a_removed_correction_leaves_only_the_species_line() {
        // XC844459: Pipistrellus pipistrellus at 38.4 kHz is case T once its
        // GUANO correction is removed. The time-expansion note gives the
        // factor in the GUANO and has the only button; the species note
        // gives the species' calls, with no "Probably time-expanded ×10"
        // guess, whatever the factor.
        let species = finding("Pipistrellus pipistrellus", ECHO, 38_400).unwrap();
        assert_eq!(
            species.case,
            crate::species_check::RangeCase::TenfoldHeaderRate
        );
        for factor in [10.0, 8.0, 2.5] {
            let mut o = opened(StatedIn::Guano);
            o.te.factor = factor;
            for correction in [Correction::Possible, Correction::Streamed] {
                let notes = rate_notes(
                    None,
                    Some(&o),
                    None,
                    Some(&species),
                    correction,
                    Kept::UntilClosed,
                );
                assert_eq!(kinds(&notes), [NoteKind::TimeExpansion, NoteKind::Species]);
                assert_eq!(
                    notes[0].action,
                    (correction == Correction::Possible)
                        .then_some(NoteAction::ApplyCorrection(o.te))
                );
                assert_eq!(
                    notes[1],
                    RateNote {
                        kind: NoteKind::Species,
                        lines: vec![crate::species_check::fact_line(&species)],
                        action: None,
                    },
                    "x{factor} {correction:?}"
                );
            }
        }
        // At a rate that is not a tenth of a full-spectrum rate (case Other),
        // the list of bat detector outputs goes too.
        let other = finding("Pipistrellus kuhlii", ECHO, 48_000).unwrap();
        assert_eq!(other.case, crate::species_check::RangeCase::Other);
        assert_eq!(crate::species_check::note_lines(&other).len(), 2);
        let o = opened(StatedIn::Guano);
        let notes = rate_notes(
            None,
            Some(&o),
            None,
            Some(&other),
            Correction::Possible,
            Kept::UntilClosed,
        );
        assert_eq!(notes[1].lines, [crate::species_check::fact_line(&other)]);
        // A level C heterodyne note is unchanged: it already has only the
        // species line.
        let species = finding("Eptesicus nilssonii", ECHO, 44_100).unwrap();
        let hint_c = hint("Device", "Pettersson D980", 44_100);
        assert_eq!(hint_c.level, HeterodyneLevel::MayBeHeterodyne);
        let with_hint = |o: Option<&OpenedCorrection>| {
            rate_notes(
                None,
                o,
                Some(&hint_c),
                Some(&species),
                Correction::Possible,
                Kept::UntilClosed,
            )
            .pop()
            .unwrap()
        };
        assert_eq!(with_hint(Some(&o)), with_hint(None));
        assert_eq!(with_hint(None).kind, NoteKind::Heterodyne);
    }

    #[test]
    fn heterodyne_levels_a_and_b_hide_the_species_note() {
        // XC924726: "Recording with Magenta Bat5" is level B.
        let species = finding("Rhinolophus pumilus", ECHO, 48_000).unwrap();
        let hint_b = hint("Remarks", "Recording with Magenta Bat5", 48_000);
        assert_eq!(hint_b.level, HeterodyneLevel::HeterodyneOnlyDetector);
        assert_eq!(
            rate_notes(
                None,
                None,
                Some(&hint_b),
                Some(&species),
                Correction::Possible,
                Kept::UntilClosed
            ),
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
            rate_notes(
                None,
                None,
                Some(&hint_a),
                Some(&species),
                Correction::Possible,
                Kept::UntilClosed
            ),
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
            rate_notes(
                None,
                None,
                Some(&hint_c),
                Some(&species),
                Correction::Possible,
                Kept::UntilClosed
            ),
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
            rate_notes(
                None,
                None,
                Some(&hint_c),
                None,
                Correction::Possible,
                Kept::UntilClosed
            ),
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
            rate_notes(
                None,
                None,
                None,
                Some(&species),
                Correction::Possible,
                Kept::UntilClosed
            ),
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
        let notes = rate_notes(
            Some(te),
            None,
            None,
            Some(&species),
            Correction::Possible,
            Kept::UntilClosed,
        );
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
        let notes = rate_notes(
            None,
            None,
            Some(&hint_a),
            None,
            Correction::Possible,
            Kept::UntilClosed,
        );
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
            let lines = rate_notes(
                None,
                None,
                Some(&hint),
                None,
                Correction::Possible,
                Kept::UntilClosed,
            )
            .remove(0)
            .lines;
            let quote = lines.iter().find(|l| l.role == LineRole::Quote).unwrap();
            assert!(quote.text.contains(quoted), "{:?}", quote.text);
        }
    }
}
