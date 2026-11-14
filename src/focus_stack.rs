use crate::state::{BandpassMode, PlaybackMode};

/// Identifies the source of a frequency focus override.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FocusSource {
    /// The user's own preference (drag handles, axis drag, input fields).
    /// This is the base layer -- always present, never popped.
    User,
    /// Bat book selection override.
    BatBook,
    /// Annotation selection override.
    Annotation,
}

impl FocusSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "User",
            Self::BatBook => "Bat Book",
            Self::Annotation => "Annotation",
        }
    }
}

/// A snapshot of the frequency focus range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FocusRange {
    pub lo: f64,
    pub hi: f64,
}

impl FocusRange {
    pub fn new(lo: f64, hi: f64) -> Self {
        Self { lo, hi }
    }

    pub fn inactive() -> Self {
        Self { lo: 0.0, hi: 0.0 }
    }

    pub fn is_active(&self) -> bool {
        self.hi > self.lo
    }

    /// The band HFR starts with when it has none, for a file whose highest
    /// frequency is `nyquist`. The first of these that still has a width once
    /// clipped to 0..`nyquist`:
    /// 1. the file species' call range,
    /// 2. the lowest bat frequency up to `nyquist`,
    /// 3. the whole file, 0 to `nyquist`.
    ///
    /// The result always has `hi > lo`. It is `None` only when `nyquist` is
    /// not above 0. The caller must not write an empty band: the effect that
    /// sets this default runs again whenever the band is empty, so an empty
    /// default makes it run forever and the page stops responding.
    pub fn hfr_default(
        species: Option<(f64, f64)>,
        lowest_bat_hz: f64,
        nyquist: f64,
    ) -> Option<Self> {
        if !(nyquist > 0.0) {
            return None;
        }
        // `f64::max` and `f64::min` return the other value for NaN.
        let clip = |(lo, hi): (f64, f64)| {
            let r = Self::new(lo.max(0.0), hi.min(nyquist));
            r.is_active().then_some(r)
        };
        species
            .and_then(clip)
            .or_else(|| clip((lowest_bat_hz, nyquist)))
            .or_else(|| clip((0.0, nyquist)))
    }
}

/// A single layer in the focus stack.
#[derive(Clone, Debug)]
pub struct FocusLayer {
    pub source: FocusSource,
    pub range: FocusRange,
    /// True when the user has manually modified this override's range.
    /// When adopted, the override becomes the user's preference and
    /// no restore happens on pop.
    pub adopted: bool,
}

/// Debug snapshot of a single layer, returned by `debug_layers()`.
pub struct DebugLayer {
    pub source: FocusSource,
    pub range: FocusRange,
    pub adopted: bool,
    /// True if this layer is currently determining the effective range.
    pub is_effective: bool,
}

/// Layered frequency focus state. Stored in a single `RwSignal<FocusStack>`.
///
/// The stack has a permanent "user" base layer and zero or more override layers
/// (BatBook, Annotation) on top. The effective range is the topmost layer when
/// HFR is enabled, or inactive (0/0) when HFR is off.
///
/// When the user manually modifies the BandFF range while an override is active,
/// the override is marked "adopted" — it won't restore the previous range
/// when popped.
#[derive(Clone, Debug)]
pub struct FocusStack {
    /// The user's base preference (always exists).
    user_range: FocusRange,
    /// Override layers. Later entries have higher priority.
    /// Invariant: no duplicate FocusSources.
    overrides: Vec<FocusLayer>,
    /// Whether HFR is enabled (orthogonal to the layer stack).
    hfr_enabled: bool,
    /// Saved playback mode for HFR restore.
    hfr_saved_playback_mode: Option<PlaybackMode>,
    /// Saved bandpass mode for HFR restore.
    hfr_saved_bandpass_mode: Option<BandpassMode>,
}

impl Default for FocusStack {
    fn default() -> Self {
        Self::new()
    }
}

impl FocusStack {
    pub fn new() -> Self {
        Self {
            user_range: FocusRange::inactive(),
            overrides: Vec::new(),
            hfr_enabled: false,
            hfr_saved_playback_mode: None,
            hfr_saved_bandpass_mode: None,
        }
    }

    // ── Queries ──────────────────────────────────────────────────────────

    /// The effective BandFF range: highest-priority override, or user_range.
    /// Returns inactive (0/0) when HFR is off.
    pub fn effective_range(&self) -> FocusRange {
        if !self.hfr_enabled {
            return FocusRange::inactive();
        }
        self.effective_range_ignoring_hfr()
    }

    /// The range that *would* be effective if HFR were on.
    /// Useful for knowing what will be restored when HFR is re-enabled.
    pub fn effective_range_ignoring_hfr(&self) -> FocusRange {
        self.overrides
            .last()
            .map(|l| l.range)
            .unwrap_or(self.user_range)
    }

    /// The user's own range (what would be active if all overrides were removed).
    pub fn user_range(&self) -> FocusRange {
        self.user_range
    }

    pub fn hfr_enabled(&self) -> bool {
        self.hfr_enabled
    }

    /// Is a particular source currently active as an override?
    pub fn has_override(&self, source: FocusSource) -> bool {
        self.overrides.iter().any(|l| l.source == source)
    }

    /// Is the topmost override (if any) adopted by the user?
    pub fn is_top_adopted(&self) -> bool {
        self.overrides.last().is_some_and(|l| l.adopted)
    }

    /// Is a specific override adopted?
    pub fn is_adopted(&self, source: FocusSource) -> bool {
        self.overrides
            .iter()
            .find(|l| l.source == source)
            .is_some_and(|l| l.adopted)
    }

    /// Get the topmost active source.
    pub fn active_source(&self) -> FocusSource {
        self.overrides
            .last()
            .map(|l| l.source)
            .unwrap_or(FocusSource::User)
    }

    /// For debug display: return all layers from bottom to top.
    pub fn debug_layers(&self) -> Vec<DebugLayer> {
        let effective_source = self.active_source();
        let mut out = vec![DebugLayer {
            source: FocusSource::User,
            range: self.user_range,
            adopted: false,
            is_effective: self.overrides.is_empty(),
        }];
        for layer in &self.overrides {
            out.push(DebugLayer {
                source: layer.source,
                range: layer.range,
                adopted: layer.adopted,
                is_effective: layer.source == effective_source,
            });
        }
        out
    }

    pub fn saved_playback_mode(&self) -> Option<PlaybackMode> {
        self.hfr_saved_playback_mode
    }

    pub fn saved_bandpass_mode(&self) -> Option<BandpassMode> {
        self.hfr_saved_bandpass_mode
    }

    // ── Mutations ────────────────────────────────────────────────────────

    /// User sets BandFF directly (drag handle, axis drag, input field).
    /// If an override is active, marks it as "adopted" (user took ownership).
    pub fn set_user_range(&mut self, range: FocusRange) {
        self.user_range = range;
        if let Some(top) = self.overrides.last_mut() {
            top.range = range; // keep override in sync with what user sees
            top.adopted = true;
        }
    }

    /// Push an override layer (bat book, annotation).
    /// If this source already exists, updates its range and resets adopted.
    pub fn push_override(&mut self, source: FocusSource, range: FocusRange) {
        if let Some(existing) = self.overrides.iter_mut().find(|l| l.source == source) {
            existing.range = range;
            existing.adopted = false;
        } else {
            self.overrides.push(FocusLayer {
                source,
                range,
                adopted: false,
            });
        }
    }

    /// Pop an override layer.
    /// Returns `Some(range_to_restore_to)` if the override was NOT adopted.
    /// Returns `None` if adopted (user already owns the range, no restore needed).
    pub fn pop_override(&mut self, source: FocusSource) -> Option<FocusRange> {
        let idx = self.overrides.iter().position(|l| l.source == source)?;
        let layer = self.overrides.remove(idx);
        if layer.adopted {
            // User modified the override — it's already their preference.
            None
        } else {
            // Restore to whatever the next layer down says.
            Some(self.effective_range_ignoring_hfr())
        }
    }

    /// Snapshot the per-file focus SELECTIONS (user range + override layers),
    /// excluding the global HFR mode (enabled + saved playback/bandpass). Used to
    /// save/restore the focus per file on switch. [cross-file state-scoping]
    pub fn focus_selections(&self) -> (FocusRange, Vec<FocusLayer>) {
        (self.user_range, self.overrides.clone())
    }

    /// Replace the focus SELECTIONS from a snapshot, leaving the HFR mode fields
    /// untouched (those are global / carry across files).
    pub fn set_focus_selections(&mut self, user_range: FocusRange, overrides: Vec<FocusLayer>) {
        self.user_range = user_range;
        self.overrides = overrides;
    }

    /// Set HFR enabled/disabled.
    pub fn set_hfr_enabled(&mut self, enabled: bool) {
        self.hfr_enabled = enabled;
    }

    pub fn set_saved_playback_mode(&mut self, mode: Option<PlaybackMode>) {
        self.hfr_saved_playback_mode = mode;
    }

    pub fn set_saved_bandpass_mode(&mut self, mode: Option<BandpassMode>) {
        self.hfr_saved_bandpass_mode = mode;
    }
}

#[cfg(test)]
mod tests {
    use super::FocusRange;

    /// The default band must not be empty and must fit the file, or turning
    /// HFR on hangs the page (see `FocusRange::hfr_default`).
    fn assert_fits(r: Option<FocusRange>, nyquist: f64) -> FocusRange {
        let r = r.expect("a band");
        assert!(r.is_active(), "empty band {r:?}");
        assert!(r.lo >= 0.0 && r.hi <= nyquist, "{r:?} outside 0..{nyquist}");
        r
    }

    #[test]
    fn hfr_default_uses_the_species_range_when_it_fits() {
        let r = FocusRange::hfr_default(Some((35_000.0, 80_000.0)), 16_000.0, 96_000.0);
        assert_eq!(r, Some(FocusRange::new(35_000.0, 80_000.0)));
        // Clipped to the file's highest frequency.
        let r = FocusRange::hfr_default(Some((35_000.0, 80_000.0)), 16_000.0, 48_000.0);
        assert_eq!(r, Some(FocusRange::new(35_000.0, 48_000.0)));
    }

    #[test]
    fn hfr_default_skips_a_species_range_above_the_file() {
        // Myotis fimbriatus (35-80 kHz) in a 44.1 kHz or 48 kHz file: the
        // inputs that hung the page when HET was chosen.
        for nyquist in [22_050.0, 24_000.0] {
            let r = FocusRange::hfr_default(Some((35_000.0, 80_000.0)), 16_000.0, nyquist);
            let r = assert_fits(r, nyquist);
            assert_eq!(r, FocusRange::new(16_000.0, nyquist));
        }
    }

    #[test]
    fn hfr_default_uses_the_whole_file_below_the_lowest_bat_frequency() {
        // A 16 kHz file with no species: the 16 kHz lowest bat frequency is
        // above its 8 kHz limit.
        let r = assert_fits(FocusRange::hfr_default(None, 16_000.0, 8_000.0), 8_000.0);
        assert_eq!(r, FocusRange::new(0.0, 8_000.0));
        let r = FocusRange::hfr_default(Some((35_000.0, 80_000.0)), 16_000.0, 8_000.0);
        assert_eq!(assert_fits(r, 8_000.0), FocusRange::new(0.0, 8_000.0));
    }

    #[test]
    fn hfr_default_is_never_empty() {
        let species = [
            None,
            Some((35_000.0, 80_000.0)),
            Some((-5_000.0, 10_000.0)),
            Some((20_000.0, 20_000.0)),
            Some((f64::NAN, 30_000.0)),
        ];
        for nyquist in [
            4_000.0, 8_000.0, 11_025.0, 22_050.0, 24_000.0, 48_000.0, 192_000.0,
        ] {
            for lowest in [0.0, 1_000.0, 16_000.0, 30_000.0, f64::NAN] {
                for s in species {
                    assert_fits(FocusRange::hfr_default(s, lowest, nyquist), nyquist);
                }
            }
        }
        // No usable range at all: nothing to set.
        for nyquist in [0.0, -1.0, f64::NAN] {
            assert_eq!(FocusRange::hfr_default(None, 16_000.0, nyquist), None);
        }
    }
}
