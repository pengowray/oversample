/// Parser and writer for GUANO metadata embedded in WAV files.
/// GUANO (Grand Unified Acoustic Notation Ontology) stores text metadata
/// as a "guan" subchunk in the RIFF structure.

#[derive(Clone, Debug, Default)]
pub struct GuanoMetadata {
    pub fields: Vec<(String, String)>,
}

impl GuanoMetadata {
    pub fn new() -> Self {
        Self { fields: Vec::new() }
    }

    pub fn add(&mut self, key: &str, value: &str) -> &mut Self {
        self.fields.push((key.to_string(), value.to_string()));
        self
    }

    /// Build the GUANO text representation (key: value lines).
    pub fn to_text(&self) -> String {
        build_guano_text(&self.fields)
    }
}

/// Build GUANO text from key-value pairs.
pub fn build_guano_text(fields: &[(String, String)]) -> String {
    let mut text = String::new();
    for (key, value) in fields {
        text.push_str(key);
        text.push_str(": ");
        text.push_str(value);
        text.push('\n');
    }
    text
}

/// Append a GUANO "guan" RIFF subchunk to WAV bytes in-place, and update
/// the RIFF size.
pub fn append_guano_chunk(wav_bytes: &mut Vec<u8>, guano_text: &str) {
    let text_bytes = guano_text.as_bytes();
    let chunk_size = text_bytes.len() as u32;

    // Every chunk before this one is padded to an even length, so an odd
    // total means the last chunk (usually `data`) is missing its pad byte.
    if wav_bytes.len() % 2 == 1 {
        wav_bytes.push(0);
    }

    // Append chunk: "guan" + size (LE u32) + text data
    wav_bytes.extend_from_slice(b"guan");
    wav_bytes.extend_from_slice(&chunk_size.to_le_bytes());
    wav_bytes.extend_from_slice(text_bytes);

    // RIFF word-alignment: pad with a zero byte if chunk data size is odd
    if !text_bytes.len().is_multiple_of(2) {
        wav_bytes.push(0);
    }

    super::wav::set_riff_size(wav_bytes);
}

/// Search raw WAV bytes for a "guan" RIFF subchunk and parse GUANO metadata.
pub fn parse_guano(bytes: &[u8]) -> Option<GuanoMetadata> {
    if !super::wav::is_riff_wave(bytes) {
        return None;
    }
    super::wav::riff_chunks(bytes, 12)
        .find(|c| c.id == b"guan" && c.complete())
        .and_then(|c| parse_guano_chunk(c.body))
}

/// Search the bytes that follow a WAV file's `data` chunk for a "guan" chunk.
/// `data_size` is the `data` chunk's size: when it is odd, the next chunk
/// starts after a pad byte. Some writers leave that byte out, so the unpadded
/// position is tried as well.
pub fn parse_guano_after_data(bytes: &[u8], data_size: u64) -> Option<GuanoMetadata> {
    let starts: &[usize] = if data_size & 1 == 1 { &[1, 0] } else { &[0] };
    starts.iter().find_map(|&start| {
        super::wav::riff_chunks(bytes, start)
            .find(|c| c.id == b"guan" && c.complete())
            .and_then(|c| parse_guano_chunk(c.body))
    })
}

/// Parse GUANO metadata from raw chunk body bytes (without the "guan" chunk header).
pub fn parse_guano_chunk(chunk_body: &[u8]) -> Option<GuanoMetadata> {
    let text = std::str::from_utf8(chunk_body).ok()?;
    Some(parse_guano_text(text))
}

/// Extra recording metadata for GUANO beyond the core fields.
#[derive(Default)]
pub struct RecordingGuanoExtra {
    /// Mic interface type: "Oboe", "WASAPI", "USB (UAC2)", "Web Audio API", etc.
    pub mic_interface: Option<String>,
    /// Mic name/description: USB device name, "Internal" (native only, not for web).
    pub mic_name: Option<String>,
    /// Web Audio API device label (only set when user selected a device via "ask" mode).
    pub mic_audio_device: Option<String>,
    /// USB mic manufacturer (for GUANO Make field).
    pub mic_make: Option<String>,
    /// GPS location: (latitude, longitude) in WGS84 decimal degrees.
    pub loc_position: Option<(f64, f64)>,
    /// Elevation in meters above mean sea level.
    pub loc_elevation: Option<f64>,
    /// Horizontal accuracy in meters.
    pub loc_accuracy: Option<f64>,
    /// Android device manufacturer (e.g. "samsung"). Privacy-controlled.
    pub device_make: Option<String>,
    /// Android device model (e.g. "SM-A556E"). Privacy-controlled.
    pub device_model: Option<String>,
    /// Pre-roll duration in seconds (listen buffer captured before user pressed record).
    /// None or 0.0 = no pre-roll.
    pub preroll_secs: Option<f64>,
}

/// Build GUANO metadata for a recording.
///
/// Field ordering follows the GUANO spec: GUANO namespace first, then standard
/// fields, then app-specific (Oversample|*) fields.
///
/// `timestamp` should be an ISO 8601 string with T separator and UTC offset
/// (e.g. "2024-03-15T10:30:00+10:00").
/// `version` should be the main Oversample app version (from the root crate).
pub fn build_recording_guano(
    sample_rate: u32,
    duration_secs: f64,
    filename: &str,
    is_tauri: bool,
    is_mobile: bool,
    extra: &RecordingGuanoExtra,
    timestamp: &str,
    version: &str,
) -> GuanoMetadata {
    let platform = if is_tauri && is_mobile {
        "Android"
    } else if is_tauri {
        "Desktop"
    } else {
        "Web"
    };

    // Make/Model: only for external mics (USB). Never use for internal/phone mic.
    let is_external_mic = extra
        .mic_interface
        .as_deref()
        .map(|i| i.contains("USB"))
        .unwrap_or(false);

    let mut g = GuanoMetadata::new();

    // ── GUANO namespace (must come first per spec) ──────────────────────
    g.add("GUANO|Version", "1.0");

    // ── Standard GUANO fields ───────────────────────────────────────────
    g.add("Timestamp", timestamp);
    g.add("Length", &format!("{:.6}", duration_secs));
    g.add("Samplerate", &sample_rate.to_string());

    // Make/Model reflect the recording hardware (mic), not the app/phone.
    // Only populated for external (USB) mics.
    if is_external_mic {
        if let Some(ref make) = extra.mic_make {
            if !make.is_empty() {
                g.add("Make", make);
            }
        }
        if let Some(ref name) = extra.mic_name {
            if !name.is_empty() {
                g.add("Model", name);
            }
        }
    }

    g.add("Original Filename", filename);

    // Location fields
    if let Some((lat, lon)) = extra.loc_position {
        g.add("Loc Position", &format!("{} {}", lat, lon));
    }
    if let Some(elev) = extra.loc_elevation {
        g.add("Loc Elevation", &format!("{:.1}", elev));
    }
    if let Some(acc) = extra.loc_accuracy {
        g.add("Loc Accuracy", &format!("{:.1}", acc));
    }

    // ── Oversample-specific fields (after standard ones) ────────────────
    g.add("Oversample|App|Version", version);
    g.add("Oversample|App|Platform", platform);

    // Device info (Android only, privacy-controlled)
    if let Some(ref make) = extra.device_make {
        if !make.is_empty() {
            g.add("Oversample|Device|Make", make);
        }
    }
    if let Some(ref model) = extra.device_model {
        if !model.is_empty() {
            g.add("Oversample|Device|Model", model);
        }
    }

    // Mic info
    if let Some(ref interface) = extra.mic_interface {
        if !interface.is_empty() {
            g.add("Oversample|Mic|Interface", interface);
        }
    }
    if let Some(ref name) = extra.mic_name {
        if !name.is_empty() {
            g.add("Oversample|Mic|Name", name);
        }
    }
    // Web Audio API device label (separate from native Mic|Name)
    if let Some(ref device) = extra.mic_audio_device {
        if !device.is_empty() {
            g.add("Oversample|Mic|Audio Device", device);
        }
    }

    // Pre-roll: seconds of listen buffer captured before the user pressed record.
    if let Some(preroll) = extra.preroll_secs {
        if preroll > 0.0 {
            g.add("Oversample|Audio|Preroll", &format!("{:.3}", preroll));
        }
    }

    g
}

fn parse_guano_text(text: &str) -> GuanoMetadata {
    let mut fields = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            fields.push((key.trim().to_string(), value.trim().to_string()));
        }
    }
    GuanoMetadata { fields }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A "guan" chunk holding `text`, as the bytes after the `data` chunk,
    /// with a pad byte first if `pad`.
    fn after_data(text: &str, pad: bool) -> Vec<u8> {
        let mut bytes = Vec::new();
        if pad {
            bytes.push(0);
        }
        bytes.extend_from_slice(b"guan");
        bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
        bytes.extend_from_slice(text.as_bytes());
        bytes
    }

    const TEXT: &str = "GUANO|Version: 1.0\nMake: TASCAM\n";

    #[test]
    fn found_after_even_sized_audio() {
        let g = parse_guano_after_data(&after_data(TEXT, false), 1000).unwrap();
        assert_eq!(g.fields[1], ("Make".to_string(), "TASCAM".to_string()));
    }

    /// 24-bit mono audio with an odd number of frames has an odd size, so the
    /// chunk after it starts after a pad byte.
    #[test]
    fn found_after_the_pad_byte_of_odd_sized_audio() {
        assert!(parse_guano_after_data(&after_data(TEXT, true), 1001).is_some());
    }

    #[test]
    fn found_when_a_writer_left_the_pad_byte_out() {
        assert!(parse_guano_after_data(&after_data(TEXT, false), 1001).is_some());
    }

    #[test]
    fn appended_chunk_round_trips_after_odd_sized_audio() {
        // RIFF header, then a 3-byte `data` chunk.
        let mut wav = b"RIFF\0\0\0\0WAVEdata\x03\0\0\0abc".to_vec();
        append_guano_chunk(&mut wav, TEXT);
        let g = parse_guano_after_data(&wav[23..], 3).unwrap();
        assert_eq!(g.fields.len(), 2);
    }
}
