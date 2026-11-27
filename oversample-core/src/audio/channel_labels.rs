//! Names for the input channels of devices that send more than 2. The
//! channel picker shows these beside the channel numbers. Display only: which
//! channels are recorded is still decided by number, and nothing here affects
//! the USB setting chosen.
//!
//! The names come from the device itself when it reports them (USB audio's
//! `iChannelNames` strings), or else from the built-in table below, which
//! names the channels as the device's own screens and manual do.
//!
//! Add a device to the table only when the order of its channels is known
//! from its descriptors, firmware or manual.

use std::borrow::Cow;

/// The names of one device's input channels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelLabels {
    /// One name for each channel, in the order the device sends them.
    channels: Vec<Cow<'static, str>>,
}

impl ChannelLabels {
    /// Names the device reported, one for each of its `channels` channels.
    /// `None` when the count differs or a name is blank.
    pub fn reported(names: &[String], channels: u32) -> Option<Self> {
        let names: Vec<Cow<'static, str>> = names
            .iter()
            .map(|n| Cow::Owned(n.trim().to_string()))
            .collect();
        (names.len() == channels as usize
            && !names.is_empty()
            && names.iter().all(|n| !n.is_empty()))
        .then_some(Self { channels: names })
    }

    fn built_in(names: &'static [&'static str]) -> Self {
        Self {
            channels: names.iter().map(|&n| Cow::Borrowed(n)).collect(),
        }
    }

    /// The name of one channel (0-based).
    pub fn channel(&self, channel: u32) -> Option<&str> {
        self.channels.get(channel as usize).map(|n| n.as_ref())
    }

    /// The name of the stereo pair that starts at `left` (0-based), when
    /// `left` and `right` are one of the device's pairs (1 and 2, 3 and 4,
    /// and so on).
    pub fn pair(&self, left: u32, right: u32) -> Option<String> {
        if left % 2 != 0 || right != left + 1 {
            return None;
        }
        Some(pair_name(self.channel(left)?, self.channel(right)?))
    }

    /// The name for a pick of one channel or of one pair (0-based).
    pub fn pick(&self, picked: &[u32]) -> Option<Cow<'_, str>> {
        match picked {
            [a] => self.channel(*a).map(Cow::Borrowed),
            [a, b] => self.pair(*a, *b).map(Cow::Owned),
            _ => None,
        }
    }

    /// The name for a pick with its source apart, for a list that shows the
    /// source in lighter text: "Input 3: EXT MIC" gives the name "Input 3"
    /// and the source "EXT MIC".
    ///
    /// `sources`, when given, has one entry for each channel and is used in
    /// place of the text after ": " (a patched X8's input report says more
    /// than its channel names do).
    pub fn pick_parts(
        &self,
        picked: &[u32],
        sources: Option<&[Option<String>]>,
    ) -> Option<PickParts> {
        let part = |c: u32| -> Option<(&str, Option<String>)> {
            let full = self.channel(c)?;
            let (name, after) = match full.split_once(": ") {
                Some((name, after)) => (name, Some(after.to_string())),
                None => (full, None),
            };
            let source = match sources {
                Some(s) => s.get(c as usize).cloned().flatten(),
                None => after,
            };
            Some((name, source))
        };
        match picked {
            [a] => {
                let (name, source) = part(*a)?;
                Some(PickParts {
                    name: name.to_string(),
                    source,
                })
            }
            [a, b] if a % 2 == 0 && *b == a + 1 => {
                let ((left, left_source), (right, right_source)) = (part(*a)?, part(*b)?);
                let source = match (left_source, right_source) {
                    (None, None) => None,
                    (l, r) if l == r => l,
                    // Each input's source after the last word of its name:
                    // "5: LINE IN; 6: EXT MIC".
                    (l, r) => Some(
                        [(left, l), (right, r)]
                            .into_iter()
                            .filter_map(|(name, source)| {
                                let short = name.rsplit(' ').next().unwrap_or(name);
                                Some(format!("{short}: {}", source?))
                            })
                            .collect::<Vec<_>>()
                            .join("; "),
                    ),
                };
                Some(PickParts {
                    name: pair_name(left, right),
                    source,
                })
            }
            _ => None,
        }
    }
}

/// A pick's name, and where its sound comes from when the device says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickParts {
    pub name: String,
    pub source: Option<String>,
}

/// One name for two channels. When the two names differ only in their last
/// word, the words before it are written once: "Mix L" and "Mix R" give
/// "Mix L and R". When both names have the same text after ": ", that text
/// is also written once: "Input 3: EXT MIC" and "Input 4: EXT MIC" give
/// "Input 3 and 4: EXT MIC". Otherwise both names are written in full:
/// "Input 5: LINE IN and Input 6: EXT MIC".
pub fn pair_name(left: &str, right: &str) -> String {
    let both = || format!("{left} and {right}");
    let (left_head, right_head, after) = match (left.split_once(": "), right.split_once(": ")) {
        (Some((lh, la)), Some((rh, ra))) if la == ra => (lh, rh, Some(la)),
        (None, None) => (left, right, None),
        _ => return both(),
    };
    match (left_head.rsplit_once(' '), right_head.rsplit_once(' ')) {
        (Some((lp, lw)), Some((rp, rw)))
            if lp == rp && !lp.is_empty() && !lw.is_empty() && !rw.is_empty() && lw != rw =>
        {
            match after {
                Some(after) => format!("{lp} {lw} and {rw}: {after}"),
                None => format!("{lp} {lw} and {rw}"),
            }
        }
        _ => both(),
    }
}

/// TASCAM Portacapture X8 (USB 0644:8074), firmware V2.03: the stereo mix,
/// then INPUT 1 to 6 taken before the faders. From the audio descriptors and
/// the audio routine, as recorded in the X8 firmware project's HOMEBREW.md
/// ("What the X8 presents over USB"). The 2-channel settings send the mix,
/// MANUAL's INPUT SELECT does not change what channels 3 to 8 carry. A patched
/// X8 reports its own names, with each input's source.
const PORTACAPTURE_X8: &[&str] = &[
    "Mix L", "Mix R", "Input 1", "Input 2", "Input 3", "Input 4", "Input 5", "Input 6",
];

/// TASCAM Portacapture X6: the stereo mix, then inputs 1 to 4, at 96 kHz at
/// most. From the Reference Manual V2.00, "Portacapture X6 USB audio channel
/// assignments". Channels 3 and 4 carry the MIC/EXT IN jack in place of inputs
/// 1 and 2 while INPUT SELECT has it chosen for tracks 1 and 2.
const PORTACAPTURE_X6: &[&str] = &["Mix L", "Mix R", "Input 1", "Input 2", "Input 3", "Input 4"];

/// Each known device: its names in lower case, and its channels. A name with
/// a space matches anywhere in the device's name; a name without one must be
/// the whole name.
///
/// The X8's USB product string is "Portacapture X8" (read from its V2.03
/// firmware). "X8" is accepted too, for a host that shortens the name.
///
/// Not here on purpose: the Sound Devices MixPre recorders. Since firmware
/// v5.00 the user chooses what each USB channel carries, and before it the
/// mix was on the last two channels, so no fixed names are right.
const DEVICES: &[(&[&str], &[&str])] = &[
    (&["portacapture x8", "x8"], PORTACAPTURE_X8),
    (&["portacapture x6"], PORTACAPTURE_X6),
];

/// The built-in channel names for a device, found by its name (the USB
/// product name, or the name the operating system gives it) and its channel
/// count.
pub fn channel_labels(device_name: &str, channels: u32) -> Option<ChannelLabels> {
    let name = device_name.trim().to_lowercase();
    DEVICES
        .iter()
        .find(|(names, labels)| {
            labels.len() == channels as usize
                && names.iter().any(|n| match n.contains(' ') {
                    true => name.contains(n),
                    false => name == *n,
                })
        })
        .map(|(_, labels)| ChannelLabels::built_in(labels))
}

/// The channel names to show for a device: the names it reported, when there
/// is one for each channel, then the built-in table, then none.
pub fn device_channel_labels(
    reported: &[String],
    device_name: &str,
    channels: u32,
) -> Option<ChannelLabels> {
    ChannelLabels::reported(reported, channels).or_else(|| channel_labels(device_name, channels))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A patched X8's strings 16 to 23, with INPUT 5 on LINE IN and INPUT 6
    /// on EXT MIC.
    fn x8_reported() -> Vec<String> {
        [
            "Mix L",
            "Mix R",
            "Input 1: INT MIC",
            "Input 2: INT MIC",
            "Input 3: EXT MIC",
            "Input 4: EXT MIC",
            "Input 5: LINE IN",
            "Input 6: EXT MIC",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn x8_inputs_are_two_channels_after_the_mix() {
        let x8 = channel_labels("TASCAM Portacapture X8", 8).unwrap();
        assert_eq!(x8.pick(&[0, 1]).as_deref(), Some("Mix L and R"));
        assert_eq!(x8.pick(&[4, 5]).as_deref(), Some("Input 3 and 4"));
        assert_eq!(x8.pick(&[0]).as_deref(), Some("Mix L"));
        assert_eq!(x8.pick(&[4]).as_deref(), Some("Input 3"));
        assert_eq!(x8.pick(&[7]).as_deref(), Some("Input 6"));
        // Not one of the device's pairs.
        assert_eq!(x8.pick(&[1, 2]), None);
        assert_eq!(x8.pick(&[8]), None);
    }

    #[test]
    fn the_x6_has_four_inputs_after_the_mix() {
        let x6 = channel_labels("Portacapture X6", 6).unwrap();
        assert_eq!(x6.pick(&[0, 1]).as_deref(), Some("Mix L and R"));
        assert_eq!(x6.pick(&[4, 5]).as_deref(), Some("Input 3 and 4"));
        assert!(channel_labels("Portacapture X6", 8).is_none());
    }

    #[test]
    fn found_by_name_and_channel_count() {
        assert!(channel_labels("Portacapture X8, USB Audio", 8).is_some());
        assert!(channel_labels("X8", 8).is_some());
        assert!(channel_labels("Mixer X8 Pro", 8).is_none());
        // The same device opened through a 2-channel setting only.
        assert!(channel_labels("Portacapture X8", 2).is_none());
        assert!(channel_labels("Some 8 channel interface", 8).is_none());
    }

    #[test]
    fn a_patched_x8_names_its_inputs_and_their_sources() {
        let x8 = device_channel_labels(&x8_reported(), "Portacapture X8", 8).unwrap();
        assert_eq!(x8.pick(&[0, 1]).as_deref(), Some("Mix L and R"));
        assert_eq!(x8.pick(&[2, 3]).as_deref(), Some("Input 1 and 2: INT MIC"));
        assert_eq!(x8.pick(&[4, 5]).as_deref(), Some("Input 3 and 4: EXT MIC"));
        // The two inputs of a pair take different sources.
        assert_eq!(
            x8.pick(&[6, 7]).as_deref(),
            Some("Input 5: LINE IN and Input 6: EXT MIC")
        );
        assert_eq!(x8.pick(&[6]).as_deref(), Some("Input 5: LINE IN"));
    }

    #[test]
    fn a_pick_s_name_and_source_come_apart() {
        let x8 = device_channel_labels(&x8_reported(), "Portacapture X8", 8).unwrap();
        let parts = |picked: &[u32]| {
            let p = x8.pick_parts(picked, None).unwrap();
            (p.name, p.source)
        };
        assert_eq!(parts(&[0, 1]), ("Mix L and R".into(), None));
        assert_eq!(
            parts(&[4, 5]),
            ("Input 3 and 4".into(), Some("EXT MIC".into()))
        );
        assert_eq!(parts(&[6]), ("Input 5".into(), Some("LINE IN".into())));
        // The two inputs of a pair take different sources.
        assert_eq!(
            parts(&[6, 7]),
            (
                "Input 5 and 6".into(),
                Some("5: LINE IN; 6: EXT MIC".into())
            )
        );
        assert_eq!(x8.pick_parts(&[1, 2], None), None);

        // Sources from an input report are used in place of the names' own.
        let mut sources: Vec<Option<String>> = vec![None; 8];
        sources[4] = Some("EXT MIC, 48 V".into());
        sources[5] = Some("EXT MIC".into());
        let p = x8.pick_parts(&[4, 5], Some(&sources)).unwrap();
        assert_eq!(p.source.as_deref(), Some("3: EXT MIC, 48 V; 4: EXT MIC"));
        assert_eq!(x8.pick_parts(&[2, 3], Some(&sources)).unwrap().source, None);

        // Built-in names have no sources.
        let x6 = channel_labels("Portacapture X6", 6).unwrap();
        let p = x6.pick_parts(&[2, 3], None).unwrap();
        assert_eq!((p.name.as_str(), p.source), ("Input 1 and 2", None));
    }

    #[test]
    fn reported_names_are_used_on_any_device() {
        let names: Vec<String> = ["Front Left", "Front Right", "Rear Left", "Rear Right"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let l = device_channel_labels(&names, "Some interface", 4).unwrap();
        assert_eq!(l.pick(&[0, 1]).as_deref(), Some("Front Left and Right"));
        assert_eq!(l.pick(&[3]).as_deref(), Some("Rear Right"));
    }

    #[test]
    fn built_in_names_are_used_when_reported_names_are_incomplete() {
        let mut names = x8_reported();
        names.pop();
        // 7 names for 8 channels: the built-in table.
        let l = device_channel_labels(&names, "Portacapture X8", 8).unwrap();
        assert_eq!(l.channel(2), Some("Input 1"));
        // A blank name.
        let mut names = x8_reported();
        names[3] = "  ".into();
        let l = device_channel_labels(&names, "Portacapture X8", 8).unwrap();
        assert_eq!(l.channel(2), Some("Input 1"));
        // No names and no table entry.
        assert!(device_channel_labels(&[], "Some interface", 8).is_none());
        assert!(device_channel_labels(&names, "Some interface", 8).is_none());
    }

    #[test]
    fn pair_names_write_matching_words_once() {
        assert_eq!(pair_name("Mix L", "Mix R"), "Mix L and R");
        assert_eq!(
            pair_name("Input 3: EXT MIC", "Input 4: EXT MIC"),
            "Input 3 and 4: EXT MIC"
        );
        // Different parts after ": ".
        assert_eq!(
            pair_name("Input 1: INT MIC", "Input 2: PLUG IN"),
            "Input 1: INT MIC and Input 2: PLUG IN"
        );
        // ": " in only one of them.
        assert_eq!(
            pair_name("Input 3", "Input 4: EXT MIC"),
            "Input 3 and Input 4: EXT MIC"
        );
        // Different first words, or one word each.
        assert_eq!(
            pair_name("Left: Mic", "Right: Mic"),
            "Left: Mic and Right: Mic"
        );
        assert_eq!(pair_name("Left", "Right"), "Left and Right");
        // The same last word.
        assert_eq!(pair_name("Line In", "Line In"), "Line In and Line In");
        assert_eq!(pair_name("Guitar", "Guitar"), "Guitar and Guitar");
    }
}
