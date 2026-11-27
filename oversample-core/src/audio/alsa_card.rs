//! What a Linux sound card really records at, and what it is called.
//!
//! Through ALSA's `plughw:`, `sysdefault:` and the sound servers (`default`,
//! `pipewire`, `pulse`), a capture device reports nearly any rate, depth and
//! channel count, because the converter in between makes them up: a Roland
//! UA-22 that records 48 kHz, 24-bit stereo reports 1 to 384000 Hz and up to
//! 64 channels through PipeWire. A recording made at one of those rates is
//! resampled, so it shows content above the device's Nyquist frequency that
//! was never there.
//!
//! This module reads the text the system gives about a card's real settings
//! and its name, so the caller can offer only those settings. Each parser is a
//! pure function of the text; the caller reads the files and runs `pactl`.
//!
//! - `/proc/asound/cards`: card numbers and ids ([`parse_cards`]).
//! - `/proc/asound/cardN/streamM`: a USB card's formats, per altset
//!   ([`parse_usb_stream_capture`]).
//! - `/proc/asound/cardN/pcmMc/sub0/hw_params`: what an open capture device
//!   is running at ([`parse_hw_params`]).
//! - `/run/udev/data/+sound:cardN`: udev's properties for the card, including
//!   the model name from the USB ID database ([`parse_udev_data`]).
//! - `pactl info` and `pactl list sources`: the sound server's default source
//!   and its sample format ([`parse_pactl_default_source`],
//!   [`parse_pactl_sources`]).

/// One line of `/proc/asound/cards`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub number: u32,
    /// The id used in `CARD=` PCM names, such as `UA22`.
    pub id: String,
    /// The driver, such as `USB-Audio` or `HDA-Intel`.
    pub driver: String,
    /// The short name, such as `UA-22`.
    pub name: String,
}

/// Read `/proc/asound/cards`. Each card has two lines; the first is
/// ` 1 [UA22           ]: USB-Audio - UA-22`.
pub fn parse_cards(text: &str) -> Vec<Card> {
    let mut cards = Vec::new();
    for line in text.lines() {
        let Some((number, rest)) = line.trim_start().split_once(' ') else {
            continue;
        };
        let Ok(number) = number.parse::<u32>() else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('[') else {
            continue;
        };
        let Some((id, rest)) = rest.split_once(']') else {
            continue;
        };
        let rest = rest.trim_start().strip_prefix(':').unwrap_or(rest).trim();
        let (driver, name) = rest.split_once(" - ").unwrap_or((rest, ""));
        cards.push(Card {
            number,
            id: id.trim().to_string(),
            driver: driver.trim().to_string(),
            name: name.trim().to_string(),
        });
    }
    cards
}

/// The card number a PCM name's `CARD=` value stands for: the value itself
/// when it is a number, otherwise the number of the card with that id.
pub fn card_number(cards: &[Card], card_arg: &str) -> Option<u32> {
    if let Ok(n) = card_arg.parse::<u32>() {
        return Some(n);
    }
    cards.iter().find(|c| c.id == card_arg).map(|c| c.number)
}

/// The sample rates a capture setting has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rates {
    /// Only these rates.
    List(Vec<u32>),
    /// Any rate from `min` to `max`.
    Range { min: u32, max: u32 },
}

impl Rates {
    pub fn max(&self) -> u32 {
        match self {
            Rates::List(list) => list.iter().copied().max().unwrap_or(0),
            Rates::Range { max, .. } => *max,
        }
    }

    /// The parts of `min..=max` this setting has, as (min, max) spans: one
    /// span per listed rate, or the overlap with a range.
    pub fn spans_within(&self, min: u32, max: u32) -> Vec<(u32, u32)> {
        match self {
            Rates::List(list) => list
                .iter()
                .filter(|&&r| r >= min && r <= max)
                .map(|&r| (r, r))
                .collect(),
            Rates::Range { min: a, max: b } => {
                let (lo, hi) = ((*a).max(min), (*b).min(max));
                if lo <= hi {
                    vec![(lo, hi)]
                } else {
                    Vec::new()
                }
            }
        }
    }
}

/// One setting a capture device records at without conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureFormat {
    pub rates: Rates,
    pub channels: u16,
    /// Bits of each sample that carry audio: 24 for a 24-bit interface, even
    /// when it sends them in 32-bit words.
    pub bits: u16,
    pub is_float: bool,
}

/// The audio bits and whether it is float, for an ALSA format name such as
/// `S24_3LE`, `S32_LE` or `FLOAT_LE`.
pub fn alsa_format_bits(format: &str) -> Option<(u16, bool)> {
    let f = format.trim().to_ascii_uppercase();
    if f.starts_with("FLOAT64") {
        return Some((64, true));
    }
    if f.starts_with("FLOAT") {
        return Some((32, true));
    }
    let digits: String = f
        .trim_start_matches(['S', 'U'])
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let bits: u16 = digits.parse().ok()?;
    matches!(bits, 8 | 16 | 18 | 20 | 24 | 32).then_some((bits, false))
}

/// Parse the `Rates:` value of a USB stream file: `48000`,
/// `44100, 48000, 96000`, or `8000 - 48000 (continuous)`.
fn parse_rates(value: &str) -> Option<Rates> {
    let value = value.trim();
    if let Some((lo, hi)) = value.split_once(" - ") {
        let lo = lo.trim().parse().ok()?;
        let hi = hi.split_whitespace().next()?.parse().ok()?;
        return Some(Rates::Range { min: lo, max: hi });
    }
    let list: Vec<u32> = value
        .split(',')
        .filter_map(|r| r.trim().parse().ok())
        .collect();
    (!list.is_empty()).then_some(Rates::List(list))
}

/// Read the `Capture:` section of `/proc/asound/cardN/streamM` (a USB audio
/// card's stream M, which is PCM device M). There is one setting for each
/// altset and format. `Bits:` (the audio bits, shown by newer kernels) is used
/// when present, otherwise the format's width.
pub fn parse_usb_stream_capture(text: &str) -> Vec<CaptureFormat> {
    #[derive(Default)]
    struct Altset {
        formats: Vec<String>,
        channels: Option<u16>,
        rates: Option<Rates>,
        bits: Option<u16>,
    }
    let mut in_capture = false;
    let mut altsets: Vec<Altset> = Vec::new();
    for line in text.lines() {
        if !line.starts_with(' ') && !line.trim().is_empty() {
            in_capture = line.trim() == "Capture:";
            continue;
        }
        if !in_capture {
            continue;
        }
        let t = line.trim();
        // "Altset 1" starts a setting; "Altset = 1" is the running status.
        if t.starts_with("Altset ") && !t.contains('=') {
            altsets.push(Altset::default());
            continue;
        }
        let Some(current) = altsets.last_mut() else {
            continue;
        };
        let Some((key, value)) = t.split_once(':') else {
            continue;
        };
        match key.trim() {
            "Format" | "Formats" => {
                current.formats = value.split_whitespace().map(str::to_string).collect()
            }
            "Channels" => current.channels = value.trim().parse().ok(),
            "Rates" => current.rates = parse_rates(value),
            "Bits" => current.bits = value.trim().parse().ok(),
            _ => {}
        }
    }
    let mut out = Vec::new();
    for a in altsets {
        let (Some(channels), Some(rates)) = (a.channels, a.rates) else {
            continue;
        };
        for format in &a.formats {
            let Some((width, is_float)) = alsa_format_bits(format) else {
                continue;
            };
            let bits = match a.bits {
                Some(b) if b > 0 && b <= width && !is_float => b,
                _ => width,
            };
            let f = CaptureFormat {
                rates: rates.clone(),
                channels,
                bits,
                is_float,
            };
            if !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out
}

/// Read `/proc/asound/cardN/pcmMc/sub0/hw_params`: the one setting an open
/// capture device is running at, or `None` when it is `closed`.
pub fn parse_hw_params(text: &str) -> Option<CaptureFormat> {
    let mut format = None;
    let mut channels = None;
    let mut rate = None;
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "format" => format = alsa_format_bits(value),
            "channels" => channels = value.parse().ok(),
            "rate" => rate = value.split_whitespace().next().and_then(|r| r.parse().ok()),
            _ => {}
        }
    }
    let (bits, is_float) = format?;
    Some(CaptureFormat {
        rates: Rates::List(vec![rate?]),
        channels: channels?,
        bits,
        is_float,
    })
}

/// The `KEY=VALUE` properties in a udev database file such as
/// `/run/udev/data/+sound:card1` (its `E:` lines).
pub fn parse_udev_data(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|l| l.strip_prefix("E:"))
        .filter_map(|kv| kv.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn prop<'a>(props: &'a [(String, String)], key: &str) -> Option<&'a str> {
    props
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.trim())
        .filter(|v| !v.is_empty())
}

/// A USB sound card's name and maker, as the system's sound settings show
/// them. The name is the model from udev's USB ID database (the name on the
/// box, such as "DUO-CAPTURE EX"), which is also what PipeWire and GNOME show,
/// otherwise the product string the device reports ("UA-22"). The maker is
/// the device's own manufacturer string ("Roland"), otherwise the database's
/// vendor name.
pub fn usb_card_name(
    udev: &[(String, String)],
    sysfs_product: Option<&str>,
    sysfs_manufacturer: Option<&str>,
) -> Option<(String, Option<String>)> {
    let clean = |s: Option<&str>| {
        s.map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    let name = clean(prop(udev, "ID_MODEL_FROM_DATABASE"))
        .or_else(|| clean(sysfs_product))
        .or_else(|| clean(prop(udev, "ID_MODEL")).map(|m| m.replace('_', " ")))?;
    let maker = clean(sysfs_manufacturer)
        .or_else(|| clean(prop(udev, "ID_VENDOR_FROM_DATABASE")))
        .or_else(|| clean(prop(udev, "ID_VENDOR")).map(|m| m.replace('_', " ")));
    Some((name, maker))
}

/// The bits and float flag of a PulseAudio sample format such as `s24le`,
/// `s24-32le`, `s32le`, `s16le` or `float32le`.
pub fn pulse_format_bits(format: &str) -> Option<(u16, bool)> {
    let f = format.trim().to_ascii_lowercase();
    if f.starts_with("float32") {
        return Some((32, true));
    }
    let digits: String = f
        .trim_start_matches(['s', 'u'])
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    let bits: u16 = digits.parse().ok()?;
    matches!(bits, 8 | 16 | 24 | 32).then_some((bits, false))
}

/// A sample specification such as `s24le 2ch 48000Hz`, as one setting.
pub fn parse_sample_spec(spec: &str) -> Option<CaptureFormat> {
    let mut parts = spec.split_whitespace();
    let (bits, is_float) = pulse_format_bits(parts.next()?)?;
    let channels = parts.next()?.strip_suffix("ch")?.parse().ok()?;
    let rate = parts.next()?.strip_suffix("Hz")?.parse().ok()?;
    Some(CaptureFormat {
        rates: Rates::List(vec![rate]),
        channels,
        bits,
        is_float,
    })
}

/// The `Default Source:` line of `pactl info` (run with `LC_ALL=C`).
pub fn parse_pactl_default_source(info: &str) -> Option<String> {
    info.lines()
        .find_map(|l| l.trim().strip_prefix("Default Source:"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// One source from `pactl list sources` (run with `LC_ALL=C`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PulseSource {
    pub name: String,
    pub description: String,
    /// The source's own sample format, rate and channels.
    pub spec: Option<CaptureFormat>,
    pub properties: Vec<(String, String)>,
}

impl PulseSource {
    pub fn property(&self, key: &str) -> Option<&str> {
        prop(&self.properties, key)
    }

    /// The ALSA card number the source records from, when it is a sound card.
    pub fn alsa_card(&self) -> Option<u32> {
        self.property("alsa.card")
            .or_else(|| self.property("api.alsa.pcm.card"))
            .and_then(|c| c.parse().ok())
    }

    /// The ALSA PCM device number on that card.
    pub fn alsa_device(&self) -> u32 {
        self.property("alsa.device")
            .and_then(|d| d.parse().ok())
            .unwrap_or(0)
    }

    /// A monitor source records what a playback device plays.
    pub fn is_monitor(&self) -> bool {
        self.property("device.class") == Some("monitor")
    }
}

/// Read `pactl list sources`.
pub fn parse_pactl_sources(text: &str) -> Vec<PulseSource> {
    let mut sources: Vec<PulseSource> = Vec::new();
    let mut in_properties = false;
    for line in text.lines() {
        if line.starts_with("Source #") {
            sources.push(PulseSource::default());
            in_properties = false;
            continue;
        }
        let Some(source) = sources.last_mut() else {
            continue;
        };
        let depth = line.len() - line.trim_start_matches('\t').len();
        let t = line.trim();
        if depth == 1 {
            in_properties = t == "Properties:";
            if let Some(v) = t.strip_prefix("Name:") {
                source.name = v.trim().to_string();
            } else if let Some(v) = t.strip_prefix("Description:") {
                source.description = v.trim().to_string();
            } else if let Some(v) = t.strip_prefix("Sample Specification:") {
                source.spec = parse_sample_spec(v);
            }
        } else if depth >= 2 && in_properties {
            if let Some((k, v)) = t.split_once(" = ") {
                let v = v.trim().trim_matches('"');
                source
                    .properties
                    .push((k.trim().to_string(), v.to_string()));
            }
        }
    }
    sources
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `/proc/asound/card1/stream0` on a Pop!_OS desktop with a Roland
    /// DUO-CAPTURE EX (UA-22) plugged in.
    const UA22_STREAM0: &str = "\
Roland UA-22 at usb-0000:0a:00.3-2, full speed : USB Audio

Playback:
  Status: Running
    Interface = 0
    Altset = 1
    Packet Size = 320
    Momentary freq = 48000 Hz (0x30.0000)
  Interface 0
    Altset 1
    Format: S24_3LE
    Channels: 2
    Endpoint: 0x01 (1 OUT) (ADAPTIVE)
    Rates: 48000
    Bits: 24
    Channel map: FL FR

Capture:
  Status: Stop
  Interface 1
    Altset 1
    Format: S24_3LE
    Channels: 2
    Endpoint: 0x82 (2 IN) (ASYNC)
    Rates: 48000
    Bits: 24
    Channel map: FL FR
    Sync Endpoint: 0x01 (1 OUT)
    Sync EP Interface: 0
    Sync EP Altset: 1
    Implicit Feedback Mode: Yes
";

    /// A two-channel UAC2 interface with a 24-bit and a 16-bit altset, while
    /// it is recording (the running status comes before the altsets).
    const UAC2_TWO_ALTSETS: &str = "\
Focusrite Scarlett 2i2 USB at usb-0000:00:14.0-1, high speed : USB Audio

Capture:
  Status: Running
    Interface = 2
    Altset = 1
    Packet Size = 72
    Momentary freq = 48000 Hz (0x6.0000)
    Feedback Format = 16.16
  Interface 2
    Altset 1
    Format: S32_LE
    Channels: 2
    Endpoint: 0x82 (2 IN) (ASYNC)
    Rates: 44100, 48000, 88200, 96000, 176400, 192000
    Data packet interval: 125 us
    Bits: 24
    Channel map: FL FR
  Interface 2
    Altset 2
    Format: S16_LE
    Channels: 2
    Endpoint: 0x82 (2 IN) (ASYNC)
    Rates: 44100, 48000, 88200, 96000
    Data packet interval: 125 us
    Bits: 16
    Channel map: FL FR
";

    /// An older USB 1.1 webcam microphone with a continuous range and no
    /// `Bits:` line.
    const CONTINUOUS: &str = "\
USB Camera at usb-0000:00:1d.0-1.2, high speed : USB Audio

Capture:
  Status: Stop
  Interface 3
    Altset 1
    Format: S16_LE
    Channels: 1
    Endpoint: 0x84 (4 IN) (ASYNC)
    Rates: 8000 - 48000 (continuous)
";

    /// A one-channel 384 kHz bat microphone.
    const BAT_MIC: &str = "\
Pettersson M500-384 at usb-0000:00:14.0-2, high speed : USB Audio

Capture:
  Status: Stop
  Interface 1
    Altset 1
    Format: S16_LE
    Channels: 1
    Endpoint: 0x81 (1 IN) (ASYNC)
    Rates: 384000
    Bits: 16
";

    #[test]
    fn the_ua22_records_48_khz_24_bit_stereo() {
        assert_eq!(
            parse_usb_stream_capture(UA22_STREAM0),
            vec![CaptureFormat {
                rates: Rates::List(vec![48000]),
                channels: 2,
                bits: 24,
                is_float: false,
            }]
        );
    }

    #[test]
    fn each_altset_is_a_setting() {
        let f = parse_usb_stream_capture(UAC2_TWO_ALTSETS);
        assert_eq!(f.len(), 2);
        assert_eq!(
            f[0].rates,
            Rates::List(vec![44100, 48000, 88200, 96000, 176400, 192000])
        );
        assert_eq!((f[0].channels, f[0].bits), (2, 24));
        assert_eq!(f[1].rates.max(), 96000);
        assert_eq!((f[1].channels, f[1].bits), (2, 16));
    }

    #[test]
    fn a_continuous_range_and_a_missing_bits_line() {
        assert_eq!(
            parse_usb_stream_capture(CONTINUOUS),
            vec![CaptureFormat {
                rates: Rates::Range {
                    min: 8000,
                    max: 48000
                },
                channels: 1,
                bits: 16,
                is_float: false,
            }]
        );
    }

    #[test]
    fn a_bat_mic_keeps_its_high_rate() {
        let f = parse_usb_stream_capture(BAT_MIC);
        assert_eq!(f[0].rates.max(), 384000);
        assert_eq!((f[0].channels, f[0].bits), (1, 16));
    }

    #[test]
    fn a_playback_only_stream_has_no_capture_settings() {
        let playback_only = UA22_STREAM0.split("Capture:").next().unwrap();
        assert!(parse_usb_stream_capture(playback_only).is_empty());
    }

    #[test]
    fn several_formats_on_one_line_are_each_a_setting() {
        let text = "x\n\nCapture:\n  Interface 1\n    Altset 1\n    Format: S16_LE S24_3LE\n    Channels: 2\n    Rates: 48000\n";
        let f = parse_usb_stream_capture(text);
        assert_eq!(f.iter().map(|f| f.bits).collect::<Vec<_>>(), vec![16, 24]);
    }

    #[test]
    fn rate_spans_are_the_real_rates_inside_a_reported_range() {
        let list = Rates::List(vec![44100, 48000, 96000]);
        assert_eq!(
            list.spans_within(1, 384000),
            vec![(44100, 44100), (48000, 48000), (96000, 96000)]
        );
        assert_eq!(list.spans_within(48000, 48000), vec![(48000, 48000)]);
        let range = Rates::Range {
            min: 8000,
            max: 48000,
        };
        assert_eq!(range.spans_within(1, 384000), vec![(8000, 48000)]);
        assert!(range.spans_within(96000, 192000).is_empty());
    }

    #[test]
    fn cards_file_gives_numbers_and_ids() {
        let text = " 0 [NVidia         ]: HDA-Intel - HDA NVidia
                      HDA NVidia at 0xfc080000 irq 104
 1 [UA22           ]: USB-Audio - UA-22
                      Roland UA-22 at usb-0000:0a:00.3-2, full speed
 2 [Generic        ]: HDA-Intel - HD-Audio Generic
                      HD-Audio Generic at 0xfc800000 irq 106
";
        let cards = parse_cards(text);
        assert_eq!(cards.len(), 3);
        assert_eq!(
            cards[1],
            Card {
                number: 1,
                id: "UA22".into(),
                driver: "USB-Audio".into(),
                name: "UA-22".into()
            }
        );
        assert_eq!(card_number(&cards, "UA22"), Some(1));
        assert_eq!(card_number(&cards, "2"), Some(2));
        assert_eq!(card_number(&cards, "Missing"), None);
    }

    #[test]
    fn hw_params_of_an_open_and_a_closed_device() {
        let open = "access: MMAP_INTERLEAVED\nformat: S24_3LE\nsubformat: STD\nchannels: 2\nrate: 48000 (48000/1)\nperiod_size: 256\nbuffer_size: 32768\n";
        assert_eq!(
            parse_hw_params(open),
            Some(CaptureFormat {
                rates: Rates::List(vec![48000]),
                channels: 2,
                bits: 24,
                is_float: false,
            })
        );
        assert_eq!(parse_hw_params("closed\n"), None);
    }

    #[test]
    fn a_usb_card_is_named_from_the_usb_id_database_first() {
        let udev = parse_udev_data(
            "I:7521191\nE:ID_VENDOR_FROM_DATABASE=Roland Corp.\nE:ID_MODEL_FROM_DATABASE=DUO-CAPTURE EX\nE:ID_MODEL=UA-22\nE:ID_VENDOR=Roland\n",
        );
        assert_eq!(
            usb_card_name(&udev, Some("UA-22"), Some("Roland")),
            Some(("DUO-CAPTURE EX".into(), Some("Roland".into())))
        );
        // Without the database entry: the device's own strings.
        assert_eq!(
            usb_card_name(&[], Some("UA-22"), Some("Roland")),
            Some(("UA-22".into(), Some("Roland".into())))
        );
        assert_eq!(
            usb_card_name(&udev[2..], None, None),
            Some(("UA-22".into(), Some("Roland".into())))
        );
        assert_eq!(usb_card_name(&[], None, None), None);
    }

    #[test]
    fn alsa_format_names() {
        assert_eq!(alsa_format_bits("S24_3LE"), Some((24, false)));
        assert_eq!(alsa_format_bits("S32_LE"), Some((32, false)));
        assert_eq!(alsa_format_bits("S16_LE"), Some((16, false)));
        assert_eq!(alsa_format_bits("FLOAT_LE"), Some((32, true)));
        assert_eq!(alsa_format_bits("U8"), Some((8, false)));
        assert_eq!(alsa_format_bits("MU_LAW"), None);
    }

    /// `pactl list sources` (LC_ALL=C) on the same desktop, cut to two
    /// sources and some of their properties.
    const PACTL_SOURCES: &str = "Source #64
\tState: RUNNING
\tName: alsa_output.usb-Roland_UA-22-00.analog-stereo.monitor
\tDescription: Monitor of DUO-CAPTURE EX Analog Stereo
\tDriver: PipeWire
\tSample Specification: s24le 2ch 48000Hz
\tChannel Map: front-left,front-right
\tVolume: front-left: 65536 / 100% / 0.00 dB,   front-right: 65536 / 100% / 0.00 dB
\t        balance 0.00
\tMonitor of Sink: alsa_output.usb-Roland_UA-22-00.analog-stereo
\tProperties:
\t\talsa.card = \"1\"
\t\tdevice.bus = \"usb\"
\t\tdevice.class = \"monitor\"
\t\tdevice.description = \"DUO-CAPTURE EX\"
\tFormats:
\t\tpcm

Source #65
\tState: SUSPENDED
\tName: alsa_input.usb-Roland_UA-22-00.analog-stereo
\tDescription: DUO-CAPTURE EX Analog Stereo
\tDriver: PipeWire
\tSample Specification: s24le 2ch 48000Hz
\tChannel Map: front-left,front-right
\tMonitor of Sink: n/a
\tProperties:
\t\talsa.card = \"1\"
\t\talsa.card_name = \"UA-22\"
\t\talsa.device = \"0\"
\t\talsa.resolution_bits = \"24\"
\t\tdevice.bus = \"usb\"
\t\tdevice.class = \"sound\"
\t\tdevice.description = \"DUO-CAPTURE EX\"
\t\tnode.name = \"alsa_input.usb-Roland_UA-22-00.analog-stereo\"
\tFormats:
\t\tpcm
";

    #[test]
    fn pactl_sources_give_the_default_sources_card_and_format() {
        let info = "Server Name: PulseAudio (on PipeWire 1.6.8)\nDefault Sample Specification: float32le 2ch 48000Hz\nDefault Sink: alsa_output.usb-Roland_UA-22-00.analog-stereo\nDefault Source: alsa_input.usb-Roland_UA-22-00.analog-stereo\n";
        let default = parse_pactl_default_source(info).unwrap();
        let sources = parse_pactl_sources(PACTL_SOURCES);
        assert_eq!(sources.len(), 2);
        assert!(sources[0].is_monitor());
        let s = sources.iter().find(|s| s.name == default).unwrap();
        assert!(!s.is_monitor());
        assert_eq!(s.description, "DUO-CAPTURE EX Analog Stereo");
        assert_eq!(s.alsa_card(), Some(1));
        assert_eq!(s.alsa_device(), 0);
        assert_eq!(s.property("device.bus"), Some("usb"));
        assert_eq!(
            s.spec,
            Some(CaptureFormat {
                rates: Rates::List(vec![48000]),
                channels: 2,
                bits: 24,
                is_float: false,
            })
        );
    }

    #[test]
    fn sample_specs() {
        let f = parse_sample_spec("float32le 2ch 48000Hz").unwrap();
        assert_eq!((f.bits, f.is_float, f.channels), (32, true, 2));
        assert_eq!(
            parse_sample_spec("s16le 1ch 384000Hz").unwrap().rates.max(),
            384000
        );
        assert_eq!(parse_sample_spec("s24-32le 2ch 96000Hz").unwrap().bits, 24);
        assert_eq!(parse_sample_spec("garbage"), None);
    }
}
