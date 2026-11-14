//! Time-expansion corrections the user applies or removes in the Info panel.
//!
//! A file whose header rate is one tenth of a full-spectrum rate and too low
//! for its species' calls gets the species note "Probably time-expanded ×10"
//! ([`crate::species_check`], case T). The note has a button that reads the
//! file at ten times its rate, and the time-expansion note that replaces it
//! has a button that goes back to the file's own rate
//! ([`crate::rate_notes::NoteAction`]).
//!
//! A file corrected when it was opened, from its own metadata (GUANO, `wamd`)
//! or from the site it came from (Wikimedia Commons, Find recordings), has
//! the same "Remove" button. After it is pressed, the note says what the
//! metadata states and has a button that makes that correction again. All
//! these buttons call [`set_time_expansion`].
//!
//! The file is changed in place: it keeps its id, its place in the file list,
//! its annotations and its settings. Its samples are not changed, only the
//! rate they are read at ([`crate::types::AudioData::set_time_expansion`]).
//! Every time and frequency the app holds for the file is converted with a
//! [`RateChange`], so annotations, the selection and the view stay on the
//! same samples.
//!
//! A change lasts until the file is closed or the page is reloaded: opening
//! the file again makes the correction it was opened with
//! ([`crate::state::LoadedFile::opened_time_expansion`]), or none. So
//! annotation files use the seconds and Hz the file opens in: [`to_file_units`]
//! converts a set before it is saved or exported (autosave, sidecars,
//! "Export .batm", project files; "Export for XC" converts each annotation
//! with [`to_file_rate`]), and [`from_file_units`] converts one that is loaded
//! or imported while the rate differs. Annota-JSON from Xeno-canto, or
//! imported by the user, is converted the same way
//! ([`annota_from_file_units_at`]).

use crate::annotations::{AnnotationKind, AnnotationSet, AudioFileMetadata};
use crate::dsp::notch::{NoiseBand, NoiseProfile};
use crate::dsp::spectral_sub::NoiseFloor;
use crate::focus_stack::FocusRange;
use crate::state::{AppState, BandpassRange, FileSettings, LoadedFile, Selection, UndoStack};
use crate::types::{AudioData, TimeExpansion, TimeExpansionOrigin};
use oversample_ipc::annota::AnnotaSet;

/// The factor the species note's button applies.
pub const TENFOLD: f64 = 10.0;

/// A change of the rate a file's samples are read at, from `old_rate` to
/// `new_rate` (Hz). A sample's time is multiplied by `old_rate / new_rate`
/// and a frequency by `new_rate / old_rate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RateChange {
    pub old_rate: u32,
    pub new_rate: u32,
}

impl RateChange {
    pub fn new(old_rate: u32, new_rate: u32) -> Self {
        Self { old_rate, new_rate }
    }

    /// The change back.
    pub fn inverse(self) -> Self {
        Self::new(self.new_rate, self.old_rate)
    }

    /// A time in seconds from the start of the file.
    pub fn time(self, secs: f64) -> f64 {
        secs * f64::from(self.old_rate) / f64::from(self.new_rate)
    }

    /// A frequency in Hz.
    pub fn freq(self, hz: f64) -> f64 {
        hz * f64::from(self.new_rate) / f64::from(self.old_rate)
    }

    fn freq_opt(self, hz: Option<f64>) -> Option<f64> {
        hz.map(|f| self.freq(f))
    }

    /// A rate in Hz that may be the old rate: such a rate becomes the new one.
    /// Any other rate is left alone.
    fn rate(self, hz: u32) -> u32 {
        if hz == self.old_rate {
            self.new_rate
        } else {
            hz
        }
    }

    pub fn selection(self, s: Selection) -> Selection {
        Selection {
            time_start: self.time(s.time_start),
            time_end: self.time(s.time_end),
            freq_low: self.freq_opt(s.freq_low),
            freq_high: self.freq_opt(s.freq_high),
        }
    }

    /// An HFR band or other frequency focus. The inactive range (0 to 0)
    /// stays inactive.
    pub fn focus_range(self, r: FocusRange) -> FocusRange {
        FocusRange::new(self.freq(r.lo), self.freq(r.hi))
    }

    pub fn annotation_kind(self, kind: &mut AnnotationKind) {
        match kind {
            AnnotationKind::Region(r) => {
                r.time_start = self.time(r.time_start);
                r.time_end = self.time(r.time_end);
                r.freq_low = self.freq_opt(r.freq_low);
                r.freq_high = self.freq_opt(r.freq_high);
            }
            AnnotationKind::Marker(m) => m.time = self.time(m.time),
            AnnotationKind::Measurement(m) => {
                m.start_time = self.time(m.start_time);
                m.end_time = self.time(m.end_time);
                m.start_freq = self.freq(m.start_freq);
                m.end_freq = self.freq(m.end_freq);
            }
            AnnotationKind::Group(_) => {}
        }
    }

    /// A notch filter band. Its Q (centre over bandwidth) is unchanged.
    pub fn noise_band(self, band: &mut NoiseBand) {
        band.center_hz = self.freq(band.center_hz);
        band.bandwidth_hz = self.freq(band.bandwidth_hz);
    }

    /// A learned noise floor. Its values are per FFT bin of the same samples,
    /// so they stay; only the rate it is marked with changes, since spectral
    /// subtraction skips a floor whose rate is not the file's.
    pub fn noise_floor(self, floor: &mut NoiseFloor) {
        floor.sample_rate = self.rate(floor.sample_rate);
        floor.analysis_duration_secs = self.time(floor.analysis_duration_secs);
    }

    pub fn noise_profile(self, profile: &mut NoiseProfile) {
        for band in &mut profile.bands {
            self.noise_band(band);
        }
        profile.source_sample_rate = self.rate(profile.source_sample_rate);
        if let Some(floor) = profile.noise_floor.as_mut() {
            self.noise_floor(floor);
        }
    }

    /// The rate and duration in a file's audio metadata.
    pub fn audio_metadata(self, meta: &mut AudioFileMetadata) {
        meta.sample_rate = self.rate(meta.sample_rate);
        meta.duration_secs = self.time(meta.duration_secs);
    }

    /// A file's annotation set: every annotation, the noise profile, and the
    /// rate and duration in its audio metadata. The per-file lowest bat
    /// frequency is a fact about bats, not about the file, so it stays.
    pub fn annotation_set(self, set: &mut AnnotationSet) {
        for ann in &mut set.annotations {
            self.annotation_kind(&mut ann.kind);
        }
        if let Some(profile) = set.noise_profile.as_mut() {
            self.noise_profile(profile);
        }
        if let Some(meta) = set.audio_metadata.as_mut() {
            self.audio_metadata(meta);
        }
    }

    /// The times and frequencies of an Annota-JSON set (Xeno-canto's format).
    pub fn annota_set(self, set: &mut AnnotaSet) {
        for ann in &mut set.annotations {
            ann.start_time = ann.start_time.map(|t| self.time(t));
            ann.end_time = ann.end_time.map(|t| self.time(t));
            ann.frequency_low = self.freq_opt(ann.frequency_low);
            ann.frequency_high = self.freq_opt(ann.frequency_high);
        }
    }

    /// The annotation sets that undo and redo would restore for `file_id`.
    pub fn undo_stack(self, stack: &mut UndoStack, file_id: u64) {
        for entry in stack.undo.iter_mut().chain(stack.redo.iter_mut()) {
            if entry.file_id == file_id {
                if let Some(set) = entry.snapshot.as_mut() {
                    self.annotation_set(set);
                }
            }
        }
    }

    /// The bandpass (PASS) band, `(low, high)` in Hz. Only a band the user
    /// locked to a range of their own (`BandpassRange::Locked`) is converted.
    /// A band that follows the HFR band takes the HFR band's range when PASS
    /// is on, and the HFR band is converted, so its stored range stays: often
    /// the app's 20 to 60 kHz default, which as 200 to 600 kHz would lie above
    /// the corrected file's top frequency.
    pub fn bandpass(self, range: BandpassRange, band: (f64, f64)) -> (f64, f64) {
        match range {
            BandpassRange::Locked => (self.freq(band.0), self.freq(band.1)),
            BandpassRange::FollowFocus => band,
        }
    }

    /// A file's saved notch bands, bandpass band and noise floor. Gain and
    /// the other settings do not depend on the rate.
    pub fn file_settings(self, settings: &mut FileSettings) {
        for band in &mut settings.notch_bands {
            self.noise_band(band);
        }
        if let Some(floor) = settings.noise_reduce_floor.as_mut() {
            self.noise_floor(floor);
        }
        let filter = &mut settings.filter;
        (filter.freq_low, filter.freq_high) =
            self.bandpass(filter.bandpass_range, (filter.freq_low, filter.freq_high));
    }
}

/// The change from the rate `audio` is read at now to the rate it opens at,
/// or `None` when they are the same. `opened` is the correction made when
/// the file was opened ([`LoadedFile::opened_time_expansion`]); the file
/// opens at the rate its header states when there was none.
pub fn to_opening_rate(audio: &AudioData, opened: Option<TimeExpansion>) -> Option<RateChange> {
    let opening = opened.map_or(audio.stored_rate(), TimeExpansion::corrected_rate);
    (audio.sample_rate != opening).then(|| RateChange::new(audio.sample_rate, opening))
}

/// The change from the file's current seconds and Hz to those of its
/// annotation files: the rate it opens at ([`to_opening_rate`]). `None`
/// while the file is at that rate.
pub fn to_file_rate(file: &LoadedFile) -> Option<RateChange> {
    to_opening_rate(&file.audio, file.opened_time_expansion)
}

/// [`to_file_rate`] for file `file_idx`. Untracked.
pub fn to_file_rate_at(state: AppState, file_idx: usize) -> Option<RateChange> {
    use crate::state::store_fields::*;
    use leptos::prelude::*;
    state
        .library
        .files()
        .with_untracked(|files| files.get(file_idx).and_then(to_file_rate))
}

/// `set` in the seconds and Hz the file opens in, for saving. Every
/// annotation file the app writes uses them, so the saved file still fits
/// the audio file when it is opened again: at the rate its header states,
/// or corrected again from its metadata.
pub fn to_file_units(
    audio: &AudioData,
    opened: Option<TimeExpansion>,
    mut set: AnnotationSet,
) -> AnnotationSet {
    if let Some(rc) = to_opening_rate(audio, opened) {
        rc.annotation_set(&mut set);
    }
    set
}

/// [`to_file_units`] for file `file_idx`. Untracked.
pub fn to_file_units_at(state: AppState, file_idx: usize, set: AnnotationSet) -> AnnotationSet {
    match to_file_rate_at(state, file_idx) {
        Some(rc) => {
            let mut set = set;
            rc.annotation_set(&mut set);
            set
        }
        None => set,
    }
}

/// A set read from an annotation file (a sidecar, autosave or an imported
/// .batm), in the seconds and Hz the file opens in, converted to the rate
/// the file is read at now.
///
/// A set whose audio metadata states the current rate is already in the
/// current units and is left as it is. Only an "Export .batm" made while
/// the user's correction was on, before exports were converted, can be one.
pub fn from_file_units(
    audio: &AudioData,
    opened: Option<TimeExpansion>,
    mut set: AnnotationSet,
) -> AnnotationSet {
    let Some(rc) = to_opening_rate(audio, opened) else {
        return set;
    };
    let in_current_units = set
        .audio_metadata
        .as_ref()
        .is_some_and(|m| m.sample_rate == audio.sample_rate);
    if !in_current_units {
        rc.inverse().annotation_set(&mut set);
    }
    set
}

/// [`from_file_units`] for file `file_idx`. Untracked.
pub fn from_file_units_at(state: AppState, file_idx: usize, set: AnnotationSet) -> AnnotationSet {
    use crate::state::store_fields::*;
    use leptos::prelude::*;
    state
        .library
        .files()
        .with_untracked(|files| match files.get(file_idx) {
            Some(f) => from_file_units(&f.audio, f.opened_time_expansion, set),
            None => set,
        })
}

/// An Annota-JSON set (from Xeno-canto, or a file the user imported), taken
/// to be in the seconds and Hz the file opens in, converted to the rate file
/// `file_idx` is read at now. "Export for XC" writes those units
/// ([`to_file_rate`]), so a set exported and imported again lands on the
/// same samples. Untracked.
pub fn annota_from_file_units_at(
    state: AppState,
    file_idx: usize,
    mut set: AnnotaSet,
) -> AnnotaSet {
    if let Some(rc) = to_file_rate_at(state, file_idx) {
        rc.inverse().annota_set(&mut set);
    }
    set
}

/// Read file `file_id` at a time-expansion correction pressed in the Info
/// panel, and convert everything the app holds for it. `te` is the factor
/// and what gave it: the user's ×10, or the correction the file was opened
/// with, made again. `None` reads the file at the rate its header states.
/// Returns `false` and changes nothing when the file is gone, still loading,
/// its rate can't be changed (a streamed file other than WAV, a `.zc` file),
/// or it is already at that rate.
///
/// Playback stops, and the "Playing 1:1" card closes. Annotations (and the
/// copies undo would restore), the selection, the view, the HFR band,
/// bookmarks and the filter bands are converted so they stay on the same
/// samples. Call detection results for
/// the file are dropped, since the detector ran at the old rate.
///
/// A correction remembers whether HFR was on. Removing it turns HFR off when
/// HFR was off at the correction and is on now: choosing PS or another mode
/// that needs HFR on the corrected file turns HFR on, and Remove undoes that
/// with the rest of the correction. The correction made when the file was
/// opened was not pressed, so the first Remove of it leaves HFR as it is.
pub fn set_time_expansion(
    state: AppState,
    file_id: u64,
    te: Option<(f64, TimeExpansionOrigin)>,
) -> bool {
    use crate::state::store_fields::*;
    use leptos::prelude::*;

    let Some(idx) = state.file_idx_for_id(file_id) else {
        return false;
    };
    let Some((mut audio, total_cols)) = state.library.files().with_untracked(|files| {
        files
            .get(idx)
            .filter(|f| f.loading_id.is_none())
            .map(|f| (f.audio.clone(), f.spectrogram.total_columns))
    }) else {
        return false;
    };
    let old_rate = audio.sample_rate;
    let (factor, origin) = match te {
        Some((factor, origin)) => (Some(factor), origin),
        None => (None, TimeExpansionOrigin::User),
    };
    if !audio.set_time_expansion(factor, origin) || audio.sample_rate == old_rate {
        return false;
    }
    let hfr_on = state
        .viewmode
        .focus_stack()
        .with_untracked(|s| s.hfr_enabled());
    let rc = RateChange::new(old_rate, audio.sample_rate);
    let is_current = state.library.current_index().get_untracked() == Some(idx);

    // Chunks already scheduled are at the old rate. Stopping first also puts
    // the scroll position back where playback started, in the old seconds,
    // before it is converted below.
    if is_current && state.playback.is_playing().get_untracked() {
        crate::audio::playback::stop(&state);
    }
    // The "Playing 1:1" card above Play says whether the file holds sound
    // above hearing range, which the new rate changes.
    if is_current {
        state.dialogs.hfr_nudge().set(false);
    }

    const HOP_SIZE: usize = 512;
    let fft_size = state
        .spect
        .fft_mode()
        .get_untracked()
        .fft_for_lod(crate::canvas::tile_cache::LOD_BASELINE);
    // The display's DC offset is measured over the first seconds of the file,
    // so it is measured again at the new rate.
    let display_dc = crate::canvas::display_dc::head_offset(&audio);
    let preview = crate::dsp::fft::compute_preview(&audio, 256, 128, display_dc);
    let rate = f64::from(audio.sample_rate);

    let mut name = String::new();
    let mut hfr_on_at_correction = None;
    state.library.files().update(|files| {
        let Some(f) = files.get_mut(idx) else {
            return;
        };
        f.audio = audio;
        f.spectrogram.freq_resolution = rate / fft_size as f64;
        f.spectrogram.time_resolution = HOP_SIZE as f64 / rate;
        f.spectrogram.max_freq = rate / 2.0;
        // The column count stays: a column is a fixed number of samples.
        f.spectrogram.sample_rate = f.audio.sample_rate;
        f.preview = Some(preview);
        f.overview_image = None;
        f.heterodyne_hint = f.site_metadata.as_ref().and_then(|m| {
            crate::heterodyne::file_hint(
                &m.fields,
                f.audio.metadata.time_expansion,
                f.audio.sample_rate,
            )
        });
        f.species_range = crate::species_check::file_finding(f);
        f.min_display_freq = rc.freq_opt(f.min_display_freq);
        f.max_display_freq = rc.freq_opt(f.max_display_freq);
        rc.file_settings(&mut f.settings);
        hfr_on_at_correction = f.hfr_on_at_correction;
        f.hfr_on_at_correction = factor.is_some().then_some(hfr_on);
        name = f.name.clone();
    });
    // NB: each store field is written on its own, never inside another's
    // update closure: `files`, `current_index` and `per_file_view` are
    // siblings in the `library` store, and a nested write is lost.
    state.library.per_file_view().update(|m| {
        if let Some(scope) = m.get_mut(&file_id) {
            if let Some(view) = scope.view.as_mut() {
                view.scroll_offset = rc.time(view.scroll_offset);
            }
            if let Some(focus) = scope.focus.as_mut() {
                focus.user_range = rc.focus_range(focus.user_range);
                for layer in &mut focus.overrides {
                    layer.range = rc.focus_range(layer.range);
                }
            }
        }
    });
    state.annotations.store().update(|store| {
        if let Some(set) = store.get_mut(file_id) {
            rc.annotation_set(set);
        }
    });
    state
        .annotations
        .undo_stack()
        .update(|stack| rc.undo_stack(stack, file_id));

    if is_current {
        convert_current_file_signals(state, rc);
    }

    crate::canvas::display_dc::reseed_mono(file_id, display_dc);
    // Resonator, chromagram and display-range tiles depend on Hz. All tile
    // caches are cleared, as when a file is closed; the visible tiles are
    // drawn again on demand.
    crate::canvas::tile_cache::clear_all_caches();
    crate::canvas::spectral_store::clear_file(idx);
    crate::canvas::spectral_store::init(idx, total_cols, fft_size);
    wasm_bindgen_futures::spawn_local(
        crate::components::file_sidebar::streaming_load::build_streaming_overview(state, idx, name),
    );

    // The detector searched the file at the old rate. A search still running
    // stops when the generation changes.
    let detection_is_for_file = state
        .call_detection
        .scanned()
        .with_untracked(|s| s.as_ref().is_some_and(|(id, _)| *id == file_id))
        || state
            .call_detection
            .scanning()
            .with_untracked(|s| s.as_ref().is_some_and(|(id, _)| *id == file_id));
    if detection_is_for_file {
        state
            .call_detection
            .scan_generation()
            .update(|g| *g = g.wrapping_add(1));
        state.call_detection.scanning().set(None);
        state.call_detection.scanned().set(None);
        state.call_detection.detected().set(Vec::new());
        state.call_detection.trains().set(Vec::new());
        state.call_detection.selected_index().set(None);
        state.call_detection.progress().set(0.0);
    }

    // Timeline segments hold each file's duration.
    if state.timeline.active().get_untracked().is_some() {
        let sel = state.timeline.selected_file_indices().get_untracked();
        let rebuilt = state
            .library
            .files()
            .with_untracked(|files| crate::timeline::TimelineView::from_files(&sel, files));
        if rebuilt.is_none() {
            state.timeline.active_track().set(None);
        }
        state.timeline.active().set(rebuilt);
    }

    // The effect that clamps the HFR band to the file's Nyquist frequency
    // runs when the current file changes, which it does not here.
    state.resync_focus_outputs();

    // HFR mode is global (app.rs keeps it when the current file changes).
    // This only undoes what the correction led to: HFR turned on while the
    // file was corrected, by the user or by a mode that needs it. The band
    // was converted above, so HFR turns off with the band in the new units,
    // and turns on again there.
    if factor.is_none()
        && is_current
        && hfr_on_at_correction == Some(false)
        && state
            .viewmode
            .focus_stack()
            .with_untracked(|s| s.hfr_enabled())
    {
        state.toggle_hfr();
    }
    state
        .viewmode
        .tile_ready_signal()
        .update(|n| *n = n.wrapping_add(1));
    true
}

/// Convert the signals that hold the current file's view, selection and
/// filter settings. The file-switch effect saves these into the file when
/// the user moves to another file, so they must be in the new units.
fn convert_current_file_signals(state: AppState, rc: RateChange) {
    use crate::state::store_fields::*;
    use leptos::prelude::*;

    // The file's own vertical range was converted first, so the effect that
    // copies the view's range into the file finds nothing to write.
    let min = state.view.min_display_freq().get_untracked();
    let max = state.view.max_display_freq().get_untracked();
    state.view.min_display_freq().set(rc.freq_opt(min));
    state.view.max_display_freq().set(rc.freq_opt(max));
    // Zoom is in pixels per spectrogram column, and a column is a fixed
    // number of samples, so the zoom stays and only the scroll position
    // (seconds) changes.
    let scroll = state.view.scroll_offset().get_untracked();
    state.view.scroll_offset().set(rc.time(scroll));

    let sel = state.interaction.selection().get_untracked();
    state
        .interaction
        .selection()
        .set(sel.map(|s| rc.selection(s)));
    let last = state.interaction.last_selection().get_untracked();
    state
        .interaction
        .last_selection()
        .set(last.map(|s| rc.selection(s)));
    let active = state.playback.active_selection().get_untracked();
    state
        .playback
        .active_selection()
        .set(active.map(|s| rc.selection(s)));
    let playhead = state.playback.playhead_time().get_untracked();
    state.playback.playhead_time().set(rc.time(playhead));
    let from_here = state.playback.from_here_time().get_untracked();
    state.playback.from_here_time().set(rc.time(from_here));
    state.viewmode.bookmarks().update(|bms| {
        for bm in bms.iter_mut() {
            bm.time = rc.time(bm.time);
        }
    });
    state.viewmode.nav_history().update(|hist| {
        for entry in hist.iter_mut() {
            entry.scroll_offset = rc.time(entry.scroll_offset);
        }
    });

    state.viewmode.focus_stack().update(|stack| {
        let (user, mut overrides) = stack.focus_selections();
        for layer in &mut overrides {
            layer.range = rc.focus_range(layer.range);
        }
        stack.set_focus_selections(rc.focus_range(user), overrides);
    });

    state.notch.bands().update(|bands| {
        for band in bands.iter_mut() {
            rc.noise_band(band);
        }
    });
    state.noise_reduce.floor().update(|floor| {
        if let Some(floor) = floor.as_mut() {
            rc.noise_floor(floor);
        }
    });
    state.display.auto_noise_floor().update(|floor| {
        if let Some(floor) = floor.as_mut() {
            rc.noise_floor(floor);
        }
    });
    let (lo, hi) = rc.bandpass(
        state.filter.bandpass_range().get_untracked(),
        (
            state.filter.freq_low().get_untracked(),
            state.filter.freq_high().get_untracked(),
        ),
    );
    state.filter.freq_low().set(lo);
    state.filter.freq_high().set(hi);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::annotations::{Annotation, AnnotationId, FileIdentity, Marker, Measurement, Region};
    use crate::audio::source::InMemorySource;
    use crate::state::UndoEntry;
    use crate::types::{FileMetadata, TimeExpansion};
    use std::sync::Arc;

    const TENFOLD_UP: RateChange = RateChange {
        old_rate: 25_600,
        new_rate: 256_000,
    };

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
    }

    fn annotation(kind: AnnotationKind) -> Annotation {
        Annotation {
            id: AnnotationId::from("a"),
            kind,
            created_at: String::new(),
            modified_at: String::new(),
            notes: None,
            parent_id: None,
            sort_order: None,
            tags: Vec::new(),
            label_default: None,
        }
    }

    fn region(t0: f64, t1: f64, f0: Option<f64>, f1: Option<f64>) -> AnnotationKind {
        AnnotationKind::Region(Region {
            time_start: t0,
            time_end: t1,
            freq_low: f0,
            freq_high: f1,
            label: None,
            color: None,
            locked: None,
        })
    }

    fn identity() -> FileIdentity {
        crate::file_identity::identity_layer1("case-t.wav", 51_244)
    }

    /// A set built field by field: `AnnotationSet::new` needs the browser
    /// for its id and dates.
    fn set_with(kinds: Vec<AnnotationKind>) -> AnnotationSet {
        AnnotationSet {
            version: crate::annotations::ANNOTATION_FORMAT_VERSION,
            id: "set".into(),
            app_version: String::new(),
            created_at: None,
            modified_at: None,
            file_identity: identity(),
            audio_metadata: None,
            annotations: kinds.into_iter().map(annotation).collect(),
            lowest_bat_hz: None,
            noise_profile: None,
        }
    }

    /// One second at 25.6 kHz, as loaded.
    fn case_t_audio() -> AudioData {
        let samples = Arc::new(vec![0.0f32; 25_600]);
        AudioData {
            samples: samples.clone(),
            source: Arc::new(InMemorySource {
                samples,
                raw_samples: None,
                sample_rate: 25_600,
                channels: 1,
            }),
            sample_rate: 25_600,
            channels: 1,
            duration_secs: 1.0,
            metadata: FileMetadata::default(),
        }
    }

    #[test]
    fn times_shrink_and_frequencies_grow_by_the_factor() {
        assert_eq!(TENFOLD_UP.time(2.4), 0.24);
        assert_eq!(TENFOLD_UP.freq(4_000.0), 40_000.0);
        let sel = TENFOLD_UP.selection(Selection {
            time_start: 0.2,
            time_end: 0.6,
            freq_low: Some(3_000.0),
            freq_high: None,
        });
        assert!(close(sel.time_start, 0.02) && close(sel.time_end, 0.06));
        assert_eq!(sel.freq_low, Some(30_000.0));
        assert_eq!(sel.freq_high, None);
        // An inactive HFR band stays inactive.
        assert!(!TENFOLD_UP.focus_range(FocusRange::inactive()).is_active());
        let band = TENFOLD_UP.focus_range(FocusRange::new(2_000.0, 6_000.0));
        assert_eq!((band.lo, band.hi), (20_000.0, 60_000.0));
    }

    #[test]
    fn every_annotation_kind_is_converted() {
        let mut set = set_with(vec![
            region(0.5, 0.75, Some(2_000.0), Some(5_000.0)),
            region(0.1, 0.2, None, None),
            AnnotationKind::Marker(Marker {
                time: 0.3,
                label: None,
                color: None,
            }),
            AnnotationKind::Measurement(Measurement {
                start_time: 0.4,
                start_freq: 1_000.0,
                end_time: 0.5,
                end_freq: 1_500.0,
                label: None,
            }),
        ]);
        set.lowest_bat_hz = Some(11_000.0);
        TENFOLD_UP.annotation_set(&mut set);
        let kinds: Vec<&AnnotationKind> = set.annotations.iter().map(|a| &a.kind).collect();
        match kinds[0] {
            AnnotationKind::Region(r) => {
                assert!(close(r.time_start, 0.05) && close(r.time_end, 0.075));
                assert_eq!((r.freq_low, r.freq_high), (Some(20_000.0), Some(50_000.0)));
            }
            k => panic!("{k:?}"),
        }
        match kinds[1] {
            AnnotationKind::Region(r) => assert_eq!((r.freq_low, r.freq_high), (None, None)),
            k => panic!("{k:?}"),
        }
        match kinds[2] {
            AnnotationKind::Marker(m) => assert!(close(m.time, 0.03)),
            k => panic!("{k:?}"),
        }
        match kinds[3] {
            AnnotationKind::Measurement(m) => {
                assert!(close(m.start_time, 0.04) && close(m.end_time, 0.05));
                assert_eq!((m.start_freq, m.end_freq), (10_000.0, 15_000.0));
            }
            k => panic!("{k:?}"),
        }
        // The lowest bat frequency is about bats, not the file.
        assert_eq!(set.lowest_bat_hz, Some(11_000.0));
    }

    #[test]
    fn apply_then_undo_gives_back_the_annotations() {
        let original = set_with(vec![
            region(0.123_456, 0.987_654, Some(1_234.5), Some(9_876.5)),
            AnnotationKind::Marker(Marker {
                time: 0.333,
                label: None,
                color: None,
            }),
        ]);
        let mut set = original.clone();
        TENFOLD_UP.annotation_set(&mut set);
        TENFOLD_UP.inverse().annotation_set(&mut set);
        for (a, b) in set.annotations.iter().zip(&original.annotations) {
            match (&a.kind, &b.kind) {
                (AnnotationKind::Region(a), AnnotationKind::Region(b)) => {
                    assert!(close(a.time_start, b.time_start));
                    assert!(close(a.time_end, b.time_end));
                    assert!(close(a.freq_low.unwrap(), b.freq_low.unwrap()));
                    assert!(close(a.freq_high.unwrap(), b.freq_high.unwrap()));
                }
                (AnnotationKind::Marker(a), AnnotationKind::Marker(b)) => {
                    assert!(close(a.time, b.time))
                }
                _ => panic!("kinds changed"),
            }
        }
    }

    #[test]
    fn noise_settings_keep_their_bins_and_follow_the_rate() {
        let mut settings = FileSettings::default();
        settings.notch_bands = vec![NoiseBand {
            center_hz: 3_000.0,
            bandwidth_hz: 300.0,
            q: 10.0,
            enabled: true,
            strength_db: 12.0,
        }];
        settings.noise_reduce_floor = Some(NoiseFloor {
            bin_magnitudes: vec![0.1, 0.2],
            fft_size: 4096,
            sample_rate: 25_600,
            analysis_duration_secs: 0.5,
            frame_count: 6,
        });
        settings.filter.bandpass_range = BandpassRange::Locked;
        settings.filter.freq_low = 2_000.0;
        settings.filter.freq_high = 6_000.0;
        TENFOLD_UP.file_settings(&mut settings);
        let band = &settings.notch_bands[0];
        assert_eq!(
            (band.center_hz, band.bandwidth_hz, band.q),
            (30_000.0, 3_000.0, 10.0)
        );
        let floor = settings.noise_reduce_floor.as_ref().unwrap();
        assert_eq!(floor.bin_magnitudes, [0.1, 0.2]);
        assert_eq!((floor.fft_size, floor.sample_rate), (4096, 256_000));
        assert!(close(floor.analysis_duration_secs, 0.05));
        assert_eq!(
            (settings.filter.freq_low, settings.filter.freq_high),
            (20_000.0, 60_000.0)
        );
        // A floor learned at some other rate is left as it is.
        let mut other = NoiseFloor {
            sample_rate: 48_000,
            ..floor.clone()
        };
        TENFOLD_UP.noise_floor(&mut other);
        assert_eq!(other.sample_rate, 48_000);
        // Converted back, the locked band is the user's 2 to 6 kHz again,
        // although 20 to 60 kHz is also the app's default band.
        TENFOLD_UP.inverse().file_settings(&mut settings);
        assert_eq!(
            (settings.filter.freq_low, settings.filter.freq_high),
            (2_000.0, 6_000.0)
        );
    }

    #[test]
    fn a_pass_band_that_follows_the_hfr_band_stays() {
        // The app's default band, which follows the HFR band.
        let mut settings = FileSettings::default();
        assert_eq!(settings.filter.bandpass_range, BandpassRange::FollowFocus);
        TENFOLD_UP.file_settings(&mut settings);
        assert_eq!(
            (settings.filter.freq_low, settings.filter.freq_high),
            (20_000.0, 60_000.0)
        );
        assert_eq!(
            TENFOLD_UP.bandpass(BandpassRange::FollowFocus, (3_000.0, 5_000.0)),
            (3_000.0, 5_000.0)
        );
        assert_eq!(
            TENFOLD_UP.bandpass(BandpassRange::Locked, (3_000.0, 5_000.0)),
            (30_000.0, 50_000.0)
        );
    }

    #[test]
    fn undo_snapshots_of_the_file_are_converted() {
        let mut stack = UndoStack::default();
        let snapshot = |t| Some(set_with(vec![region(t, t + 0.1, Some(1_000.0), None)]));
        stack.undo.push_back(UndoEntry {
            file_id: 7,
            snapshot: snapshot(0.5),
        });
        stack.undo.push_back(UndoEntry {
            file_id: 8,
            snapshot: snapshot(0.5),
        });
        stack.redo.push_back(UndoEntry {
            file_id: 7,
            snapshot: snapshot(0.9),
        });
        stack.undo.push_back(UndoEntry {
            file_id: 7,
            snapshot: None,
        });
        TENFOLD_UP.undo_stack(&mut stack, 7);
        let start = |e: &UndoEntry| match &e.snapshot.as_ref().unwrap().annotations[0].kind {
            AnnotationKind::Region(r) => (r.time_start, r.freq_low),
            _ => unreachable!(),
        };
        let (t, f) = start(&stack.undo[0]);
        assert!(close(t, 0.05));
        assert_eq!(f, Some(10_000.0));
        // Another file's snapshot is left alone.
        assert_eq!(start(&stack.undo[1]), (0.5, Some(1_000.0)));
        assert!(close(start(&stack.redo[0]).0, 0.09));
        assert!(stack.undo[2].snapshot.is_none());
    }

    #[test]
    fn files_on_disk_keep_the_file_rate() {
        let set = set_with(vec![region(0.5, 0.6, Some(4_000.0), Some(5_000.0))]);
        let mut audio = case_t_audio();
        // Not corrected: saved as it is.
        let saved = to_file_units(&audio, None, set.clone());
        match &saved.annotations[0].kind {
            AnnotationKind::Region(r) => assert_eq!(r.time_start, 0.5),
            _ => unreachable!(),
        }
        // Corrected by the user: the in-memory set is in the new units, and
        // the saved one in the file's.
        assert!(audio.set_time_expansion(Some(TENFOLD), TimeExpansionOrigin::User));
        assert_eq!(
            to_opening_rate(&audio, None),
            Some(RateChange::new(256_000, 25_600))
        );
        let mut in_memory = set.clone();
        TENFOLD_UP.annotation_set(&mut in_memory);
        let saved = to_file_units(&audio, None, in_memory);
        match &saved.annotations[0].kind {
            AnnotationKind::Region(r) => {
                assert!(close(r.time_start, 0.5));
                assert!(close(r.freq_low.unwrap(), 4_000.0));
            }
            _ => unreachable!(),
        }
        // Loading the saved set converts it back to the corrected units.
        let loaded = from_file_units(&audio, None, saved);
        match &loaded.annotations[0].kind {
            AnnotationKind::Region(r) => {
                assert!(close(r.time_start, 0.05));
                assert!(close(r.freq_low.unwrap(), 40_000.0));
            }
            _ => unreachable!(),
        }
    }

    fn metadata(sample_rate: u32, duration_secs: f64) -> AudioFileMetadata {
        AudioFileMetadata {
            sample_rate,
            total_samples: 512_000,
            channels: 1,
            duration_secs,
            format: "WAV".into(),
            bits_per_sample: Some(16),
            data_offset: None,
            data_size: None,
            peak_db_30s: None,
            peak_db_full: None,
        }
    }

    fn region_start(set: &AnnotationSet) -> (f64, Option<f64>) {
        match &set.annotations[0].kind {
            AnnotationKind::Region(r) => (r.time_start, r.freq_low),
            k => panic!("{k:?}"),
        }
    }

    #[test]
    fn a_saved_set_states_the_file_rate_and_duration() {
        let mut audio = case_t_audio();
        assert!(audio.set_time_expansion(Some(TENFOLD), TimeExpansionOrigin::User));
        // In memory, the set states the corrected rate and duration.
        let mut set = set_with(vec![region(0.05, 0.06, Some(40_000.0), None)]);
        set.audio_metadata = Some(metadata(256_000, 0.1));
        let saved = to_file_units(&audio, None, set);
        let meta = saved.audio_metadata.as_ref().unwrap();
        assert_eq!(meta.sample_rate, 25_600);
        assert!(close(meta.duration_secs, 1.0));
        let (t, f) = region_start(&saved);
        assert!(close(t, 0.5));
        assert!(close(f.unwrap(), 4_000.0));
        // The project file's copy of the metadata is converted the same way.
        let mut meta = metadata(256_000, 0.1);
        to_opening_rate(&audio, None)
            .unwrap()
            .audio_metadata(&mut meta);
        assert_eq!(meta.sample_rate, 25_600);
        assert!(close(meta.duration_secs, 1.0));
    }

    #[test]
    fn an_imported_set_is_converted_unless_it_states_the_current_rate() {
        let mut audio = case_t_audio();
        assert!(audio.set_time_expansion(Some(TENFOLD), TimeExpansionOrigin::User));
        let file_units = |meta: Option<AudioFileMetadata>| {
            let mut set = set_with(vec![region(0.5, 0.6, Some(4_000.0), None)]);
            set.audio_metadata = meta;
            set
        };
        // Saved at the file's rate, or with no metadata: converted.
        for meta in [Some(metadata(25_600, 1.0)), None] {
            let (t, f) = region_start(&from_file_units(&audio, None, file_units(meta)));
            assert!(close(t, 0.05));
            assert!(close(f.unwrap(), 40_000.0));
        }
        // Saved at the corrected rate: already in the corrected units.
        let (t, f) = region_start(&from_file_units(
            &audio,
            None,
            file_units(Some(metadata(256_000, 0.1))),
        ));
        assert_eq!((t, f), (0.5, Some(4_000.0)));
        // Not corrected: never converted.
        let (t, _) = region_start(&from_file_units(
            &case_t_audio(),
            None,
            file_units(Some(metadata(25_600, 1.0))),
        ));
        assert_eq!(t, 0.5);
    }

    #[test]
    fn annota_json_times_and_frequencies_are_converted() {
        let mut set: AnnotaSet = serde_json::from_str(
            r#"{"set_name": "s", "annotations": [
                {"start_time": 10.0, "end_time": 12.0,
                 "frequency_low": 3000, "frequency_high": 5000},
                {"start_time": 1.0}
            ]}"#,
        )
        .unwrap();
        TENFOLD_UP.annota_set(&mut set);
        let a = &set.annotations[0];
        assert!(close(a.start_time.unwrap(), 1.0) && close(a.end_time.unwrap(), 1.2));
        assert_eq!(
            (a.frequency_low, a.frequency_high),
            (Some(30_000.0), Some(50_000.0))
        );
        let b = &set.annotations[1];
        assert!(close(b.start_time.unwrap(), 0.1));
        assert_eq!((b.end_time, b.frequency_low), (None, None));
    }

    #[test]
    fn a_correction_the_file_opens_with_sets_the_units_on_disk() {
        // Corrected x10 from the file's GUANO when it was opened:
        // annotations are saved in the corrected units, as before.
        let mut audio = case_t_audio();
        assert!(audio.apply_time_expansion(10.0, TimeExpansionOrigin::File));
        let opened = audio.metadata.time_expansion;
        assert_eq!(
            opened,
            Some(TimeExpansion {
                factor: 10.0,
                stored_rate: 25_600,
                origin: TimeExpansionOrigin::File,
            })
        );
        assert_eq!(to_opening_rate(&audio, opened), None);
        let corrected = set_with(vec![region(0.05, 0.06, Some(40_000.0), None)]);
        let saved = to_file_units(&audio, opened, corrected.clone());
        assert_eq!(region_start(&saved), (0.05, Some(40_000.0)));

        // The user removed it: the file is at 25.6 kHz, and a set is saved
        // in the corrected units the file opens in again.
        assert!(audio.set_time_expansion(None, TimeExpansionOrigin::User));
        assert_eq!(
            to_opening_rate(&audio, opened),
            Some(RateChange::new(25_600, 256_000))
        );
        let mut in_memory = corrected.clone();
        TENFOLD_UP.inverse().annotation_set(&mut in_memory);
        let (t, f) = region_start(&in_memory);
        assert!(close(t, 0.5) && close(f.unwrap(), 4_000.0));
        let saved = to_file_units(&audio, opened, in_memory);
        let (t, f) = region_start(&saved);
        assert!(close(t, 0.05) && close(f.unwrap(), 40_000.0));
        // A set saved before, in the corrected units, loads onto the same
        // samples at 25.6 kHz.
        let loaded = from_file_units(&audio, opened, corrected.clone());
        let (t, f) = region_start(&loaded);
        assert!(close(t, 0.5) && close(f.unwrap(), 4_000.0));

        // Made again with the factor and origin it was opened with: back in
        // the units on disk.
        let te = opened.unwrap();
        assert!(audio.set_time_expansion(Some(te.factor), te.origin));
        assert_eq!(to_opening_rate(&audio, opened), None);
        // The user's own x10 on a file opened without a correction is
        // converted back to the header rate.
        let mut plain = case_t_audio();
        assert!(plain.set_time_expansion(Some(TENFOLD), TimeExpansionOrigin::User));
        assert_eq!(
            to_opening_rate(&plain, None),
            Some(RateChange::new(256_000, 25_600))
        );
    }
}
