//! Frequencies and sample rates written in kHz, for text that can wrap.
//!
//! [`khz`], [`khz_range`] and [`sample_rate_khz`] put a no-break space
//! (U+00A0) between the number and "kHz", so the unit stays on its number's
//! line when the text wraps. The notes under the Info panel's Sample rate row
//! ([`crate::rate_notes`]) and the "Heterodyne recording" badge tooltip use
//! them.
//!
//! The toolbar's sample rate chip ("48kHz") and the frequency axis labels
//! have their own formats and are not written here. The axis labels use
//! [`khz_value`] for their numbers.

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
