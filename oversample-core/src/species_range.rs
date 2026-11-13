//! Whether a recording's sample rate is too low for the echolocation calls of
//! the species it is labelled with.
//!
//! A file can only contain frequencies up to half its sample rate. When that
//! limit is below the species' lowest echolocation call frequency, the file
//! can't hold the calls at their real frequencies. Of 11,502 Xeno-canto bat
//! recordings (November 2026), 94 are like this, all at 96 kHz or less.
//! Nearly all are a bat detector's output (heterodyne, frequency division, or
//! time expansion the file's rate doesn't show) or real-time recordings of
//! the bat's audible social calls.
//!
//! [`range_case`] gives a result when all of these hold:
//! 1. The species is a bat ([`is_bat_family`]) that echolocates, with a
//!    lowest call frequency above 0.
//! 2. The recording's sound type is absent, or may include echolocation
//!    ([`sound_type_may_include_echolocation`]).
//! 3. Half the sample rate, after any time-expansion correction, is strictly
//!    below the species' lowest call frequency. There is no margin: the 12
//!    files between 0.8 and 1 times the lowest frequency were as often
//!    detector output as the 82 further below.
//!
//! The result tells two cases apart ([`RangeCase`]).

use crate::heterodyne_hint::TE_HEADER_RATES;

/// Families of bats (order Chiroptera). The bat book also lists whales,
/// dolphins, a porpoise and seals. The toothed whales and the porpoise
/// echolocate (Phocoena dioptrica at 110 to 140 kHz), so `echolocates` alone
/// doesn't leave them out.
const BAT_FAMILIES: &[&str] = &[
    "Cistugidae",
    "Craseonycteridae",
    "Emballonuridae",
    "Furipteridae",
    "Hipposideridae",
    "Megadermatidae",
    "Miniopteridae",
    "Molossidae",
    "Mormoopidae",
    "Mystacinidae",
    "Myzopodidae",
    "Natalidae",
    "Noctilionidae",
    "Nycteridae",
    "Phyllostomidae",
    "Pteropodidae",
    "Rhinolophidae",
    "Rhinonycteridae",
    "Rhinopomatidae",
    "Thyropteridae",
    "Vespertilionidae",
];

/// Whether `family` is a bat family. Case is ignored.
pub fn is_bat_family(family: &str) -> bool {
    let family = family.trim();
    BAT_FAMILIES.iter().any(|f| f.eq_ignore_ascii_case(family))
}

/// Which of two kinds of file a [`range_case`] result is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangeCase {
    /// The header rate is a rate that full-spectrum and time-expanded files
    /// get when they are saved at one tenth of their real rate (19.2, 25,
    /// 25.6, 30, 31.25, 38.4 or 50 kHz), the app didn't correct it, and at ten
    /// times the rate the file would reach the species' calls. All 10 such
    /// Xeno-canto recordings were time-expanded ×10 (256 or 384 kHz
    /// recordings).
    TenfoldHeaderRate,
    /// Any other file below the species' calls: a bat detector's output
    /// (heterodyne, frequency division, or time expansion by a factor the rate
    /// doesn't give away), or a recording of only the bat's lower sounds.
    /// 27 of the 37 such Xeno-canto recordings were detector output.
    Other,
}

/// What the check needs to know about a species.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpeciesCallRange<'a> {
    /// Taxonomic family, such as "Vespertilionidae".
    pub family: &'a str,
    pub echolocates: bool,
    /// Lowest echolocation call frequency, Hz.
    pub freq_lo_hz: f64,
}

/// A file's sample rate, before and after any time-expansion correction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FileRate {
    /// The rate as loaded, after any time-expansion correction.
    pub sample_rate: u32,
    /// The rate the file header states. Equal to `sample_rate` when no
    /// correction was applied.
    pub header_rate: u32,
    /// Whether the app corrected the rate for time expansion when it loaded
    /// the file (a factor other than 1).
    pub time_expansion_applied: bool,
}

impl FileRate {
    /// The highest frequency the file can contain, Hz: half the sample rate.
    pub fn limit_hz(self) -> f64 {
        f64::from(self.sample_rate) / 2.0
    }
}

/// Whether a recording whose source gives this sound type may contain
/// echolocation calls: no sound type, or one that contains "echoloc" or
/// "uncertain" (case ignored).
///
/// A sound type that names only other sounds (social call, song, distress
/// call) gives `false`: 45 of the 47 such low-rate Xeno-canto recordings were
/// real-time recordings of audible calls. So does "feeding buzz" without
/// "echolocation": all 20 such recordings were audible noctule calls.
pub fn sound_type_may_include_echolocation(sound_type: Option<&str>) -> bool {
    let Some(sound_type) = sound_type.map(str::trim).filter(|t| !t.is_empty()) else {
        return true;
    };
    let sound_type = sound_type.to_lowercase();
    sound_type.contains("echoloc") || sound_type.contains("uncertain")
}

/// Whether the file's sample rate is too low for the species' echolocation
/// calls, and which case it is. `None` when the rule doesn't apply or the
/// file's limit reaches the calls. See the module documentation.
pub fn range_case(
    species: SpeciesCallRange,
    sound_type: Option<&str>,
    rate: FileRate,
) -> Option<RangeCase> {
    let lowest = species.freq_lo_hz;
    if !species.echolocates || !is_bat_family(species.family) || lowest.is_nan() || lowest <= 0.0 {
        return None;
    }
    if rate.sample_rate == 0 || !sound_type_may_include_echolocation(sound_type) {
        return None;
    }
    let limit = rate.limit_hz();
    if limit >= lowest {
        return None;
    }
    let tenfold = !rate.time_expansion_applied
        && TE_HEADER_RATES.contains(&rate.header_rate)
        && limit * 10.0 >= lowest;
    Some(if tenfold {
        RangeCase::TenfoldHeaderRate
    } else {
        RangeCase::Other
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use RangeCase::{Other, TenfoldHeaderRate};

    const VESPER: &str = "Vespertilionidae";

    fn bat(freq_lo_khz: f64) -> SpeciesCallRange<'static> {
        SpeciesCallRange {
            family: VESPER,
            echolocates: true,
            freq_lo_hz: freq_lo_khz * 1000.0,
        }
    }

    /// A file loaded at its header rate, with no time-expansion correction.
    fn plain(rate: u32) -> FileRate {
        FileRate {
            sample_rate: rate,
            header_rate: rate,
            time_expansion_applied: false,
        }
    }

    /// A file whose header rate was corrected by `factor` when it loaded.
    fn corrected(header_rate: u32, factor: u32) -> FileRate {
        FileRate {
            sample_rate: header_rate * factor,
            header_rate,
            time_expansion_applied: true,
        }
    }

    const ECHO: Option<&str> = Some("echolocation");

    #[test]
    fn tenfold_header_rates() {
        // XC892810, Pteronotus parnellii (44-62 kHz) at 38.4 kHz.
        assert_eq!(
            range_case(bat(44.0), ECHO, plain(38_400)),
            Some(TenfoldHeaderRate)
        );
        // XC892815, Pteronotus quadridens (65-100 kHz).
        assert_eq!(
            range_case(bat(65.0), ECHO, plain(38_400)),
            Some(TenfoldHeaderRate)
        );
        // XC1001172, Eptesicus nilssonii (25-45 kHz) at 25.6 kHz.
        assert_eq!(
            range_case(bat(25.0), ECHO, plain(25_600)),
            Some(TenfoldHeaderRate)
        );
        // XC844459, Pipistrellus pipistrellus (42-51 kHz) at 38.4 kHz.
        assert_eq!(
            range_case(bat(42.0), ECHO, plain(38_400)),
            Some(TenfoldHeaderRate)
        );
        for rate in [19_200, 25_000, 25_600, 30_000, 31_250, 38_400, 50_000] {
            let lowest = f64::from(rate) / 2000.0 + 1.0;
            assert_eq!(
                range_case(bat(lowest), ECHO, plain(rate)),
                Some(TenfoldHeaderRate),
                "{rate}"
            );
        }
    }

    #[test]
    fn other_rates() {
        // XC1099113, Pipistrellus kuhlii (35-45 kHz) at 48 kHz.
        assert_eq!(range_case(bat(35.0), ECHO, plain(48_000)), Some(Other));
        // XC759031, P. nathusii (35-44 kHz) at 32 kHz with no sound type:
        // 32 kHz is not in the ×10 list.
        assert_eq!(range_case(bat(35.0), None, plain(32_000)), Some(Other));
        assert_eq!(range_case(bat(35.0), Some(""), plain(32_000)), Some(Other));
        // XC1099116, P. pygmaeus (51-60 kHz) at 96 kHz.
        assert_eq!(range_case(bat(51.0), ECHO, plain(96_000)), Some(Other));
        // XC1136340, Rhinolophus affinis (66-90 kHz) lower harmonic at 96 kHz.
        let harmonic = Some("echolocation, echolocation signals of the lower harmonic (H1)");
        assert_eq!(
            range_case(
                SpeciesCallRange {
                    family: "Rhinolophidae",
                    ..bat(66.0)
                },
                harmonic,
                plain(96_000)
            ),
            Some(Other)
        );
        // XC961389, Nyctalus leisleri (21-30 kHz) at 24 kHz.
        assert_eq!(range_case(bat(21.0), ECHO, plain(24_000)), Some(Other));
        // XC924726, Rhinolophus pumilus (104-121 kHz) at 48 kHz. The app shows
        // the heterodyne note instead (level B).
        assert_eq!(range_case(bat(104.0), ECHO, plain(48_000)), Some(Other));
        // XC933228, Eptesicus nilssonii (25-45 kHz) at 44.1 kHz.
        let buzz = Some("echolocation, feeding buzz");
        assert_eq!(range_case(bat(25.0), buzz, plain(44_100)), Some(Other));
        // Aeorestes cinereus (17-36 kHz) at 22.05 kHz.
        assert_eq!(range_case(bat(17.0), ECHO, plain(22_050)), Some(Other));
    }

    #[test]
    fn a_tenfold_rate_too_low_even_at_ten_times_is_other() {
        // Rhinolophus hipposideros (105-115 kHz) at 19.2 kHz: ×10 gives a
        // 96 kHz limit, still below 105 kHz.
        assert_eq!(range_case(bat(105.0), ECHO, plain(19_200)), Some(Other));
        // Exactly at ×10 counts as reaching the calls.
        assert_eq!(
            range_case(bat(96.0), ECHO, plain(19_200)),
            Some(TenfoldHeaderRate)
        );
    }

    #[test]
    fn a_corrected_file_is_never_tenfold() {
        // 25.6 kHz corrected ×2 (51.2 kHz), P. pygmaeus (51-60 kHz).
        assert_eq!(
            range_case(bat(51.0), ECHO, corrected(25_600, 2)),
            Some(Other)
        );
        // 38.4 kHz corrected ×10 (384 kHz), P. pipistrellus: no result.
        assert_eq!(range_case(bat(42.0), ECHO, corrected(38_400, 10)), None);
    }

    #[test]
    fn limit_at_or_above_the_lowest_call_gives_nothing() {
        // XC891469, Nyctinomops macrotis (10-33 kHz) at 38.4 kHz.
        assert_eq!(range_case(bat(10.0), ECHO, plain(38_400)), None);
        // XC1002662, Plecotus ognevi (22-55 kHz) at 44.1 kHz: 22.05 kHz is not
        // below 22 kHz.
        assert_eq!(range_case(bat(22.0), ECHO, plain(44_100)), None);
        // Strict comparison: 25 kHz is not below 25 kHz.
        assert_eq!(range_case(bat(25.0), ECHO, plain(50_000)), None);
        // XC1062080, Myotis blythii (20-120 kHz) at 44.1 kHz.
        assert_eq!(range_case(bat(20.0), ECHO, plain(44_100)), None);
        // XC1154390, Tadarida teniotis (8-18 kHz) at 22.05 kHz.
        assert_eq!(range_case(bat(8.0), ECHO, plain(22_050)), None);
        // XC1062211, Pipistrellus hanaki (44-52 kHz) at 441 kHz.
        assert_eq!(range_case(bat(44.0), ECHO, plain(441_000)), None);
        // XC928094, Myotis dasycneme (22-97 kHz) at 256 kHz.
        assert_eq!(range_case(bat(22.0), ECHO, plain(256_000)), None);
    }

    #[test]
    fn sound_types_that_name_only_other_calls_give_nothing() {
        for sound_type in [
            "social call",
            "song",
            "distress call",
            "feeding buzz",
            "feeding buzz, Wind.",
            "social call, aggressive call",
        ] {
            assert_eq!(
                range_case(bat(35.0), Some(sound_type), plain(44_100)),
                None,
                "{sound_type}"
            );
        }
        for sound_type in ["Echolocation", "uncertain", "Uncertain, social call", "  "] {
            assert!(
                sound_type_may_include_echolocation(Some(sound_type)),
                "{sound_type}"
            );
        }
        assert!(sound_type_may_include_echolocation(None));
    }

    #[test]
    fn species_that_dont_count() {
        // Non-echolocating fruit bats (Pteropus, Cynopterus brachyotis, which
        // has a 10-60 kHz range).
        let fruit_bat = SpeciesCallRange {
            family: "Pteropodidae",
            echolocates: false,
            freq_lo_hz: 10_000.0,
        };
        assert_eq!(range_case(fruit_bat, ECHO, plain(4_000)), None);
        // Phocoena dioptrica (110-140 kHz) at 96 kHz: not a bat.
        let porpoise = SpeciesCallRange {
            family: "Phocoenidae",
            echolocates: true,
            freq_lo_hz: 110_000.0,
        };
        assert_eq!(range_case(porpoise, ECHO, plain(96_000)), None);
        // No range, or no rate.
        assert_eq!(range_case(bat(0.0), ECHO, plain(44_100)), None);
        assert_eq!(range_case(bat(f64::NAN), ECHO, plain(44_100)), None);
        assert_eq!(range_case(bat(35.0), ECHO, plain(0)), None);
    }

    #[test]
    fn bat_families() {
        assert!(is_bat_family("Vespertilionidae"));
        assert!(is_bat_family("rhinolophidae"));
        for family in [
            "Phocoenidae",
            "Delphinidae",
            "Ziphiidae",
            "Physeteridae",
            "Balaenopteridae",
            "Balaenidae",
            "Phocidae",
            "",
        ] {
            assert!(!is_bat_family(family), "{family}");
        }
    }
}
