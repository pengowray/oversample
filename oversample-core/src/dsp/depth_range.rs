//! How far down a spectrogram can usefully reach for a file's sample format.
//!
//! The display and reassignment settings were tuned for 16-bit audio. Each
//! extra bit lowers the quantization noise floor by about 6 dB, so deeper
//! formats get that much more range. Samples and FFTs are f32, whose 24-bit
//! mantissa caps the gain: 32-bit integer and float files count as 24-bit.

/// dB of noise floor per bit of sample depth (20·log10(2)).
const DB_PER_BIT: f32 = 6.0206;

/// Display range for 16-bit audio, the Range slider's top before depths varied.
pub const RANGE_16_BIT_DB: f32 = 120.0;

/// Bits the spectrogram can resolve: the integer depth capped at 24, or 24 for
/// float. An unknown depth (0) counts as 16.
pub fn spectrogram_bits(bits_per_sample: u16, is_float: bool) -> u16 {
    match (is_float, bits_per_sample) {
        (true, _) => 24,
        (false, 0) => 16,
        (false, b) => b.min(24),
    }
}

/// dB of range a format has beyond 16-bit audio. 0 for 16 bits or fewer, so
/// shallower formats keep the 16-bit settings.
pub fn depth_extra_db(bits_per_sample: u16, is_float: bool) -> f32 {
    let bits = spectrogram_bits(bits_per_sample, is_float);
    DB_PER_BIT * bits.saturating_sub(16) as f32
}

/// Top of the spectrogram Range slider: 120 dB for 16-bit audio, 170 dB for
/// 24-bit, 32-bit and float (rounded up to a multiple of 10 dB).
pub fn max_display_range_db(bits_per_sample: u16, is_float: bool) -> f32 {
    ((RANGE_16_BIT_DB + depth_extra_db(bits_per_sample, is_float)) / 10.0).ceil() * 10.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_bit_and_shallower_keep_the_old_range() {
        for bits in [0, 8, 12, 16] {
            assert_eq!(depth_extra_db(bits, false), 0.0);
            assert_eq!(max_display_range_db(bits, false), 120.0);
        }
    }

    #[test]
    fn deeper_formats_reach_further_down() {
        assert!((depth_extra_db(24, false) - 48.16).abs() < 0.01);
        assert_eq!(max_display_range_db(24, false), 170.0);
        // f32 FFTs can't resolve more than 24 bits.
        assert_eq!(max_display_range_db(32, false), 170.0);
        assert_eq!(max_display_range_db(32, true), 170.0);
        assert_eq!(spectrogram_bits(64, true), 24);
    }
}
