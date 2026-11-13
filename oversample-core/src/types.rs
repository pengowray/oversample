use crate::audio::guano::GuanoMetadata;
use crate::audio::source::{AudioSource, InMemorySource};
use crate::audio::zc::ZcData;
use std::sync::Arc;

/// Ceiling on the rate a time-expansion correction may produce. A 384 kHz
/// recording expanded 10x lands at 3.84 MHz, so the limit only rejects
/// factors that are clearly bad metadata rather than an unusual detector.
pub const MAX_CORRECTED_SAMPLE_RATE: f64 = 10_000_000.0;

/// A metadata block a recorder wrote in its own layout rather than as a
/// standard chunk. Fields keep the recorder's own key names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RecorderBlock {
    /// Recorder model, e.g. "Pettersson D500X".
    pub recorder: &'static str,
    pub fields: Vec<(String, String)>,
}

/// How a file's sample rate was corrected when it was loaded, for recordings stored
/// time-expanded (slowed down). See [`AudioData::apply_time_expansion`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TimeExpansion {
    /// What the recording was slowed down by: 10 for a file played 10 times slower.
    pub factor: f64,
    /// The sample rate the file itself states, before the correction.
    pub stored_rate: u32,
}

/// WAV-specific facts beyond rate, channels and sample width.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WavDetails {
    /// Significant bits per sample, when the file declares fewer than the
    /// sample width (12-bit audio stored in 16-bit samples, say).
    pub valid_bits: Option<u16>,
    /// The `fmt ` chunk is WAVE_FORMAT_EXTENSIBLE.
    pub extensible: bool,
    /// The container is RF64 (64-bit sizes in a `ds64` chunk).
    pub rf64: bool,
    /// The audio is in a `wavl` list (data and silence chunks), 1991 spec.
    pub wave_list: bool,
    /// Speaker positions from an extensible header, when non-zero.
    pub channel_mask: Option<u32>,
    pub recorder_block: Option<RecorderBlock>,
    /// Wildlife Acoustics metadata from a `wamd` chunk (see [`crate::audio::wamd`]).
    pub wamd: Option<RecorderBlock>,
    /// Problems found in the file's structure and how they were handled,
    /// written for the user.
    pub notes: Vec<WavNote>,
}

/// One problem found in a WAV file's structure, for the metadata panel.
#[derive(Clone, Debug, PartialEq)]
pub struct WavNote {
    pub text: String,
    /// The problem changes which audio is read (or how other programs read
    /// it). False for header details that don't affect the audio.
    pub warning: bool,
    /// More detail, shown as a tooltip.
    pub detail: Option<String>,
}

/// What a loaded file is: its container or codec, or a live-microphone entry.
/// Shown to the user by [`AudioFormat::as_str`] ("WAV", "MP3", ...).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AudioFormat {
    /// Not set: a `FileMetadata` built with `Default::default()`. Shows as "".
    #[default]
    Unknown,
    Wav,
    /// Wildlife Acoustics W4V: a WAV container holding a lossy block codec
    /// that decodes onto a 16-bit grid.
    W4v,
    Flac,
    /// Ogg Vorbis.
    Ogg,
    Mp3,
    /// MP4 audio: AAC (lossy) or ALAC (lossless). `bits_per_sample` tells
    /// them apart.
    M4a,
    /// Anabat zero-crossing.
    Zc,
    /// A recording made in the app, once recording has started.
    Rec,
    /// The live microphone entry before recording starts.
    Mic,
}

impl AudioFormat {
    /// Every variant, in declaration order.
    pub const ALL: [AudioFormat; 10] = [
        Self::Unknown,
        Self::Wav,
        Self::W4v,
        Self::Flac,
        Self::Ogg,
        Self::Mp3,
        Self::M4a,
        Self::Zc,
        Self::Rec,
        Self::Mic,
    ];

    /// The name shown to the user and written to sidecars.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "",
            Self::Wav => "WAV",
            Self::W4v => "W4V",
            Self::Flac => "FLAC",
            Self::Ogg => "OGG",
            Self::Mp3 => "MP3",
            Self::M4a => "M4A",
            Self::Zc => "ZC",
            Self::Rec => "REC",
            Self::Mic => "MIC",
        }
    }
}

impl std::fmt::Display for AudioFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.pad(self.as_str())
    }
}

#[derive(Clone, Debug, Default)]
pub struct FileMetadata {
    pub file_size: usize,
    /// [`AudioFormat::Unknown`] when built with `Default::default()`.
    pub format: AudioFormat,
    /// PCM bit depth of the stored samples, or [`FileMetadata::LOSSY_BITS`]
    /// (0) for a file decoded from a lossy codec (MP3, Vorbis, AAC): those
    /// codecs store no PCM samples, and the decoder produces floats. Read it
    /// with [`FileMetadata::pcm_bits`], which gives `None` for those files.
    pub bits_per_sample: u16,
    /// The stored samples are IEEE float. False for lossy files.
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
    /// WAV-only details. None for other formats.
    pub wav: Option<WavDetails>,
    /// Set when the file was stored time-expanded and its rate was corrected on load;
    /// `sample_rate` is then the corrected rate. `None` for a real-time recording.
    pub time_expansion: Option<TimeExpansion>,
}

impl FileMetadata {
    /// The time expansion factor the file itself states: GUANO's `TE` field, else the
    /// `Time Expansion` field of a Wildlife Acoustics `wamd` chunk. `None` for a file
    /// that states none, or states 1 (recorded in real time).
    pub fn declared_time_expansion(&self) -> Option<f64> {
        declared_time_expansion(
            self.format,
            self.guano.as_ref(),
            self.wav.as_ref().and_then(|w| w.wamd.as_ref()),
        )
    }

    /// `bits_per_sample` of a file decoded from a lossy codec.
    pub const LOSSY_BITS: u16 = 0;

    /// The file's PCM bit depth, or `None` for a file decoded from a lossy
    /// codec.
    pub fn pcm_bits(&self) -> Option<u16> {
        (self.bits_per_sample != Self::LOSSY_BITS).then_some(self.bits_per_sample)
    }

    /// The bit depth as shown to the user: "16-bit", "32-bit float", or
    /// "Lossy" for a file decoded from a lossy codec. Short, because it fills
    /// a narrow stat tile in the Analysis panel; the format is shown beside it.
    pub fn bit_depth_label(&self) -> String {
        match self.pcm_bits() {
            Some(bits) if self.is_float => format!("{bits}-bit float"),
            Some(bits) => format!("{bits}-bit"),
            None => "Lossy".to_string(),
        }
    }
}

/// A time expansion factor that is worth applying: a number from 0.01 to 100, and not
/// 1, which means the recording was made in real time.
fn usable_factor(factor: f64) -> Option<f64> {
    (factor.is_finite() && (0.01..=100.0).contains(&factor) && (factor - 1.0).abs() > 1e-9)
        .then_some(factor)
}

/// The time expansion factor a file states, from the metadata read from it: GUANO's
/// `TE` field, else a Wildlife Acoustics `wamd` chunk's. Only WAV and W4V files are
/// read, since `guano` also holds an M4A file's own tags.
///
/// SonoBat writes an empty `TE`, and a recorder that records in real time writes 1;
/// both give `None`.
pub fn declared_time_expansion(
    format: AudioFormat,
    guano: Option<&GuanoMetadata>,
    wamd: Option<&RecorderBlock>,
) -> Option<f64> {
    if !matches!(format, AudioFormat::Wav | AudioFormat::W4v) {
        return None;
    }
    let field = |fields: &[(String, String)], key: &str| {
        fields
            .iter()
            .find(|(k, _)| k.trim() == key)
            .and_then(|(_, v)| v.trim().parse::<f64>().ok())
    };
    guano
        .and_then(|g| field(&g.fields, "TE"))
        .or_else(|| wamd.and_then(|w| field(&w.fields, "Time Expansion")))
        .and_then(usable_factor)
}

/// The factor to apply when a file is loaded: the one the source it came from gave
/// (Wikimedia Commons, the recording search), else the one the file itself states. The
/// source's wins, so a file is never corrected twice when the source read the same
/// header.
pub fn time_expansion_to_apply(source: Option<f64>, meta: &FileMetadata) -> Option<f64> {
    source
        .and_then(usable_factor)
        .or_else(|| meta.declared_time_expansion())
}

#[derive(Clone)]
pub struct AudioData {
    /// Zero-copy in-memory mono buffer. For in-memory sources this is the whole
    /// file (sharing `source`'s Arc); for streaming sources it's the decoded
    /// head. Kept in lock-step with `source` (e.g. the live-recording snapshot
    /// rebuilds both together), so it stays the fast path for MonoMix reads —
    /// prefer it over `source.read_region(MonoMix, ..)`, which allocates.
    pub samples: Arc<Vec<f32>>,
    /// AudioSource abstraction for on-demand sample access (random-access reads,
    /// non-mono channel views, streaming prefetch).
    pub source: Arc<dyn AudioSource>,
    pub sample_rate: u32,
    /// Original channel count (before mono mixing).
    pub channels: u32,
    pub duration_secs: f64,
    pub metadata: FileMetadata,
}

impl AudioData {
    /// Reinterpret a recording that was stored time-expanded — a 10x bat
    /// detector file, say — at the rate the original event happened at.
    /// Frequencies scale up by `factor` and the duration shrinks by it; the
    /// samples themselves are untouched, only the rate they're read at
    /// changes.
    ///
    /// Returns `false` and leaves `self` alone when `factor` isn't a usable
    /// expansion: non-finite, zero or negative, exactly 1, or large enough to
    /// push the corrected rate past [`MAX_CORRECTED_SAMPLE_RATE`].
    pub fn apply_time_expansion(&mut self, factor: f64) -> bool {
        if !factor.is_finite() || factor <= 0.0 || (factor - 1.0).abs() < 1e-9 {
            return false;
        }
        let corrected = (self.sample_rate as f64 * factor).round();
        if !(1.0..=MAX_CORRECTED_SAMPLE_RATE).contains(&corrected) {
            return false;
        }
        let corrected = corrected as u32;
        self.metadata.time_expansion = Some(TimeExpansion {
            // Two corrections in a row multiply; the stored rate stays the file's own.
            factor: self
                .metadata
                .time_expansion
                .map_or(factor, |t| t.factor * factor),
            stored_rate: self
                .metadata
                .time_expansion
                .map_or(self.sample_rate, |t| t.stored_rate),
        });
        self.sample_rate = corrected;
        self.duration_secs /= factor;
        // Keep the source's own rate in step. Only an in-memory source can be
        // rebuilt this cheaply (everything in it is behind an Arc), and it is
        // the only kind that reaches this path — corrections are applied right
        // after a full decode, never to a streaming source.
        if let Some(mem) = self.source.as_any().downcast_ref::<InMemorySource>() {
            self.source = Arc::new(InMemorySource {
                samples: mem.samples.clone(),
                raw_samples: mem.raw_samples.clone(),
                sample_rate: corrected,
                channels: mem.channels,
            });
        }
        true
    }
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
    /// In-memory STFT columns. May NOT be the full spectrogram — for large
    /// files this is empty and the columns live in the spectral store instead
    /// (see `total_columns` and [`SpectrogramData::is_store_backed`]). Treat
    /// `total_columns` as the authoritative width; only iterate `columns`
    /// directly after checking `is_store_backed()`.
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

impl SpectrogramData {
    /// True when `columns` is NOT the full spectrogram — the columns live in the
    /// spectral store (LRU) and must be read through it, not by iterating
    /// `columns` directly. For large files `columns` is empty while
    /// `total_columns` is non-zero (see the field docs). Consumers that walk
    /// `columns` assuming completeness (e.g. the non-tiled renderers) should
    /// check this first, or use `total_columns` as the authoritative width.
    pub fn is_store_backed(&self) -> bool {
        self.columns.len() != self.total_columns
    }

    /// Number of columns actually resident in the in-memory `columns` vec
    /// (0 for a store-backed spectrogram).
    pub fn columns_in_memory(&self) -> usize {
        self.columns.len()
    }
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
        Self {
            floor_db: -80.0,
            range_db: 80.0,
            gamma: 1.0,
            gain_db: 0.0,
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dummy_audio(sample_rate: u32) -> AudioData {
        let samples = Arc::new(vec![0.0f32; sample_rate as usize]);
        AudioData {
            samples: samples.clone(),
            source: Arc::new(InMemorySource {
                samples,
                raw_samples: None,
                sample_rate,
                channels: 1,
            }),
            sample_rate,
            channels: 1,
            duration_secs: 1.0,
            metadata: FileMetadata::default(),
        }
    }

    #[test]
    fn lossy_files_have_no_pcm_bits() {
        let meta = |format, bits_per_sample, is_float| FileMetadata {
            format,
            bits_per_sample,
            is_float,
            ..FileMetadata::default()
        };
        let mp3 = meta(AudioFormat::Mp3, FileMetadata::LOSSY_BITS, false);
        assert_eq!(mp3.pcm_bits(), None);
        assert_eq!(mp3.bit_depth_label(), "Lossy");
        let wav = meta(AudioFormat::Wav, 24, false);
        assert_eq!(wav.pcm_bits(), Some(24));
        assert_eq!(wav.bit_depth_label(), "24-bit");
        assert_eq!(
            meta(AudioFormat::Wav, 32, true).bit_depth_label(),
            "32-bit float"
        );
        assert_eq!(
            meta(AudioFormat::Unknown, 0, false).bit_depth_label(),
            "Lossy"
        );
    }

    /// The names the UI shows and sidecars store. Changing one changes the
    /// text users see and the text new sidecars store.
    #[test]
    fn audio_format_names() {
        assert_eq!(
            AudioFormat::ALL.map(AudioFormat::as_str),
            ["", "WAV", "W4V", "FLAC", "OGG", "MP3", "M4A", "ZC", "REC", "MIC"],
        );
        for format in AudioFormat::ALL {
            assert_eq!(format.to_string(), format.as_str());
        }
        assert_eq!(FileMetadata::default().format, AudioFormat::Unknown);
        // Padding works as it does for a &str.
        assert_eq!(format!("[{:>4}]", AudioFormat::Zc), "[  ZC]");
    }

    fn guano(fields: &[(&str, &str)]) -> GuanoMetadata {
        let mut g = GuanoMetadata::new();
        for (k, v) in fields {
            g.add(k, v);
        }
        g
    }

    fn wamd(te: &str) -> RecorderBlock {
        RecorderBlock {
            recorder: "Wildlife Acoustics",
            fields: vec![("Time Expansion".into(), te.into())],
        }
    }

    fn file(
        format: AudioFormat,
        g: Option<GuanoMetadata>,
        w: Option<RecorderBlock>,
    ) -> FileMetadata {
        FileMetadata {
            format,
            guano: g,
            wav: w.map(|wamd| WavDetails {
                wamd: Some(wamd),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// Which factor a file is loaded with: the one its source gave, else its own.
    #[test]
    fn time_expansion_a_file_states() {
        let te10 = || Some(guano(&[("Samplerate", "384000"), ("TE", "10")]));
        // GUANO, a `wamd` chunk, or both.
        assert_eq!(
            file(AudioFormat::Wav, te10(), None).declared_time_expansion(),
            Some(10.0)
        );
        assert_eq!(
            file(AudioFormat::Wav, None, Some(wamd("10"))).declared_time_expansion(),
            Some(10.0)
        );
        assert_eq!(
            file(AudioFormat::W4v, te10(), Some(wamd("4"))).declared_time_expansion(),
            Some(10.0)
        );
        // Recorded in real time, or no value at all: SonoBat writes "TE:  ".
        for g in [
            vec![("TE", "1")],
            vec![("TE", "")],
            vec![("Samplerate", "500000")],
        ] {
            let meta = file(AudioFormat::Wav, Some(guano(&g)), None);
            assert_eq!(meta.declared_time_expansion(), None, "{g:?}");
        }
        assert_eq!(
            file(AudioFormat::Wav, None, Some(wamd("1"))).declared_time_expansion(),
            None
        );
        // `guano` also holds an M4A file's own tags, which are not GUANO.
        assert_eq!(
            file(AudioFormat::M4a, te10(), None).declared_time_expansion(),
            None
        );

        // The source's factor wins, so a file is corrected once, not twice.
        let meta = file(AudioFormat::Wav, te10(), None);
        assert_eq!(time_expansion_to_apply(Some(10.0), &meta), Some(10.0));
        assert_eq!(time_expansion_to_apply(Some(0.5), &meta), Some(0.5));
        assert_eq!(time_expansion_to_apply(None, &meta), Some(10.0));
        // A source that gives 1, or nonsense, falls back to the file.
        assert_eq!(time_expansion_to_apply(Some(1.0), &meta), Some(10.0));
        assert_eq!(time_expansion_to_apply(Some(f64::NAN), &meta), Some(10.0));
        assert_eq!(
            time_expansion_to_apply(None, &FileMetadata::default()),
            None
        );
    }

    #[test]
    fn time_expansion_is_recorded_for_the_panel() {
        let mut audio = dummy_audio(38_400);
        assert!(audio.apply_time_expansion(10.0));
        assert_eq!(
            audio.metadata.time_expansion,
            Some(TimeExpansion {
                factor: 10.0,
                stored_rate: 38_400
            })
        );
        // A second correction multiplies; the stored rate stays the file's own.
        assert!(audio.apply_time_expansion(2.0));
        assert_eq!(
            audio.metadata.time_expansion,
            Some(TimeExpansion {
                factor: 20.0,
                stored_rate: 38_400
            })
        );
        assert_eq!(audio.sample_rate, 768_000);
        // A rejected factor records nothing.
        let mut audio = dummy_audio(44_100);
        assert!(!audio.apply_time_expansion(1.0));
        assert_eq!(audio.metadata.time_expansion, None);
    }

    #[test]
    fn time_expansion_scales_rate_and_duration() {
        let mut audio = dummy_audio(44_100);
        assert!(audio.apply_time_expansion(10.0));
        assert_eq!(audio.sample_rate, 441_000);
        assert_eq!(audio.source.sample_rate(), 441_000);
        assert!((audio.duration_secs - 0.1).abs() < 1e-9);
        // Samples are reinterpreted, never resampled.
        assert_eq!(audio.samples.len(), 44_100);
    }

    #[test]
    fn time_expansion_rejects_useless_factors() {
        for factor in [f64::NAN, f64::INFINITY, 0.0, -10.0, 1.0] {
            let mut audio = dummy_audio(44_100);
            assert!(
                !audio.apply_time_expansion(factor),
                "factor {factor} should be rejected"
            );
            assert_eq!(audio.sample_rate, 44_100);
            assert_eq!(audio.duration_secs, 1.0);
        }
    }

    #[test]
    fn time_expansion_rejects_absurd_corrected_rate() {
        let mut audio = dummy_audio(384_000);
        assert!(!audio.apply_time_expansion(1000.0));
        assert_eq!(audio.sample_rate, 384_000);
        // Just under the ceiling still applies.
        assert!(audio.apply_time_expansion(10.0));
        assert_eq!(audio.sample_rate, 3_840_000);
    }

    #[test]
    fn time_expansion_below_one_compresses() {
        let mut audio = dummy_audio(48_000);
        assert!(audio.apply_time_expansion(0.5));
        assert_eq!(audio.sample_rate, 24_000);
        assert!((audio.duration_secs - 2.0).abs() < 1e-9);
    }
}
