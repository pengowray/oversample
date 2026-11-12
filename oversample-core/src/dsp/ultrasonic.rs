//! Where bat calls start, and which recordings can hold them. One place for
//! the split between audible and ultrasonic sound that call detection, the
//! loudest-frequency line, zero-crossing division and the default HFR band
//! share. The lowest bat frequency itself is a user setting
//! (`AppState::lowest_bat_hz`); this module has its default.

/// Default for the user's "Lowest bat frequency" setting. Free-tailed bats
/// call as low as about 11 kHz; most other bats stay above 16 kHz.
pub const DEFAULT_LOWEST_BAT_HZ: f32 = 16_000.0;

/// Gap between the lowest bat frequency and the bottom of the band that is
/// searched or filtered for calls, so a call at the lowest frequency is not
/// cut by the band's own edge.
pub const BAND_BELOW_LOWEST_BAT_HZ: f32 = 1_000.0;

/// Recordings sampled faster than this count as ultrasonic: their Nyquist
/// frequency is above 24 kHz, beyond what CD and DAT-rate recorders capture.
pub const ULTRASONIC_ABOVE_SAMPLE_RATE: u32 = 48_000;

/// True when a recording at `sample_rate` can hold ultrasonic sound.
pub fn is_ultrasonic_rate(sample_rate: u32) -> bool {
    sample_rate > ULTRASONIC_ABOVE_SAMPLE_RATE
}

/// Bottom of the band searched or filtered for bat calls.
pub fn bat_band_lo_hz(lowest_bat_hz: f32) -> f32 {
    (lowest_bat_hz - BAND_BELOW_LOWEST_BAT_HZ).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rates_above_48_khz_are_ultrasonic() {
        assert!(!is_ultrasonic_rate(44_100));
        assert!(!is_ultrasonic_rate(48_000));
        assert!(is_ultrasonic_rate(96_000));
        assert!(is_ultrasonic_rate(384_000));
    }

    #[test]
    fn default_band_starts_at_15_khz() {
        assert_eq!(bat_band_lo_hz(DEFAULT_LOWEST_BAT_HZ), 15_000.0);
    }
}
