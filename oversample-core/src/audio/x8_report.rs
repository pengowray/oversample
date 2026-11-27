//! The input report a TASCAM Portacapture X8 sends when it runs the patched
//! firmware from the batchi-tascam-oversample-colab project: the source of
//! each input, phantom power, INPUT SELECT for each track, and so on. The
//! host asks for it with vendor request 0x58 (`bmRequestType` 0xC0, `wValue`
//! 1, `wIndex` 0). The layout is in that project's HOMEBREW.md, "Channel
//! names and an input report over USB (2026-11-27)".
//!
//! Version 1 is 28 bytes. The spec says a later version may add bytes after
//! these and raise the length in byte 3, so only the first 28 bytes of a
//! longer report are read. Reports of version 2 and later are read the same
//! way as version 1, on the assumption that a later version only adds bytes.
//! The Android plugin passes on version 1 reports only.

/// Bytes in a version 1 report.
pub const X8_REPORT_LEN: usize = 28;

/// Where one of the X8's six inputs takes its sound from (bytes 8 to 13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum X8InputSource {
    /// 0: the firmware did not know.
    NotKnown,
    /// 1: `INT MIC`, the built-in mics (inputs 1 and 2).
    IntMic,
    /// 2: `PLUG IN`, the plug-in power jack (inputs 1 and 2).
    PlugIn,
    /// 3: `EXT MIC` (inputs 3 to 6).
    ExtMic,
    /// 4: `LINE IN` (inputs 3 to 6).
    LineIn,
    /// 5: `EXT IN` (inputs 5 and 6).
    ExtIn,
    /// A value this version does not define.
    Other(u8),
}

impl X8InputSource {
    fn from_byte(b: u8) -> Self {
        match b {
            0 => Self::NotKnown,
            1 => Self::IntMic,
            2 => Self::PlugIn,
            3 => Self::ExtMic,
            4 => Self::LineIn,
            5 => Self::ExtIn,
            b => Self::Other(b),
        }
    }

    /// The name the X8 shows for this source, as in its channel names
    /// ("Input 3: EXT MIC"). `None` when not known.
    pub fn name(self) -> Option<&'static str> {
        match self {
            Self::IntMic => Some("INT MIC"),
            Self::PlugIn => Some("PLUG IN"),
            Self::ExtMic => Some("EXT MIC"),
            Self::LineIn => Some("LINE IN"),
            Self::ExtIn => Some("EXT IN"),
            Self::NotKnown | Self::Other(_) => None,
        }
    }
}

/// What one track records, MANUAL's INPUT SELECT (bytes 18 to 23).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum X8TrackInput {
    /// 1: INPUT 1-2.
    Input12,
    /// 2: INPUT 3-4.
    Input34,
    /// 3: INPUT 5-6.
    Input56,
    /// 4: EXT IN.
    ExtIn,
    /// 5: USB, audio from the host.
    Usb,
    /// 0, or a value this version does not define.
    Other(u8),
}

impl X8TrackInput {
    fn from_byte(b: u8) -> Self {
        match b {
            1 => Self::Input12,
            2 => Self::Input34,
            3 => Self::Input56,
            4 => Self::ExtIn,
            5 => Self::Usb,
            b => Self::Other(b),
        }
    }
}

/// A parsed X8 input report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct X8Report {
    /// Report version (byte 2).
    pub version: u8,
    /// The length the X8 gives in byte 3.
    pub length: u8,
    /// The patch's build number (bytes 4 and 5).
    pub build: u16,
    /// Inputs 1 and 2 are the built-in mics (byte 6, bit 0).
    pub built_in_mics: bool,
    /// PHANTOM VOLTAGE is 24 V, not 48 V (byte 6, bit 1).
    pub phantom_24v: bool,
    /// MS DECODE: `None` when off, or the pair it decodes, 1 to 3 (byte 7).
    pub ms_decode: Option<u8>,
    /// The source of INPUT 1 to 6.
    pub sources: [X8InputSource; 6],
    /// Phantom power for INPUT 3 to 6, as switched.
    pub phantom: [bool; 4],
    /// INPUT SELECT for tracks 1 to 6.
    pub input_select: [X8TrackInput; 6],
    /// Tracks turned on in MANUAL, bit 0 for track 1.
    pub tracks_on: u8,
    /// MIC TRIM for MIC L and MIC R in dB, -3 to 3, or `None` for a value
    /// out of range.
    pub mic_trim_db: [Option<i8>; 2],
}

impl X8Report {
    /// Whether track `track` (1 to 6) is turned on in MANUAL.
    pub fn track_on(&self, track: u8) -> bool {
        (1..=6).contains(&track) && self.tracks_on & (1 << (track - 1)) != 0
    }

    /// What each of the X8's 8 USB channels carries, for the Channels list:
    /// nothing for the mix (channels 1 and 2), then each input's source, with
    /// the phantom voltage when phantom power is switched on for that input
    /// ("EXT MIC, 48 V").
    pub fn channel_sources(&self) -> Vec<Option<String>> {
        let volts = if self.phantom_24v { "24 V" } else { "48 V" };
        let input = |i: usize| -> Option<String> {
            let source = self.sources[i].name();
            // Phantom power is for inputs 3 to 6.
            let phantom = i >= 2 && self.phantom[i - 2];
            match (source, phantom) {
                (Some(s), true) => Some(format!("{s}, {volts}")),
                (Some(s), false) => Some(s.to_string()),
                (None, true) => Some(volts.to_string()),
                (None, false) => None,
            }
        };
        [None, None].into_iter().chain((0..6).map(input)).collect()
    }

    /// The source of INPUT `input` (1 to 6).
    pub fn source(&self, input: u8) -> Option<X8InputSource> {
        self.sources.get((input as usize).checked_sub(1)?).copied()
    }
}

/// Why a reply is not an X8 report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum X8ReportError {
    /// Fewer bytes than a version 1 report, or a length byte below 28.
    TooShort(usize),
    /// The reply does not start with `X8`.
    NotX8,
    /// Version 0.
    BadVersion(u8),
}

/// Read an X8 input report from the vendor request's reply.
pub fn parse_x8_report(bytes: &[u8]) -> Result<X8Report, X8ReportError> {
    if bytes.len() < 2 || &bytes[..2] != b"X8" {
        return Err(X8ReportError::NotX8);
    }
    if bytes.len() < X8_REPORT_LEN {
        return Err(X8ReportError::TooShort(bytes.len()));
    }
    let version = bytes[2];
    if version == 0 {
        return Err(X8ReportError::BadVersion(version));
    }
    let length = bytes[3];
    if (length as usize) < X8_REPORT_LEN {
        return Err(X8ReportError::TooShort(length as usize));
    }
    let b = &bytes[..X8_REPORT_LEN];
    let trim = |raw: u8| (raw <= 6).then(|| raw as i8 - 3);
    Ok(X8Report {
        version,
        length,
        build: u16::from_le_bytes([b[4], b[5]]),
        built_in_mics: b[6] & 0x01 != 0,
        phantom_24v: b[6] & 0x02 != 0,
        ms_decode: (b[7] != 0).then_some(b[7]),
        sources: std::array::from_fn(|i| X8InputSource::from_byte(b[8 + i])),
        phantom: std::array::from_fn(|i| b[14 + i] != 0),
        input_select: std::array::from_fn(|i| X8TrackInput::from_byte(b[18 + i])),
        tracks_on: b[24],
        mic_trim_db: [trim(b[25]), trim(b[26])],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A version 1 report: build 280, built-in mics, 48 V, MS DECODE off,
    /// inputs INT MIC x2, EXT MIC, LINE IN, EXT IN x2, phantom on for input
    /// 3 only, tracks 1-2 on INPUT 1-2 and 3-4 on INPUT 3-4, 5 on EXT IN, 6
    /// on USB, tracks 1 to 4 on, trims 0 dB and +2 dB.
    fn sample() -> Vec<u8> {
        vec![
            0x58, 0x38, 1, 28, // magic, version, length
            0x18, 0x01, // build 280
            0x01, 0, // built-in mics, MS DECODE off
            1, 1, 3, 4, 5, 5, // sources
            1, 0, 0, 0, // phantom for 3 to 6
            1, 1, 2, 2, 4, 5,    // INPUT SELECT
            0x0F, // tracks on
            3, 5, // MIC TRIM
            0,
        ]
    }

    #[test]
    fn reads_every_field_of_a_version_1_report() {
        let r = parse_x8_report(&sample()).unwrap();
        assert_eq!((r.version, r.length, r.build), (1, 28, 280));
        assert!(r.built_in_mics && !r.phantom_24v);
        assert_eq!(r.ms_decode, None);
        {
            use X8InputSource::*;
            assert_eq!(r.sources, [IntMic, IntMic, ExtMic, LineIn, ExtIn, ExtIn]);
        }
        assert_eq!(r.source(3).and_then(X8InputSource::name), Some("EXT MIC"));
        assert_eq!(r.source(0), None);
        assert_eq!(r.phantom, [true, false, false, false]);
        {
            use X8TrackInput::*;
            assert_eq!(
                r.input_select,
                [Input12, Input12, Input34, Input34, ExtIn, Usb]
            );
        }
        assert!(r.track_on(1) && r.track_on(4) && !r.track_on(5));
        assert_eq!(r.mic_trim_db, [Some(0), Some(2)]);
    }

    #[test]
    fn usb_channels_get_their_input_s_source_and_phantom_voltage() {
        let mut b = sample();
        b[9] = 0; // INPUT 2: source not known
        b[17] = 1; // phantom on for INPUT 6
        let s = parse_x8_report(&b).unwrap().channel_sources();
        let want = [
            None,
            None,
            Some("INT MIC"),
            None,
            Some("EXT MIC, 48 V"),
            Some("LINE IN"),
            Some("EXT IN"),
            Some("EXT IN, 48 V"),
        ];
        assert_eq!(s, want.map(|w| w.map(String::from)));
        b[6] |= 0x02;
        let s = parse_x8_report(&b).unwrap().channel_sources();
        assert_eq!(s[4].as_deref(), Some("EXT MIC, 24 V"));
    }

    #[test]
    fn reads_flags_and_a_high_build_byte() {
        let mut b = sample();
        b[4] = 0xFF;
        b[5] = 0x80;
        b[6] = 0x02;
        b[7] = 2;
        let r = parse_x8_report(&b).unwrap();
        assert_eq!(r.build, 0x80FF);
        assert!(!r.built_in_mics && r.phantom_24v);
        assert_eq!(r.ms_decode, Some(2));
        assert_eq!(r.source(1), Some(X8InputSource::IntMic));
    }

    #[test]
    fn unknown_values_are_kept_not_refused() {
        let mut b = sample();
        b[8] = 0;
        b[9] = 9;
        b[18] = 0;
        b[19] = 7;
        b[25] = 7;
        let r = parse_x8_report(&b).unwrap();
        assert_eq!(r.sources[0], X8InputSource::NotKnown);
        assert_eq!(r.sources[1], X8InputSource::Other(9));
        assert_eq!(r.sources[1].name(), None);
        assert_eq!(r.input_select[0], X8TrackInput::Other(0));
        assert_eq!(r.input_select[1], X8TrackInput::Other(7));
        assert_eq!(r.mic_trim_db, [None, Some(2)]);
    }

    #[test]
    fn a_longer_report_of_a_later_version_is_read() {
        let mut b = sample();
        b[2] = 2;
        b[3] = 32;
        b.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]);
        let r = parse_x8_report(&b).unwrap();
        assert_eq!((r.version, r.length), (2, 32));
        assert_eq!(r.mic_trim_db, [Some(0), Some(2)]);
        // A 64-byte read of a 28-byte report with padding after it.
        let mut padded = sample();
        padded.resize(64, 0);
        assert_eq!(parse_x8_report(&padded).unwrap().length, 28);
    }

    #[test]
    fn a_short_or_wrong_reply_is_refused() {
        let b = sample();
        assert_eq!(parse_x8_report(&b[..27]), Err(X8ReportError::TooShort(27)));
        assert_eq!(parse_x8_report(&[]), Err(X8ReportError::NotX8));
        let mut junk = b.clone();
        junk[0] = 0;
        assert_eq!(parse_x8_report(&junk), Err(X8ReportError::NotX8));
        let mut v0 = b.clone();
        v0[2] = 0;
        assert_eq!(parse_x8_report(&v0), Err(X8ReportError::BadVersion(0)));
        let mut short_len = b.clone();
        short_len[3] = 20;
        assert_eq!(
            parse_x8_report(&short_len),
            Err(X8ReportError::TooShort(20))
        );
    }
}
