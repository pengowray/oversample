use leptos::prelude::*;
use crate::types::{AudioData, PreviewImage, SpectrogramData};

#[derive(Clone, Debug)]
pub struct LoadedFile {
    pub name: String,
    pub audio: AudioData,
    pub spectrogram: SpectrogramData,
    pub preview: Option<PreviewImage>,
    /// Higher-resolution overview image computed after full spectrogram is ready.
    /// Falls back to `preview` when not yet available.
    pub overview_image: Option<PreviewImage>,
    pub xc_metadata: Option<Vec<(String, String)>>,
    pub is_recording: bool,  // true = unsaved recording (show indicator on web)
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Selection {
    pub time_start: f64,
    pub time_end: f64,
    pub freq_low: f64,
    pub freq_high: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlaybackMode {
    Normal,
    Heterodyne,
    TimeExpansion,
    PitchShift,
    ZeroCrossing,
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

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum FlowColorScheme {
    #[default]
    RedBlue,
    CoolWarm,
    TealOrange,
    PurpleGreen,
    Spectral,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum RightSidebarTab {
    #[default]
    Metadata,
    Spectrogram,
    Selection,
    Analysis,
    Harmonics,
    Notch,
    Pulses,
    DebugLog,
}

impl RightSidebarTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Metadata => "Info",
            Self::Spectrogram => "Display",
            Self::Selection => "Selection",
            Self::Analysis => "Analysis",
            Self::Harmonics => "Harmonics (beta)",
            Self::Notch => "Noise Filter",
            Self::Pulses => "Pulses",
            Self::DebugLog => "Debug Log",
        }
    }

    pub const ALL: &'static [RightSidebarTab] = &[
        Self::Metadata,
        Self::Spectrogram,
        Self::Selection,
        Self::Analysis,
        Self::Harmonics,
        Self::Notch,
        Self::Pulses,
        Self::DebugLog,
    ];
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum FilterQuality {
    #[default]
    Fast,
    HQ,
}

// ── New enums ────────────────────────────────────────────────────────────────

/// Bandpass filter mode: Auto (from FF), Off, or On (manual).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum BandpassMode {
    #[default]
    Auto,
    Off,
    On,
}

/// Whether the bandpass frequency range follows the Focus or is set independently.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum BandpassRange {
    #[default]
    FollowFocus,
    Custom,
}

/// Which spectrogram overlay handle is being dragged / hovered.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpectrogramHandle {
    FfUpper,       // FF upper boundary
    FfLower,       // FF lower boundary
    FfMiddle,      // FF midpoint (transpose whole range)
    HetCenter,     // HET center freq
    HetBandUpper,  // HET upper band edge
    HetBandLower,  // HET lower band edge
}

/// How TE / PS factors are auto-computed from the FF range.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum AutoFactorMode {
    #[default]
    Target3k,    // factor = FF_center / 3000
    MinAudible,  // factor = FF_high / 20000
    Fixed10x,    // factor = 10
}

/// Active interaction tool for the main spectrogram canvas.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum CanvasTool {
    #[default]
    Hand,      // drag to pan
    Selection, // drag to select
}

/// What the overview strip shows.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum OverviewView {
    #[default]
    Spectrogram,
    Waveform,
}

/// What the main panel displays.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum MainView {
    #[default]
    Spectrogram,
    Waveform,
    ZcChart,
    Flow,
    Chromagram,
}

impl MainView {
    pub fn label(self) -> &'static str {
        match self {
            Self::Spectrogram => "Spectrogram",
            Self::Waveform => "Waveform",
            Self::ZcChart => "ZC Chart",
            Self::Flow => "Flow",
            Self::Chromagram => "Chromagram",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Spectrogram => "Spec",
            Self::Waveform => "Wave",
            Self::ZcChart => "ZC",
            Self::Flow => "Flow",
            Self::Chromagram => "Chroma",
        }
    }

    pub const ALL: &'static [MainView] = &[
        Self::Spectrogram,
        Self::Waveform,
        Self::ZcChart,
        Self::Flow,
        Self::Chromagram,
    ];
}

/// Which frequency range the overview displays.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum OverviewFreqMode {
    #[default]
    All,
    Human,      // 20 Hz – 20 kHz
    MatchMain,  // tracks max_display_freq
}

// ── Multi-resolution FFT ─────────────────────────────────────────────────────

/// FFT window mode for spectrogram computation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FftMode {
    /// Single FFT window size (e.g. 256, 512, 1024, 2048, 4096, 8192).
    Single(usize),
    /// 2-band multi-resolution: 4096 low, 1024 high (crossover at 25% Nyquist).
    MultiRes2,
    /// 3-band multi-resolution: 8192 low, 2048 mid, 512 high.
    MultiRes3,
}

/// A frequency band within a multi-resolution FFT configuration.
#[derive(Clone, Copy, Debug)]
pub struct MultiResBand {
    pub fft_size: usize,
    /// First output bin (inclusive) in the merged output grid.
    pub output_bin_start: usize,
    /// Last output bin (exclusive) in the merged output grid.
    pub output_bin_end: usize,
}

impl FftMode {
    /// The largest FFT size used by this mode.
    /// Determines the output tile height: `max_fft_size() / 2 + 1` bins.
    pub fn max_fft_size(&self) -> usize {
        match self {
            FftMode::Single(sz) => *sz,
            FftMode::MultiRes2 => 4096,
            FftMode::MultiRes3 => 8192,
        }
    }

    /// Whether this mode uses multiple FFT sizes.
    pub fn is_multi_res(&self) -> bool {
        !matches!(self, FftMode::Single(_))
    }

    /// Band definitions for multi-resolution modes.
    /// Each band specifies its FFT size and the output bin range it fills.
    pub fn bands(&self) -> Vec<MultiResBand> {
        match self {
            FftMode::Single(sz) => vec![MultiResBand {
                fft_size: *sz,
                output_bin_start: 0,
                output_bin_end: sz / 2 + 1,
            }],
            FftMode::MultiRes2 => {
                // Output: 4096/2+1 = 2049 bins total
                // Low: 0–25% Nyquist → bins 0..512 with 4096 FFT
                // High: 25–100% Nyquist → bins 512..2049 with 1024 FFT
                vec![
                    MultiResBand { fft_size: 4096, output_bin_start: 0, output_bin_end: 512 },
                    MultiResBand { fft_size: 1024, output_bin_start: 512, output_bin_end: 2049 },
                ]
            }
            FftMode::MultiRes3 => {
                // Output: 8192/2+1 = 4097 bins total
                // Low: 0–10% Nyquist → bins 0..410 with ~~8192~~ FFT
                // Mid: 10–40% Nyquist → bins 410..1639 with ~~2048~~ FFT
                // High: 40–100% Nyquist → bins 1639..4097 with 512 FFT
                vec![
                    MultiResBand { fft_size: 2048, output_bin_start: 0, output_bin_end: 410 },
                    MultiResBand { fft_size: 1024, output_bin_start: 410, output_bin_end: 1639 },
                    MultiResBand { fft_size: 512, output_bin_start: 1639, output_bin_end: 4097 },
                ]
            }
        }
    }
}

/// Which floating layer panel is currently open (only one at a time).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LayerPanel {
    OverviewLayers,
    HfrMode,
    Tool,
    FreqRange,
    MainView,
}

/// A navigation history entry (for overview back/forward buttons).
#[derive(Clone, Copy, Debug)]
pub struct NavEntry {
    pub scroll_offset: f64,
    pub zoom_level: f64,
}

/// A time-position bookmark created during or after playback.
#[derive(Clone, Copy, Debug)]
pub struct Bookmark {
    pub time: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum ColormapPreference {
    #[default]
    Viridis,
    Inferno,
    Magma,
    Plasma,
    Cividis,
    Turbo,
    Greyscale,
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

// ── AppState ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub struct AppState {
    pub files: RwSignal<Vec<LoadedFile>>,
    pub current_file_index: RwSignal<Option<usize>>,
    pub selection: RwSignal<Option<Selection>>,
    pub playback_mode: RwSignal<PlaybackMode>,
    pub het_frequency: RwSignal<f64>,
    pub te_factor: RwSignal<f64>,
    pub zoom_level: RwSignal<f64>,
    pub scroll_offset: RwSignal<f64>,
    pub is_playing: RwSignal<bool>,
    pub playhead_time: RwSignal<f64>,
    pub loading_count: RwSignal<usize>,
    pub ps_factor: RwSignal<f64>,
    pub zc_factor: RwSignal<f64>,
    pub het_interacting: RwSignal<bool>,
    pub is_dragging: RwSignal<bool>,
    pub spectrogram_display: RwSignal<SpectrogramDisplay>,
    pub flow_enabled: RwSignal<bool>,
    pub right_sidebar_tab: RwSignal<RightSidebarTab>,
    pub right_sidebar_collapsed: RwSignal<bool>,
    pub right_sidebar_width: RwSignal<f64>,
    pub right_sidebar_dropdown_open: RwSignal<bool>,
    pub flow_intensity_gate: RwSignal<f32>,
    pub flow_gate: RwSignal<f32>,
    pub flow_opacity: RwSignal<f32>,
    pub flow_shift_gain: RwSignal<f32>,
    pub flow_color_gamma: RwSignal<f32>,
    pub flow_color_scheme: RwSignal<FlowColorScheme>,
    pub min_display_freq: RwSignal<Option<f64>>,
    pub max_display_freq: RwSignal<Option<f64>>,
    pub mouse_freq: RwSignal<Option<f64>>,
    pub mouse_canvas_x: RwSignal<f64>,
    pub mouse_in_label_area: RwSignal<bool>,
    pub label_hover_opacity: RwSignal<f64>,
    pub follow_cursor: RwSignal<bool>,
    pub follow_suspended: RwSignal<bool>,
    pub follow_visible_since: RwSignal<Option<f64>>,
    pub pre_play_scroll: RwSignal<f64>,
    // Filter EQ (driven by bandpass_mode effect)
    pub filter_enabled: RwSignal<bool>,
    pub filter_band_mode: RwSignal<u8>,
    pub filter_freq_low: RwSignal<f64>,
    pub filter_freq_high: RwSignal<f64>,
    pub filter_db_below: RwSignal<f64>,
    pub filter_db_selected: RwSignal<f64>,
    pub filter_db_harmonics: RwSignal<f64>,
    pub filter_db_above: RwSignal<f64>,
    pub filter_hovering_band: RwSignal<Option<u8>>,
    pub filter_quality: RwSignal<FilterQuality>,
    pub het_cutoff: RwSignal<f64>,
    pub sidebar_collapsed: RwSignal<bool>,
    pub sidebar_width: RwSignal<f64>,
    // Gain
    pub gain_db: RwSignal<f64>,
    pub auto_gain: RwSignal<bool>,

    // ── New signals ──────────────────────────────────────────────────────────

    // Tool
    pub canvas_tool: RwSignal<CanvasTool>,

    // HFR (High Frequency Range) mode
    pub hfr_enabled: RwSignal<bool>,

    // Bandpass
    pub bandpass_mode: RwSignal<BandpassMode>,
    pub bandpass_range: RwSignal<BandpassRange>,

    // Overview
    pub overview_view: RwSignal<OverviewView>,
    pub overview_freq_mode: RwSignal<OverviewFreqMode>,

    // Navigation history (for back/forward buttons in overview)
    pub nav_history: RwSignal<Vec<NavEntry>>,
    pub nav_index: RwSignal<usize>,

    // Bookmarks
    pub bookmarks: RwSignal<Vec<Bookmark>>,
    pub show_bookmark_popup: RwSignal<bool>,

    // Play-from-here time (updated by Spectrogram on scroll/zoom change)
    pub play_from_here_time: RwSignal<f64>,

    // Tile system: incrementing this triggers a spectrogram redraw
    pub tile_ready_signal: RwSignal<u32>,

    /// Generation counter for background preload. Incremented when file/LOD changes
    /// to cancel stale preload jobs.
    pub bg_preload_gen: RwSignal<u32>,

    // Spectrogram display settings (applied at render time, no tile regen needed)
    /// dB floor (default -80.0). Values below this map to black.
    pub spect_floor_db: RwSignal<f32>,
    /// dB range (default 80.0). floor + range = ceiling.
    pub spect_range_db: RwSignal<f32>,
    /// Gamma curve (default 1.0 = linear). <1 = brighter darks, >1 = more contrast.
    pub spect_gamma: RwSignal<f32>,
    /// Additive dB gain offset (default 0.0).
    pub spect_gain_db: RwSignal<f32>,
    /// Show tile debug overlay (borders, LOD labels) on the spectrogram canvas.
    pub debug_tiles: RwSignal<bool>,
    /// FFT window mode for spectrogram computation.
    /// Single size or multi-resolution (different sizes per frequency band).
    pub spect_fft_mode: RwSignal<FftMode>,

    /// Enable reassignment spectrogram (sharper time-frequency localization).
    pub reassign_enabled: RwSignal<bool>,

    // Which floating layer panel is currently open
    pub layer_panel_open: RwSignal<Option<LayerPanel>>,

    // Actual pixel width of the main spectrogram canvas (written by Spectrogram, read by Overview)
    pub spectrogram_canvas_width: RwSignal<f64>,

    // Main panel view mode
    pub main_view: RwSignal<MainView>,

    // Spectrogram drag handles (FF + HET)
    pub spec_drag_handle: RwSignal<Option<SpectrogramHandle>>,
    pub spec_hover_handle: RwSignal<Option<SpectrogramHandle>>,

    // FF frequency range (0.0 = no FF active)
    pub ff_freq_lo: RwSignal<f64>,
    pub ff_freq_hi: RwSignal<f64>,

    // Per-parameter auto flags (true = computed from FF)
    pub het_freq_auto: RwSignal<bool>,
    pub het_cutoff_auto: RwSignal<bool>,
    pub te_factor_auto: RwSignal<bool>,
    pub ps_factor_auto: RwSignal<bool>,
    pub auto_factor_mode: RwSignal<AutoFactorMode>,

    // Microphone (independent listen + record)
    pub mic_listening: RwSignal<bool>,
    pub mic_recording: RwSignal<bool>,
    pub mic_sample_rate: RwSignal<u32>,
    pub mic_samples_recorded: RwSignal<usize>,
    pub mic_bits_per_sample: RwSignal<u16>,
    pub mic_max_sample_rate: RwSignal<u32>, // 0 = auto (device default)
    pub mic_mode: RwSignal<MicMode>,
    pub mic_supported_rates: RwSignal<Vec<u32>>, // actual rates from cpal device query
    /// File index of the currently-recording live file (None if not recording).
    /// Used to update the live file in-place during recording and finalization.
    pub mic_live_file_idx: RwSignal<Option<usize>>,
    /// Wall-clock time (Date.now()) when recording started, for timer display.
    pub mic_recording_start_time: RwSignal<Option<f64>>,
    /// Wrapping counter incremented by setInterval(100ms) while recording.
    pub mic_timer_tick: RwSignal<u32>,
    /// Current mic device name (populated on open or query).
    pub mic_device_name: RwSignal<Option<String>>,
    /// Connection type: "USB", "Internal", "Bluetooth", etc.
    pub mic_connection_type: RwSignal<Option<String>>,
    /// Whether a USB audio device is currently connected.
    pub mic_usb_connected: RwSignal<bool>,
    /// What Auto mode resolved to (Cpal or RawUsb). Ignored when mode is not Auto.
    pub mic_effective_mode: RwSignal<MicMode>,
    /// Target scroll offset during recording. The rAF animation loop interpolates
    /// scroll_offset toward this value for smooth waterfall scrolling.
    pub mic_recording_target_scroll: RwSignal<f64>,
    /// Rightmost spectrogram column with actual data during recording.
    /// Used to clip the canvas so partial tiles don't show black padding.
    pub mic_live_data_cols: RwSignal<usize>,

    // Transient status message (e.g. permission errors)
    pub status_message: RwSignal<Option<String>>,
    pub status_level: RwSignal<StatusLevel>,

    // Debug log entries: (timestamp_ms, level, message)
    pub debug_log_entries: RwSignal<Vec<(f64, String, String)>>,

    // Platform detection
    pub is_mobile: RwSignal<bool>,
    pub is_tauri: bool,

    // XC browser
    pub xc_browser_open: RwSignal<bool>,

    // HFR saved settings (restored when toggling HFR back on)
    pub hfr_saved_ff_lo: RwSignal<Option<f64>>,
    pub hfr_saved_ff_hi: RwSignal<Option<f64>>,
    pub hfr_saved_playback_mode: RwSignal<Option<PlaybackMode>>,
    pub hfr_saved_bandpass_mode: RwSignal<Option<BandpassMode>>,

    // Axis drag (left axis frequency range selection)
    pub axis_drag_start_freq: RwSignal<Option<f64>>,
    pub axis_drag_current_freq: RwSignal<Option<f64>>,

    // Cursor time at mouse position (for bottom bar feedback)
    pub cursor_time: RwSignal<Option<f64>>,

    // Left sidebar settings page
    pub settings_page_open: RwSignal<bool>,

    // User colormap preference (when not overridden by HFR/flow)
    pub colormap_preference: RwSignal<ColormapPreference>,
    // Chromagram colormap mode
    pub chroma_colormap: RwSignal<ChromaColormap>,
    // Chromagram display: brightness multiplier (1.0 = default)
    pub chroma_gain: RwSignal<f32>,
    // Chromagram display: gamma curve (1.0 = linear)
    pub chroma_gamma: RwSignal<f32>,
    // Colormap preference used when HFR mode is active
    pub hfr_colormap_preference: RwSignal<ColormapPreference>,
    // When false, the Range button is hidden at full range
    pub always_show_view_range: RwSignal<bool>,

    // Notch noise filtering
    pub notch_enabled: RwSignal<bool>,
    pub notch_bands: RwSignal<Vec<crate::dsp::notch::NoiseBand>>,
    pub notch_detecting: RwSignal<bool>,
    pub notch_profile_name: RwSignal<String>,
    pub notch_hovering_band: RwSignal<Option<usize>>,
    /// Harmonic suppression strength (0.0–1.0). Attenuates 2x and 3x harmonics of noise.
    pub notch_harmonic_suppression: RwSignal<f64>,

    // Spectral subtraction noise reduction
    pub noise_reduce_enabled: RwSignal<bool>,
    pub noise_reduce_strength: RwSignal<f64>,
    pub noise_reduce_floor: RwSignal<Option<crate::dsp::spectral_sub::NoiseFloor>>,
    pub noise_reduce_learning: RwSignal<bool>,

    // Pulse detection
    pub detected_pulses: RwSignal<Vec<crate::dsp::pulse_detect::DetectedPulse>>,
    pub pulse_overlay_enabled: RwSignal<bool>,
    pub selected_pulse_index: RwSignal<Option<usize>>,
    pub pulse_detecting: RwSignal<bool>,

    // Display-affecting checkboxes (spectrogram intensity settings)
    pub display_auto_gain: RwSignal<bool>,
    pub display_eq: RwSignal<bool>,
    pub display_noise_filter: RwSignal<bool>,
    // ZC saved display settings (restored when entering ZC; defaults: eq=true, noise=true)
    pub zc_saved_display_auto_gain: RwSignal<bool>,
    pub zc_saved_display_eq: RwSignal<bool>,
    pub zc_saved_display_noise_filter: RwSignal<bool>,
    // Normal saved display settings (restored when leaving ZC; defaults: all false)
    pub normal_saved_display_auto_gain: RwSignal<bool>,
    pub normal_saved_display_eq: RwSignal<bool>,
    pub normal_saved_display_noise_filter: RwSignal<bool>,
}

fn detect_tauri() -> bool {
    let Some(window) = web_sys::window() else { return false };
    js_sys::Reflect::get(&window, &wasm_bindgen::JsValue::from_str("__TAURI_INTERNALS__"))
        .map(|v| !v.is_undefined())
        .unwrap_or(false)
}

fn detect_mobile() -> bool {
    let window = match web_sys::window() {
        Some(w) => w,
        None => return false,
    };
    if let Ok(ua) = window.navigator().user_agent() {
        let ua_lower = ua.to_lowercase();
        if ua_lower.contains("android") || ua_lower.contains("iphone") || ua_lower.contains("ipad") || ua_lower.contains("mobile") {
            return true;
        }
    }
    if let Ok(w) = window.inner_width() {
        if let Some(w) = w.as_f64() {
            return w < 768.0;
        }
    }
    false
}

impl AppState {
    pub fn new() -> Self {
        let s = Self {
            files: RwSignal::new(Vec::new()),
            current_file_index: RwSignal::new(None),
            selection: RwSignal::new(None),
            playback_mode: RwSignal::new(PlaybackMode::Normal),
            het_frequency: RwSignal::new(45_000.0),
            te_factor: RwSignal::new(10.0),
            zoom_level: RwSignal::new(1.0),
            scroll_offset: RwSignal::new(0.0),
            is_playing: RwSignal::new(false),
            playhead_time: RwSignal::new(0.0),
            loading_count: RwSignal::new(0),
            ps_factor: RwSignal::new(10.0),
            zc_factor: RwSignal::new(8.0),
            het_interacting: RwSignal::new(false),
            is_dragging: RwSignal::new(false),
            spectrogram_display: RwSignal::new(SpectrogramDisplay::FlowOptical),
            flow_enabled: RwSignal::new(false),
            right_sidebar_tab: RwSignal::new(RightSidebarTab::Metadata),
            right_sidebar_collapsed: RwSignal::new(true),
            right_sidebar_width: RwSignal::new(220.0),
            right_sidebar_dropdown_open: RwSignal::new(false),
            flow_intensity_gate: RwSignal::new(0.5),
            flow_gate: RwSignal::new(0.75),
            flow_opacity: RwSignal::new(0.75),
            flow_shift_gain: RwSignal::new(3.0),
            flow_color_gamma: RwSignal::new(1.0),
            flow_color_scheme: RwSignal::new(FlowColorScheme::default()),
            min_display_freq: RwSignal::new(None),
            max_display_freq: RwSignal::new(None),
            mouse_freq: RwSignal::new(None),
            mouse_canvas_x: RwSignal::new(0.0),
            mouse_in_label_area: RwSignal::new(false),
            label_hover_opacity: RwSignal::new(0.0),
            follow_cursor: RwSignal::new(true),
            follow_suspended: RwSignal::new(false),
            follow_visible_since: RwSignal::new(None),
            pre_play_scroll: RwSignal::new(0.0),
            filter_enabled: RwSignal::new(false),
            filter_band_mode: RwSignal::new(3),
            filter_freq_low: RwSignal::new(20_000.0),
            filter_freq_high: RwSignal::new(60_000.0),
            filter_db_below: RwSignal::new(-40.0),
            filter_db_selected: RwSignal::new(0.0),
            filter_db_harmonics: RwSignal::new(-30.0),
            filter_db_above: RwSignal::new(-40.0),
            filter_hovering_band: RwSignal::new(None),
            filter_quality: RwSignal::new(FilterQuality::HQ),
            het_cutoff: RwSignal::new(15_000.0),
            sidebar_collapsed: RwSignal::new(false),
            sidebar_width: RwSignal::new(220.0),
            gain_db: RwSignal::new(0.0),
            auto_gain: RwSignal::new(true),

            // New
            canvas_tool: RwSignal::new(CanvasTool::Hand),
            hfr_enabled: RwSignal::new(false),
            bandpass_mode: RwSignal::new(BandpassMode::Auto),
            bandpass_range: RwSignal::new(BandpassRange::FollowFocus),
            overview_view: RwSignal::new(OverviewView::Spectrogram),
            overview_freq_mode: RwSignal::new(OverviewFreqMode::All),
            nav_history: RwSignal::new(Vec::new()),
            nav_index: RwSignal::new(0),
            bookmarks: RwSignal::new(Vec::new()),
            show_bookmark_popup: RwSignal::new(false),
            play_from_here_time: RwSignal::new(0.0),
            tile_ready_signal: RwSignal::new(0),
            bg_preload_gen: RwSignal::new(0),
            spect_floor_db: RwSignal::new(-80.0),
            spect_range_db: RwSignal::new(80.0),
            spect_gamma: RwSignal::new(1.0),
            spect_gain_db: RwSignal::new(0.0),
            debug_tiles: RwSignal::new(false),
            spect_fft_mode: RwSignal::new(FftMode::Single(1024)),
            reassign_enabled: RwSignal::new(false),
            layer_panel_open: RwSignal::new(None),
            spectrogram_canvas_width: RwSignal::new(1000.0),
            main_view: RwSignal::new(MainView::Spectrogram),
            spec_drag_handle: RwSignal::new(None),
            spec_hover_handle: RwSignal::new(None),
            ff_freq_lo: RwSignal::new(0.0),
            ff_freq_hi: RwSignal::new(0.0),
            het_freq_auto: RwSignal::new(true),
            het_cutoff_auto: RwSignal::new(true),
            te_factor_auto: RwSignal::new(true),
            ps_factor_auto: RwSignal::new(true),
            auto_factor_mode: RwSignal::new(AutoFactorMode::Target3k),
            mic_listening: RwSignal::new(false),
            mic_recording: RwSignal::new(false),
            mic_sample_rate: RwSignal::new(0),
            mic_samples_recorded: RwSignal::new(0),
            mic_bits_per_sample: RwSignal::new(16),
            mic_max_sample_rate: RwSignal::new(0),
            mic_mode: RwSignal::new(if detect_tauri() { MicMode::Auto } else { MicMode::Browser }),
            mic_supported_rates: RwSignal::new(Vec::new()),
            mic_live_file_idx: RwSignal::new(None),
            mic_recording_start_time: RwSignal::new(None),
            mic_timer_tick: RwSignal::new(0),
            mic_device_name: RwSignal::new(None),
            mic_connection_type: RwSignal::new(None),
            mic_usb_connected: RwSignal::new(false),
            mic_effective_mode: RwSignal::new(if detect_tauri() { MicMode::Cpal } else { MicMode::Browser }),
            mic_recording_target_scroll: RwSignal::new(0.0),
            mic_live_data_cols: RwSignal::new(0),
            status_message: RwSignal::new(None),
            status_level: RwSignal::new(StatusLevel::Error),
            debug_log_entries: RwSignal::new(Vec::new()),
            is_mobile: RwSignal::new(detect_mobile()),
            is_tauri: detect_tauri(),
            xc_browser_open: RwSignal::new(false),
            hfr_saved_ff_lo: RwSignal::new(None),
            hfr_saved_ff_hi: RwSignal::new(None),
            hfr_saved_playback_mode: RwSignal::new(None),
            hfr_saved_bandpass_mode: RwSignal::new(None),
            axis_drag_start_freq: RwSignal::new(None),
            axis_drag_current_freq: RwSignal::new(None),
            cursor_time: RwSignal::new(None),
            settings_page_open: RwSignal::new(false),
            colormap_preference: RwSignal::new(ColormapPreference::Viridis),
            chroma_colormap: RwSignal::new(ChromaColormap::PitchClass),
            chroma_gain: RwSignal::new(1.0),
            chroma_gamma: RwSignal::new(1.0),
            hfr_colormap_preference: RwSignal::new(ColormapPreference::Inferno),
            always_show_view_range: RwSignal::new(false),

            notch_enabled: RwSignal::new(false),
            notch_bands: RwSignal::new(Vec::new()),
            notch_detecting: RwSignal::new(false),
            notch_profile_name: RwSignal::new(String::new()),
            notch_hovering_band: RwSignal::new(None),
            notch_harmonic_suppression: RwSignal::new(0.0),

            noise_reduce_enabled: RwSignal::new(false),
            noise_reduce_strength: RwSignal::new(1.0),
            noise_reduce_floor: RwSignal::new(None),
            noise_reduce_learning: RwSignal::new(false),

            detected_pulses: RwSignal::new(Vec::new()),
            pulse_overlay_enabled: RwSignal::new(true),
            selected_pulse_index: RwSignal::new(None),
            pulse_detecting: RwSignal::new(false),

            display_auto_gain: RwSignal::new(false),
            display_eq: RwSignal::new(false),
            display_noise_filter: RwSignal::new(false),
            zc_saved_display_auto_gain: RwSignal::new(false),
            zc_saved_display_eq: RwSignal::new(true),
            zc_saved_display_noise_filter: RwSignal::new(true),
            normal_saved_display_auto_gain: RwSignal::new(false),
            normal_saved_display_eq: RwSignal::new(false),
            normal_saved_display_noise_filter: RwSignal::new(false),
        };

        // On mobile, start with sidebar collapsed
        if s.is_mobile.get_untracked() {
            s.sidebar_collapsed.set(true);
        }

        s
    }

    pub fn current_file(&self) -> Option<LoadedFile> {
        let files = self.files.get();
        let idx = self.current_file_index.get()?;
        files.get(idx).cloned()
    }

    pub fn show_info_toast(&self, msg: impl Into<String>) {
        self.status_level.set(StatusLevel::Info);
        self.status_message.set(Some(msg.into()));
    }

    pub fn show_error_toast(&self, msg: impl Into<String>) {
        self.status_level.set(StatusLevel::Error);
        self.status_message.set(Some(msg.into()));
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

    /// Temporarily suspend follow-cursor so the user can scroll freely.
    /// Re-engagement happens automatically once the playhead is visible for 500ms.
    pub fn suspend_follow(&self) {
        if self.follow_cursor.get_untracked() && self.is_playing.get_untracked() {
            self.follow_suspended.set(true);
            self.follow_visible_since.set(None);
        }
    }

    pub fn compute_auto_gain(&self) -> f64 {
        let files = self.files.get();
        let idx = self.current_file_index.get();
        let Some(file) = idx.and_then(|i| files.get(i)) else { return 0.0 };
        let peak = file.audio.samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        if peak < 1e-10 { return 0.0; }
        let peak_db = 20.0 * (peak as f64).log10();
        // Cap at +30 dB to avoid extreme amplification of very quiet recordings
        (-3.0 - peak_db).min(30.0)
    }
}
