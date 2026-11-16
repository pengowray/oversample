//! Choosing a USB Audio Class input alternate setting for the channels the user
//! picked, from the device's descriptors alone (no vendor or product rules).
//!
//! A multi-channel device often offers several input alternate settings with
//! different channel counts and sample formats (the Portacapture X8 offers 8
//! channels of 24-bit, 2 channels of 24-bit and 8 channels of 32-bit float).
//! [`choose_usb_alt`] takes the setting that carries the picked channels at the
//! most depth, and [`usb_channel_plan`] adds the default pick and the fallback
//! when no setting carries the picked channels.

/// One input alternate setting, as the USB plugin reads it from the descriptors.
#[derive(Clone, Debug, PartialEq)]
pub struct UsbAlt {
    pub interface_number: u32,
    pub alternate_setting: u32,
    /// Channels in each frame (UAC1 bNrChannels, UAC2 AS_GENERAL bNrChannels).
    pub channels: u32,
    /// Bytes per sample in a packet (UAC1 bSubframeSize, UAC2 bSubslotSize).
    pub subslot_size: u32,
    /// IEEE float samples (UAC1 wFormatTag 3, UAC2 bmFormats bit 2).
    pub is_float: bool,
    /// The highest rate the setting lists (UAC1), or the clock's rate (UAC2).
    pub sample_rate: u32,
    /// The endpoint's raw wMaxPacketSize, including the high-bandwidth bits 11-12.
    pub max_packet_size: u32,
    /// The endpoint's bInterval, or 0 when not reported.
    pub interval: u32,
}

impl UsbAlt {
    /// Most bytes one packet can hold: bits 0-10 of wMaxPacketSize, times the
    /// transactions per microframe in bits 11-12.
    pub fn packet_capacity(&self) -> u32 {
        let w = self.max_packet_size;
        (w & 0x7FF) * (1 + ((w >> 11) & 3))
    }

    /// True when a packet can hold the frames for `rate`. The bus speed is not in
    /// the descriptors, so this takes the most packets a second a setting could
    /// have (high speed, 8000 / 2^(bInterval-1)): it rejects only a setting that
    /// is too small at any speed.
    pub fn carries_rate(&self, rate: u32) -> bool {
        let shift = self.interval.clamp(1, 16) - 1;
        let packets_per_sec = (8000u64 >> shift).max(1);
        let frames = (rate as u64).div_ceil(packets_per_sec);
        let bytes = frames * self.channels as u64 * self.subslot_size.max(1) as u64;
        bytes <= self.packet_capacity() as u64
    }

    /// Bits each sample takes in a packet.
    pub fn sample_bits(&self) -> u32 {
        self.subslot_size * 8
    }

    /// Larger is deeper: 32-bit float, then 32-bit PCM, 24-bit, 16-bit.
    fn depth_rank(&self) -> u32 {
        match (self.subslot_size, self.is_float) {
            (4, true) => 4,
            (4, false) => 3,
            (3, _) => 2,
            _ => 1,
        }
    }
}

/// The input alternate setting for `picked` channels (0-based), or `None` when no
/// setting has enough channels and packet room for `rate`. A UAC2 setting is
/// checked at `rate` (its clock's rate is shared by every setting); a UAC1
/// setting at `rate` or its own highest rate, whichever is lower.
///
/// Among settings that carry them: those within `max_bit_depth` (0 = no limit)
/// first, then the deepest (32-bit float, 32-bit, 24-bit, 16-bit), then the one
/// with fewest channels, then the lowest setting number.
pub fn choose_usb_alt<'a>(
    alts: &'a [UsbAlt],
    uac_version: u32,
    picked: &[u32],
    rate: u32,
    max_bit_depth: u32,
) -> Option<&'a UsbAlt> {
    let width = picked.iter().max()? + 1;
    let rate_for = |a: &UsbAlt| {
        if uac_version == 2 || a.sample_rate == 0 {
            rate
        } else {
            rate.min(a.sample_rate)
        }
    };
    alts.iter()
        .filter(|a| a.channels >= width)
        .filter(|a| a.carries_rate(rate_for(a)))
        .min_by_key(|a| {
            let over_limit = max_bit_depth > 0 && a.sample_bits() > max_bit_depth;
            (
                over_limit,
                std::cmp::Reverse(a.depth_rank()),
                a.channels,
                a.alternate_setting,
            )
        })
}

/// The rate a device will run at for a requested rate: for UAC2, the requested
/// rate if the clock lists it, or else the highest listed rate below it, or else
/// the lowest listed rate. With no list (or UAC1) it is the requested rate. A
/// request of 0 means 384 kHz. Matches the USB plugin's own choice.
pub fn usb_target_rate(requested: u32, uac_version: u32, listed: &[u32]) -> u32 {
    let requested = if requested > 0 { requested } else { 384_000 };
    if uac_version != 2 || listed.is_empty() {
        return requested;
    }
    listed
        .iter()
        .copied()
        .filter(|&r| r <= requested)
        .max()
        .unwrap_or_else(|| listed.iter().copied().min().unwrap_or(requested))
}

/// The widest input setting's channel count.
pub fn usb_max_channels(alts: &[UsbAlt]) -> u32 {
    alts.iter().map(|a| a.channels).max().unwrap_or(0)
}

/// The channels used when the user has picked none: 1 and 2, or 1 on a
/// 1-channel device.
pub fn default_usb_pick(max_channels: u32) -> Vec<u32> {
    if max_channels >= 2 {
        vec![0, 1]
    } else {
        vec![0]
    }
}

/// What to stream from a device with more than 2 channels.
#[derive(Clone, Debug, PartialEq)]
pub struct UsbChannelPlan {
    pub interface_number: u32,
    pub alternate_setting: u32,
    /// Channels to keep (0-based), 1 or 2 of them.
    pub picked: Vec<u32>,
    /// True when the user's pick could not be streamed and `picked` is the
    /// default instead.
    pub fell_back: bool,
}

/// The setting and channels for a device: the user's `pick` (0-based, empty for
/// the default), or channels 1 and 2 when no setting carries the pick. `None`
/// when no setting carries the default either, which leaves the choice to the
/// plugin.
pub fn usb_channel_plan(
    alts: &[UsbAlt],
    uac_version: u32,
    pick: &[u32],
    rate: u32,
    max_bit_depth: u32,
) -> Option<UsbChannelPlan> {
    let default = default_usb_pick(usb_max_channels(alts));
    let wanted = if pick.is_empty() || pick.len() > 2 {
        default.clone()
    } else {
        pick.to_vec()
    };
    if let Some(alt) = choose_usb_alt(alts, uac_version, &wanted, rate, max_bit_depth) {
        return Some(UsbChannelPlan {
            interface_number: alt.interface_number,
            alternate_setting: alt.alternate_setting,
            picked: wanted,
            fell_back: false,
        });
    }
    let alt = choose_usb_alt(alts, uac_version, &default, rate, max_bit_depth)?;
    Some(UsbChannelPlan {
        interface_number: alt.interface_number,
        alternate_setting: alt.alternate_setting,
        picked: default,
        fell_back: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A high-speed setting (bInterval 1) at 192 kHz: 24 frames a microframe.
    fn alt(n: u32, channels: u32, bytes: u32, float: bool) -> UsbAlt {
        UsbAlt {
            interface_number: 1,
            alternate_setting: n,
            channels,
            subslot_size: bytes,
            is_float: float,
            sample_rate: 192_000,
            // Room for 25 frames, as devices size their packets.
            max_packet_size: 25 * channels * bytes,
            interval: 1,
        }
    }

    /// The patched Portacapture X8's input settings, in descriptor order.
    fn x8() -> Vec<UsbAlt> {
        vec![alt(1, 8, 3, false), alt(2, 2, 3, false), alt(3, 8, 4, true)]
    }

    fn pick(alts: &[UsbAlt], picked: &[u32], max_bits: u32) -> Option<u32> {
        choose_usb_alt(alts, 2, picked, 192_000, max_bits).map(|a| a.alternate_setting)
    }

    #[test]
    fn channels_5_and_6_take_the_8_channel_float_setting() {
        assert_eq!(pick(&x8(), &[4, 5], 0), Some(3));
    }

    #[test]
    fn channels_5_and_6_take_24_bit_within_a_24_bit_limit() {
        assert_eq!(pick(&x8(), &[4, 5], 24), Some(1));
    }

    #[test]
    fn a_single_channel_takes_the_deepest_setting_that_has_it() {
        assert_eq!(pick(&x8(), &[6], 0), Some(3));
        assert_eq!(pick(&x8(), &[0], 0), Some(3));
    }

    #[test]
    fn channels_1_and_2_take_float_then_the_2_channel_setting_at_24_bits() {
        assert_eq!(pick(&x8(), &[0, 1], 0), Some(3));
        // Within 24 bits, the 2-channel setting beats the 8-channel one.
        assert_eq!(pick(&x8(), &[0, 1], 24), Some(2));
    }

    #[test]
    fn a_setting_over_the_bit_limit_is_used_when_nothing_else_carries_the_channels() {
        let alts = vec![alt(3, 8, 4, true), alt(2, 2, 3, false)];
        assert_eq!(pick(&alts, &[4, 5], 16), Some(3));
    }

    #[test]
    fn float_beats_32_bit_pcm_beats_24_bit_beats_16_bit() {
        let alts = vec![
            alt(1, 2, 2, false),
            alt(2, 2, 3, false),
            alt(3, 2, 4, false),
            alt(4, 2, 4, true),
        ];
        assert_eq!(pick(&alts, &[0, 1], 0), Some(4));
        assert_eq!(pick(&alts[..3], &[0, 1], 0), Some(3));
        assert_eq!(pick(&alts[..2], &[0, 1], 0), Some(2));
    }

    #[test]
    fn channels_past_the_widest_setting_have_no_setting() {
        assert_eq!(pick(&x8(), &[8], 0), None);
        assert_eq!(pick(&[alt(2, 2, 3, false)], &[4, 5], 0), None);
    }

    #[test]
    fn a_setting_too_small_for_the_rate_is_skipped() {
        let mut small = alt(3, 8, 4, true);
        // Sized for 96 kHz: 13 frames of 8 x 4 bytes.
        small.max_packet_size = 13 * 32;
        let alts = vec![alt(1, 8, 3, false), small];
        assert_eq!(pick(&alts, &[4, 5], 0), Some(1));
        assert_eq!(
            choose_usb_alt(&alts, 2, &[4, 5], 96_000, 0)
                .unwrap()
                .alternate_setting,
            3
        );
    }

    #[test]
    fn packet_capacity_counts_high_bandwidth_transactions() {
        let mut a = alt(1, 8, 4, false);
        a.max_packet_size = (1 << 11) | 1024; // 2 transactions of 1024 bytes
        assert_eq!(a.packet_capacity(), 2048);
        // 384 kHz at 8000 packets a second: 48 frames x 32 bytes = 1536.
        assert!(a.carries_rate(384_000));
        a.max_packet_size = 1024;
        assert!(!a.carries_rate(384_000));
    }

    #[test]
    fn a_longer_interval_needs_bigger_packets() {
        let mut a = alt(1, 2, 3, false);
        a.interval = 2; // 4000 packets a second: 48 frames at 192 kHz
        a.max_packet_size = 25 * 6;
        assert!(!a.carries_rate(192_000));
        a.max_packet_size = 48 * 6;
        assert!(a.carries_rate(192_000));
    }

    #[test]
    fn plan_uses_the_pick_when_a_setting_carries_it() {
        let plan = usb_channel_plan(&x8(), 2, &[4, 5], 192_000, 0).unwrap();
        assert_eq!(
            (plan.alternate_setting, plan.picked, plan.fell_back),
            (3, vec![4, 5], false)
        );
    }

    #[test]
    fn plan_defaults_to_channels_1_and_2() {
        let plan = usb_channel_plan(&x8(), 2, &[], 192_000, 24).unwrap();
        assert_eq!(
            (plan.alternate_setting, plan.picked, plan.fell_back),
            (2, vec![0, 1], false)
        );
    }

    #[test]
    fn plan_falls_back_to_channels_1_and_2_when_the_pick_is_not_carried() {
        // A 4-channel device with a saved pick of channels 7 and 8.
        let alts = vec![alt(1, 4, 3, false), alt(2, 2, 3, false)];
        let plan = usb_channel_plan(&alts, 2, &[6, 7], 192_000, 0).unwrap();
        assert_eq!(
            (plan.alternate_setting, plan.picked, plan.fell_back),
            (2, vec![0, 1], true)
        );
    }

    #[test]
    fn plan_is_none_when_nothing_carries_the_rate() {
        let mut a = alt(1, 8, 3, false);
        a.max_packet_size = 64;
        assert_eq!(usb_channel_plan(&[a], 2, &[0, 1], 192_000, 0), None);
    }

    #[test]
    fn uac1_checks_packet_room_at_the_setting_s_own_highest_rate() {
        let mut a = alt(1, 4, 2, false);
        a.sample_rate = 48_000;
        a.max_packet_size = 49 * 8; // room for 48 kHz at 1 ms packets
                                    // The request is above the setting's rate; the setting still qualifies.
        assert_eq!(
            choose_usb_alt(&[a], 1, &[2, 3], 384_000, 0).map(|a| a.alternate_setting),
            Some(1)
        );
    }

    #[test]
    fn target_rate_matches_the_plugin() {
        let listed = [44_100, 48_000, 96_000, 192_000];
        assert_eq!(usb_target_rate(0, 2, &listed), 192_000);
        assert_eq!(usb_target_rate(100_000, 2, &listed), 96_000);
        assert_eq!(usb_target_rate(8_000, 2, &listed), 44_100);
        assert_eq!(usb_target_rate(250_000, 1, &listed), 250_000);
        assert_eq!(usb_target_rate(0, 2, &[]), 384_000);
    }
}
