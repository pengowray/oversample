//! Cross-file state-scoping policy — the single, explicit reference for "what
//! follows the file, what stays global, and which relationship a per-file
//! setting shares across".
//!
//! The save/restore *wiring* lives in the file-switch Effect (`components/app.rs`),
//! `FileSettings` (`state.rs`), and the `library.per_file_view` side-table. This
//! module owns the *policy* they implement, so the sharing rules live in one
//! place — and it's the hook for the future "let the user control which settings
//! are shared / lock a global setting to a file / presets" work.
//!
//! Relationships are resolved by `file_groups::{multitrack_members,
//! sequential_members}` (auto-detected today; project groups will override later).
//!
//! Mental model: **sequential groups share audio-character settings (gain +
//! denoise); multitrack groups share viewport settings (scroll + freq range).**

use crate::components::file_sidebar::file_groups::{
    multitrack_members, sequential_members, FileGroupInfo,
};

/// Base scope of a setting across files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    /// Carries across all files — a "how I'm working right now" preference.
    Global,
    /// Belongs to the recording; saved/restored on file switch.
    PerFile,
    /// Reset on every switch (transient view state).
    Transient,
}

/// For a `PerFile` setting, which file-relationship its value propagates within.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShareAcross {
    /// Strictly per file.
    None,
    /// Shared across simultaneous channels of one recording (same track group_key).
    Multitrack,
    /// Shared across consecutive recordings on the same channel (same sequence).
    Sequential,
}

/// The per-file settings this app scopes. `policy()` is the source of truth.
///
/// | Setting                                    | Scope     | Shares across |
/// |--------------------------------------------|-----------|---------------|
/// | denoise: notch + spectral-sub + EQ curve   | PerFile   | Sequential    |
/// | gain                                       | PerFile   | Sequential    |
/// | vertical frequency range                   | PerFile   | Multitrack    |
/// | horizontal scroll                          | PerFile   | Multitrack    |
/// | frequency-focus selections                 | PerFile   | None          |
///
/// Global (not modelled here): HFR on/off, playback mode + transform params,
/// `bandpass_mode` (HFR-coupled, pending the split), channel view, colormap.
/// Transient: annotation selection, hover/marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    /// gain + notch + spectral-subtraction + bandpass-EQ curve (the FileSettings bundle).
    AudioCharacter,
    VerticalRange,
    HorizontalScroll,
    FocusSelections,
}

impl Setting {
    pub fn policy(self) -> (Scope, ShareAcross) {
        match self {
            Setting::AudioCharacter => (Scope::PerFile, ShareAcross::Sequential),
            Setting::VerticalRange => (Scope::PerFile, ShareAcross::Multitrack),
            Setting::HorizontalScroll => (Scope::PerFile, ShareAcross::Multitrack),
            Setting::FocusSelections => (Scope::PerFile, ShareAcross::None),
        }
    }
}

/// Indices of the files a per-file `setting` value propagates to when it changes
/// on `idx` (always includes `idx`). The single dispatch point for the sharing
/// policy — change `Setting::policy` and every propagation site follows.
pub fn share_members(setting: Setting, groups: &[FileGroupInfo], idx: usize) -> Vec<usize> {
    match setting.policy().1 {
        ShareAcross::Multitrack => multitrack_members(groups, idx),
        ShareAcross::Sequential => sequential_members(groups, idx),
        ShareAcross::None => vec![idx],
    }
}
