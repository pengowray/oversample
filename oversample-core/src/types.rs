use crate::audio::guano::GuanoMetadata;
use crate::audio::source::AudioSource;
use crate::audio::zc::ZcData;
use std::sync::Arc;

#[derive(Clone, Debug, Default)]
pub struct FileMetadata {
    pub file_size: usize,
    /// Container format tag: WAV / FLAC / OGG / MP3 / M4A / W4V / ZC.
    /// Empty string when constructed via `Default::default()`.
    pub format: &'static str,
    pub bits_per_sample: u16,
    pub is_float: bool,
    pub guano: Option<GuanoMetadata>,
    /// Byte offset of audio data within the file (WAV: data chunk start). None for non-WAV.
    pub data_offset: Option<u64>,
    /// Byte length of audio data region. None for non-WAV.
    pub data_size: Option<u64>,
    /// Anabat zero-crossing dot data. Populated for `.zc` files only.
    /// When `Some`, the file is a dot-plot recording (no continuous
    /// waveform); the `samples` field on `AudioData` may be a
    /// synthesised placeholder, and the renderer should switch to a
    /// `ZcPlot` view.
    pub zc_data: Option<Arc<ZcData>>,
}

#[derive(Clone)]
pub struct AudioData {
    /// Mono-mixed samples. Kept during migration; new code should use `source`.
    pub samples: Arc<Vec<f32>>,
    /// AudioSource abstraction for on-demand sample access.
    pub source: Arc<dyn AudioSource>,
    pub sample_rate: u32,
    /// Original channel count (before mono mixing).
    pub channels: u32,
    pub duration_secs: f64,
    pub metadata: FileMetadata,
}

impl std::fmt::Debug for AudioData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioData")
            .field("samples_len", &self.samples.len())
            .field("sample_rate", &self.sample_rate)
            .field("channels", &self.channels)
            .field("duration_secs", &self.duration_secs)
            .field("metadata", &self.metadata)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct SpectrogramColumn {
    pub magnitudes: Vec<f32>,
    pub time_offset: f64,
}

#[derive(Clone, Debug)]
pub struct SpectrogramData {
    pub columns: Arc<Vec<SpectrogramColumn>>,
    /// Total number of STFT columns in the full spectrogram.
    /// For large files, `columns` may be empty while `total_columns` is non-zero
    /// (columns are kept in the spectral store with LRU eviction instead).
    pub total_columns: usize,
    pub freq_resolution: f64,
    pub time_resolution: f64,
    pub max_freq: f64,
    pub sample_rate: u32,
}

#[derive(Clone, Debug)]
pub struct PreviewImage {
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<Vec<u8>>, // RGBA, row-major, row 0 = highest freq
}

#[derive(Clone, Debug)]
pub struct ZeroCrossingResult {
    pub estimated_frequency_hz: f64,
    pub crossing_count: usize,
    pub duration_secs: f64,
}

/// Pre-rendered spectrogram image data.
///
/// Normal spectrogram tiles store `db_data` (f32 dB values per pixel) so that
/// gain, contrast, and dynamic range can be adjusted at render time without
/// regenerating tiles.  Flow tiles store `db_data` + `flow_shifts` for deferred
/// compositing.  Coherence and chromagram tiles store pre-colored `pixels`
/// (RGBA u8) because their color encoding is coupled to the data.
pub struct PreRendered {
    pub width: u32,
    pub height: u32,
    /// RGBA pixel data (4 bytes/pixel).  Used by coherence, chromagram
    /// tiles and legacy non-tiled rendering.  Empty for dB tiles.
    pub pixels: Vec<u8>,
    /// dB values per pixel (one f32 per pixel, row-major, row 0 = highest freq).
    /// Used by normal spectrogram tiles and flow tiles.  Empty for pre-colored tiles.
    pub db_data: Vec<f32>,
    /// Per-pixel frequency shift values (same layout as db_data).
    /// Non-empty only for flow tiles.  Used with `db_data` for deferred flow compositing.
    pub flow_shifts: Vec<f32>,
}

impl PreRendered {
    /// Total memory footprint in bytes (for LRU cache accounting).
    pub fn byte_len(&self) -> usize {
        self.pixels.len()
            + self.db_data.len() * std::mem::size_of::<f32>()
            + self.flow_shifts.len() * std::mem::size_of::<f32>()
    }
}

/// Display settings for converting dB tile data to pixels at render time.
#[derive(Clone, Copy)]
pub struct SpectDisplaySettings {
    /// dB floor (e.g. -80.0).  Values below this map to black.
    pub floor_db: f32,
    /// dB range (e.g. 80.0).  `floor_db + range_db` = ceiling.
    pub range_db: f32,
    /// Gamma curve (1.0 = linear, <1 = brighter darks, >1 = more contrast).
    pub gamma: f32,
    /// Additive dB gain offset applied before floor/range mapping.
    pub gain_db: f32,
}

impl Default for SpectDisplaySettings {
    fn default() -> Self {
        Self { floor_db: -80.0, range_db: 80.0, gamma: 1.0, gain_db: 0.0 }
    }
}

/// A cue-point marker embedded in a WAV file (from the `cue ` and `LIST`/`adtl` chunks).
#[derive(Clone, Debug)]
pub struct WavMarker {
    /// Cue point ID (from the WAV cue chunk).
    pub id: u32,
    /// Sample position within the data chunk.
    pub position: u64,
    /// Label text from the `labl` sub-chunk, if present.
    pub label: Option<String>,
    /// Note text from the `note` sub-chunk, if present.
    pub note: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum FlowColorScheme {
    #[default]
    RedBlue,
    CoolWarm,
    TealOrange,
    PurpleGreen,
    Spectral,
}
