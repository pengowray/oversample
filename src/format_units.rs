//! Frequencies and sample rates written in kHz, for text that can wrap.
//!
//! [`khz`], [`khz_range`] and [`sample_rate_khz`] put a no-break space
//! (U+00A0) between the number and "kHz", so the unit stays on its number's
//! line when the text wraps. The notes under the Info panel's Sample rate row
//! ([`crate::rate_notes`]) and the "Heterodyne recording" badge tooltip use
//! them. [`keep_units_with_numbers`] does the same in text quoted from a
//! recording's metadata.
//!
//! The numbers follow one of two rules, which round differently:
//!
//! - [`khz_value`], for frequencies: rounded half up, with no trailing zeros.
//!   31,250 Hz gives "31.3" and 11,025 Hz gives "11".
//! - [`sample_rate_khz_value`], for sample rates: no decimals for a whole
//!   number of kHz, otherwise exactly one. 31,250 Hz gives "31.2" and
//!   11,025 Hz gives "11.0".
//!
//! The toolbar's sample rate chip ("48kHz") and the frequency axis labels
//! (`freq_marker_label` in oversample-core) have their own formats and are
//! not written here. The frequency axis hover text
//! ([`crate::heterodyne::axis_call_suffixes`]) uses [`khz_value`] for its
//! numbers.

/// The space between a number and its "kHz".
const NO_BREAK_SPACE: char = '\u{a0}';

/// A word joiner. After a range's en dash, it keeps the second number on the
/// first number's line.
const WORD_JOINER: char = '\u{2060}';

/// A positive frequency in Hz as a number of kHz with no unit, rounded to at
/// most `decimals` (1 or 2) decimal places, with no trailing zeros: 21,900 Hz
/// gives "21.9", 40,000 Hz gives "40" and, with 2 decimals, 31,950 Hz gives
/// "31.95".
pub fn khz_value(hz: i64, decimals: u32) -> String {
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

/// `number` followed by a no-break space and "kHz".
fn with_unit(number: &str) -> String {
    format!("{number}{NO_BREAK_SPACE}kHz")
}

/// A frequency in Hz as kHz with its unit: "21.9 kHz", with a no-break space.
/// The number is [`khz_value`]'s.
pub fn khz(hz: i64, decimals: u32) -> String {
    with_unit(&khz_value(hz, decimals))
}

/// A frequency range with its unit: "35–45 kHz", with an en dash as the app's
/// other ranges have. A word joiner after the dash and a no-break space
/// before "kHz" keep the whole range on one line.
pub fn khz_range(lo_hz: i64, hi_hz: i64, decimals: u32) -> String {
    format!(
        "{}\u{2013}{WORD_JOINER}{}",
        khz_value(lo_hz, decimals),
        khz(hi_hz, decimals)
    )
}

/// A sample rate in Hz as a number of kHz with no unit. A rate that is a
/// whole number of kHz has no decimals ("384"); any other rate has exactly
/// one ("44.1", "38.4", and "11.0" for 11,025 Hz).
pub fn sample_rate_khz_value(hz: u32) -> String {
    if hz.is_multiple_of(1000) {
        (hz / 1000).to_string()
    } else {
        format!("{:.1}", f64::from(hz) / 1000.0)
    }
}

/// A sample rate with its unit: "38.4 kHz", with a no-break space. The
/// number is [`sample_rate_khz_value`]'s.
pub fn sample_rate_khz(hz: u32) -> String {
    with_unit(&sample_rate_khz_value(hz))
}

/// A sample rate in the short form of the microphone lists (the Choose
/// Microphone rows and the Max sample rate list): "44.1k", "48k", "384k".
/// A rate under 1 kHz is written in Hz with no unit.
pub fn sample_rate_short(hz: u32) -> String {
    if hz >= 1000 {
        format!("{}k", sample_rate_khz_value(hz))
    } else {
        hz.to_string()
    }
}

/// `text` with the space between a number and its "kHz" or "Hz" changed to a
/// no-break space, so the unit stays on its number's line when the text
/// wraps: "set at 31,9 kHz" keeps "31,9 kHz" together.
///
/// This is for text quoted from a recording's metadata, which keeps the
/// recordist's spelling. Any letter case counts ("khz", "KHz"), and so does a
/// space inside the unit ("45 k Hz"), which also becomes a no-break space.
/// Only a single ordinary space after a digit changes, and only when the unit
/// ends the word, so "5 hours" and "40 khzs" stay as they are.
pub fn keep_units_with_numbers(text: &str) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    for i in 1..chars.len() {
        if chars[i] == ' ' && chars[i - 1].is_ascii_digit() {
            if let Some(inner_space) = unit_after(&chars, i + 1) {
                chars[i] = NO_BREAK_SPACE;
                if let Some(at) = inner_space {
                    chars[at] = NO_BREAK_SPACE;
                }
            }
        }
    }
    chars.into_iter().collect()
}

/// Whether `chars[from..]` starts with "Hz", "kHz" or "k Hz" in any letter
/// case, followed by the end of the text or a character that is not a letter
/// or digit. For "k Hz", it gives the index of the space inside the unit.
fn unit_after(chars: &[char], from: usize) -> Option<Option<usize>> {
    let at = |i: usize| chars.get(i).map(|c| c.to_ascii_lowercase());
    let mut i = from;
    let mut inner_space = None;
    if at(i) == Some('k') {
        i += 1;
        if at(i) == Some(' ') {
            inner_space = Some(i);
            i += 1;
        }
    }
    let ends_word = !chars.get(i + 2).is_some_and(|c| c.is_alphanumeric());
    (at(i) == Some('h') && at(i + 1) == Some('z') && ends_word).then_some(inner_space)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_sample_rates() {
        assert_eq!(sample_rate_short(44_100), "44.1k");
        assert_eq!(sample_rate_short(48_000), "48k");
        assert_eq!(sample_rate_short(384_000), "384k");
        assert_eq!(sample_rate_short(800), "800");
    }

    #[test]
    fn khz_values_round_and_drop_trailing_zeros() {
        assert_eq!(khz_value(21_900, 1), "21.9");
        assert_eq!(khz_value(40_000, 1), "40");
        assert_eq!(khz_value(1_900, 1), "1.9");
        assert_eq!(khz_value(31_950, 1), "32");
        assert_eq!(khz_value(31_950, 2), "31.95");
        assert_eq!(khz_value(31_905, 2), "31.91");
        assert_eq!(khz_value(31_050, 2), "31.05");
        assert_eq!(khz_value(31_900, 2), "31.9");
        // The same rates as sample_rate_khz_value's "31.2" and "11.0".
        assert_eq!(khz_value(31_250, 1), "31.3");
        assert_eq!(khz_value(11_025, 1), "11");
    }

    #[test]
    fn the_unit_follows_a_no_break_space() {
        assert_eq!(khz(21_900, 1), "21.9\u{a0}kHz");
        assert_eq!(khz(31_950, 2), "31.95\u{a0}kHz");
        assert_eq!(
            khz_range(35_000, 45_000, 1),
            "35\u{2013}\u{2060}45\u{a0}kHz"
        );
        assert_eq!(sample_rate_khz(38_400), "38.4\u{a0}kHz");
        for text in [
            khz(40_000, 1),
            khz_range(105_000, 115_000, 1),
            sample_rate_khz(44_100),
        ] {
            assert!(!text.contains(' '), "{text:?}");
        }
    }

    #[test]
    fn sample_rates_keep_the_tenth_of_a_kilohertz() {
        assert_eq!(sample_rate_khz_value(384_000), "384");
        assert_eq!(sample_rate_khz_value(38_400), "38.4");
        assert_eq!(sample_rate_khz_value(44_100), "44.1");
        assert_eq!(sample_rate_khz_value(500_000), "500");
        assert_eq!(sample_rate_khz_value(31_250), "31.2");
        assert_eq!(sample_rate_khz_value(22_050), "22.1");
        // Not a whole number of kHz, so the decimal stays.
        assert_eq!(sample_rate_khz_value(11_025), "11.0");
    }

    #[test]
    fn a_quoted_unit_stays_with_its_number() {
        let nb = |s: &str| s.replace('_', "\u{a0}");
        // Spellings found in Xeno-canto remarks.
        for (text, expected) in [
            (
                "hetrodyne detector set at 31,9 kHz",
                "hetrodyne detector set at 31,9_kHz",
            ),
            (
                "bat detector set at 31.9 KHz",
                "bat detector set at 31.9_KHz",
            ),
            ("set on 51 3 khz", "set on 51 3_khz"),
            ("tuned to 45 k Hz", "tuned to 45_k_Hz"),
            ("set to 23,5 k hz", "set to 23,5_k_hz"),
            ("312 500 Hz is the rate", "312 500_Hz is the rate"),
            (
                "tuned to 40 kHz, later 50 kHz.",
                "tuned to 40_kHz, later 50_kHz.",
            ),
            ("(45 kHz)", "(45_kHz)"),
        ] {
            assert_eq!(keep_units_with_numbers(text), nb(expected), "{text:?}");
        }
        // Left as they are: no unit, a longer word, no digit before the space,
        // a unit already joined to its number, and two spaces.
        for text in [
            "Recording with Magenta Bat5",
            "after 5 hours",
            "40 khzs",
            "set to forty kHz",
            "Audiomoth 192kHz",
            "45  kHz",
            "45 k  Hz",
            "",
            "5 ",
            "5 k",
            "5 kH",
        ] {
            assert_eq!(keep_units_with_numbers(text), text, "{text:?}");
        }
        // Text that is not ASCII is kept, character for character.
        assert_eq!(
            keep_units_with_numbers("détecteur hétérodyne à 40 kHz…"),
            "détecteur hétérodyne à 40\u{a0}kHz…"
        );
    }
}
