//! Linux: the files and `pactl` output that say what a sound card really
//! records at and what it is called. The parsing is in
//! `oversample_core::audio::alsa_card`; this module reads the files, runs
//! `pactl`, and caches what it found for one listing of the microphones.
//!
//! Every source is optional: a missing file or a missing `pactl` gives `None`
//! or an empty list, and the caller falls back to what cpal reports.

use oversample_core::audio::alsa_card::{self, CaptureFormat, Card, PulseSource};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// What a capture device is and who made it, when the system knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceIdentity {
    /// The device's name: a USB card's model, such as "DUO-CAPTURE EX", or
    /// the sound server's description of its source.
    pub name: String,
    pub manufacturer: Option<String>,
    pub is_usb: bool,
}

/// The sound server's default source: the setting it records at, and the
/// device behind it.
#[derive(Clone, Debug)]
pub struct DefaultSource {
    pub format: CaptureFormat,
    pub identity: Option<SourceIdentity>,
}

fn read(path: impl AsRef<Path>) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Run a command with `LC_ALL=C` and return its output, or `None` when it is
/// missing, fails, or takes more than two seconds (a sound server that does
/// not answer).
fn run(program: &str, args: &[&str]) -> Option<String> {
    let mut child = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut out = String::new();
        let _ = std::io::Read::read_to_string(&mut stdout, &mut out);
        let _ = tx.send(out);
    });
    match rx.recv_timeout(Duration::from_secs(2)) {
        Ok(out) => {
            let ok = child.wait().map(|s| s.success()).unwrap_or(false);
            ok.then_some(out)
        }
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            None
        }
    }
}

/// What is known about the system's sound cards, read once per listing.
#[derive(Default)]
pub struct AlsaContext {
    cards: Option<Vec<Card>>,
    identities: Vec<(u32, Option<SourceIdentity>)>,
    default_source: Option<Option<DefaultSource>>,
}

impl AlsaContext {
    pub fn new() -> Self {
        Self::default()
    }

    fn cards(&mut self) -> &[Card] {
        self.cards.get_or_insert_with(|| {
            read("/proc/asound/cards")
                .map(|t| alsa_card::parse_cards(&t))
                .unwrap_or_default()
        })
    }

    /// The card number for a PCM name's `CARD=` value.
    pub fn card_number(&mut self, card_arg: &str) -> Option<u32> {
        alsa_card::card_number(self.cards(), card_arg)
    }

    /// A USB card's name and maker, or `None` for other cards.
    pub fn usb_identity(&mut self, card: u32) -> Option<SourceIdentity> {
        if let Some((_, id)) = self.identities.iter().find(|(n, _)| *n == card) {
            return id.clone();
        }
        let id = read_usb_identity(card);
        self.identities.push((card, id.clone()));
        id
    }

    /// The sound server's default source, from `pactl`. `None` when `pactl`
    /// is missing, no server runs, or the source's format is not shown.
    pub fn default_source(&mut self) -> Option<DefaultSource> {
        if self.default_source.is_none() {
            let found = read_default_source(self);
            self.default_source = Some(found);
        }
        self.default_source.clone().flatten()
    }
}

fn read_usb_identity(card: u32) -> Option<SourceIdentity> {
    // Only USB cards have a usbid file.
    if !Path::new(&format!("/proc/asound/card{card}/usbid")).exists() {
        return None;
    }
    let udev = read(format!("/run/udev/data/+sound:card{card}"))
        .map(|t| alsa_card::parse_udev_data(&t))
        .unwrap_or_default();
    // The sound card's device is the USB interface; its parent is the USB
    // device, which has the product and manufacturer strings.
    let usb_dir = format!("/sys/class/sound/card{card}/device/..");
    let product = read(format!("{usb_dir}/product"));
    let manufacturer = read(format!("{usb_dir}/manufacturer"));
    let (name, manufacturer) =
        alsa_card::usb_card_name(&udev, product.as_deref(), manufacturer.as_deref())?;
    Some(SourceIdentity {
        name,
        manufacturer,
        is_usb: true,
    })
}

/// How many capture devices (PCM devices) a card has.
pub fn capture_device_count(card: u32) -> usize {
    std::fs::read_dir(format!("/proc/asound/card{card}"))
        .map(|dir| {
            dir.flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().to_string();
                    n.starts_with("pcm") && n.ends_with('c')
                })
                .count()
        })
        .unwrap_or(0)
}

/// The settings PCM device `dev` of `card` records at without conversion:
/// a USB card's stream file, otherwise what the device is running at when a
/// program (such as the sound server) has it open. Empty when neither is
/// known.
pub fn card_capture_formats(card: u32, dev: u32) -> Vec<CaptureFormat> {
    if let Some(text) = read(format!("/proc/asound/card{card}/stream{dev}")) {
        let formats = alsa_card::parse_usb_stream_capture(&text);
        if !formats.is_empty() {
            return formats;
        }
    }
    read(format!("/proc/asound/card{card}/pcm{dev}c/sub0/hw_params"))
        .and_then(|t| alsa_card::parse_hw_params(&t))
        .into_iter()
        .collect()
}

fn read_default_source(ctx: &mut AlsaContext) -> Option<DefaultSource> {
    let info = run("pactl", &["info"])?;
    let name = alsa_card::parse_pactl_default_source(&info)?;
    let list = run("pactl", &["list", "sources"])?;
    let source: PulseSource = alsa_card::parse_pactl_sources(&list)
        .into_iter()
        .find(|s| s.name == name)?;
    let format = source.spec.clone()?;
    let identity = match source.alsa_card() {
        Some(card) if !source.is_monitor() => ctx.usb_identity(card),
        _ => None,
    }
    .or_else(|| {
        let name = source
            .property("device.description")
            .map(str::to_string)
            .unwrap_or_else(|| source.description.clone());
        (!name.is_empty()).then(|| SourceIdentity {
            name,
            manufacturer: source.property("device.vendor.name").map(str::to_string),
            is_usb: !source.is_monitor() && source.property("device.bus") == Some("usb"),
        })
    });
    Some(DefaultSource { format, identity })
}
