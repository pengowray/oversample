//! Which ALSA PCMs the microphone list shows, and in what order.
//!
//! cpal's ALSA host lists every PCM that ALSA knows about: plugins such as
//! `null`, the rate converters and the up/down mixers, the sound servers
//! (`pipewire`, `pulse`, `jack`), and up to a dozen names for each sound card
//! (`hw:`, `plughw:`, `sysdefault:`, `front:`, `dsnoop:`, `surround51:` and
//! more, plus `hw:CARD=1,DEV=0` copies of `hw:CARD=UA22,DEV=0`). Most of these
//! names share one description, so the list repeated "UA-22, USB Audio" and
//! opening one by name could pick a PCM that does not record.
//!
//! [`arrange`] turns that into: the `default` PCM, one entry for each card's
//! capture device, then the sound servers that are running, then any other PCM
//! that reports input settings (a user's own `.asoundrc` device, say). Plugins
//! and the other names for a card are left out. A PCM's kind comes from its
//! name and whether it reported any input settings, never from the English
//! description; descriptions are only compared to spot a numbered copy of a
//! card and to tell apart two rows that would show the same text.

/// What kind of PCM an ALSA PCM name is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PcmKind {
    /// `default`: whatever the system's ALSA configuration points at.
    Default,
    /// A sound server, ranked: `pipewire` 0, `pulse` 1, `jack` 2.
    SoundServer(u8),
    /// A name for a sound card's device.
    Card {
        alias: CardAlias,
        /// The `CARD=` value: a card id such as `UA22`, or a number.
        card: String,
        /// The `DEV=` value (0 when the name has none, as with `sysdefault:`).
        dev: u32,
    },
    /// An ALSA plugin that is not a place sound comes from: `null`, the rate
    /// converters, the mixers, `speex`, `oss`.
    Plugin,
    /// Any other name, such as a PCM defined in `~/.asoundrc`.
    Other,
}

/// The prefix of a sound card PCM name, best for recording first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CardAlias {
    /// `hw:`: the device itself, at the rates and formats it has.
    Hw,
    /// `plughw:`: the device with ALSA's format and rate conversion.
    PlugHw,
    /// `sysdefault:`: the card's default device, with conversion.
    SysDefault,
    /// `front:`
    Front,
    /// `dsnoop:`: capture shared between programs.
    Dsnoop,
    /// `surround51:`, `iec958:`, `hdmi:`, `dmix:`, `usbstream:` and the rest:
    /// outputs or special streams, never used for the microphone list.
    Unused,
}

/// Bare PCM names that are ALSA plugins rather than sound sources.
const PLUGINS: &[&str] = &[
    "null",
    "samplerate",
    "speexrate",
    "lavrate",
    "lavcrate",
    "lavrate_higher",
    "lavrate_fast",
    "lavrate_faster",
    "samplerate_best",
    "samplerate_medium",
    "samplerate_order",
    "samplerate_linear",
    "speexrate_best",
    "speexrate_medium",
    "upmix",
    "vdownmix",
    "speex",
    "oss",
    "a52",
    "dmix",
    "dsnoop",
];

/// Sort a PCM name into a [`PcmKind`].
pub fn classify(pcm_id: &str) -> PcmKind {
    match pcm_id {
        "default" => return PcmKind::Default,
        "pipewire" => return PcmKind::SoundServer(0),
        "pulse" => return PcmKind::SoundServer(1),
        "jack" => return PcmKind::SoundServer(2),
        _ => {}
    }
    if PLUGINS.contains(&pcm_id) {
        return PcmKind::Plugin;
    }
    let Some((prefix, args)) = pcm_id.split_once(':') else {
        return PcmKind::Other;
    };
    let alias = match prefix {
        "hw" => CardAlias::Hw,
        "plughw" => CardAlias::PlugHw,
        "sysdefault" => CardAlias::SysDefault,
        "front" => CardAlias::Front,
        "dsnoop" => CardAlias::Dsnoop,
        "dmix" | "usbstream" | "iec958" | "spdif" | "hdmi" | "rear" | "center_lfe" | "side"
        | "surround21" | "surround40" | "surround41" | "surround50" | "surround51"
        | "surround71" => CardAlias::Unused,
        _ => return PcmKind::Other,
    };
    match parse_card_args(args) {
        Some((card, dev)) => PcmKind::Card { alias, card, dev },
        None => PcmKind::Other,
    }
}

/// Read `CARD=x,DEV=n` (or the short `x,n` form) from a PCM name's arguments.
fn parse_card_args(args: &str) -> Option<(String, u32)> {
    let mut card = None;
    let mut dev = 0u32;
    for (i, part) in args.split(',').enumerate() {
        let part = part.trim();
        let (key, value) = match part.split_once('=') {
            Some((k, v)) => (Some(k.trim()), v.trim()),
            None => (None, part),
        };
        match (key, i) {
            (Some("CARD"), _) | (None, 0) => card = Some(value.to_string()),
            (Some("DEV"), _) | (None, 1) => dev = value.parse().ok()?,
            _ => {}
        }
    }
    card.filter(|c| !c.is_empty()).map(|c| (c, dev))
}

/// Whether to open a PCM to ask for its input settings. Plugins and the
/// unused card names are never opened: opening some ALSA plugins fails in a
/// way that leaves file handles open (see cpal's ALSA host), and the results
/// would not be shown.
pub fn worth_probing(kind: &PcmKind) -> bool {
    match kind {
        PcmKind::Plugin => false,
        PcmKind::Card { alias, .. } => *alias != CardAlias::Unused,
        _ => true,
    }
}

/// One PCM from cpal's list, as [`arrange`] needs it.
#[derive(Clone, Debug)]
pub struct PcmEntry<'a> {
    /// The ALSA PCM name, such as `hw:CARD=UA22,DEV=0`.
    pub pcm_id: &'a str,
    /// The first line of ALSA's description, such as "UA-22, USB Audio".
    pub name: &'a str,
    /// Whether opening it for capture reported at least one input setting.
    pub has_input: bool,
}

/// One row of the arranged list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedPcm {
    /// Index into the entries passed to [`arrange`].
    pub index: usize,
    /// The name to show and to find the PCM by again. The description, with
    /// the PCM name added in brackets when two rows would share a description.
    pub name: String,
    pub is_default: bool,
}

/// The microphone list for ALSA: `default` first, then one entry for each
/// card device (in the order cpal listed them), then the running sound
/// servers (PipeWire, PulseAudio, JACK), then other PCMs that report input
/// settings.
///
/// For each card device the entry is `hw:` when it reports input settings
/// (the device's own rates), otherwise the first of `plughw:`, `sysdefault:`,
/// `front:`, `dsnoop:` that does, otherwise `plughw:` (or whichever name
/// exists) so the card is still listed. A numbered card (`hw:CARD=1,DEV=0`) is
/// left out when a named card has the same description, since cpal lists both
/// for each card.
pub fn arrange(entries: &[PcmEntry]) -> Vec<ListedPcm> {
    let kinds: Vec<PcmKind> = entries.iter().map(|e| classify(e.pcm_id)).collect();

    let mut default = None;
    let mut servers: Vec<(u8, usize)> = Vec::new();
    let mut others = Vec::new();
    // (card, dev) in first-seen order, with each alias's entry index.
    let mut cards: Vec<((String, u32), Vec<(CardAlias, usize)>)> = Vec::new();

    for (i, kind) in kinds.iter().enumerate() {
        match kind {
            PcmKind::Default => {
                default.get_or_insert(i);
            }
            PcmKind::SoundServer(rank) if entries[i].has_input => servers.push((*rank, i)),
            PcmKind::SoundServer(_) | PcmKind::Plugin => {}
            PcmKind::Other if entries[i].has_input => others.push(i),
            PcmKind::Other => {}
            PcmKind::Card { alias, card, dev } => {
                if *alias == CardAlias::Unused {
                    continue;
                }
                let key = (card.clone(), *dev);
                match cards.iter_mut().find(|(k, _)| *k == key) {
                    Some((_, aliases)) => aliases.push((*alias, i)),
                    None => cards.push((key, vec![(*alias, i)])),
                }
            }
        }
    }
    servers.sort_by_key(|&(rank, _)| rank);

    let mut hardware: Vec<(bool, usize)> = Vec::new(); // (numbered card, index)
    for ((card, _), mut aliases) in cards {
        aliases.sort_by_key(|&(alias, _)| alias);
        let chosen = aliases
            .iter()
            .find(|&&(_, i)| entries[i].has_input)
            .or_else(|| aliases.iter().find(|&&(a, _)| a == CardAlias::PlugHw))
            .or_else(|| aliases.first())
            .map(|&(_, i)| i);
        if let Some(i) = chosen {
            let numbered = card.chars().all(|c| c.is_ascii_digit());
            hardware.push((numbered, i));
        }
    }
    let named_cards: Vec<&str> = hardware
        .iter()
        .filter(|(numbered, _)| !numbered)
        .map(|&(_, i)| entries[i].name)
        .collect();
    hardware.retain(|&(numbered, i)| !numbered || !named_cards.contains(&entries[i].name));

    let order = default
        .into_iter()
        .chain(hardware.into_iter().map(|(_, i)| i))
        .chain(servers.into_iter().map(|(_, i)| i))
        .chain(others);

    let mut listed: Vec<ListedPcm> = Vec::new();
    for i in order {
        let e = &entries[i];
        let base = if e.name.is_empty() { e.pcm_id } else { e.name };
        let name = if listed.iter().any(|l| l.name == base) {
            format!("{base} ({})", e.pcm_id)
        } else {
            base.to_string()
        };
        listed.push(ListedPcm {
            index: i,
            name,
            is_default: kinds[i] == PcmKind::Default,
        });
    }
    listed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e<'a>(pcm_id: &'a str, name: &'a str, has_input: bool) -> PcmEntry<'a> {
        PcmEntry {
            pcm_id,
            name,
            has_input,
        }
    }

    fn pcm_ids<'a>(entries: &'a [PcmEntry<'a>], listed: &[ListedPcm]) -> Vec<&'a str> {
        listed.iter().map(|l| entries[l.index].pcm_id).collect()
    }

    /// The input list cpal 0.18 gave on a Pop!_OS desktop running PipeWire,
    /// with a Roland UA-22 and the motherboard's Realtek ALCS1200A
    /// (has_input = cpal reported at least one input setting).
    fn pop_os_pipewire() -> Vec<PcmEntry<'static>> {
        vec![
            e(
                "null",
                "Discard all samples (playback) or generate zero samples (capture)",
                true,
            ),
            e(
                "lavrate",
                "Rate Converter Plugin Using Libav/FFmpeg Library",
                false,
            ),
            e(
                "samplerate",
                "Rate Converter Plugin Using Samplerate Library",
                false,
            ),
            e(
                "speexrate",
                "Rate Converter Plugin Using Speex Resampler",
                false,
            ),
            e("jack", "JACK Audio Connection Kit", false),
            e("oss", "Open Sound System", false),
            e("pipewire", "PipeWire Sound Server", true),
            e("pulse", "PulseAudio Sound Server", true),
            e(
                "speex",
                "Plugin using Speex DSP (resample, agc, denoise, echo, dereverb)",
                false,
            ),
            e("upmix", "Plugin for channel upmix (4,6,8)", false),
            e(
                "vdownmix",
                "Plugin for channel downmix (stereo) with a simple spacialization",
                false,
            ),
            e(
                "default",
                "Default ALSA Output (currently PipeWire Media Server)",
                true,
            ),
            e("usbstream:CARD=NVidia", "HDA NVidia", false),
            e("hw:CARD=UA22,DEV=0", "UA-22, USB Audio", false),
            e("plughw:CARD=UA22,DEV=0", "UA-22, USB Audio", true),
            e("sysdefault:CARD=UA22", "UA-22, USB Audio", true),
            e("front:CARD=UA22,DEV=0", "UA-22, USB Audio", false),
            e("dsnoop:CARD=UA22,DEV=0", "UA-22, USB Audio", false),
            e("usbstream:CARD=UA22", "UA-22", false),
            e(
                "hw:CARD=Generic,DEV=0",
                "HD-Audio Generic, ALCS1200A Analog",
                true,
            ),
            e(
                "hw:CARD=Generic,DEV=2",
                "HD-Audio Generic, ALCS1200A Alt Analog",
                true,
            ),
            e(
                "plughw:CARD=Generic,DEV=0",
                "HD-Audio Generic, ALCS1200A Analog",
                true,
            ),
            e(
                "plughw:CARD=Generic,DEV=2",
                "HD-Audio Generic, ALCS1200A Alt Analog",
                true,
            ),
            e(
                "sysdefault:CARD=Generic",
                "HD-Audio Generic, ALCS1200A Analog",
                true,
            ),
            e(
                "front:CARD=Generic,DEV=0",
                "HD-Audio Generic, ALCS1200A Analog",
                true,
            ),
            e(
                "dsnoop:CARD=Generic,DEV=0",
                "HD-Audio Generic, ALCS1200A Analog",
                true,
            ),
            e(
                "dsnoop:CARD=Generic,DEV=2",
                "HD-Audio Generic, ALCS1200A Alt Analog",
                true,
            ),
            e("usbstream:CARD=Generic", "HD-Audio Generic", false),
            e("hw:CARD=1,DEV=0", "UA-22, USB Audio", false),
            e("plughw:CARD=1,DEV=0", "UA-22, USB Audio", true),
            e(
                "hw:CARD=2,DEV=0",
                "HD-Audio Generic, ALCS1200A Analog",
                true,
            ),
            e(
                "plughw:CARD=2,DEV=0",
                "HD-Audio Generic, ALCS1200A Analog",
                true,
            ),
            e(
                "hw:CARD=2,DEV=2",
                "HD-Audio Generic, ALCS1200A Alt Analog",
                true,
            ),
            e(
                "plughw:CARD=2,DEV=2",
                "HD-Audio Generic, ALCS1200A Alt Analog",
                true,
            ),
        ]
    }

    #[test]
    fn pop_os_list_is_default_then_cards_then_sound_servers() {
        let entries = pop_os_pipewire();
        let listed = arrange(&entries);
        assert_eq!(
            pcm_ids(&entries, &listed),
            vec![
                "default",
                "plughw:CARD=UA22,DEV=0",
                "hw:CARD=Generic,DEV=0",
                "hw:CARD=Generic,DEV=2",
                "pipewire",
                "pulse",
            ]
        );
        assert!(listed[0].is_default);
        assert!(listed[1..].iter().all(|l| !l.is_default));
        let names: Vec<&str> = listed.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names[1], "UA-22, USB Audio");
        assert_eq!(names[2], "HD-Audio Generic, ALCS1200A Analog");
    }

    #[test]
    fn plugins_are_never_listed_even_when_they_report_settings() {
        let entries = vec![
            e("null", "Discard all samples", true),
            e("upmix", "Upmix", true),
        ];
        assert!(arrange(&entries).is_empty());
    }

    #[test]
    fn plugins_and_unused_card_names_are_not_probed() {
        for id in [
            "null",
            "lavrate",
            "samplerate",
            "speexrate",
            "upmix",
            "vdownmix",
            "speex",
            "oss",
        ] {
            assert!(!worth_probing(&classify(id)), "{id}");
        }
        for id in [
            "usbstream:CARD=UA22",
            "surround51:CARD=Generic,DEV=0",
            "hdmi:CARD=NVidia,DEV=0",
        ] {
            assert!(!worth_probing(&classify(id)), "{id}");
        }
        for id in [
            "default",
            "pipewire",
            "pulse",
            "jack",
            "hw:CARD=UA22,DEV=0",
            "mymic",
        ] {
            assert!(worth_probing(&classify(id)), "{id}");
        }
    }

    #[test]
    fn a_sound_server_that_is_not_running_is_left_out() {
        let entries = vec![
            e("jack", "JACK Audio Connection Kit", false),
            e("pulse", "PulseAudio Sound Server", true),
            e("pipewire", "PipeWire Sound Server", true),
        ];
        let listed = arrange(&entries);
        assert_eq!(pcm_ids(&entries, &listed), vec!["pipewire", "pulse"]);
    }

    #[test]
    fn hw_is_chosen_when_it_reports_settings() {
        let entries = vec![
            e("plughw:CARD=X,DEV=0", "Card X", true),
            e("hw:CARD=X,DEV=0", "Card X", true),
        ];
        assert_eq!(
            pcm_ids(&entries, &arrange(&entries)),
            vec!["hw:CARD=X,DEV=0"]
        );
    }

    #[test]
    fn a_card_with_no_working_name_is_still_listed_as_plughw() {
        let entries = vec![
            e("hw:CARD=X,DEV=0", "Card X", false),
            e("plughw:CARD=X,DEV=0", "Card X", false),
            e("front:CARD=X,DEV=0", "Card X", false),
        ];
        assert_eq!(
            pcm_ids(&entries, &arrange(&entries)),
            vec!["plughw:CARD=X,DEV=0"]
        );
    }

    #[test]
    fn a_numbered_card_is_kept_when_no_named_card_matches() {
        let entries = vec![e("hw:CARD=0,DEV=0", "Card Zero", true)];
        assert_eq!(
            pcm_ids(&entries, &arrange(&entries)),
            vec!["hw:CARD=0,DEV=0"]
        );
    }

    #[test]
    fn two_cards_with_one_description_get_the_pcm_name_added() {
        let entries = vec![
            e("hw:CARD=UA22,DEV=0", "UA-22, USB Audio", true),
            e("hw:CARD=UA22_1,DEV=0", "UA-22, USB Audio", true),
        ];
        let names: Vec<String> = arrange(&entries).into_iter().map(|l| l.name).collect();
        assert_eq!(
            names,
            vec![
                "UA-22, USB Audio",
                "UA-22, USB Audio (hw:CARD=UA22_1,DEV=0)"
            ]
        );
    }

    #[test]
    fn a_users_own_pcm_is_listed_after_the_sound_servers() {
        let entries = vec![
            e("mymic", "My mic", true),
            e("broken", "Broken", false),
            e("pipewire", "PipeWire Sound Server", true),
            e("hw:CARD=X,DEV=0", "Card X", true),
        ];
        assert_eq!(
            pcm_ids(&entries, &arrange(&entries)),
            vec!["hw:CARD=X,DEV=0", "pipewire", "mymic"]
        );
    }

    #[test]
    fn card_arguments_parse_in_both_forms() {
        assert_eq!(
            classify("hw:CARD=UA22,DEV=3"),
            PcmKind::Card {
                alias: CardAlias::Hw,
                card: "UA22".into(),
                dev: 3
            }
        );
        assert_eq!(
            classify("plughw:1,2"),
            PcmKind::Card {
                alias: CardAlias::PlugHw,
                card: "1".into(),
                dev: 2
            }
        );
        assert_eq!(
            classify("sysdefault:CARD=Generic"),
            PcmKind::Card {
                alias: CardAlias::SysDefault,
                card: "Generic".into(),
                dev: 0
            }
        );
        assert_eq!(classify("hw:"), PcmKind::Other);
    }
}
