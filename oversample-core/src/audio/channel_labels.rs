//! Names for the input channels of devices that send more than 2, as the
//! device's own screens and manual name them. The channel picker shows these
//! beside the channel numbers. Display only: which channels are recorded is
//! still decided by number, and nothing here affects the USB setting chosen.
//!
//! Add a device only when the order of its channels is known from its
//! descriptors, firmware or manual.

/// The names of one device's input channels.
#[derive(Debug, PartialEq, Eq)]
pub struct ChannelLabels {
    /// One name for each channel, in the order the device sends them.
    pub channels: &'static [&'static str],
    /// One name for each stereo pair: channels 1 and 2, then 3 and 4, and so on.
    pub pairs: &'static [&'static str],
}

impl ChannelLabels {
    /// The name of one channel (0-based).
    pub fn channel(&self, channel: u32) -> Option<&'static str> {
        self.channels.get(channel as usize).copied()
    }

    /// The name of the stereo pair that starts at `left` (0-based), when
    /// `left` and `right` are one of the device's pairs.
    pub fn pair(&self, left: u32, right: u32) -> Option<&'static str> {
        (left % 2 == 0 && right == left + 1)
            .then(|| self.pairs.get((left / 2) as usize).copied())
            .flatten()
    }

    /// The name for a pick of one channel or of one pair (0-based).
    pub fn pick(&self, picked: &[u32]) -> Option<&'static str> {
        match picked {
            [a] => self.channel(*a),
            [a, b] => self.pair(*a, *b),
            _ => None,
        }
    }
}

/// TASCAM Portacapture X8 (USB 0644:8074), firmware V2.03: the stereo mix,
/// then INPUT 1 to 6 taken before the faders. From the audio descriptors and
/// the audio routine, as recorded in the X8 firmware project's HOMEBREW.md
/// ("What the X8 presents over USB"). The 2-channel settings send the mix.
const PORTACAPTURE_X8: ChannelLabels = ChannelLabels {
    channels: &[
        "Mix L", "Mix R", "Input 1", "Input 2", "Input 3", "Input 4", "Input 5", "Input 6",
    ],
    pairs: &[
        "Mix L and R",
        "Input 1 and 2",
        "Input 3 and 4",
        "Input 5 and 6",
    ],
};

/// Each known device: a part of its name in lower case, and its channels.
const DEVICES: &[(&str, &ChannelLabels)] = &[("portacapture x8", &PORTACAPTURE_X8)];

/// The channel names for a device, found by its name (the USB product name,
/// or the name the operating system gives it) and its channel count.
pub fn channel_labels(device_name: &str, channels: u32) -> Option<&'static ChannelLabels> {
    let name = device_name.to_lowercase();
    DEVICES
        .iter()
        .find(|(part, labels)| name.contains(part) && labels.channels.len() == channels as usize)
        .map(|(_, labels)| *labels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x8_inputs_are_two_channels_after_the_mix() {
        let x8 = channel_labels("TASCAM Portacapture X8", 8).unwrap();
        assert_eq!(x8.pick(&[0, 1]), Some("Mix L and R"));
        assert_eq!(x8.pick(&[4, 5]), Some("Input 3 and 4"));
        assert_eq!(x8.pick(&[0]), Some("Mix L"));
        assert_eq!(x8.pick(&[4]), Some("Input 3"));
        assert_eq!(x8.pick(&[7]), Some("Input 6"));
        // Not one of the device's pairs.
        assert_eq!(x8.pick(&[1, 2]), None);
        assert_eq!(x8.pick(&[8]), None);
    }

    #[test]
    fn every_device_has_a_pair_name_for_each_two_channels() {
        for (part, labels) in DEVICES {
            assert_eq!(labels.pairs.len(), labels.channels.len() / 2, "{part}");
            assert_eq!(*part, part.to_lowercase());
        }
    }

    #[test]
    fn found_by_name_and_channel_count() {
        assert!(channel_labels("Portacapture X8, USB Audio", 8).is_some());
        // The same device opened through a 2-channel setting only.
        assert!(channel_labels("Portacapture X8", 2).is_none());
        assert!(channel_labels("Some 8 channel interface", 8).is_none());
    }
}
