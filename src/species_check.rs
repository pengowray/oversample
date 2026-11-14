//! Says when a recording's sample rate is too low for the echolocation calls
//! of the species it is labelled with
//! ([`oversample_core::species_range`]).
//!
//! [`file_finding`] looks the species up in the bat book the way the
//! typical-calls strip does, and is stored on the `LoadedFile` when the file
//! is built, as the heterodyne hint is. [`note_lines`] gives the lines of the
//! species note under the Info panel's Sample rate row. [`crate::rate_notes`]
//! decides when the note is shown: a file with a heterodyne hint keeps the
//! heterodyne note, and only [`fact_line`] is added to it, at level C.

use crate::bat_book::auto_resolve::{country_region, find_species, get_scientific_name};
use crate::bat_book::types::BatSpecies;
use crate::format_units;
use crate::note_line::NoteLine;
use crate::state::LoadedFile;
use crate::types::TimeExpansion;
pub use oversample_core::species_range::RangeCase;
use oversample_core::species_range::{range_case, FileRate, SpeciesCallRange};

/// The site metadata label of the recording's sound type ("echolocation",
/// "social call"). Xeno-canto gives it; other sources don't.
const SOUND_TYPE_LABEL: &str = "Sound type";

/// A file whose sample rate is too low for its species' echolocation calls.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeciesRangeFinding {
    pub case: RangeCase,
    /// The species name as the recording's metadata gives it, such as
    /// "Lasiurus cinereus" for the bat book's Aeorestes cinereus.
    pub name: String,
    /// The species' echolocation call range, Hz. Always the species range,
    /// not a regional book's narrower one.
    pub freq_lo_hz: f64,
    pub freq_hi_hz: f64,
    pub rate: FileRate,
}

/// The finding for a file, from its species name (source site, then GUANO),
/// its sound type, and its sample rate. `None` when the file has no species,
/// the bat book doesn't have it, or the rate reaches its calls.
///
/// Neither the metadata nor the time expansion changes after a file is
/// built; recompute this if either ever does.
pub fn file_finding(file: &LoadedFile) -> Option<SpeciesRangeFinding> {
    let name = get_scientific_name(file)?;
    let species = find_species(&name, country_region(file))?;
    let sound_type = file.site_metadata.as_ref().and_then(|m| {
        m.fields
            .iter()
            .find(|(label, _)| label == SOUND_TYPE_LABEL)
            .map(|(_, value)| value.as_str())
    });
    finding(
        &name,
        species,
        sound_type,
        file.audio.metadata.time_expansion,
        file.audio.sample_rate,
    )
}

/// The finding for a species and a file's rate.
///
/// - `time_expansion`: the correction the app applied when it loaded the
///   file. A factor other than 1 counts as applied, as for the heterodyne hint.
/// - `sample_rate`: the file's rate as loaded, after any correction.
pub(crate) fn finding(
    name: &str,
    species: &BatSpecies,
    sound_type: Option<&str>,
    time_expansion: Option<TimeExpansion>,
    sample_rate: u32,
) -> Option<SpeciesRangeFinding> {
    let rate = FileRate {
        sample_rate,
        header_rate: time_expansion.map_or(sample_rate, |te| te.stored_rate),
        time_expansion_applied: time_expansion.is_some_and(|te| te.factor != 1.0),
    };
    let range = SpeciesCallRange {
        family: &species.family,
        echolocates: species.echolocates,
        freq_lo_hz: species.freq_lo_hz,
    };
    let case = range_case(range, sound_type, rate)?;
    Some(SpeciesRangeFinding {
        case,
        name: name.split_whitespace().collect::<Vec<_>>().join(" "),
        freq_lo_hz: species.freq_lo_hz,
        freq_hi_hz: species.freq_hi_hz,
        rate,
    })
}

/// A frequency in Hz as kHz, whole or with one decimal: "22.1 kHz",
/// "12.8 kHz", "11 kHz".
fn khz(hz: f64) -> String {
    format_units::khz(hz.round() as i64, 1)
}

/// A frequency range: "35–45 kHz".
fn khz_range(lo_hz: f64, hi_hz: f64) -> String {
    format_units::khz_range(lo_hz.round() as i64, hi_hz.round() as i64, 1)
}

/// The species line: "Pipistrellus kuhlii: echolocation calls at 35–45 kHz,
/// but this file can only contain frequencies up to 24 kHz." When the app
/// corrected the file for time expansion, the line says the limit is after
/// that correction, since the Sample rate row above shows the corrected rate
/// and the time-expansion note the file's own.
pub fn fact_line(f: &SpeciesRangeFinding) -> NoteLine {
    let after = if f.rate.time_expansion_applied {
        "after time-expansion correction, "
    } else {
        ""
    };
    NoteLine::muted(format!(
        "{}: echolocation calls at {}, but {after}this file can only contain frequencies \
         up to {}.",
        f.name,
        khz_range(f.freq_lo_hz, f.freq_hi_hz),
        khz(f.rate.limit_hz()),
    ))
}

/// The second line of the note for a file at any rate other than a ×10
/// time-expansion rate, up to [`OTHER_LINE_MAX_HEADER_RATE`]. It names
/// possibilities, not a finding: in the Xeno-canto data most such files were
/// detector output, but some were real-time recordings of audible calls or
/// of a lower harmonic.
const OTHER_LINE: &str = "This file may be a bat detector's output (heterodyne, time expansion \
                          or frequency division), or contain only lower-frequency sounds such \
                          as social calls.";

/// The highest header rate whose note gets [`OTHER_LINE`]. All 94
/// Xeno-canto records below their species' calls were at 96 kHz or less, so
/// there is no data on what a faster file like this is, and its note has
/// only the species line.
const OTHER_LINE_MAX_HEADER_RATE: u32 = 96_000;

/// The lines of the species note, for a file with no heterodyne hint.
///
/// - At a ×10 header rate, the finding first, in the warning colour, then
///   the species line. All 10 such Xeno-canto records were time-expanded
///   ×10, though the app corrects 2 of them from their GUANO (see
///   [`RangeCase::TenfoldHeaderRate`]). The note says how to read the file, since the app has no control to
///   correct it after loading.
/// - At any other rate, the species line, then the possibilities, all muted.
///   Above a 96 kHz header rate, the species line alone.
pub fn note_lines(f: &SpeciesRangeFinding) -> Vec<NoteLine> {
    match f.case {
        RangeCase::TenfoldHeaderRate => vec![
            NoteLine::warning(format!(
                "Probably time-expanded \u{d7}10 (recorded at {}). Frequencies shown are \
                 one tenth of the real ones, and durations are ten times longer.",
                khz(f64::from(f.rate.header_rate) * 10.0)
            )),
            fact_line(f),
        ],
        RangeCase::Other => {
            let mut lines = vec![fact_line(f)];
            if f.rate.header_rate <= OTHER_LINE_MAX_HEADER_RATE {
                lines.push(NoteLine::muted(OTHER_LINE));
            }
            lines
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bat_book::data::all_species;
    use crate::bat_book::types::BatBookRegion;
    use crate::note_line::LineRole::{self, Muted, Warning};

    /// `s` with the no-break space before each "kHz" and the word joiner
    /// after each range's en dash.
    fn spaced(s: &str) -> String {
        s.replace(" kHz", "\u{a0}kHz")
            .replace('\u{2013}', "\u{2013}\u{2060}")
    }

    /// The finding for a recording labelled `name`, looked up as the app
    /// does with no country.
    fn check(name: &str, sound_type: Option<&str>, rate: u32) -> Option<SpeciesRangeFinding> {
        check_te(name, sound_type, rate, None)
    }

    fn check_te(
        name: &str,
        sound_type: Option<&str>,
        rate: u32,
        te: Option<TimeExpansion>,
    ) -> Option<SpeciesRangeFinding> {
        let species = find_species(name, None)?;
        finding(name, species, sound_type, te, rate)
    }

    fn texts(lines: &[NoteLine]) -> Vec<String> {
        lines.iter().map(|l| l.text.clone()).collect()
    }

    fn roles(lines: &[NoteLine]) -> Vec<LineRole> {
        lines.iter().map(|l| l.role).collect()
    }

    const ECHO: Option<&str> = Some("echolocation");

    #[test]
    fn tenfold_note() {
        // XC892810.
        let f = check("Pteronotus parnellii", ECHO, 38_400).unwrap();
        assert_eq!(f.case, RangeCase::TenfoldHeaderRate);
        let lines = note_lines(&f);
        assert_eq!(
            texts(&lines),
            [
                spaced(
                    "Probably time-expanded \u{d7}10 (recorded at 384 kHz). Frequencies shown \
                     are one tenth of the real ones, and durations are ten times longer."
                ),
                spaced(
                    "Pteronotus parnellii: echolocation calls at 44\u{2013}62 kHz, but this file \
                     can only contain frequencies up to 19.2 kHz."
                ),
            ]
        );
        assert_eq!(roles(&lines), [Warning, Muted]);
        // XC1001172.
        let f = check("Eptesicus nilssonii", ECHO, 25_600).unwrap();
        assert_eq!(
            texts(&note_lines(&f)),
            [
                spaced(
                    "Probably time-expanded \u{d7}10 (recorded at 256 kHz). Frequencies shown \
                     are one tenth of the real ones, and durations are ten times longer."
                ),
                spaced(
                    "Eptesicus nilssonii: echolocation calls at 25\u{2013}45 kHz, but this file \
                     can only contain frequencies up to 12.8 kHz."
                ),
            ]
        );
        // A 31.25 kHz header rate: recorded at 312.5 kHz.
        let f = check("Pipistrellus pipistrellus", ECHO, 31_250).unwrap();
        assert!(note_lines(&f)[0]
            .text
            .contains(&spaced("(recorded at 312.5 kHz)")));
        // XC892815, XC844459.
        for (name, rate) in [
            ("Pteronotus quadridens", 38_400),
            ("Pipistrellus pipistrellus", 38_400),
        ] {
            let f = check(name, ECHO, rate).unwrap();
            assert_eq!(f.case, RangeCase::TenfoldHeaderRate, "{name}");
        }
    }

    #[test]
    fn other_note() {
        let other = OTHER_LINE.to_string();
        // XC1099113.
        let f = check("Pipistrellus kuhlii", ECHO, 48_000).unwrap();
        assert_eq!(f.case, RangeCase::Other);
        let lines = note_lines(&f);
        assert_eq!(
            texts(&lines),
            [
                spaced(
                    "Pipistrellus kuhlii: echolocation calls at 35\u{2013}45 kHz, but this file \
                     can only contain frequencies up to 24 kHz."
                ),
                other.clone(),
            ]
        );
        assert_eq!(roles(&lines), [Muted, Muted]);
        assert_eq!(
            OTHER_LINE,
            "This file may be a bat detector's output (heterodyne, time expansion or \
             frequency division), or contain only lower-frequency sounds such as social calls."
        );
        let fact = |name: &str, sound_type: Option<&str>, rate: u32| {
            let f = check(name, sound_type, rate).unwrap();
            assert_eq!(f.case, RangeCase::Other, "{name}");
            fact_line(&f).text
        };
        // XC1099116.
        assert_eq!(
            fact("Pipistrellus pygmaeus", ECHO, 96_000),
            spaced(
                "Pipistrellus pygmaeus: echolocation calls at 51\u{2013}60 kHz, but this file \
                 can only contain frequencies up to 48 kHz."
            )
        );
        // XC759031: no sound type; 32 kHz is not a ×10 rate.
        assert_eq!(
            fact("Pipistrellus nathusii", None, 32_000),
            spaced(
                "Pipistrellus nathusii: echolocation calls at 35\u{2013}44 kHz, but this file \
                 can only contain frequencies up to 16 kHz."
            )
        );
        // 19.2 kHz is a ×10 rate, but ×10 still doesn't reach 105 kHz.
        assert_eq!(
            fact("Rhinolophus hipposideros", ECHO, 19_200),
            spaced(
                "Rhinolophus hipposideros: echolocation calls at 105\u{2013}115 kHz, but this \
                 file can only contain frequencies up to 9.6 kHz."
            )
        );
        // A synonym keeps the recording's name; 11.025 kHz shows as 11.
        assert_eq!(
            fact("Lasiurus cinereus", ECHO, 22_050),
            spaced(
                "Lasiurus cinereus: echolocation calls at 17\u{2013}36 kHz, but this file can \
                 only contain frequencies up to 11 kHz."
            )
        );
        // XC1136340 (lower harmonic) and XC961389 (audible calls): known
        // false alarms of the rule.
        fact(
            "Rhinolophus affinis",
            Some("echolocation, echolocation signals of the lower harmonic (H1)"),
            96_000,
        );
        fact("Nyctalus leisleri", ECHO, 24_000);
    }

    /// The Xeno-canto data behind the second line has no file above 96 kHz.
    #[test]
    fn faster_files_get_only_the_species_line() {
        for rate in [192_000, 200_000] {
            let f = check("Rhinolophus hipposideros", ECHO, rate).unwrap();
            assert_eq!(f.case, RangeCase::Other);
            assert_eq!(texts(&note_lines(&f)), [fact_line(&f).text], "{rate}");
        }
        let f = check("Rhinolophus hipposideros", ECHO, 96_000).unwrap();
        assert_eq!(texts(&note_lines(&f)).last(), Some(&OTHER_LINE.to_string()));
        // A file corrected for time expansion goes by its header rate.
        let te = TimeExpansion {
            factor: 4.0,
            stored_rate: 48_000,
            origin: crate::types::TimeExpansionOrigin::File,
        };
        let f = check_te("Rhinolophus hipposideros", ECHO, 192_000, Some(te)).unwrap();
        assert_eq!(texts(&note_lines(&f)).len(), 2);
    }

    #[test]
    fn a_corrected_file_says_so() {
        let te = TimeExpansion {
            factor: 2.0,
            stored_rate: 25_600,
            origin: crate::types::TimeExpansionOrigin::File,
        };
        let f = check_te("Pipistrellus pygmaeus", ECHO, 51_200, Some(te)).unwrap();
        assert_eq!(f.case, RangeCase::Other);
        assert_eq!(
            texts(&note_lines(&f)),
            [
                spaced(
                    "Pipistrellus pygmaeus: echolocation calls at 51\u{2013}60 kHz, but after \
                     time-expansion correction, this file can only contain frequencies up to \
                     25.6 kHz."
                ),
                OTHER_LINE.to_string(),
            ]
        );
        // Corrected ×10 from 38.4 kHz: the limit reaches the calls.
        let te = TimeExpansion {
            factor: 10.0,
            stored_rate: 38_400,
            origin: crate::types::TimeExpansionOrigin::File,
        };
        assert_eq!(
            check_te("Pipistrellus pipistrellus", ECHO, 384_000, Some(te)),
            None
        );
        // A factor of 1 is no correction.
        let te = TimeExpansion {
            factor: 1.0,
            stored_rate: 38_400,
            origin: crate::types::TimeExpansionOrigin::File,
        };
        let f = check_te("Pipistrellus pipistrellus", ECHO, 38_400, Some(te)).unwrap();
        assert_eq!(f.case, RangeCase::TenfoldHeaderRate);
    }

    #[test]
    fn files_with_no_note() {
        for (name, sound_type, rate) in [
            // Limit at or above the lowest call frequency.
            ("Nyctinomops macrotis", ECHO, 38_400),
            ("Plecotus ognevi", ECHO, 44_100),
            ("Nyctalus aviator", ECHO, 44_100),
            ("Austronomus australis", ECHO, 48_000),
            ("Scotophilus kuhlii", ECHO, 96_000),
            ("Myotis blythii", ECHO, 44_100),
            ("Tadarida teniotis", ECHO, 22_050),
            ("Myotis dasycneme", ECHO, 256_000),
            // Only in the direct lookup; still above.
            ("Pipistrellus hanaki", ECHO, 441_000),
            // Sound types that name only other calls.
            ("Rhinolophus pearsonii", Some("social call"), 48_000),
            ("Pipistrellus nathusii", Some("social call"), 38_400),
            ("Nyctalus leisleri", Some("feeding buzz"), 24_000),
            ("Pipistrellus kuhlii", Some("social call"), 44_100),
            ("Vespertilio murinus", Some("song"), 192_000),
            // Non-echolocating fruit bats.
            ("Pteropus mariannus", Some("social call"), 44_100),
            ("Pteropus conspicillatus", ECHO, 48_000),
            ("Cynopterus brachyotis", Some("defensive call"), 44_100),
            ("Cynopterus brachyotis", ECHO, 4_000),
            // Not a bat.
            ("Phocoena dioptrica", ECHO, 96_000),
            // No species, or a name the book doesn't have.
            ("Mystery mystery", ECHO, 8_000),
            ("Sonus naturalis", ECHO, 8_000),
            ("PIPPIP", ECHO, 8_000),
            ("", ECHO, 8_000),
        ] {
            assert_eq!(check(name, sound_type, rate), None, "{name} at {rate}");
        }
    }

    #[test]
    fn names_are_matched_like_the_typical_calls_strip() {
        // A trinomial matches its binomial.
        let f = check("Myotis lucifugus alascensis", ECHO, 22_050).unwrap();
        assert_eq!(f.name, "Myotis lucifugus alascensis");
        assert_eq!(
            f.freq_lo_hz,
            find_species("Myotis lucifugus", None).unwrap().freq_lo_hz
        );
        // Pipistrellus hanaki is in no regional book.
        assert_eq!(
            find_species("Pipistrellus hanaki", None).map(|s| s.id.as_str()),
            Some("pipistrellus_hanaki")
        );
        // Two entries share "Eptesicus nilssonii". The recording's region
        // picks one, as in the strip; with no region, the first regional
        // book's (25-45 kHz, as in XC933228).
        assert_eq!(
            find_species("Eptesicus nilssonii", None).map(|s| s.id.as_str()),
            Some("eptesicus_nilssonii")
        );
        assert_eq!(
            find_species("Eptesicus nilssonii", Some(BatBookRegion::Japan)).map(|s| s.id.as_str()),
            Some("eptesicus_nilssonii_jp")
        );
        let f = check("Eptesicus nilssonii", ECHO, 44_100).unwrap();
        assert_eq!((f.freq_lo_hz, f.freq_hi_hz), (25_000.0, 45_000.0));
    }

    /// The check leaves out non-bats by family. A family added to the bat
    /// book must be classified here.
    #[test]
    fn every_family_in_the_book_is_classified() {
        const NOT_BATS: &[&str] = &[
            "Balaenidae",
            "Balaenopteridae",
            "Delphinidae",
            "Phocidae",
            "Phocoenidae",
            "Physeteridae",
            "Ziphiidae",
        ];
        for s in all_species() {
            let bat = oversample_core::species_range::is_bat_family(&s.family);
            assert!(
                bat != NOT_BATS.contains(&s.family.as_str()),
                "{} ({}): family {:?} is in neither list, or in both",
                s.id,
                s.scientific_name,
                s.family
            );
        }
    }
}
