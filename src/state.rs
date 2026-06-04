use leptos::prelude::*;
use reactive_stores::Store;
use crate::audio::source::ChannelView;
use crate::canvas::spectrogram_renderer::Colormap;
use crate::canvas::flow::FlowAlgo;
use crate::annotations::AnnotationKind;
use crate::types::{AudioData, PreviewImage, SpectrogramData};
use crate::annotations::{AnnotationId, AnnotationStore, FileIdentity};

/// Hash data extracted from an XC sidecar JSON file.
///
/// Canonical definition lives in the dependency-light `oversample-ipc` crate
/// (shared with `xc-lib` and the Tauri backend); re-exported here so existing
/// `crate::state::SidecarHashes` references keep working.
pub use oversample_ipc::SidecarHashes;

/// Overall verification result against reference hashes (XC sidecar or .batm).
#[derive(Clone, Debug, Default, PartialEq)]
pub enum VerifyOutcome {
    #[default]
    /// No verification attempted yet (hashes still computing or no reference available).
    Pending,
    /// Primary hash matched reference.
    Match,
    /// Primary hash failed, but content_hash matched (header-only change).
    ContentMatch,
    /// All verification failed.
    Mismatch,
}

/// Per-file settings that persist when switching between files.
/// Files in the same sequence group share settings.
#[derive(Clone, Debug)]
pub struct FileSettings {
    pub gain_mode: GainMode,
    pub gain_db: f64,
    /// Stashed gain for the other HFR state (swapped on HFR toggle).
    pub gain_db_stash: f64,
    pub notch_enabled: bool,
    pub notch_bands: Vec<crate::dsp::notch::NoiseBand>,
    pub notch_profile_name: String,
    pub notch_harmonic_suppression: f64,
    pub noise_reduce_enabled: bool,
    pub noise_reduce_strength: f64,
    pub noise_reduce_floor: Option<crate::dsp::spectral_sub::NoiseFloor>,
}

impl Default for FileSettings {
    fn default() -> Self {
        Self {
            gain_mode: GainMode::Off,
            gain_db: 0.0,
            gain_db_stash: 0.0,
            notch_enabled: false,
            notch_bands: Vec::new(),
            notch_profile_name: String::new(),
            notch_harmonic_suppression: 0.0,
            noise_reduce_enabled: false,
            noise_reduce_strength: 0.6,
            noise_reduce_floor: None,
        }
    }
}

thread_local! {
    /// Monotonic source of stable per-file ids. Starts at 1 so 0 can be a
    /// sentinel if ever needed. Never reused, so a removed file's id can't
    /// collide with a later one (WASM is single-threaded, so a plain Cell
    /// is sufficient).
    static NEXT_FILE_ID: std::cell::Cell<u64> = const { std::cell::Cell::new(1) };
}

/// Mint a fresh, process-unique stable id for a `LoadedFile`. Used as the
/// annotation-store key so annotations track their file across list
/// reordering and removal. Call once per `LoadedFile` at construction.
pub fn next_file_id() -> u64 {
    NEXT_FILE_ID.with(|c| {
        let id = c.get();
        c.set(id.wrapping_add(1));
        id
    })
}

#[derive(Clone, Debug)]
pub struct LoadedFile {
    /// Stable, process-unique id (minted via `next_file_id()` at
    /// construction). Used as the annotation-store key so annotations stay
    /// bound to this exact file regardless of its position in `files`.
    pub id: u64,
    pub name: String,
    pub audio: AudioData,
    pub spectrogram: SpectrogramData,
    pub preview: Option<PreviewImage>,
    /// Higher-resolution overview image computed after full spectrogram is ready.
    /// Falls back to `preview` when not yet available.
    pub overview_image: Option<PreviewImage>,
    pub xc_metadata: Option<Vec<(String, String)>>,
    /// Hash data from XC sidecar (for verification against computed identity).
    pub xc_hashes: Option<SidecarHashes>,
    /// Loaded from the bat-demo-sounds archive (not directly from XC or user's disk).
    pub is_demo: bool,
    pub is_recording: bool,  // true = unsaved recording (show indicator on web)
    /// Transient listening file — auto-removed when listening stops, converted to
    /// a recording file when the user hits record.
    pub is_live_listen: bool,
    /// Per-file gain and noise filter settings.
    pub settings: FileSettings,
    /// Insertion order (index at time of push).
    pub add_order: usize,
    /// File.lastModified timestamp from the File API (ms since epoch), if available.
    pub last_modified_ms: Option<f64>,
    /// Multi-layered file identity for annotation matching. Computed progressively after load.
    pub identity: Option<FileIdentity>,
    /// File handle for on-demand range reading (Layer 3/4 hash computation).
    pub file_handle: Option<crate::audio::streaming_source::FileHandle>,
    /// Cached peak level (dBFS) of first 30s. None = not yet computed (e.g. streaming still loading).
    pub cached_peak_db: Option<f64>,
    /// Cached peak level (dBFS) of entire file. None = not yet computed.
    pub cached_full_peak_db: Option<f64>,
    /// Read-only mode: annotations are ephemeral, no auto-save to central store or sidecar.
    pub read_only: bool,
    /// A file-adjacent .batm sidecar existed when this file was loaded.
    /// When true, auto-save updates the sidecar alongside the central store.
    pub had_sidecar: bool,
    /// Overall verification result against reference hashes.
    pub verify_outcome: VerifyOutcome,
    /// True after user clicks "Calculate all hashes" — enables indicators on all hash rows.
    pub all_hashes_verified: bool,
    /// WAV cue-point markers parsed from the file (read-only display).
    pub wav_markers: Vec<crate::types::WavMarker>,
    /// Loading entry ID while this file is still loading (for inline progress display).
    /// Set when the file is pushed to `files`, cleared by `loading_done`.
    pub loading_id: Option<u64>,
    /// Per-file vertical zoom: lower frequency bound in Hz. `None` = default (0 Hz).
    /// Persisted so switching between files restores each file's view.
    pub min_display_freq: Option<f64>,
    /// Per-file vertical zoom: upper frequency bound in Hz. `None` = default (Nyquist).
    pub max_display_freq: Option<f64>,
}

impl LoadedFile {
    /// Get the recording start time as milliseconds since Unix epoch, if available.
    ///
    /// Sources (in priority order):
    /// 1. GUANO "Timestamp" field (ISO 8601) — actual recording start
    /// 2. `last_modified_ms` from the File API — file modification time as fallback,
    ///    adjusted backwards by the file duration to approximate recording start
    pub fn recording_start_epoch_ms(&self) -> Option<f64> {
        self.recording_start_info().map(|(ms, _)| ms)
    }

    /// Get the recording start time and its source description.
    ///
    /// Returns `(epoch_ms, source_label)` where `source_label` describes the
    /// origin: "GUANO Timestamp" or "File modified date (approx.)".
    pub fn recording_start_info(&self) -> Option<(f64, &'static str)> {
        // Try GUANO Timestamp first
        if let Some(ref guano) = self.audio.metadata.guano {
            if let Some((_, ts)) = guano.fields.iter().find(|(k, _)| k == "Timestamp") {
                if let Some(epoch) = parse_iso8601_to_epoch_ms(ts) {
                    return Some((epoch, "GUANO Timestamp"));
                }
            }
        }
        // Fallback: file last-modified minus duration ≈ recording start
        self.last_modified_ms
            .map(|lm| (lm - self.audio.duration_secs * 1000.0, "File modified date (approx.)"))
    }
}

/// Parse a subset of ISO 8601 timestamps to epoch milliseconds.
/// Handles formats like "2023-07-15T22:30:45", "2023-07-15T22:30:45Z",
/// "2023-07-15T22:30:45.123+02:00", "2023-07-15T22:30:45-05:00".
fn parse_iso8601_to_epoch_ms(s: &str) -> Option<f64> {
    // Use js_sys::Date.parse() which handles ISO 8601 natively
    let ms = js_sys::Date::parse(s);
    if ms.is_nan() { None } else { Some(ms) }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Selection {
    pub time_start: f64,
    pub time_end: f64,
    /// None means no frequency constraint (time-only selection).
    pub freq_low: Option<f64>,
    /// None means no frequency constraint (time-only selection).
    pub freq_high: Option<f64>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlaybackMode {
    Normal,
    Heterodyne,
    TimeExpansion,
    PitchShift,
    PhaseVocoder,
    ZeroCrossing,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ExportFormat {
    #[default]
    Wav,
    Mp4,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum VideoResolution {
    #[default]
    Hd720,
    Hd1080,
    MatchCanvas,
}

impl VideoResolution {
    pub fn dimensions(self, canvas_w: u32, canvas_h: u32) -> (u32, u32) {
        match self {
            Self::Hd720 => (1280, 720),
            Self::Hd1080 => (1920, 1080),
            Self::MatchCanvas => (canvas_w, canvas_h),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Hd720 => "720p",
            Self::Hd1080 => "1080p",
            Self::MatchCanvas => "Match canvas",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum VideoCodec {
    #[default]
    H264,
    Av1,
}

impl VideoCodec {
    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264",
            Self::Av1 => "AV1",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum AudioCodecOption {
    /// Automatically pick the best available codec (AAC preferred, then Opus).
    #[default]
    Auto,
    /// Force AAC audio.
    Aac,
    /// Force Opus audio.
    Opus,
    /// No audio track in the exported video.
    NoAudio,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum VideoViewMode {
    /// Keep the spectrogram static, only move the playhead line.
    #[default]
    StaticPlayhead,
    /// Scroll the spectrogram so the playhead stays near the left quarter.
    ScrollingView,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum SpectrogramDisplay {
    #[default]
    FlowOptical,
    PhaseCoherence,
    FlowCentroid,
    FlowGradient,
    Phase,
}

impl SpectrogramDisplay {
    pub fn flow_algo(self) -> FlowAlgo {
        match self {
            Self::FlowOptical => FlowAlgo::Optical,
            Self::PhaseCoherence => FlowAlgo::PhaseCoherence,
            Self::FlowCentroid => FlowAlgo::Centroid,
            Self::FlowGradient => FlowAlgo::Gradient,
            Self::Phase => FlowAlgo::Phase,
        }
    }
}

// FlowColorScheme is defined in oversample-core and re-exported here for backward compatibility.
pub use oversample_core::types::FlowColorScheme;

// Resonator layout lives in the DSP crate — it needs to be the same type used
// by compute_resonator_columns. Re-exported so UI code can reference it via
// `crate::state::ResonatorLayout`.
pub use oversample_core::dsp::resonators::ResonatorLayout;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum RightSidebarTab {
    #[default]
    Metadata,
    Selection,
    Psd,
    Analysis,
    Harmonics,
    Pulses,
    DebugLog,
}

impl RightSidebarTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Metadata => "Info",
            Self::Selection => "Annotations",
            Self::Psd => "Power spectrum",
            Self::Analysis => "Analysis",
            Self::Harmonics => "Harmonics (beta)",
            Self::Pulses => "Pulses",
            Self::DebugLog => "Debug Log",
        }
    }

    pub const ALL: &'static [RightSidebarTab] = &[
        Self::Metadata,
        Self::Selection,
        Self::Psd,
        Self::Analysis,
        Self::Harmonics,
        Self::Pulses,
        Self::DebugLog,
    ];
}

/// Snap policy for the Output Range gutter.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum OutputSnap {
    /// Continuous — factor at 0.1 resolution, carrier at 100 Hz.
    Free,
    /// Powers of 2 + 10 for divide; multiples of 5 kHz for carrier.
    #[default]
    Standard,
    /// Powers of 2 only — preserves pitch-class / musical intervals.
    EqualChroma,
}

/// How the Info / Metadata panel renders values.
///
/// - `Formatted` pretty-prints JSON blobs, localizes dates (with a
///   humanized "X ago" hint), and shows temperatures with their °F
///   conversion in brackets.
/// - `Original` shows the raw value exactly as it appeared in the file.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MetadataView {
    #[default]
    Formatted,
    Original,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum FilterQuality {
    #[default]
    Fast,
    Spectral,
}

// ── New enums ────────────────────────────────────────────────────────────────

/// Bandpass filter mode: Auto (from BandFF), Off, or On (manual).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum BandpassMode {
    #[default]
    Auto,
    Off,
    On,
}

/// Whether the bandpass frequency range follows the Focus or is locked
/// to an independent range that doesn't track the focus.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum BandpassRange {
    #[default]
    FollowFocus,
    Locked,
}

/// Which spectrogram overlay handle is being dragged / hovered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpectrogramHandle {
    BandFfUpper,       // BandFF upper boundary
    BandFfLower,       // BandFF lower boundary
    BandFfMiddle,      // BandFF midpoint (transpose whole range)
    HetCenter,     // HET center freq
    HetBandUpper,  // HET upper band edge
    HetBandLower,  // HET lower band edge
}

/// How the Play button initiates playback.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum PlayStartMode {
    #[default]
    Auto,      // automatically choose: Selected > FromHere > All
    All,       // play from start of file
    FromHere,  // play from current scroll position
    Selected,  // play selection (falls back to All if no selection)
}

impl PlayStartMode {
    /// Whether this mode uses "from-here" scrolling (negative scroll allowed, "here" marker shown).
    pub fn uses_from_here(&self) -> bool {
        matches!(self, PlayStartMode::FromHere | PlayStartMode::Auto)
    }
}

/// What happens when the Record button is pressed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RecordMode {
    ToFile,      // save to filesystem (Tauri only)
    ToMemory,    // keep in browser memory
    ListenOnly,  // grey out record, user can only listen
}

/// Waveform sub-view mode.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum WaveformView {
    /// Plain waveform (green).
    #[default]
    Simple,
    /// Full waveform behind + selected frequency band in blue overlay.
    Frequency,
    /// Three stacked channels: above, selected, below frequency bands.
    Triple,
}

impl WaveformView {
    pub const ALL: [WaveformView; 3] = [
        WaveformView::Simple,
        WaveformView::Frequency,
        WaveformView::Triple,
    ];

    pub fn label(self) -> &'static str {
        match self {
            WaveformView::Simple => "Simple",
            WaveformView::Frequency => "Band wave",
            WaveformView::Triple => "Triple",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            WaveformView::Simple => "Simple",
            WaveformView::Frequency => "Band",
            WaveformView::Triple => "Triple",
        }
    }
}

/// Active interaction tool for the main spectrogram canvas.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum CanvasTool {
    #[default]
    Hand,      // drag to pan
    Selection, // drag to select
}

/// Which entity type currently has interactive focus.
/// Controls handle visibility, overflow menu display, and drag gating.
/// Only one entity can be focused at a time, but all entities persist
/// regardless of focus state.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ActiveFocus {
    /// The transient drag selection (blue rectangle) is focused.
    TransientSelection,
    /// One or more annotations are focused (gold rectangles get handles).
    Annotations,
    /// The Frequency Focus overlay is focused (amber lines get drag handles).
    FrequencyFocus,
}

/// Position of a resize handle on an annotation bounding box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResizeHandlePosition {
    TopLeft, Top, TopRight,
    Left, Right,
    BottomLeft, Bottom, BottomRight,
}

/// What the overview strip shows.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum OverviewView {
    Spectrogram,
    #[default]
    Waveform,
}

/// What the main panel displays.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MainView {
    #[default]
    Spectrogram,
    XformedSpec,
    Waveform,
    ZcChart,
    Flow,
    Chromagram,
    Resonators,
}

impl MainView {
    pub fn label(self) -> &'static str {
        match self {
            Self::Spectrogram => "Spectrogram",
            Self::XformedSpec => "Transformed Spec",
            Self::Waveform => "Waveform",
            Self::ZcChart => "ZC Chart",
            Self::Flow => "Flow",
            Self::Chromagram => "Chromagram",
            Self::Resonators => "Resonators",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Spectrogram => "Spec",
            Self::XformedSpec => "Xform S",
            Self::Waveform => "Wave",
            Self::ZcChart => "ZC",
            Self::Flow => "Flow",
            Self::Chromagram => "Chroma",
            Self::Resonators => "Reson",
        }
    }

    /// Whether this view mode uses the spectrogram renderer.
    pub fn is_spectrogram(self) -> bool {
        matches!(self, Self::Spectrogram | Self::XformedSpec | Self::Flow | Self::Chromagram | Self::Resonators)
    }

    /// Views that make sense for an Anabat .zc file. The recording has no
    /// continuous waveform — `audio.samples` is a synthesised reconstruction
    /// from the dot frequencies, so anything that does heavy DSP on the
    /// samples (transformed spec, flow, chromagram, resonators) just
    /// measures the synth and would mislead the user.
    pub fn is_sensible_for_zc(self) -> bool {
        matches!(self, Self::ZcChart | Self::Spectrogram | Self::Waveform)
    }

    pub const ALL: &'static [MainView] = &[
        Self::Spectrogram,
        Self::XformedSpec,
        Self::Waveform,
        Self::ZcChart,
        Self::Flow,
        Self::Chromagram,
        Self::Resonators,
    ];
}

// ── FFT mode ─────────────────────────────────────────────────────────────────

/// FFT window mode for spectrogram computation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FftMode {
    /// Fixed FFT size at all LOD levels (128–8192).
    Single(usize),
    /// Adaptive XS: [1024, 1024, 512, 256, 128, 64, 32, 16]
    /// Halves FFT at each LOD past baseline for maximum temporal detail.
    AdaptiveXS,
    /// Adaptive S: [1024, 1024, 512, 512, 256, 128, 64, 32]
    AdaptiveS,
    /// Adaptive M: [1024, 1024, 1024, 512, 512, 256, 128, 64]
    AdaptiveM,
    /// Adaptive L: [2048, 2048, 2048, 1024, 512, 512, 256, 128]
    AdaptiveL,
}

impl FftMode {
    /// Per-LOD FFT sizes for each adaptive mode. Index = LOD level (0–7).
    /// XS halves at every step past baseline (LOD 2) — finest time, coarsest freq.
    const ADAPTIVE_XS: [usize; 8] = [1024, 1024, 512, 256, 128, 64, 32, 16];
    const ADAPTIVE_S: [usize; 8] = [1024, 1024, 512, 512, 256, 128, 64, 32];
    const ADAPTIVE_M: [usize; 8] = [1024, 1024, 1024, 512, 512, 256, 128, 64];
    const ADAPTIVE_L: [usize; 8] = [2048, 2048, 2048, 1024, 512, 512, 256, 128];

    /// The actual FFT size to use for a given LOD level (0–7).
    pub fn fft_for_lod(&self, lod: u8) -> usize {
        let idx = (lod as usize).min(7);
        match self {
            FftMode::Single(sz) => *sz,
            FftMode::AdaptiveXS => Self::ADAPTIVE_XS[idx],
            FftMode::AdaptiveS => Self::ADAPTIVE_S[idx],
            FftMode::AdaptiveM => Self::ADAPTIVE_M[idx],
            FftMode::AdaptiveL => Self::ADAPTIVE_L[idx],
        }
    }

    /// The maximum FFT size this mode will ever produce (across all LODs).
    /// Determines the output tile height: `max_fft() / 2 + 1` bins.
    pub fn max_fft_size(&self) -> usize {
        match self {
            FftMode::Single(sz) => *sz,
            FftMode::AdaptiveXS => 1024,
            FftMode::AdaptiveS => 1024,
            FftMode::AdaptiveM => 1024,
            FftMode::AdaptiveL => 2048,
        }
    }
}

// ── Resonator bandwidth slider mapping ───────────────────────────────────────

/// Minimum selectable resonator bandwidth (Hz).
pub const RESONATOR_BW_MIN: f32 = 5.0;
/// Maximum selectable resonator bandwidth (Hz).
pub const RESONATOR_BW_MAX: f32 = 100.0;
/// Slider position range (0..=RESONATOR_BW_SLIDER_MAX) mapped log-scale to
/// [RESONATOR_BW_MIN, RESONATOR_BW_MAX]. The log mapping gives a gentle bias
/// toward the low end, where differences between 5 and 20 Hz matter more than
/// differences between 80 and 100 Hz.
pub const RESONATOR_BW_SLIDER_MAX: f32 = 1000.0;

/// Convert a bandwidth in Hz to a slider position 0..RESONATOR_BW_SLIDER_MAX.
pub fn resonator_bw_to_slider(bw: f32) -> f32 {
    let bw = bw.clamp(RESONATOR_BW_MIN, RESONATOR_BW_MAX);
    (bw / RESONATOR_BW_MIN).ln() / (RESONATOR_BW_MAX / RESONATOR_BW_MIN).ln()
        * RESONATOR_BW_SLIDER_MAX
}

/// Convert a slider position 0..RESONATOR_BW_SLIDER_MAX back to a bandwidth in Hz.
pub fn resonator_slider_to_bw(pos: f32) -> f32 {
    let pos = pos.clamp(0.0, RESONATOR_BW_SLIDER_MAX);
    RESONATOR_BW_MIN
        * (RESONATOR_BW_MAX / RESONATOR_BW_MIN).powf(pos / RESONATOR_BW_SLIDER_MAX)
}

// ── Resonator FFT mode ───────────────────────────────────────────────────────

/// Frequency-bin count mode for the Resonators view.
///
/// Resonator compute cost scales as `O(hop × num_bins)` per column, so coarse
/// LODs (huge hops) need fewer bins to stay tractable. Unlike STFT's Adaptive
/// pattern (which gives coarse LODs *larger* FFTs for more frequency detail),
/// Adaptive here goes the other way: coarse LODs get fewer bins, fine LODs
/// get the full count.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ResonatorFftMode {
    /// Fixed equivalent-FFT size at all LOD levels (num_bins = size/2 + 1).
    Single(usize),
    /// Adaptive: fewer bins at coarse LODs for cheaper tiles, full bins when
    /// detail matters.
    Adaptive,
}

impl ResonatorFftMode {
    /// Per-LOD equivalent FFT size for Adaptive mode. Index = LOD level (0–7).
    ///
    /// Resonator cost ≈ `hop × num_bins`, so bins are scaled inversely to hop
    /// size to keep per-tile compute roughly constant across LODs. Baseline
    /// (LOD 2, hop=512) holds 1024 eq. FFT (513 bins); coarser LODs shrink
    /// 4× per step to match, finer LODs keep the 1024 ceiling since further
    /// bins wouldn't add visible detail past the display's vertical resolution.
    const ADAPTIVE_FFT: [usize; 8] = [64, 256, 1024, 1024, 1024, 1024, 1024, 1024];

    /// The equivalent FFT size to use for a given LOD level (0–7).
    pub fn fft_for_lod(&self, lod: u8) -> usize {
        let idx = (lod as usize).min(7);
        match self {
            Self::Single(sz) => *sz,
            Self::Adaptive => Self::ADAPTIVE_FFT[idx],
        }
    }

    /// The maximum equivalent FFT size this mode will ever produce.
    pub fn max_fft_size(&self) -> usize {
        match self {
            Self::Single(sz) => *sz,
            Self::Adaptive => *Self::ADAPTIVE_FFT.iter().max().unwrap(),
        }
    }
}

/// Display filter mode: controls how each processing stage affects the spectrogram.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum DisplayFilterMode {
    /// Stage is disabled for display.
    #[default]
    Off,
    /// Automatic/smart defaults for display.
    Auto,
    /// Use same settings as playback.
    Same,
    /// Custom display-only settings (NR strength, Gain brightness).
    Custom,
}

impl DisplayFilterMode {
    pub const ALL: [DisplayFilterMode; 4] = [
        DisplayFilterMode::Off,
        DisplayFilterMode::Auto,
        DisplayFilterMode::Same,
        DisplayFilterMode::Custom,
    ];

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Auto => "aut",
            Self::Same => "sam",
            Self::Custom => "cst",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Auto => "Auto",
            Self::Same => "Same",
            Self::Custom => "Custom",
        }
    }
}

/// Auto-gain strategy.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum GainMode {
    /// No gain adjustment at all.
    #[default]
    Off,
    /// Manual dB boost only (from slider).
    Manual,
    /// Peak normalization: scan first N seconds, boost so peak ≈ −3 dBFS.
    /// Manual slider adds on top.
    AutoPeak,
    /// AGC (Automatic Gain Control): smooth per-sample leveler that targets
    /// −3 dBFS with attack/release envelope following, noise gate, and limiter.
    /// Manual slider adds on top.
    Adaptive,
}

impl GainMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Manual => "Manual",
            Self::AutoPeak => "Peak",
            Self::Adaptive => "AGC",
        }
    }

    pub fn is_auto(self) -> bool {
        matches!(self, Self::AutoPeak | Self::Adaptive)
    }
}

/// Where the peak is measured for AutoPeak gain mode.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum PeakSource {
    /// Raw audio, first 30 seconds (or full file if shorter).
    #[default]
    First30s,
    /// Raw audio, entire file.
    FullWave,
    /// Raw audio, current selection range only.
    Selection,
    /// Post-DSP peak (after bandpass/HFR/NR chain), computed on demand.
    Processed,
}

impl PeakSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::First30s => "30s",
            Self::FullWave => "Full",
            Self::Selection => "Sel",
            Self::Processed => "DSP",
        }
    }
}

/// Which floating layer panel is currently open (only one at a time).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayerPanel {
    HfrMode,
    Tool,
    FreqRange,
    MainView,
    PlayMode,
    RecordMode,
    Channel,
    Gain,
    ListenMode,
    /// Band presets dropdown in the Hearing bar.
    BandPresets,
    /// Notch combo dropdown in the Hearing bar.
    Notch,
    /// Noise-reduction (spectral subtraction) combo in the Hearing bar.
    NoiseReduce,
    /// Bandpass+EQ combo dropdown in the Hearing bar.
    Bandpass,
    /// Mic settings dropdown in the Transport bar (strategy + device + capture format).
    Mic,
    /// Output range dropdown in the Hearing bar — visualises and edits how
    /// the active playback mode maps input frequencies into the 0–2000 Hz
    /// target listening range.
    OutputRange,
}

impl LayerPanel {
    /// Which bar this panel belongs to. Each bar uses this to decide
    /// whether to lift its own z-index above sibling bars — set the
    /// `.panel-open` class only for the bar that actually owns the open
    /// panel, otherwise an unrelated bar lifts itself and (being later
    /// in DOM) hides the real popup.
    pub fn bar(self) -> Bar {
        match self {
            LayerPanel::HfrMode
            | LayerPanel::BandPresets
            | LayerPanel::Bandpass
            | LayerPanel::Notch
            | LayerPanel::NoiseReduce
            | LayerPanel::Gain
            | LayerPanel::ListenMode
            | LayerPanel::OutputRange => Bar::Hearing,
            LayerPanel::MainView | LayerPanel::Tool => Bar::View,
            LayerPanel::PlayMode | LayerPanel::RecordMode | LayerPanel::Channel | LayerPanel::Mic => Bar::Transport,
            // FreqRange floats over the canvas — not anchored to a bar.
            LayerPanel::FreqRange => Bar::Floating,
        }
    }
}

/// Which toolbar (if any) a `LayerPanel` is anchored to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bar {
    Hearing,
    View,
    Transport,
    /// Not anchored to a bar — opened from a floating overlay button.
    Floating,
}

/// A navigation history entry (for overview back/forward buttons).
#[derive(Clone, Copy, Debug)]
pub struct NavEntry {
    pub scroll_offset: f64,
    pub zoom_level: f64,
}

/// A snapshot of a file's annotation set for undo/redo.
#[derive(Clone, Debug)]
pub struct UndoEntry {
    /// Stable id (`LoadedFile.id`) of the file this snapshot belongs to, so
    /// undo/redo restores onto the correct file even after the list changes.
    pub file_id: u64,
    pub snapshot: Option<crate::annotations::AnnotationSet>,
}

/// Undo/redo stack for annotation operations.
#[derive(Clone, Debug, Default)]
pub struct UndoStack {
    pub undo: Vec<UndoEntry>,
    pub redo: Vec<UndoEntry>,
}

impl UndoStack {
    const MAX_SIZE: usize = 100;

    pub fn push_undo(&mut self, entry: UndoEntry) {
        self.undo.push(entry);
        if self.undo.len() > Self::MAX_SIZE {
            self.undo.remove(0);
        }
        self.redo.clear();
    }
}

/// A time-position bookmark created during or after playback.
#[derive(Clone, Copy, Debug)]
pub struct Bookmark {
    pub time: f64,
}


#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ChromaColormap {
    Warm,
    #[default]
    PitchClass,
    Solid,
    Octave,
    Flow,
}

impl ChromaColormap {
    pub fn label(self) -> &'static str {
        match self {
            Self::PitchClass => "Pitch Class",
            Self::Warm => "Warm",
            Self::Solid => "Solid",
            Self::Octave => "Octave",
            Self::Flow => "Flow",
        }
    }

    pub const ALL: &'static [ChromaColormap] = &[
        Self::PitchClass,
        Self::Warm,
        Self::Solid,
        Self::Octave,
        Self::Flow,
    ];
}

/// Frequency range preset for the chromagram view.
///
/// Each preset defines which octaves to display. Octave numbering follows
/// scientific pitch notation extended upward: octave 0 starts at C0 (16.35 Hz),
/// octave 10 at C10 (~16.7 kHz), octave 13 at C13 (~134 kHz), etc.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ChromaRange {
    /// All octaves from C0 to the highest representable.
    #[default]
    Full,
    /// C0–B10 (~16 Hz – 31.6 kHz) — human hearing range.
    Audible,
    /// C0–B8 (~16 Hz – 8.4 kHz) — A0 to ~D8 musical range.
    Musical,
    /// C10–B15 (~16.7 kHz – max) — ultrasound only.
    Ultrasound,
}

impl ChromaRange {
    /// (min_octave, num_octaves) — which octave indices to include.
    pub fn octave_params(self) -> (usize, usize) {
        match self {
            Self::Full       => (0, 16),   // oct 0–15
            Self::Audible    => (0, 11),   // oct 0–10 (~16 Hz – 31.6 kHz)
            Self::Musical    => (0, 9),    // oct 0–8  (~16 Hz – 8.4 kHz)
            Self::Ultrasound => (10, 6),   // oct 10–15 (~16.7 kHz – max)
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Full       => "Full",
            Self::Audible    => "Audible (20\u{2013}20k)",
            Self::Musical    => "Musical (A0\u{2013}D8)",
            Self::Ultrasound => "Ultrasound (18k+)",
        }
    }

    pub const ALL: &'static [ChromaRange] = &[
        Self::Full,
        Self::Audible,
        Self::Musical,
        Self::Ultrasound,
    ];
}

/// Backend used to compute chromagram columns.
///
/// `Resonators` (default) uses a constant-Q resonator bank with one resonator
/// per note — uniform pitch selectivity from sub-bass to ultrasound. `Fft`
/// re-bins linear STFT magnitudes into notes, which is fast and shares cached
/// STFT columns with the spectrogram but coarsens at low frequencies (one bin
/// can span several semitones) and blurs at high ones.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ChromaSource {
    #[default]
    Resonators,
    Fft,
}

impl ChromaSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Resonators => "Resonators",
            Self::Fft => "FFT",
        }
    }

    pub const ALL: &'static [ChromaSource] = &[Self::Resonators, Self::Fft];
}

/// Style for frequency shield/flag color bars on the spectrogram edge.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ShieldStyle {
    /// Single solid color per band based on the changing digit.
    #[default]
    Solid,
    /// Three-band resistor color encoding (heraldic bend shield).
    Resistor,
    /// No shields — frequency color bars are hidden.
    Off,
}

impl ShieldStyle {
    pub const ALL: [ShieldStyle; 3] = [Self::Solid, Self::Resistor, Self::Off];

    pub fn label(self) -> &'static str {
        match self {
            Self::Solid => "Solid",
            Self::Resistor => "Resistor bands",
            Self::Off => "Off",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Solid => "solid",
            Self::Resistor => "resistor",
            Self::Off => "off",
        }
    }

    pub fn from_key(s: &str) -> Self {
        match s {
            "resistor" => Self::Resistor,
            "off" => Self::Off,
            _ => Self::Solid,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum FileSortMode {
    #[default]
    GroupedAdded,
    AddOrder,
    ByName,
    ByDate,
    ByMetadataDate,
    Grouped,
    ByDateGrouped,
}

impl FileSortMode {
    pub const ALL: &[FileSortMode] = &[
        Self::GroupedAdded,
        Self::AddOrder,
        Self::ByName,
        Self::ByDate,
        Self::ByMetadataDate,
        Self::Grouped,
        Self::ByDateGrouped,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::GroupedAdded => "Grouped + added",
            Self::AddOrder => "Added",
            Self::ByName => "Name",
            Self::ByDate => "Date",
            Self::ByMetadataDate => "Meta date",
            Self::Grouped => "Grouped",
            Self::ByDateGrouped => "Date, grouped",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum LeftSidebarTab {
    #[default]
    Files,
    Project,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum StatusLevel {
    #[default]
    Error,
    Info,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MicMode {
    Auto,
    Browser,
    Cpal,
    RawUsb,
}

/// Microphone acquisition strategy.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MicStrategy {
    Ask,
    Selected,
    Browser,
    None,
}

/// Which backend is handling mic audio.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MicBackend {
    Browser,
    Cpal,
    RawUsb,
}

/// State of mic acquisition lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MicAcquisitionState {
    #[default]
    Idle,
    AwaitingChoice,
    Acquiring,
    Ready,
    Failed,
}

/// Pending mic action (what to do once mic is acquired).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MicPendingAction {
    Listen,
    Record,
    /// Open the mic and create an empty live document, but don't start
    /// streaming yet. The user adjusts HFR settings before pressing Listen
    /// or Record on the armed doc.
    Arm,
}

/// Whether a recording is ready to begin.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum RecordReadyState {
    #[default]
    None,
    AwaitingConfirmation,
    Confirmed,
}

/// Mono or stereo channel mode for mic recording.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ChannelMode {
    #[default]
    Mono,
    Stereo,
}

/// Information about the selected mic device.
#[derive(Clone, Debug)]
pub struct MicDeviceInfo {
    pub name: String,
    pub connection_type: String,
    pub supported_rates: Vec<u32>,
    pub supported_bit_depths: Vec<u16>,
    pub max_channels: u16,
}

/// GPS location fix for embedding in recording GUANO metadata.
#[derive(Clone, Debug)]
pub struct GpsLocation {
    pub latitude: f64,
    pub longitude: f64,
    pub elevation: Option<f64>,
    pub accuracy: Option<f64>,
}

// ── Loading progress ─────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum LoadingStage {
    Decoding,
    Preview,
    Spectrogram(u16), // 0–100 %
    Finalizing,
    Streaming,
}

#[derive(Clone, Debug)]
pub struct LoadingEntry {
    pub id: u64,
    pub name: String,
    pub stage: LoadingStage,
}

// ── Reactive-store groups ────────────────────────────────────────────────────
//
// Cohesive slices of former `AppState` signals, grouped into `#[derive(Store)]`
// plain-data structs and held as `Store<…>` fields on `AppState`. Each field is
// still independently reactive (subscribing to `state.flow.gate()` only re-runs
// on `gate` changes, exactly like the old per-`RwSignal` model), but the struct
// is now organized rather than 290 flat signals. Access pattern:
//
//     state.flow.gate().get()           // was state.flow_gate.get()
//     state.flow.enabled().set(true)    // was state.flow_enabled.set(true)
//
// Consumers must bring the generated `*StoreFields` accessor trait into scope;
// the [`store_fields`] prelude re-exports them all for a single glob import.
//
// NB: `Store::new` requires the inner type to be `Send + Sync + 'static`. Groups
// that hold non-`Send` values (JS/web-sys handles, etc.) need a different
// storage; `FlowState` is all plain `Copy` data so the default storage applies.

/// Optical-flow overlay settings (the "Flow" spectrogram view).
#[derive(Clone, Debug, Default, Store)]
pub struct FlowState {
    pub enabled: bool,
    pub intensity_gate: f32,
    pub gate: f32,
    pub opacity: f32,
    pub shift_gain: f32,
    pub color_gamma: f32,
    pub color_scheme: FlowColorScheme,
}

/// Chromagram view settings.
#[derive(Clone, Debug, Store)]
pub struct ChromaState {
    /// Chromagram colormap mode.
    pub colormap: ChromaColormap,
    /// Display gain boost in dB (0 = none, positive = amplify).
    pub gain: f32,
    /// Display gamma curve (1.0 = linear).
    pub gamma: f32,
    /// Frequency range preset.
    pub range: ChromaRange,
    /// Compute backend (resonator bank vs FFT re-binning).
    pub source: ChromaSource,
    /// Local-AGC strength: 0 = global normalisation, 1 = per-column smoothed
    /// local max. Lifts quiet passages so relative note brightness stays visible.
    pub adapt: f32,
    /// Hard dB floor below the (adapt-adjusted) effective max; ratios below map
    /// to black. -80 dB ≈ off; raise toward 0 to sharpen contrast.
    pub floor_db: f32,
}

/// Resonator (per-bin EMA) view settings.
#[derive(Clone, Debug, Store)]
pub struct ResonatorState {
    /// Per-bin EMA bandwidth in Hz (controls time-frequency tradeoff).
    pub bandwidth_hz: f32,
    /// Bin-count mode (fixed or adaptive-per-LOD).
    pub fft_mode: ResonatorFftMode,
    /// Frequency-bin spacing (linear or log).
    pub layout: ResonatorLayout,
    /// Concentrate the bank's range on the visible viewport instead of 0..Nyquist.
    pub viewport_bins: bool,
    /// Freq range (Hz) of tiles currently in the resonator cache; `None` = default.
    pub viewport_range: Option<(f64, f64)>,
}

/// Notch noise filtering.
#[derive(Clone, Debug, Store)]
pub struct NotchState {
    pub enabled: bool,
    pub bands: Vec<crate::dsp::notch::NoiseBand>,
    pub detecting: bool,
    pub profile_name: String,
    pub hovering_band: Option<usize>,
    /// Harmonic suppression strength (0.0–1.0). Attenuates 2x and 3x harmonics.
    pub harmonic_suppression: f64,
}

/// Spectral-subtraction noise reduction.
#[derive(Clone, Debug, Store)]
pub struct NoiseReduceState {
    pub enabled: bool,
    pub strength: f64,
    pub floor: Option<crate::dsp::spectral_sub::NoiseFloor>,
    pub learning: bool,
}

/// Pulse detection overlay.
#[derive(Clone, Debug, Store)]
pub struct PulseState {
    pub detected: Vec<crate::dsp::pulse_detect::DetectedPulse>,
    pub overlay_enabled: bool,
    pub selected_index: Option<usize>,
    pub detecting: bool,
}

/// Power-spectral-density panel settings.
#[derive(Clone, Debug, Store)]
pub struct PsdState {
    pub nfft: usize,
    pub apply_eq: bool,
    pub apply_notch: bool,
    pub apply_nr: bool,
    /// Temporary frequency overlays from PSD hover: (freq_hz, label, color_css).
    pub hover_freqs: Vec<(f64, String, String)>,
}

/// Project (.batproj) state.
#[derive(Clone, Debug, Store)]
pub struct ProjectState {
    /// Whether the Projects beta feature is enabled (persisted to localStorage).
    pub enabled: bool,
    /// Currently loaded .batproj project (None = no project open).
    pub current: Option<crate::project::BatProject>,
    /// Whether the project has unsaved changes.
    pub dirty: bool,
    /// Save status for UI feedback: "", "Saving...", "Saved".
    pub save_status: &'static str,
}

/// Timeline / multi-file selection state.
#[derive(Clone, Debug, Store)]
pub struct TimelineState {
    /// Multi-selected file indices for timeline creation (separate from current_file_index).
    pub selected_file_indices: Vec<usize>,
    /// Active timeline view (when Some, spectrogram/waveform render in timeline mode).
    pub active: Option<crate::timeline::TimelineView>,
    /// Currently selected multitrack track label (None = primary/default).
    pub active_track: Option<String>,
    /// Show wall-clock time instead of file-relative time.
    pub show_clock_time: bool,
}

/// Bat Book reference subsystem state.
#[derive(Clone, Debug, Store)]
pub struct BatBookState {
    pub open: bool,
    /// Auto or Manual(region). Drives `region` via an Effect.
    pub mode: crate::bat_book::types::BatBookMode,
    /// Effective region — set by the auto-resolve Effect or manual selection.
    pub region: crate::bat_book::types::BatBookRegion,
    /// Result of auto-resolution (None when in Manual mode).
    pub auto_resolved: Option<crate::bat_book::types::AutoResolved>,
    /// User's starred/favourite bat book regions.
    pub favourites: Vec<crate::bat_book::types::BatBookRegion>,
    /// Currently selected bat book entry IDs (multi-select via shift-click).
    pub selected_ids: Vec<String>,
    pub ref_open: bool,
    /// Last-clicked bat book entry ID, used for shift-click range selection.
    pub last_clicked_id: Option<String>,
    /// When true, selecting bat book entries pushes their frequency focus override.
    pub auto_focus: bool,
}

/// Export / video-export UI state.
#[derive(Clone, Debug, Store)]
pub struct ExportState {
    /// Whether the export section is expanded/collapsed.
    pub section_open: bool,
    /// Selected export format: WAV or MP4.
    pub format: ExportFormat,
    /// Video export progress (0.0 to 1.0), None = not exporting.
    pub video_progress: Option<f64>,
    /// Video export status message.
    pub video_status: Option<String>,
    /// Set to true to request cancellation of an in-progress video export.
    pub video_cancel: bool,
    pub video_resolution: VideoResolution,
    pub video_codec: VideoCodec,
    /// Selected audio codec for video export.
    pub video_audio_codec: AudioCodecOption,
    /// Video view mode: static playhead vs scrolling.
    pub video_view_mode: VideoViewMode,
}

/// Transform / DSP playback parameters (heterodyne, time-expansion, pitch
/// shift, phase vocoder, zero-crossing) plus their BandFF-derived auto flags.
#[derive(Clone, Debug, Store)]
pub struct TransformState {
    pub het_frequency: f64,
    pub het_cutoff: f64,
    /// Number of heterodyne carriers (1 = classic single-carrier, >1 = comb).
    pub het_comb_count: u32,
    /// Spacing (Hz) between adjacent comb carriers.
    pub het_comb_spacing: f64,
    /// When true, comb count/spacing are derived from BandFF width + LP cutoff.
    pub het_comb_auto: bool,
    pub het_interacting: bool,
    pub het_freq_auto: bool,
    pub het_cutoff_auto: bool,
    pub te_factor: f64,
    pub te_factor_auto: bool,
    pub ps_factor: f64,
    pub ps_factor_auto: bool,
    /// Output-side shift (Hz) applied AFTER pitch shifting in PS/PV modes.
    pub ps_shift_hz: f64,
    pub pv_factor: f64,
    pub pv_factor_auto: bool,
    pub pv_hq: bool,
    pub zc_factor: f64,
}

/// Viewport: zoom, scroll, display frequency bounds, follow-cursor behaviour.
#[derive(Clone, Debug, Store)]
pub struct ViewState {
    pub zoom_level: f64,
    pub scroll_offset: f64,
    pub min_display_freq: Option<f64>,
    pub max_display_freq: Option<f64>,
    pub follow_cursor: bool,
    pub follow_suspended: bool,
    pub follow_visible_since: Option<f64>,
    pub pre_play_scroll: f64,
    pub user_panned_during_playback: bool,
}

/// Bandpass / EQ filter + BandFF gutter settings.
#[derive(Clone, Debug, Store)]
pub struct FilterState {
    pub enabled: bool,
    pub band_mode: u8,
    pub freq_low: f64,
    pub freq_high: f64,
    pub db_below: f64,
    pub db_selected: f64,
    pub db_harmonics: f64,
    pub db_above: f64,
    pub hovering_band: Option<u8>,
    pub quality: FilterQuality,
    pub bandpass_mode: BandpassMode,
    pub bandpass_range: BandpassRange,
    /// BandFF frequency range (0.0 = no BandFF active).
    pub band_ff_freq_lo: f64,
    pub band_ff_freq_hi: f64,
    /// True while the user is live-dragging the band gutter.
    pub band_ff_dragging: bool,
}

/// Gain settings: audio playback/live gain + waveform-view (visual) gain.
#[derive(Clone, Debug, Store)]
pub struct GainState {
    pub db: f64,
    /// Stashed gain for the other HFR state (swapped on HFR toggle).
    pub db_stash: f64,
    /// Manual gain (dB) applied while live listening / recording.
    pub live_db: f64,
    pub auto: bool,
    pub mode: GainMode,
    /// Remembers last auto-gain mode so toggle restores it.
    pub mode_last_auto: GainMode,
    /// Where to measure peak for AutoPeak gain mode.
    pub peak_source: PeakSource,
    /// Cache for recently computed selection peak values.
    pub selection_peak_cache: crate::audio::peak::PeakCache,
    /// Whether a peak scan is currently in progress (for UI indicator).
    pub peak_scanning: bool,
    /// Waveform-view gain (visual only, independent of audio gain).
    pub wave_view_db: f64,
    pub wave_view_auto: bool,
}

/// Single-import prelude for the generated `#[derive(Store)]` accessor traits.
/// Consumers `use crate::state::store_fields::*;` once instead of importing each
/// `FooStateStoreFields` trait individually. Extend as more groups migrate.
pub mod store_fields {
    pub use super::{
        FlowStateStoreFields,
        ChromaStateStoreFields,
        ResonatorStateStoreFields,
        NotchStateStoreFields,
        NoiseReduceStateStoreFields,
        PulseStateStoreFields,
        PsdStateStoreFields,
        ProjectStateStoreFields,
        TimelineStateStoreFields,
        BatBookStateStoreFields,
        ExportStateStoreFields,
        TransformStateStoreFields,
        ViewStateStoreFields,
        FilterStateStoreFields,
        GainStateStoreFields,
        MicStateStoreFields,
        RecordingMetaStateStoreFields,
        SpectStateStoreFields,
        AnnotationsStateStoreFields,
        DisplayStateStoreFields,
        PanelsStateStoreFields,
        DialogsStateStoreFields,
    };
}

/// Microphone capture + recording lifecycle (independent listen + record).
#[derive(Clone, Debug, Store)]
pub struct MicState {
    pub listening: bool,
    pub recording: bool,
    pub sample_rate: u32,
    pub samples_recorded: usize,
    pub bits_per_sample: u16,
    /// 0 = auto (device default).
    pub max_sample_rate: u32,
    /// Maximum seconds of listen buffer to capture on long-press record.
    pub preroll_buffer_secs: u32,
    pub mode: MicMode,
    /// Actual rates from cpal device query.
    pub supported_rates: Vec<u32>,
    /// File index of the currently-recording live file (None if not recording).
    pub live_file_idx: Option<usize>,
    /// Generation counter for the live processing loop (older loops self-exit).
    pub processing_gen: u32,
    /// Number of pre-roll samples captured from the listen buffer on long-press.
    pub preroll_samples: usize,
    /// Wall-clock ms when the long-press gesture started.
    pub gesture_start_ms: Option<f64>,
    /// Wall-clock ms when recording started, for timer display.
    pub recording_start_time: Option<f64>,
    /// Wrapping counter incremented by setInterval(100ms) while recording.
    pub timer_tick: u32,
    /// Current mic device name (populated on open or query).
    pub device_name: Option<String>,
    /// Connection type: "USB", "Internal", "Bluetooth", etc.
    pub connection_type: Option<String>,
    /// USB mic manufacturer name (from USB descriptors), if available.
    pub manufacturer: Option<String>,
    /// Whether a USB audio device is currently connected.
    pub usb_connected: bool,
    /// What Auto mode resolved to (Cpal or RawUsb).
    pub effective_mode: MicMode,
    /// Target scroll offset during recording (rAF interpolates toward it).
    pub recording_target_scroll: f64,
    /// Epoch ms until which the waterfall smooth-scroll leaves scroll alone.
    pub scroll_user_pan_until: f64,
    /// Rightmost spectrogram column with actual data during recording.
    pub live_data_cols: usize,
    /// User's preferred device name for mic input. None = system default.
    pub selected_device: Option<String>,
    /// Whether the mic chooser modal dialog is visible.
    pub show_chooser: bool,
    /// User dismissed the "Mic detected" chip for this session.
    pub chip_dismissed: bool,
    /// Peak audio level from mic (0.0..1.0).
    pub peak_level: f32,
    /// Mic acquisition strategy (Ask, Selected, Browser, None).
    pub strategy: MicStrategy,
    /// Which backend is handling mic audio.
    pub backend: Option<MicBackend>,
    /// State of mic acquisition lifecycle.
    pub acquisition_state: MicAcquisitionState,
    /// Pending mic action (Listen or Record).
    pub pending_action: Option<MicPendingAction>,
    /// Whether a recording is ready to begin.
    pub record_ready_state: RecordReadyState,
    /// Debounce: true from Record press until running/abandoned.
    pub starting_recording: bool,
    /// Whether the mic permission dialog has been shown.
    pub permission_dialog_shown: bool,
    /// Maximum bit depth for mic recording (0 = auto).
    pub max_bit_depth: u16,
    /// Mono or stereo channel mode for mic recording.
    pub channel_mode: ChannelMode,
    /// Information about the selected mic device.
    pub device_info: Option<MicDeviceInfo>,
    /// Context window size in samples for PS/PV overlap-save buffering.
    pub listen_context_samples: usize,
    /// When true, mic input is processed but the speakers stay silent.
    pub mute_output: bool,
}

/// Recording metadata / privacy: GPS, device id, home-wifi suppression.
#[derive(Clone, Debug, Store)]
pub struct RecordingMetaState {
    /// Whether GPS location embedding is enabled (persisted).
    pub gps_enabled: bool,
    /// GPS location acquired at recording start (cleared after finalization).
    pub location: Option<GpsLocation>,
    /// WiFi SSIDs where location embedding is suppressed (persisted).
    pub home_wifi_ssids: Vec<String>,
    /// Whether to include phone model in recording metadata (persisted).
    pub device_model_enabled: bool,
    /// Cached device manufacturer (Android only).
    pub cached_make: Option<String>,
    /// Cached device model (Android only).
    pub cached_model: Option<String>,
}

/// Spectrogram display + colormap settings (applied at render time).
#[derive(Clone, Debug, Store)]
pub struct SpectState {
    /// Spectrogram display mode (linear / optical-flow / etc).
    pub display: SpectrogramDisplay,
    /// dB floor. Values below this map to black.
    pub floor_db: f32,
    /// dB range. floor + range = ceiling.
    pub range_db: f32,
    /// Gamma curve (1.0 = linear).
    pub gamma: f32,
    /// Additive dB gain offset.
    pub gain_db: f32,
    /// Show tile debug overlay (borders, LOD labels).
    pub debug_tiles: bool,
    /// FFT window mode (single size or multi-resolution).
    pub fft_mode: FftMode,
    /// Enable reassignment spectrogram (sharper time-frequency localization).
    pub reassign_enabled: bool,
    /// Colormap preference (when not overridden by HFR/flow).
    pub colormap_preference: Colormap,
    /// Colormap preference used when HFR mode is active.
    pub hfr_colormap_preference: Colormap,
}

/// Annotation subsystem: store, selection, drag/drop, resize, undo, editing.
#[derive(Clone, Debug, Store)]
pub struct AnnotationsState {
    pub store: AnnotationStore,
    pub dirty: bool,
    pub selected_ids: Vec<AnnotationId>,
    /// Anchor for shift-click range selection in the annotation tree.
    pub last_clicked_id: Option<AnnotationId>,
    /// When true, finalizing a transient selection sets the frequency focus to match.
    pub selection_auto_focus: bool,
    /// When true, clicking an annotation pushes its frequency focus override.
    pub auto_focus: bool,
    /// When true, export uses each region's own freq bounds for DSP (else global HFR).
    pub export_use_region_focus: bool,
    /// Id of annotation currently being dragged in the sidebar tree.
    pub dragging_id: Option<AnnotationId>,
    /// Drop target: (target_id, position "before"/"after"/"inside").
    pub drop_target: Option<(AnnotationId, String)>,
    /// Undo/redo stack for annotation operations.
    pub undo_stack: UndoStack,
    /// Active annotation resize drag: (annotation_id, handle position).
    pub drag_handle: Option<(AnnotationId, ResizeHandlePosition)>,
    /// Hovered annotation resize handle (for cursor + highlight).
    pub hover_handle: Option<(AnnotationId, ResizeHandlePosition)>,
    /// Snapshot of original bounds before resize drag.
    pub drag_original: Option<(f64, f64, Option<f64>, Option<f64>)>,
    /// Whether the annotation label editing panel is active.
    pub editing: bool,
    /// True when editing a just-created annotation (Escape = cancel/delete).
    pub is_new_edit: bool,
    /// Whether saved annotations are drawn on the spectrogram.
    pub visible: bool,
}

/// Display-DSP / spectrogram-processing settings: the per-stage display filter
/// panel, Xformed-Spec view intensity, decimation, and saved ZC/normal states.
#[derive(Clone, Debug, Store)]
pub struct DisplayState {
    // Display-affecting checkboxes (spectrogram intensity).
    pub auto_gain: bool,
    pub eq: bool,
    pub noise_filter: bool,
    /// Compute spectrogram tiles from DSP-transformed audio (pitch shift, het, …).
    pub transform: bool,
    // Saved display settings restored on ZC enter/leave.
    pub zc_saved_auto_gain: bool,
    pub zc_saved_eq: bool,
    pub zc_saved_noise_filter: bool,
    pub normal_saved_auto_gain: bool,
    pub normal_saved_eq: bool,
    pub normal_saved_noise_filter: bool,
    // Independent gain/intensity for the Xformed-Spec view.
    pub xform_gain_db: f32,
    pub xform_floor_db: f32,
    pub xform_range_db: f32,
    pub xform_gamma: f32,
    // Per-stage display DSP filter panel.
    pub filter_enabled: bool,
    pub filter_eq: DisplayFilterMode,
    pub filter_notch: DisplayFilterMode,
    pub filter_nr: DisplayFilterMode,
    pub filter_transform: DisplayFilterMode,
    pub filter_gain: DisplayFilterMode,
    pub filter_decimate: DisplayFilterMode,
    /// Extra dB boost applied to spectrogram display from Auto/Same gain modes.
    pub gain_boost: f32,
    /// Target decimation sample rate in Hz (Custom mode; Auto derives from transform).
    pub decimate_rate: u32,
    /// Effective decimation target rate resolved from `filter_decimate` (0 = none).
    pub decimate_effective: u32,
    /// Browser's default audio output sample rate.
    pub browser_sample_rate: u32,
    /// Custom NR strength (display-only).
    pub nr_strength: f64,
    /// Auto-learned noise floor for display (from first ~500ms of file).
    pub auto_noise_floor: Option<crate::dsp::spectral_sub::NoiseFloor>,
}

/// Sidebar / panel chrome (left + right sidebars, layer panel, status bar).
#[derive(Clone, Debug, Store)]
pub struct PanelsState {
    pub right_tab: RightSidebarTab,
    pub right_collapsed: bool,
    pub right_width: f64,
    pub right_dropdown_open: bool,
    pub metadata_view: MetadataView,
    /// Left (main) sidebar collapsed.
    pub left_collapsed: bool,
    /// Left (main) sidebar width.
    pub left_width: f64,
    pub left_tab: LeftSidebarTab,
    /// Which floating layer panel is currently open.
    pub layer_panel_open: Option<LayerPanel>,
    /// Whether the analysis/status bar is visible (persisted).
    pub show_status_bar: bool,
}

/// Modal dialogs / one-time hint visibility flags.
#[derive(Clone, Debug, Store)]
pub struct DialogsState {
    pub bookmark_popup: bool,
    pub privacy_settings: bool,
    pub about: bool,
    pub background_audio_hint: bool,
    /// Persisted: background-audio guidance already dismissed.
    pub background_hint_dismissed: bool,
    pub notif_rationale: bool,
    /// Persisted: notification rationale already surfaced.
    pub notif_perm_asked: bool,
    pub xc_browser_open: bool,
}

// ── AppState ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub struct AppState {
    pub files: RwSignal<Vec<LoadedFile>>,
    pub current_file_index: RwSignal<Option<usize>>,
    pub file_sort_mode: RwSignal<FileSortMode>,
    pub show_file_previews: RwSignal<bool>,
    /// Sidebar / panel chrome (grouped reactive store).
    pub panels: Store<PanelsState>,
    /// Modal dialogs / one-time hint flags (grouped reactive store).
    pub dialogs: Store<DialogsState>,
    pub selection: RwSignal<Option<Selection>>,
    pub last_selection: RwSignal<Option<Selection>>,
    pub playback_mode: RwSignal<PlaybackMode>,
    /// Modes selected via ctrl+click in the Mode radio group, in addition
    /// to `playback_mode`. When non-empty, the bottom toolbar renders one
    /// Play button per selected mode (in addition to `playback_mode`).
    /// `playback_mode` is always implicitly part of the selection — only
    /// extras are stored here so the simple "no ctrl-clicks yet" case
    /// stays a no-op.
    pub playback_modes_extra: RwSignal<Vec<PlaybackMode>>,
    /// Set when the user clicked ▶ on a 1:1 button while HFR was on (in a
    /// multi-selection containing both 1:1 and an HFR-only mode). HFR is
    /// turned off for the duration of that playback and restored when
    /// playback stops or the user switches to another mode.
    pub paused_hfr_for_normal: RwSignal<bool>,
    /// Transform / DSP playback parameters: heterodyne, time-expansion, pitch
    /// shift, phase vocoder, zero-crossing — plus BandFF-derived auto flags
    /// (grouped reactive store).
    pub transform: Store<TransformState>,
    /// Viewport: zoom, scroll, display freq bounds, follow-cursor (grouped store).
    pub view: Store<ViewState>,
    /// Bandpass / EQ filter + BandFF gutter settings (grouped reactive store).
    pub filter: Store<FilterState>,
    /// Gain (audio + waveform-view) settings (grouped reactive store).
    pub gain: Store<GainState>,
    pub is_playing: RwSignal<bool>,
    /// True when playback is frozen waiting for streaming chunks to decode.
    /// Drives the "Buffering…" toast and pauses the playhead animation.
    pub is_buffering: RwSignal<bool>,
    pub playhead_time: RwSignal<f64>,
    pub active_playback_selection: RwSignal<Option<Selection>>,
    pub loading_files: RwSignal<Vec<LoadingEntry>>,
    pub loading_next_id: RwSignal<u64>,
    pub is_dragging: RwSignal<bool>,
    /// True while any pointer button is held down on the spectrogram canvas.
    pub pointer_is_down: RwSignal<bool>,
    /// Spectrogram display + colormap settings (grouped reactive store).
    pub spect: Store<SpectState>,
    /// Optical-flow overlay settings (grouped reactive store). Replaces the
    /// former flat `flow_enabled` / `flow_gate` / `flow_*` signals.
    pub flow: Store<FlowState>,
    pub mouse_freq: RwSignal<Option<f64>>,
    pub mouse_canvas_x: RwSignal<f64>,
    pub mouse_in_label_area: RwSignal<bool>,
    pub label_hover_opacity: RwSignal<f64>,

    // Channel
    pub channel_view: RwSignal<ChannelView>,

    // ── New signals ──────────────────────────────────────────────────────────

    // Tool
    pub canvas_tool: RwSignal<CanvasTool>,

    // HFR (High Frequency Range) mode
    pub hfr_enabled: RwSignal<bool>,

    // Waveform sub-view mode
    pub waveform_view: RwSignal<WaveformView>,

    // Overview
    pub overview_view: RwSignal<OverviewView>,

    // Navigation history (for back/forward buttons in overview)
    pub nav_history: RwSignal<Vec<NavEntry>>,
    pub nav_index: RwSignal<usize>,

    // Bookmarks
    pub bookmarks: RwSignal<Vec<Bookmark>>,

    // Play start mode (All / FromHere / Selected)
    pub play_start_mode: RwSignal<PlayStartMode>,

    // Record mode (ToFile / ToMemory / ListenOnly)
    pub record_mode: RwSignal<RecordMode>,

    // Play-from-here time (updated by Spectrogram on scroll/zoom change)
    pub play_from_here_time: RwSignal<f64>,

    // Tile system: incrementing this triggers a spectrogram redraw
    pub tile_ready_signal: RwSignal<u32>,

    /// Generation counter for background preload. Incremented when file/LOD changes
    /// to cancel stale preload jobs.
    pub bg_preload_gen: RwSignal<u32>,


    // Which floating layer panel is currently open

    // Actual pixel width of the main spectrogram canvas (written by Spectrogram, read by Overview)
    pub spectrogram_canvas_width: RwSignal<f64>,

    // Main panel view mode
    pub main_view: RwSignal<MainView>,

    // Spectrogram drag handles (BandFF + HET)
    pub spec_drag_handle: RwSignal<Option<SpectrogramHandle>>,
    pub spec_hover_handle: RwSignal<Option<SpectrogramHandle>>,

    /// Output frequency range to highlight on spectrogram (set by hover in HFR panel).
    pub output_freq_highlight: RwSignal<Option<(f64, f64)>>,
    /// Snap policy for the Output Range gutter when dragging.
    /// Free = continuous, Standard = powers of 2 + 10 / 5 kHz multiples,
    /// EqualChroma = powers of 2 only (preserves musical intervals).
    pub output_snap: RwSignal<OutputSnap>,

    // Microphone (independent listen + record)
    /// Mic capture + recording lifecycle (grouped reactive store).
    pub mic: Store<MicState>,
    /// Recording metadata / privacy: GPS, device id, home-wifi (grouped store).
    pub recording_meta: Store<RecordingMetaState>,

    /// Whether the privacy settings modal dialog is visible.
    /// Whether the about dialog is visible.
    /// True when OS-throttled background audio was detected; surfaces the
    /// one-time battery-optimization guidance. Cleared when acted on/dismissed.
    /// Persisted flag: background-audio guidance already dismissed
    /// (`oversample_bg_audio_hint_dismissed`).
    /// True when the in-app notification-permission rationale modal should show
    /// (Android, before the OS POST_NOTIFICATIONS prompt).
    /// Persisted flag: notification rationale already surfaced
    /// (`oversample_notif_perm_asked`).

    // Transient status message (e.g. permission errors)
    pub status_message: RwSignal<Option<String>>,
    pub status_level: RwSignal<StatusLevel>,

    // Debug log entries: (timestamp_ms, level, message)
    pub debug_log_entries: RwSignal<Vec<(f64, String, String)>>,

    // Platform detection
    pub is_mobile: RwSignal<bool>,
    pub is_tauri: bool,
    /// Stable "running on a mobile platform" flag, fixed at startup from the
    /// user-agent only (NOT viewport width). Unlike `is_mobile` — which is a
    /// layout signal that flips when a desktop window is narrowed below the
    /// mobile breakpoint — this stays put, so it's the correct discriminator
    /// for platform-specific behaviour like Android MediaStore vs a desktop
    /// save dialog.
    pub is_mobile_platform: bool,

    /// True when the browser viewport is pinch-zoomed in (visualViewport.scale > 1).
    /// Used to show a zoom-out button and disable custom pinch handlers.
    pub viewport_zoomed: RwSignal<bool>,
    /// Visual viewport position/size for placing the zoom-out button in the
    /// visible area when pinch-zoomed. (offset_top, offset_left, vp_width, scale)
    pub visual_viewport_rect: RwSignal<(f64, f64, f64, f64)>,

    // XC browser

    // (hfr_saved_* signals removed — now in FocusStack)

    // Axis drag (left axis frequency range selection)
    pub axis_drag_start_freq: RwSignal<Option<f64>>,
    pub axis_drag_current_freq: RwSignal<Option<f64>>,

    // Cursor time at mouse position (for bottom bar feedback)
    pub cursor_time: RwSignal<Option<f64>>,

    // Left sidebar settings page

    /// Chromagram view settings (grouped reactive store).
    pub chroma: Store<ChromaState>,

    /// Resonator-view settings (grouped reactive store).
    pub resonator: Store<ResonatorState>,
    // When false, the Range button is hidden at full range
    pub always_show_view_range: RwSignal<bool>,

    /// Notch noise-filtering settings (grouped reactive store).
    pub notch: Store<NotchState>,

    /// Spectral-subtraction noise-reduction settings (grouped reactive store).
    pub noise_reduce: Store<NoiseReduceState>,

    /// Pulse-detection overlay settings (grouped reactive store).
    pub pulse: Store<PulseState>,

    // File identity hashing
    /// Whether a full hash computation (Layer 3/4) is currently running.
    pub hash_computing: RwSignal<bool>,
    /// Generation counter for cancelling in-progress hash computations.
    pub hash_generation: RwSignal<u32>,

    /// Annotation subsystem (store/selection/drag/resize/undo) (grouped store).
    pub annotations: Store<AnnotationsState>,

    /// Project (.batproj) state (grouped reactive store).
    pub project: Store<ProjectState>,

    /// Timeline / multi-file selection state (grouped reactive store).
    pub timeline: Store<TimelineState>,

    /// Display-DSP / spectrogram-processing settings (grouped reactive store).
    pub display: Store<DisplayState>,

    /// PSD (Power Spectral Density) panel settings (grouped reactive store).
    pub psd: Store<PsdState>,

    /// Bat Book reference subsystem state (grouped reactive store).
    pub bat_book: Store<BatBookState>,

    /// Frequency shield/flag color bar style (persisted to localStorage).
    pub shield_style: RwSignal<ShieldStyle>,

    /// Whether the analysis/status bar is visible (persisted to localStorage).

    // Layered frequency focus stack
    pub focus_stack: RwSignal<crate::focus_stack::FocusStack>,

    // Clean view: hide all overlays while holding backtick
    pub clean_view: RwSignal<bool>,

    /// Export / video-export UI state (grouped reactive store).
    pub export: Store<ExportState>,

    // Selection focus
    /// Which entity type currently has interactive focus (handles, overflow menu).
    pub active_focus: RwSignal<Option<ActiveFocus>>,
    /// Whether the transient-selection "..." overflow menu is open.
    pub selection_overflow_open: RwSignal<bool>,
    /// Whether an annotation "..." overflow menu is open.
    pub annotation_overflow_open: RwSignal<bool>,
}

fn detect_tauri() -> bool {
    let Some(window) = web_sys::window() else { return false };
    js_sys::Reflect::get(&window, &wasm_bindgen::JsValue::from_str("__TAURI_INTERNALS__"))
        .map(|v| !v.is_undefined())
        .unwrap_or(false)
}

/// Returns true if the user-agent string indicates a mobile device.
/// This is a one-time check (UA doesn't change during the session).
fn detect_mobile_ua() -> bool {
    let Some(window) = web_sys::window() else { return false };
    if let Ok(ua) = window.navigator().user_agent() {
        let ua_lower = ua.to_lowercase();
        if ua_lower.contains("android") || ua_lower.contains("iphone") || ua_lower.contains("ipad") || ua_lower.contains("mobile") {
            return true;
        }
    }
    false
}

/// Returns true if the viewport width is below the mobile breakpoint.
pub fn is_mobile_viewport() -> bool {
    let Some(window) = web_sys::window() else { return false };
    if let Ok(w) = window.inner_width() {
        if let Some(w) = w.as_f64() {
            return w < 768.0;
        }
    }
    false
}

fn detect_mobile() -> bool {
    detect_mobile_ua() || is_mobile_viewport()
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

impl AppState {
    pub fn new() -> Self {
        let s = Self {
            files: RwSignal::new(Vec::new()),
            current_file_index: RwSignal::new(None),
            file_sort_mode: RwSignal::new(FileSortMode::AddOrder),
            show_file_previews: RwSignal::new(false),
            selection: RwSignal::new(None),
            last_selection: RwSignal::new(None),
            playback_mode: RwSignal::new(PlaybackMode::Normal),
            playback_modes_extra: RwSignal::new(Vec::new()),
            paused_hfr_for_normal: RwSignal::new(false),
            spect: Store::new(SpectState {
                display: SpectrogramDisplay::FlowOptical,
                floor_db: -120.0,
                range_db: 120.0,
                gamma: 1.0,
                gain_db: 0.0,
                debug_tiles: false,
                fft_mode: FftMode::AdaptiveM,
                reassign_enabled: false,
                colormap_preference: Colormap::Viridis,
                hfr_colormap_preference: Colormap::Inferno,
            }),

            transform: Store::new(TransformState {
                het_frequency: 45_000.0,
                het_cutoff: 15_000.0,
                het_comb_count: 1,
                het_comb_spacing: 30_000.0,
                het_comb_auto: true,
                het_interacting: false,
                het_freq_auto: true,
                het_cutoff_auto: true,
                te_factor: 10.0,
                te_factor_auto: true,
                ps_factor: 10.0,
                ps_factor_auto: true,
                ps_shift_hz: 0.0,
                pv_factor: 10.0,
                pv_factor_auto: true,
                pv_hq: true,
                zc_factor: 8.0,
            }),
            view: Store::new(ViewState {
                zoom_level: 1.0,
                scroll_offset: 0.0,
                min_display_freq: None,
                max_display_freq: None,
                follow_cursor: true,
                follow_suspended: false,
                follow_visible_since: None,
                pre_play_scroll: 0.0,
                user_panned_during_playback: false,
            }),
            filter: Store::new(FilterState {
                enabled: false,
                band_mode: 3,
                freq_low: 20_000.0,
                freq_high: 60_000.0,
                db_below: -60.0,
                db_selected: 0.0,
                db_harmonics: -30.0,
                db_above: -60.0,
                hovering_band: None,
                quality: FilterQuality::Spectral,
                bandpass_mode: BandpassMode::Auto,
                bandpass_range: BandpassRange::FollowFocus,
                band_ff_freq_lo: 0.0,
                band_ff_freq_hi: 0.0,
                band_ff_dragging: false,
            }),
            gain: Store::new(GainState {
                db: 0.0,
                db_stash: 0.0,
                live_db: 0.0,
                auto: false,
                mode: GainMode::Off,
                mode_last_auto: GainMode::AutoPeak,
                peak_source: PeakSource::First30s,
                selection_peak_cache: crate::audio::peak::PeakCache::default(),
                peak_scanning: false,
                wave_view_db: 0.0,
                wave_view_auto: false,
            }),

            is_playing: RwSignal::new(false),
            is_buffering: RwSignal::new(false),
            playhead_time: RwSignal::new(0.0),
            active_playback_selection: RwSignal::new(None),
            loading_files: RwSignal::new(Vec::new()),
            loading_next_id: RwSignal::new(0),
            is_dragging: RwSignal::new(false),
            pointer_is_down: RwSignal::new(false),
            flow: Store::new(FlowState {
                enabled: false,
                intensity_gate: 0.5,
                gate: 0.75,
                opacity: 0.75,
                shift_gain: 3.0,
                color_gamma: 1.0,
                color_scheme: FlowColorScheme::default(),
            }),
            mouse_freq: RwSignal::new(None),
            mouse_canvas_x: RwSignal::new(0.0),
            mouse_in_label_area: RwSignal::new(false),
            label_hover_opacity: RwSignal::new(0.0),
            // Default: single carrier — comb engages only when the user opts in.
            // Default spacing ~ 2× cutoff so initial comb mode covers cleanly.
            // On by default — auto-fit carrier count + spacing to the
            // focus range. Toggle "A" off in the Carriers row to pick a
            // fixed count manually.

            channel_view: RwSignal::new(ChannelView::Stereo),

            // New
            canvas_tool: RwSignal::new(CanvasTool::Hand),
            hfr_enabled: RwSignal::new(false),
            waveform_view: RwSignal::new(WaveformView::Frequency),
            overview_view: RwSignal::new(OverviewView::Waveform),
            nav_history: RwSignal::new(Vec::new()),
            nav_index: RwSignal::new(0),
            bookmarks: RwSignal::new(Vec::new()),
            play_start_mode: RwSignal::new(PlayStartMode::Auto),
            record_mode: RwSignal::new(if detect_tauri() { RecordMode::ToFile } else { RecordMode::ToMemory }),
            play_from_here_time: RwSignal::new(0.0),
            tile_ready_signal: RwSignal::new(0),
            bg_preload_gen: RwSignal::new(0),
            spectrogram_canvas_width: RwSignal::new(1000.0),
            main_view: RwSignal::new(MainView::Spectrogram),
            spec_drag_handle: RwSignal::new(None),
            spec_hover_handle: RwSignal::new(None),
            output_freq_highlight: RwSignal::new(None),
            output_snap: RwSignal::new(OutputSnap::Standard),
            mic: Store::new(MicState {
                listening: false,
                recording: false,
                sample_rate: 0,
                samples_recorded: 0,
                bits_per_sample: 16,
                max_sample_rate: 0,
                preroll_buffer_secs: 10,
                mode: if detect_tauri() { MicMode::Auto } else { MicMode::Browser },
                supported_rates: Vec::new(),
                live_file_idx: None,
                processing_gen: 0,
                preroll_samples: 0,
                gesture_start_ms: None,
                recording_start_time: None,
                timer_tick: 0,
                device_name: None,
                connection_type: None,
                manufacturer: None,
                usb_connected: false,
                effective_mode: if detect_tauri() { MicMode::Cpal } else { MicMode::Browser },
                recording_target_scroll: 0.0,
                scroll_user_pan_until: 0.0,
                live_data_cols: 0,
                selected_device: None,
                show_chooser: false,
                chip_dismissed: false,
                peak_level: 0.0,
                strategy: if detect_tauri() { MicStrategy::Ask } else { MicStrategy::Browser },
                backend: None,
                acquisition_state: MicAcquisitionState::Idle,
                pending_action: None,
                record_ready_state: RecordReadyState::None,
                starting_recording: false,
                permission_dialog_shown: false,
                max_bit_depth: 0,
                channel_mode: ChannelMode::Mono,
                device_info: None,
                listen_context_samples: 65536,
                mute_output: false,
            }),
            recording_meta: Store::new(RecordingMetaState {
                gps_enabled: {
                    web_sys::window()
                        .and_then(|w| w.local_storage().ok().flatten())
                        .and_then(|ls| ls.get_item("oversample_gps_enabled").ok().flatten())
                        .map(|v| v == "true")
                        .unwrap_or(false)
                },
                location: None,
                home_wifi_ssids: {
                    web_sys::window()
                        .and_then(|w| w.local_storage().ok().flatten())
                        .and_then(|ls| ls.get_item("oversample_home_wifi").ok().flatten())
                        .map(|v| {
                            v.split('\n')
                                .filter(|s| !s.is_empty())
                                .map(|s| s.to_string())
                                .collect()
                        })
                        .unwrap_or_default()
                },
                device_model_enabled: {
                    web_sys::window()
                        .and_then(|w| w.local_storage().ok().flatten())
                        .and_then(|ls| ls.get_item("oversample_device_model").ok().flatten())
                        .map(|v| v != "false")
                        .unwrap_or(true) // default on
                },
                cached_make: None,
                cached_model: None,
            }),
            dialogs: Store::new(DialogsState {
                bookmark_popup: false,
                privacy_settings: false,
                about: false,
                background_audio_hint: false,
                background_hint_dismissed: {
                    web_sys::window()
                        .and_then(|w| w.local_storage().ok().flatten())
                        .and_then(|ls| ls.get_item("oversample_bg_audio_hint_dismissed").ok().flatten())
                        .map(|v| v == "true")
                        .unwrap_or(false)
                },
                notif_rationale: false,
                notif_perm_asked: {
                    web_sys::window()
                        .and_then(|w| w.local_storage().ok().flatten())
                        .and_then(|ls| ls.get_item("oversample_notif_perm_asked").ok().flatten())
                        .map(|v| v == "true")
                        .unwrap_or(false)
                },
                xc_browser_open: false,
            }),
            status_message: RwSignal::new(None),
            status_level: RwSignal::new(StatusLevel::Error),
            debug_log_entries: RwSignal::new(Vec::new()),
            is_mobile: RwSignal::new(detect_mobile()),
            is_tauri: detect_tauri(),
            is_mobile_platform: detect_mobile_ua(),
            viewport_zoomed: RwSignal::new(false),
            visual_viewport_rect: RwSignal::new((0.0, 0.0, 0.0, 1.0)),
            axis_drag_start_freq: RwSignal::new(None),
            axis_drag_current_freq: RwSignal::new(None),
            cursor_time: RwSignal::new(None),
            chroma: Store::new(ChromaState {
                colormap: ChromaColormap::PitchClass,
                gain: 0.0,
                gamma: 1.0,
                range: ChromaRange::Full,
                source: ChromaSource::Resonators,
                adapt: 0.0,
                floor_db: -80.0,
            }),
            resonator: Store::new(ResonatorState {
                bandwidth_hz: 20.0,
                fft_mode: ResonatorFftMode::Single(512),
                layout: ResonatorLayout::Linear,
                viewport_bins: true,
                viewport_range: None,
            }),
            always_show_view_range: RwSignal::new(false),

            notch: Store::new(NotchState {
                enabled: false,
                bands: Vec::new(),
                detecting: false,
                profile_name: String::new(),
                hovering_band: None,
                harmonic_suppression: 0.0,
            }),

            noise_reduce: Store::new(NoiseReduceState {
                enabled: false,
                strength: 0.6,
                floor: None,
                learning: false,
            }),

            pulse: Store::new(PulseState {
                detected: Vec::new(),
                overlay_enabled: false,
                selected_index: None,
                detecting: false,
            }),

            hash_computing: RwSignal::new(false),
            hash_generation: RwSignal::new(0),

            annotations: Store::new(AnnotationsState {
                store: AnnotationStore::default(),
                dirty: false,
                selected_ids: Vec::new(),
                last_clicked_id: None,
                selection_auto_focus: false,
                auto_focus: false,
                export_use_region_focus: true,
                dragging_id: None,
                drop_target: None,
                undo_stack: UndoStack::default(),
                drag_handle: None,
                hover_handle: None,
                drag_original: None,
                editing: false,
                is_new_edit: false,
                visible: true,
            }),

            project: Store::new(ProjectState {
                enabled: {
                    web_sys::window()
                        .and_then(|w: web_sys::Window| w.local_storage().ok().flatten())
                        .and_then(|ls: web_sys::Storage| ls.get_item("oversample_projects_enabled").ok().flatten())
                        .map(|v| v == "true")
                        .unwrap_or(false)
                },
                current: None,
                dirty: false,
                save_status: "",
            }),

            timeline: Store::new(TimelineState {
                selected_file_indices: Vec::new(),
                active: None,
                active_track: None,
                show_clock_time: false,
            }),

            display: Store::new(DisplayState {
                auto_gain: false,
                eq: false,
                noise_filter: false,
                transform: false,
                zc_saved_auto_gain: false,
                zc_saved_eq: true,
                zc_saved_noise_filter: true,
                normal_saved_auto_gain: false,
                normal_saved_eq: false,
                normal_saved_noise_filter: false,
                xform_gain_db: 0.0,
                xform_floor_db: -120.0,
                xform_range_db: 120.0,
                xform_gamma: 1.0,
                filter_enabled: false,
                filter_eq: DisplayFilterMode::Off,
                filter_notch: DisplayFilterMode::Off,
                filter_nr: DisplayFilterMode::Auto,
                filter_transform: DisplayFilterMode::Off,
                filter_gain: DisplayFilterMode::Auto,
                filter_decimate: DisplayFilterMode::Auto,
                gain_boost: 0.0,
                decimate_rate: 48000,
                decimate_effective: 0,
                browser_sample_rate: 0,
                nr_strength: 0.8,
                auto_noise_floor: None,
            }),

            psd: Store::new(PsdState {
                nfft: 1024,
                apply_eq: false,
                apply_notch: false,
                apply_nr: false,
                hover_freqs: Vec::new(),
            }),

            bat_book: Store::new(BatBookState {
                open: false,
                mode: {
                    use crate::bat_book::types::{BatBookMode, BatBookRegion};
                    let ls = web_sys::window()
                        .and_then(|w: web_sys::Window| w.local_storage().ok().flatten());
                    // Try new-format key first
                    let new_key = ls.as_ref()
                        .and_then(|s| s.get_item("oversample_bat_book_mode").ok().flatten());
                    match new_key {
                        Some(k) => BatBookMode::from_storage_key(&k),
                        None => {
                            // Migration: check legacy key
                            let legacy = ls.as_ref()
                                .and_then(|s| s.get_item("oversample_bat_book_region").ok().flatten());
                            match legacy {
                                Some(k) => BatBookRegion::from_storage_key(&k)
                                    .map(BatBookMode::Manual)
                                    .unwrap_or(BatBookMode::Auto),
                                None => BatBookMode::Auto, // brand new user
                            }
                        }
                    }
                },
                region: crate::bat_book::types::BatBookRegion::Global,
                auto_resolved: None,
                favourites: {
                    web_sys::window()
                        .and_then(|w: web_sys::Window| w.local_storage().ok().flatten())
                        .and_then(|ls: web_sys::Storage| ls.get_item("oversample_bat_book_favourites").ok().flatten())
                        .map(|v| {
                            v.split(',')
                                .filter_map(|k| crate::bat_book::types::BatBookRegion::from_storage_key(k.trim()))
                                .collect()
                        })
                        .unwrap_or_default()
                },
                selected_ids: Vec::new(),
                ref_open: false,
                last_clicked_id: None,
                auto_focus: true,
            }),
            shield_style: RwSignal::new({
                web_sys::window()
                    .and_then(|w| w.local_storage().ok().flatten())
                    .and_then(|ls| ls.get_item("oversample_shield_style").ok().flatten())
                    .map(|v| ShieldStyle::from_key(&v))
                    .unwrap_or_default()
            }),
            panels: Store::new(PanelsState {
                right_tab: RightSidebarTab::Metadata,
                right_collapsed: true,
                right_width: 220.0,
                right_dropdown_open: false,
                metadata_view: MetadataView::default(),
                left_collapsed: false,
                left_width: 220.0,
                left_tab: LeftSidebarTab::default(),
                layer_panel_open: None,
                show_status_bar: {
                    web_sys::window()
                        .and_then(|w| w.local_storage().ok().flatten())
                        .and_then(|ls| ls.get_item("oversample_show_status_bar").ok().flatten())
                        .map(|v| v == "true")
                        .unwrap_or(false)
                },
            }),
            focus_stack: RwSignal::new(crate::focus_stack::FocusStack::new()),
            clean_view: RwSignal::new(false),

            // Export UI
            export: Store::new(ExportState {
                section_open: false,
                format: ExportFormat::default(),
                video_progress: None,
                video_status: None,
                video_cancel: false,
                video_resolution: VideoResolution::default(),
                video_codec: VideoCodec::default(),
                video_audio_codec: AudioCodecOption::default(),
                video_view_mode: VideoViewMode::default(),
            }),

            active_focus: RwSignal::new(None),
            selection_overflow_open: RwSignal::new(false),
            annotation_overflow_open: RwSignal::new(false),
        };

        // On mobile, start with sidebar collapsed
        if s.is_mobile.get_untracked() {
            s.panels.left_collapsed().set(true);
        }

        s
    }

    /// Returns the single selected annotation ID, or None if zero or multiple are selected.
    pub fn selected_annotation_id(&self) -> Option<AnnotationId> {
        let ids = self.annotations.selected_ids().get();
        if ids.len() == 1 { Some(ids[0].clone()) } else { None }
    }

    pub fn current_file(&self) -> Option<LoadedFile> {
        let files = self.files.get();
        let idx = self.current_file_index.get()?;
        files.get(idx).cloned()
    }

    /// True when the currently selected file is an Anabat zero-crossing
    /// (`.zc`) recording. Reactive — subscribes to `files` and
    /// `current_file_index`. Use to gate options/views that don't make
    /// sense on a dot-plot recording (the underlying samples are a
    /// synthesised reconstruction, not the original data).
    pub fn current_is_zc(&self) -> bool {
        let files = self.files.get();
        let Some(idx) = self.current_file_index.get() else { return false };
        files.get(idx)
            .map(|f| f.audio.metadata.zc_data.is_some())
            .unwrap_or(false)
    }

    /// Push current scroll/zoom onto the navigation history stack.
    pub fn push_nav(&self) {
        let entry = NavEntry {
            scroll_offset: self.view.scroll_offset().get_untracked(),
            zoom_level: self.view.zoom_level().get_untracked(),
        };
        let idx = self.nav_index.get_untracked();
        self.nav_history.update(|hist| {
            hist.truncate(idx + 1);
            if hist.last().map(|e: &NavEntry| (e.scroll_offset - entry.scroll_offset).abs() < 0.05).unwrap_or(false) {
                return;
            }
            hist.push(entry);
            if hist.len() > 100 {
                hist.remove(0);
            }
        });
        let new_len = self.nav_history.get_untracked().len();
        self.nav_index.set(new_len.saturating_sub(1));
    }

    /// Stable annotation key for the file at list position `idx` (untracked).
    pub fn file_id_at(&self, idx: usize) -> Option<u64> {
        self.files.with_untracked(|files| files.get(idx).map(|f| f.id))
    }

    /// List position of the file with stable id `id`, if it's still loaded.
    pub fn file_idx_for_id(&self, id: u64) -> Option<usize> {
        self.files.with_untracked(|files| files.iter().position(|f| f.id == id))
    }

    /// Stable annotation key for the currently-selected file (untracked).
    pub fn current_file_id(&self) -> Option<u64> {
        let idx = self.current_file_index.get_untracked()?;
        self.file_id_at(idx)
    }

    /// Reactive variant of [`current_file_id`] — tracks `files` and
    /// `current_file_index` so callers inside Effects/views re-run when the
    /// active file changes.
    pub fn current_file_id_tracked(&self) -> Option<u64> {
        let idx = self.current_file_index.get()?;
        self.files.with(|files| files.get(idx).map(|f| f.id))
    }

    /// Apply a snapshot to the annotation store: `Some` inserts/replaces,
    /// `None` clears the file's entry. Used by undo/redo restore.
    fn restore_annotation_snapshot(&self, file_id: u64, snapshot: Option<crate::annotations::AnnotationSet>) {
        self.annotations.store().update(|store| {
            match snapshot {
                Some(set) => store.insert(file_id, set),
                None => { store.remove(file_id); }
            }
        });
        // Persist the file we actually mutated. The global autosave Effect only
        // saves the currently-DISPLAYED file, so undoing/redoing a change on a
        // file the user has since switched away from would otherwise never reach
        // disk. If that file is no longer loaded, skip — its orphaned snapshot
        // can't be persisted meaningfully.
        if let Some(idx) = self.file_idx_for_id(file_id) {
            crate::opfs::save_annotations(*self, idx);
        }
    }

    /// Snapshot the current file's annotation set onto the undo stack.
    /// Call this BEFORE making any annotation mutation.
    pub fn snapshot_annotations(&self) {
        let file_id = match self.current_file_id() {
            Some(id) => id,
            None => return,
        };
        let store = self.annotations.store().get_untracked();
        let snapshot = store.get(file_id).cloned();
        self.annotations.undo_stack().update(|stack| {
            stack.push_undo(UndoEntry { file_id, snapshot });
        });
    }

    /// Undo the last annotation operation.
    pub fn undo_annotations(&self) {
        let entry = {
            let mut popped = None;
            self.annotations.undo_stack().update(|stack| {
                popped = stack.undo.pop();
            });
            match popped {
                Some(e) => e,
                None => return,
            }
        };

        // Save current state to redo before restoring
        let store = self.annotations.store().get_untracked();
        let current = store.get(entry.file_id).cloned();
        self.annotations.undo_stack().update(|stack| {
            stack.redo.push(UndoEntry { file_id: entry.file_id, snapshot: current });
        });

        // Restore the snapshot
        self.restore_annotation_snapshot(entry.file_id, entry.snapshot);
        self.annotations.dirty().set(true);
    }

    /// Redo the last undone annotation operation.
    pub fn redo_annotations(&self) {
        let entry = {
            let mut popped = None;
            self.annotations.undo_stack().update(|stack| {
                popped = stack.redo.pop();
            });
            match popped {
                Some(e) => e,
                None => return,
            }
        };

        // Save current state to undo before restoring
        let store = self.annotations.store().get_untracked();
        let current = store.get(entry.file_id).cloned();
        self.annotations.undo_stack().update(|stack| {
            stack.undo.push(UndoEntry { file_id: entry.file_id, snapshot: current });
        });

        // Restore the snapshot
        self.restore_annotation_snapshot(entry.file_id, entry.snapshot);
        self.annotations.dirty().set(true);
    }

    /// Whether there's something to undo.
    pub fn can_undo(&self) -> bool {
        !self.annotations.undo_stack().get().undo.is_empty()
    }

    /// Whether there's something to redo.
    pub fn can_redo(&self) -> bool {
        !self.annotations.undo_stack().get().redo.is_empty()
    }

    pub fn show_info_toast(&self, msg: impl Into<String>) {
        self.status_level.set(StatusLevel::Info);
        self.status_message.set(Some(msg.into()));
    }

    pub fn show_error_toast(&self, msg: impl Into<String>) {
        self.status_level.set(StatusLevel::Error);
        self.status_message.set(Some(msg.into()));
    }

    /// Start tracking a loading file. Returns a unique ID for updates.
    pub fn loading_start(&self, name: &str) -> u64 {
        let id = self.loading_next_id.get_untracked();
        self.loading_next_id.set(id + 1);
        self.loading_files.update(|v| {
            v.push(LoadingEntry {
                id,
                name: name.to_string(),
                stage: LoadingStage::Decoding,
            });
        });
        id
    }

    /// Update the stage for a loading file by ID.
    pub fn loading_update(&self, id: u64, stage: LoadingStage) {
        self.loading_files.update(|v| {
            if let Some(entry) = v.iter_mut().find(|e| e.id == id) {
                entry.stage = stage;
            }
        });
    }

    /// Remove a loading entry (finished or failed) and clear the loading_id on the file.
    pub fn loading_done(&self, id: u64) {
        self.loading_files.update(|v| v.retain(|e| e.id != id));
        self.files.update(|files| {
            if let Some(f) = files.iter_mut().find(|f| f.loading_id == Some(id)) {
                f.loading_id = None;
            }
        });
    }

    pub fn log_debug(&self, level: &str, msg: impl Into<String>) {
        let timestamp = js_sys::Date::now();
        let msg_str = msg.into();
        log::info!("[{}] {}", level, &msg_str);
        self.debug_log_entries.update(|entries| {
            entries.push((timestamp, level.to_string(), msg_str));
            if entries.len() > 500 {
                entries.drain(0..entries.len() - 500);
            }
        });
    }

    /// Temporarily suspend follow-cursor when the user scrolls or pans.
    /// Re-engagement happens automatically once the playhead is on-screen
    /// and 200ms have passed since the last scroll action.
    pub fn suspend_follow(&self) {
        if self.is_playing.get_untracked() {
            self.view.user_panned_during_playback().set(true);
        }
        if self.view.follow_cursor().get_untracked() && self.is_playing.get_untracked() {
            self.view.follow_suspended().set(true);
            self.view.follow_visible_since().set(Some(js_sys::Date::now()));
        }
    }

    /// Suspend the waterfall smooth-scroll animation for `delay_ms` from now so
    /// the user can drag backwards during live listening/recording without
    /// the display immediately snapping back to the live edge. Called on every
    /// pan tick so the timer always extends past the last gesture.
    pub fn suspend_waterfall_follow(&self, delay_ms: f64) {
        if self.mic.recording().get_untracked() || self.mic.listening().get_untracked() {
            let until = js_sys::Date::now() + delay_ms;
            self.mic.scroll_user_pan_until().set(until);
        }
    }

    pub fn compute_auto_gain(&self) -> f64 {
        let files = self.files.get();
        let idx = self.current_file_index.get();
        self.compute_auto_gain_inner(&files, idx)
    }

    /// Untracked version for use inside render Effects that already subscribe
    /// to `files` and `current_file_index`. Avoids redundant Vec clone + subscription.
    pub fn compute_auto_gain_untracked(&self) -> f64 {
        self.files.with_untracked(|files| {
            let idx = self.current_file_index.get_untracked();
            self.compute_auto_gain_inner(files, idx)
        })
    }

    fn compute_auto_gain_inner(&self, files: &[LoadedFile], idx: Option<usize>) -> f64 {
        let Some(file_index) = idx else { return 0.0 };
        let Some(file) = files.get(file_index) else { return 0.0 };

        let peak_db = match self.gain.peak_source().get() {
            PeakSource::First30s => file.cached_peak_db,
            PeakSource::FullWave => {
                // Fall back to 30s peak while full scan is in progress.
                // If playing, prefer the 30s peak to avoid mid-play gain jumps
                // when the full scan completes.
                if self.is_playing.get_untracked() && file.cached_full_peak_db.is_none() {
                    file.cached_peak_db
                } else {
                    file.cached_full_peak_db.or(file.cached_peak_db)
                }
            }
            PeakSource::Selection => {
                self.lookup_selection_peak(file_index, file).or(file.cached_peak_db)
            }
            PeakSource::Processed => {
                // Post-DSP peak: for now fall back to raw peak.
                // Full implementation requires running the DSP chain on a sample window.
                file.cached_peak_db
            }
        };
        let Some(peak_db) = peak_db else { return 0.0 };
        // Cap at +60 dB to avoid extreme amplification of very quiet recordings
        (-3.0 - peak_db).min(60.0)
    }

    /// Look up cached selection peak, or trigger an async scan if not cached.
    /// Returns None if no selection or not yet computed.
    /// Does not start new scans while audio is playing to avoid mid-play gain jumps.
    fn lookup_selection_peak(&self, file_index: usize, file: &LoadedFile) -> Option<f64> {
        let sel = self.selection.get()?;
        let sr = file.audio.sample_rate as f64;
        let start_sample = (sel.time_start * sr) as u64;
        let end_sample = (sel.time_end * sr) as u64;
        if end_sample <= start_sample { return None; }

        let key = (file_index, start_sample, end_sample);

        // Check cache (reactive read so we re-run when cache updates)
        if let Some(&peak_db) = self.gain.selection_peak_cache().get().get(&key) {
            return peak_db;
        }

        // Don't start new scans while playing — avoid mid-play gain jumps
        if self.is_playing.get_untracked() {
            return None;
        }

        // Not cached — kick off async scan
        crate::audio::peak::start_selection_peak_scan(
            *self, file_index, start_sample, end_sample,
        );

        // Return None for now; will re-run when cache is updated
        None
    }

    // ── Focus Stack helpers ─────────────────────────────────────────────

    /// Called by drag handles, axis drag, input fields.
    /// Updates the focus stack and syncs output signals immediately.
    pub fn set_band_ff_range(&self, lo: f64, hi: f64) {
        use crate::focus_stack::FocusRange;
        self.focus_stack.update(|s| {
            s.set_user_range(FocusRange::new(lo, hi));
        });
        self.sync_focus_outputs();
    }

    /// Set only the lower BandFF bound (for drag handles).
    pub fn set_band_ff_lo(&self, lo: f64) {
        let hi = self.filter.band_ff_freq_hi().get_untracked();
        self.set_band_ff_range(lo, hi);
    }

    /// Set only the upper BandFF bound (for drag handles).
    pub fn set_band_ff_hi(&self, hi: f64) {
        let lo = self.filter.band_ff_freq_lo().get_untracked();
        self.set_band_ff_range(lo, hi);
    }

    /// Push a bat book BandFF override. Enables HFR if not already on.
    pub fn push_bat_book_ff(&self, lo: f64, hi: f64) {
        use crate::focus_stack::{FocusRange, FocusSource};
        self.focus_stack.update(|s| {
            s.push_override(FocusSource::BatBook, FocusRange::new(lo, hi));
            if !s.hfr_enabled() {
                s.set_hfr_enabled(true);
            }
        });
        // Ensure playback mode is not Normal when HFR is on
        if self.playback_mode.get_untracked() == PlaybackMode::Normal {
            let saved = self.focus_stack.get_untracked().saved_playback_mode();
            self.playback_mode.set(saved.unwrap_or(PlaybackMode::PitchShift));
        }
        if self.filter.bandpass_mode().get_untracked() == BandpassMode::Off {
            let saved = self.focus_stack.get_untracked().saved_bandpass_mode();
            self.filter.bandpass_mode().set(saved.unwrap_or(BandpassMode::Auto));
        }
        self.sync_focus_outputs();
    }

    /// Pop the bat book BandFF override. Restores previous state if not adopted.
    pub fn pop_bat_book_ff(&self) {
        use crate::focus_stack::{FocusRange, FocusSource};
        let mut restore: Option<FocusRange> = None;
        self.focus_stack.update(|s| {
            restore = s.pop_override(FocusSource::BatBook);
        });
        if let Some(range) = restore {
            if !range.is_active() {
                // No active focus to restore — turn off HFR
                self.focus_stack.update(|s| s.set_hfr_enabled(false));
                self.playback_mode.set(PlaybackMode::Normal);
                self.filter.bandpass_mode().set(BandpassMode::Off);
            }
        }
        // If adopted (restore is None): user range is correct, HFR stays as-is
        self.sync_focus_outputs();
    }

    /// Push an annotation BandFF override. Only for annotations with freq bounds.
    pub fn push_annotation_ff(&self, lo: f64, hi: f64) {
        use crate::focus_stack::{FocusRange, FocusSource};
        self.focus_stack.update(|s| {
            s.push_override(FocusSource::Annotation, FocusRange::new(lo, hi));
            if !s.hfr_enabled() {
                s.set_hfr_enabled(true);
            }
        });
        if self.playback_mode.get_untracked() == PlaybackMode::Normal {
            let saved = self.focus_stack.get_untracked().saved_playback_mode();
            self.playback_mode.set(saved.unwrap_or(PlaybackMode::PitchShift));
        }
        if self.filter.bandpass_mode().get_untracked() == BandpassMode::Off {
            let saved = self.focus_stack.get_untracked().saved_bandpass_mode();
            self.filter.bandpass_mode().set(saved.unwrap_or(BandpassMode::Auto));
        }
        self.sync_focus_outputs();
    }

    /// Pop the annotation BandFF override.
    pub fn pop_annotation_ff(&self) {
        use crate::focus_stack::{FocusRange, FocusSource};
        let mut restore: Option<FocusRange> = None;
        self.focus_stack.update(|s| {
            restore = s.pop_override(FocusSource::Annotation);
        });
        if let Some(range) = restore {
            if !range.is_active() && !self.focus_stack.get_untracked().has_override(FocusSource::BatBook) {
                self.focus_stack.update(|s| s.set_hfr_enabled(false));
                self.playback_mode.set(PlaybackMode::Normal);
                self.filter.bandpass_mode().set(BandpassMode::Off);
            }
        }
        self.sync_focus_outputs();
    }

    /// Frequency bounds implied by the current annotation selection, if any.
    pub fn selected_annotation_focus_range(&self) -> Option<(f64, f64)> {
        let file_id = self.current_file_id()?;
        let ids = self.annotations.selected_ids().get_untracked();
        if ids.is_empty() {
            return None;
        }

        let store = self.annotations.store().get_untracked();
        let set = store.get(file_id)?;

        let mut freq_lo = f64::MAX;
        let mut freq_hi = f64::MIN;
        let mut found = false;

        for ann in &set.annotations {
            if !ids.contains(&ann.id) {
                continue;
            }

            let range = match &ann.kind {
                AnnotationKind::Region(region) => match (region.freq_low, region.freq_high) {
                    (Some(lo), Some(hi)) => Some((lo.min(hi), lo.max(hi))),
                    _ => None,
                },
                AnnotationKind::Measurement(measurement) => Some((
                    measurement.start_freq.min(measurement.end_freq),
                    measurement.start_freq.max(measurement.end_freq),
                )),
                _ => None,
            };

            if let Some((lo, hi)) = range {
                freq_lo = freq_lo.min(lo);
                freq_hi = freq_hi.max(hi);
                found = true;
            }
        }

        if found && freq_hi - freq_lo > 100.0 {
            Some((freq_lo, freq_hi))
        } else {
            None
        }
    }

    /// Keep the annotation focus override in sync with the current selection.
    pub fn sync_annotation_auto_focus(&self) {
        if !self.annotations.auto_focus().get_untracked() {
            self.pop_annotation_ff();
            return;
        }

        if let Some((lo, hi)) = self.selected_annotation_focus_range() {
            self.push_annotation_ff(lo, hi);
        } else {
            self.pop_annotation_ff();
        }
    }

    /// Toggle HFR on/off. Saves/restores playback mode, bandpass, and gain.
    pub fn toggle_hfr(&self) {
        // Swap gain_db between HFR-on and HFR-off so we don't blast eardrums
        let current_gain = self.gain.db().get_untracked();
        let stashed_gain = self.gain.db_stash().get_untracked();
        self.gain.db().set(stashed_gain);
        self.gain.db_stash().set(current_gain);

        let stack = self.focus_stack.get_untracked();
        if stack.hfr_enabled() {
            // Turning off: save current mode
            let current_mode = self.playback_mode.get_untracked();
            let current_bp = self.filter.bandpass_mode().get_untracked();
            self.focus_stack.update(|s| {
                s.set_saved_playback_mode(Some(current_mode));
                s.set_saved_bandpass_mode(Some(current_bp));
                s.set_hfr_enabled(false);
            });
            self.filter.bandpass_mode().set(BandpassMode::Off);
            self.playback_mode.set(PlaybackMode::Normal);
        } else {
            // Turning on
            self.focus_stack.update(|s| {
                s.set_hfr_enabled(true);
            });
            let stack = self.focus_stack.get_untracked();
            match stack.saved_playback_mode() {
                Some(mode) => self.playback_mode.set(mode),
                None => {
                    if self.playback_mode.get_untracked() == PlaybackMode::Normal {
                        // For ≤48 kHz files, keep 1:1 — HF is used for bandpass only.
                        let sample_rate = self.files.with_untracked(|files| {
                            self.current_file_index
                                .get_untracked()
                                .and_then(|i| files.get(i))
                                .map(|f| f.audio.sample_rate)
                                .unwrap_or(0)
                        });
                        if sample_rate == 0 || sample_rate > 48_000 {
                            self.playback_mode.set(PlaybackMode::PitchShift);
                        }
                    }
                }
            }
            self.filter.bandpass_mode().set(
                stack.saved_bandpass_mode().unwrap_or(BandpassMode::Auto),
            );
        }
        self.sync_focus_outputs();
    }

    /// Public re-sync of focus outputs. Call this when the active Nyquist
    /// changes (mic opened, listen/record toggled, current file changed) so
    /// the band-FF output signals re-clamp without losing user intent stored
    /// in the focus stack.
    pub fn resync_focus_outputs(&self) {
        self.sync_focus_outputs();
    }

    /// Sync the focus stack's effective range to the output signals
    /// (band_ff_freq_lo, band_ff_freq_hi, hfr_enabled). The output is clamped
    /// to the active Nyquist (mic SR/2 when listening or recording, file SR/2
    /// otherwise) so the band can never exceed what the source can resolve.
    /// The unclamped user intent stays in the focus stack and re-applies when
    /// the source changes back.
    fn sync_focus_outputs(&self) {
        let stack = self.focus_stack.get_untracked();
        let eff = stack.effective_range();
        let hfr = stack.hfr_enabled();
        let nyq = self.active_nyquist();
        let clamped_lo = eff.lo.clamp(0.0, nyq);
        let clamped_hi = eff.hi.clamp(clamped_lo, nyq);
        if self.filter.band_ff_freq_lo().get_untracked() != clamped_lo {
            self.filter.band_ff_freq_lo().set(clamped_lo);
        }
        if self.filter.band_ff_freq_hi().get_untracked() != clamped_hi {
            self.filter.band_ff_freq_hi().set(clamped_hi);
        }
        if self.hfr_enabled.get_untracked() != hfr {
            self.hfr_enabled.set(hfr);
        }
    }

    /// Highest frequency the active source can carry. When the current file
    /// is the live mic document (armed, listening, or recording), this is the
    /// mic's Nyquist. Otherwise it's the file's spectrogram max_freq. Falls
    /// back to 96 kHz if neither source has reported a sample rate.
    pub fn active_nyquist(&self) -> f64 {
        let cur = self.current_file_index.get_untracked();
        let live = self.mic.live_file_idx().get_untracked();
        let is_live_doc = matches!((cur, live), (Some(c), Some(l)) if c == l);
        if is_live_doc {
            let sr = self.mic.sample_rate().get_untracked();
            if sr > 0 {
                return sr as f64 / 2.0;
            }
        }
        let files = self.files.get_untracked();
        cur.and_then(|i| files.get(i))
            .map(|f| f.spectrogram.max_freq)
            .filter(|m| *m > 0.0)
            .unwrap_or(96_000.0)
    }
}
