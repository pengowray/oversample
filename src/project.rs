use serde::{Serialize, Deserialize};
use crate::annotations::{FileIdentity, AudioFileMetadata, Annotation, generate_uuid, now_iso8601};
use crate::dsp::notch::NoiseProfile;

/// A batmonic project file (.batproj) — groups multiple audio files with their
/// annotations, sequence/multitrack definitions, and shared settings.
///
/// Serialized as YAML. Stored in OPFS on web, filesystem on Tauri.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BatProject {
    /// Format version (currently 1).
    pub version: u32,
    /// Unique project ID (UUID v4).
    pub id: String,
    /// App version that last wrote this file.
    #[serde(default)]
    pub app_version: String,
    /// When this project was created.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub created_at: Option<String>,
    /// When this project was last modified.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub modified_at: Option<String>,
    /// User-assigned project name.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub name: Option<String>,
    /// Notes/description for the whole project.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub notes: Option<String>,

    /// Ordered list of audio files in this project.
    #[serde(default)]
    pub files: Vec<ProjectFile>,

    /// Explicitly defined sequence groupings (auto-detected ones are also stored
    /// here once confirmed, so they survive file renames).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub sequences: Vec<SequenceDefinition>,

    /// Explicitly defined multitrack groupings.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub multitrack_groups: Vec<MultitrackGroup>,

    /// Shared project-level settings (playback, export, display — expanded later).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub config: Option<ProjectConfig>,

    /// Log of .batm sidecar files that were merged into this project.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub merge_history: Vec<MergeRecord>,
}

/// A single audio file within a project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProjectFile {
    /// Multi-layer file identity for matching.
    pub identity: FileIdentity,

    /// Audio metadata snapshot.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub audio_metadata: Option<AudioFileMetadata>,

    /// Annotations for this file.
    #[serde(default)]
    pub annotations: Vec<Annotation>,

    /// Noise reduction profile.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub noise_profile: Option<NoiseProfile>,

    /// Date/time adjustment in seconds. Added to the file's detected recording
    /// start time. Useful when the recorder's clock was wrong.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub time_offset_secs: f64,

    /// Override recording start time (ms since epoch). When set, takes priority
    /// over GUANO timestamps and file dates for this file.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub recording_start_override_ms: Option<f64>,

    /// Whether this file's metadata has been enriched from a Tauri/desktop session
    /// (which has access to filesystem creation dates, etc.).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub metadata_from_tauri: bool,

    /// File creation time (ms since epoch), if available. On many systems this is
    /// the recording start time (while last_modified is the end).
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub creation_time_ms: Option<f64>,
}

/// Explicit sequence definition — groups files into a continuous recording sequence.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SequenceDefinition {
    pub id: String,
    /// Ordered list of file indices into `BatProject::files`.
    pub file_indices: Vec<usize>,
    /// Maximum gap (seconds) to still consider files contiguous.
    #[serde(default = "default_gap_threshold")]
    pub gap_threshold_secs: f64,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub label: Option<String>,
}

/// Explicit multitrack group — groups files that are simultaneous multi-channel recordings.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MultitrackGroup {
    pub id: String,
    /// File indices into `BatProject::files`.
    pub file_indices: Vec<usize>,
    /// Group key (e.g. "260305_0058") — same as file_groups::TrackInfo::group_key.
    pub group_key: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub label: Option<String>,
}

/// Shared project-level settings. Expanded over time.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProjectConfig {
    // Future: playback mode, export settings, display preferences, etc.
}

/// Record of a .batm sidecar that was merged into this project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MergeRecord {
    /// When the merge happened.
    pub merged_at: String,
    /// OPFS storage key of the merged .batm file.
    pub batm_key: String,
    /// Filename of the audio file (for display).
    pub filename: String,
    /// Whether the original .batm was deleted after merging.
    #[serde(default)]
    pub deleted: bool,
}

fn is_zero(v: &f64) -> bool { *v == 0.0 }
fn default_gap_threshold() -> f64 { 60.0 }

impl BatProject {
    /// Create a new empty project.
    pub fn new() -> Self {
        Self {
            version: 1,
            id: generate_uuid(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            created_at: Some(now_iso8601()),
            modified_at: Some(now_iso8601()),
            name: None,
            notes: None,
            files: Vec::new(),
            sequences: Vec::new(),
            multitrack_groups: Vec::new(),
            config: None,
            merge_history: Vec::new(),
        }
    }

    /// Touch the modified_at timestamp and app_version.
    pub fn touch(&mut self) {
        self.modified_at = Some(now_iso8601());
        self.app_version = env!("CARGO_PKG_VERSION").to_string();
    }

    /// Find the project file entry matching a FileIdentity, returning its index.
    pub fn find_file(&self, identity: &FileIdentity) -> Option<usize> {
        self.files.iter().position(|pf| {
            // Match by spot hash first, then fallback to filename+size
            if let (Some(a), Some(b)) = (&pf.identity.spot_hash_b3, &identity.spot_hash_b3) {
                return a == b;
            }
            pf.identity.filename == identity.filename && pf.identity.file_size == identity.file_size
        })
    }

    /// Merge a .batm AnnotationSet into the corresponding project file entry.
    /// Returns true if annotations were merged, false if no matching file found.
    pub fn merge_batm(&mut self, set: &crate::annotations::AnnotationSet, batm_key: &str) -> bool {
        let Some(idx) = self.find_file(&set.file_identity) else { return false };
        let pf = &mut self.files[idx];

        // Merge annotations (append, skip duplicates by id)
        for ann in &set.annotations {
            if !pf.annotations.iter().any(|a| a.id == ann.id) {
                pf.annotations.push(ann.clone());
            }
        }

        // Take noise profile if the project file doesn't have one
        if pf.noise_profile.is_none() {
            pf.noise_profile = set.noise_profile.clone();
        }

        // Update audio metadata if missing
        if pf.audio_metadata.is_none() {
            pf.audio_metadata = set.audio_metadata.clone();
        }

        // Record the merge
        self.merge_history.push(MergeRecord {
            merged_at: now_iso8601(),
            batm_key: batm_key.to_string(),
            filename: set.file_identity.filename.clone(),
            deleted: false,
        });

        self.touch();
        true
    }

    /// Check if a .batm key has already been merged (avoids re-prompting).
    pub fn was_merged(&self, batm_key: &str) -> bool {
        self.merge_history.iter().any(|r| r.batm_key == batm_key)
    }

    /// Add a file to the project from a FileIdentity and optional audio metadata.
    pub fn add_file(&mut self, identity: FileIdentity, audio_metadata: Option<AudioFileMetadata>) -> usize {
        let idx = self.files.len();
        self.files.push(ProjectFile {
            identity,
            audio_metadata,
            annotations: Vec::new(),
            noise_profile: None,
            time_offset_secs: 0.0,
            recording_start_override_ms: None,
            metadata_from_tauri: false,
            creation_time_ms: None,
        });
        self.touch();
        idx
    }
}
