use serde::{Serialize, Deserialize};

/// Multi-layered file identity for matching annotations to audio files across sessions.
/// Layers are computed progressively — Layer 1 is instant, higher layers are lazy.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FileIdentity {
    /// Layer 1: filename (basename only).
    pub filename: String,
    /// Layer 1: file size in bytes.
    pub file_size: u64,

    /// Layer 2: Spot-check hash. SHA-256 of (first 4KB + middle 4KB + last 4KB).
    /// Fast even for multi-GB files. None until computed.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub spot_hash: Option<String>,

    /// Layer 3: Audio-data-only hash. SHA-256 of just PCM/audio bytes (no headers).
    /// Survives metadata edits. Lazy — computed on demand.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub audio_hash: Option<String>,

    /// Layer 4: Full file SHA-256. Computed lazily on explicit request.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub full_sha256: Option<String>,

    /// Original file path (Tauri only). Used for sidecar file placement and re-finding.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub file_path: Option<String>,
}

/// Confidence level when matching a FileIdentity against another.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MatchConfidence {
    /// No match at all.
    None,
    /// Only filename + size match (fragile).
    Weak,
    /// Spot-check hash matches (very likely the same file).
    Likely,
    /// Audio-data hash matches (same audio content, possibly different metadata).
    High,
    /// Full SHA-256 matches (identical files).
    Certain,
}

impl FileIdentity {
    /// Compare two identities and return the highest confidence match.
    pub fn match_confidence(&self, other: &FileIdentity) -> MatchConfidence {
        // Check from highest confidence downward
        if let (Some(a), Some(b)) = (&self.full_sha256, &other.full_sha256) {
            if a == b {
                return MatchConfidence::Certain;
            }
            // Different full hashes = definitely different files
            return MatchConfidence::None;
        }
        if let (Some(a), Some(b)) = (&self.audio_hash, &other.audio_hash) {
            if a == b {
                return MatchConfidence::High;
            }
            return MatchConfidence::None;
        }
        if let (Some(a), Some(b)) = (&self.spot_hash, &other.spot_hash) {
            if a == b {
                return MatchConfidence::Likely;
            }
            return MatchConfidence::None;
        }
        // Fall back to filename + size
        if self.filename == other.filename && self.file_size == other.file_size {
            return MatchConfidence::Weak;
        }
        MatchConfidence::None
    }
}

/// Unique annotation identifier (UUID v4 as string).
pub type AnnotationId = String;

/// A saved time+frequency region selection.
/// Frequency bounds are optional — a time-only selection has no freq_low/freq_high.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SavedSelection {
    pub time_start: f64,
    pub time_end: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub freq_low: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub freq_high: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub color: Option<String>,
}

/// A time-position marker (future).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Marker {
    pub time: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub color: Option<String>,
}

/// A measurement between two time+frequency points (future).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Measurement {
    pub start_time: f64,
    pub start_freq: f64,
    pub end_time: f64,
    pub end_freq: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub label: Option<String>,
}

/// Tagged annotation kind — extensible for future types.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AnnotationKind {
    Selection(SavedSelection),
    Marker(Marker),
    Measurement(Measurement),
}

/// A single annotation with metadata.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Annotation {
    pub id: AnnotationId,
    #[serde(flatten)]
    pub kind: AnnotationKind,
    pub created_at: String,
    pub modified_at: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub notes: Option<String>,
}

/// Per-file annotation collection — serialized to .batm sidecar files (YAML).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnnotationSet {
    pub version: u32,
    pub file_identity: FileIdentity,
    pub annotations: Vec<Annotation>,
    pub app_version: String,
}

/// In-memory annotation store, indexed parallel to AppState::files.
#[derive(Clone, Debug, Default)]
pub struct AnnotationStore {
    pub sets: Vec<Option<AnnotationSet>>,
}

impl AnnotationStore {
    /// Ensure the store has at least `len` slots (push None for new files).
    pub fn ensure_len(&mut self, len: usize) {
        while self.sets.len() < len {
            self.sets.push(None);
        }
    }

    /// Remove the entry at `index`, shifting subsequent entries.
    pub fn remove(&mut self, index: usize) {
        if index < self.sets.len() {
            self.sets.remove(index);
        }
    }
}

/// Generate a UUID v4 string using js_sys::Math::random().
pub fn generate_uuid() -> String {
    let random_bytes: Vec<u8> = (0..16)
        .map(|_| (js_sys::Math::random() * 256.0) as u8)
        .collect();

    // Set version (4) and variant (10xx) bits
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&random_bytes);
    bytes[6] = (bytes[6] & 0x0f) | 0x40; // version 4
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant 10xx

    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

/// Get the current time as an ISO 8601 string using js_sys::Date.
pub fn now_iso8601() -> String {
    let date = js_sys::Date::new_0();
    date.to_iso_string().as_string().unwrap_or_default()
}
