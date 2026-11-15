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
//! sequential_members, denoise_pair_members}` (auto-detected from file names
//! today; project groups will override later).
//!
//! Mental model: **sequential groups share audio-character settings (gain +
//! noise settings); multitrack groups share viewport settings (scroll + freq
//! range) and the playhead; an original and its denoised copy share
//! everything needed to compare them (viewport, gain, focus selections,
//! playhead) but not the noise settings, which are the difference being
//! compared.**

use crate::components::file_sidebar::file_groups::{
    denoise_pair_members, multitrack_members, sequential_members, FileGroupInfo,
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

/// A way two loaded files can be related. A `PerFile` setting names the
/// relationships its value is shared within.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relationship {
    /// Simultaneous channels of one recording (same track group_key).
    Multitrack,
    /// Consecutive recordings on the same channel (same sequence).
    Sequential,
    /// A recording and the denoised copies made from it
    /// ("bat.wav" and "bat (denoised).wav").
    DenoisedPair,
}

impl Relationship {
    /// Indices of the files related to `idx` this way, including `idx`.
    fn members(self, groups: &[FileGroupInfo], idx: usize) -> Vec<usize> {
        match self {
            Relationship::Multitrack => multitrack_members(groups, idx),
            Relationship::Sequential => sequential_members(groups, idx),
            Relationship::DenoisedPair => denoise_pair_members(groups, idx),
        }
    }
}

/// The per-file settings this app scopes. `policy()` is the source of truth.
///
/// | Setting                                         | Scope   | Shared within             |
/// |-------------------------------------------------|---------|---------------------------|
/// | noise settings: notch + spectral-sub + EQ curve | PerFile | Sequential                |
/// | gain                                            | PerFile | Sequential, DenoisedPair  |
/// | vertical frequency range                        | PerFile | Multitrack, DenoisedPair  |
/// | horizontal scroll                               | PerFile | Multitrack, DenoisedPair  |
/// | view history (Back and Forward)                 | PerFile | Multitrack, DenoisedPair  |
/// | bookmarks                                       | PerFile | Multitrack, DenoisedPair  |
/// | frequency-focus selections                      | PerFile | DenoisedPair              |
/// | playhead (position, and playing or stopped)     | Global  | Multitrack, DenoisedPair  |
///
/// The playhead is one global signal: it is not saved or restored per file.
/// A switch to another channel of the same recording, or between a recording
/// and its denoised copy, keeps it: during playback the file-switch Effect
/// restarts the audio in the other file at the same position, so the two can
/// be compared by ear. Any other switch (an unrelated file, or the next
/// recording of a sequence) stops playback and puts the playhead at the start.
///
/// The view history and the bookmarks hold times in a recording, so they
/// go with the horizontal scroll: each file has its own, and files that show
/// the same recording at the same place share them. The file-switch Effect
/// stores the outgoing file's and brings back the incoming file's, so Back
/// never goes to a position from another recording.
///
/// The selection (`interaction.selection`) is also one global signal. It is
/// kept on a switch between files that share the horizontal scroll, and
/// cleared on any other switch.
///
/// Global (not modelled here): HFR on/off, playback mode + transform params,
/// `bandpass_mode` (HFR-coupled, pending the split), channel view, colormap.
/// Transient: annotation selection, hover/marker, and the selection and
/// playback outside the files described above.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    /// Gain mode and gain values (the gain fields of `FileSettings`).
    Gain,
    /// Notch + spectral subtraction + bandpass-EQ curve (the other fields of
    /// `FileSettings`).
    NoiseFilters,
    VerticalRange,
    HorizontalScroll,
    /// The Back/Forward history of view positions (`viewmode.nav_history`).
    ViewHistory,
    /// Bookmarks: times marked during playback (`viewmode.bookmarks`).
    Bookmarks,
    FocusSelections,
    /// Where the playhead is, and whether it is playing. Within the files that
    /// share it, a switch during playback keeps playing from the same position
    /// in the other file. See the note under the table.
    Playhead,
}

impl Setting {
    pub fn policy(self) -> (Scope, &'static [Relationship]) {
        use Relationship::*;
        match self {
            Setting::Gain => (Scope::PerFile, &[Sequential, DenoisedPair]),
            Setting::NoiseFilters => (Scope::PerFile, &[Sequential]),
            Setting::VerticalRange => (Scope::PerFile, &[Multitrack, DenoisedPair]),
            Setting::HorizontalScroll => (Scope::PerFile, &[Multitrack, DenoisedPair]),
            Setting::ViewHistory => (Scope::PerFile, &[Multitrack, DenoisedPair]),
            Setting::Bookmarks => (Scope::PerFile, &[Multitrack, DenoisedPair]),
            Setting::FocusSelections => (Scope::PerFile, &[DenoisedPair]),
            Setting::Playhead => (Scope::Global, &[Multitrack, DenoisedPair]),
        }
    }
}

/// Indices of the files a per-file `setting` value propagates to when it changes
/// on `idx` (always includes `idx`, sorted). The single dispatch point for the
/// sharing policy — change `Setting::policy` and every propagation site follows.
///
/// When a setting is shared within more than one relationship, the result
/// follows the links from file to file: the copy of channel 1 shares its scroll
/// with channel 1, and so also with channel 2.
pub fn share_members(setting: Setting, groups: &[FileGroupInfo], idx: usize) -> Vec<usize> {
    let relationships = setting.policy().1;
    let mut members = vec![idx];
    let mut next = 0;
    while next < members.len() {
        for relationship in relationships {
            for j in relationship.members(groups, members[next]) {
                if !members.contains(&j) {
                    members.push(j);
                }
            }
        }
        next += 1;
    }
    members.sort_unstable();
    members
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::file_sidebar::file_groups::compute_all_groups;

    fn groups(names: &[&str]) -> Vec<FileGroupInfo> {
        let names: Vec<String> = names.iter().map(|n| n.to_string()).collect();
        compute_all_groups(&names, &[])
    }

    const ALL: [Setting; 8] = [
        Setting::Gain,
        Setting::NoiseFilters,
        Setting::VerticalRange,
        Setting::HorizontalScroll,
        Setting::ViewHistory,
        Setting::Bookmarks,
        Setting::FocusSelections,
        Setting::Playhead,
    ];

    #[test]
    fn a_denoised_pair_shares_everything_but_the_noise_settings() {
        let g = groups(&["bat.wav", "bat (denoised).wav", "cat.wav"]);
        for setting in ALL {
            let expected = if setting == Setting::NoiseFilters {
                vec![0]
            } else {
                vec![0, 1]
            };
            assert_eq!(share_members(setting, &g, 0), expected, "{setting:?}");
            assert_eq!(share_members(setting, &g, 2), vec![2], "{setting:?}");
        }
        assert_eq!(share_members(Setting::NoiseFilters, &g, 1), vec![1]);
        assert_eq!(share_members(Setting::Gain, &g, 1), vec![0, 1]);
    }

    #[test]
    fn a_sequence_still_shares_gain_and_noise_settings() {
        // Two takes on channel 1, and a denoised copy of the first take.
        let g = groups(&[
            "260212_0041_1.wav",
            "260212_0042_1.wav",
            "260212_0041_1 (denoised).wav",
        ]);
        // The takes share the noise settings with each other, never with the copy.
        assert_eq!(share_members(Setting::NoiseFilters, &g, 0), vec![0, 1]);
        assert_eq!(share_members(Setting::NoiseFilters, &g, 1), vec![0, 1]);
        assert_eq!(share_members(Setting::NoiseFilters, &g, 2), vec![2]);
        // Gain follows both links, from the copy to its original to the sequence.
        assert_eq!(share_members(Setting::Gain, &g, 2), vec![0, 1, 2]);
        assert_eq!(share_members(Setting::Gain, &g, 1), vec![0, 1, 2]);
        // A sequence alone does not share the view, focus or playhead.
        assert_eq!(share_members(Setting::HorizontalScroll, &g, 1), vec![1]);
        assert_eq!(share_members(Setting::ViewHistory, &g, 1), vec![1]);
        assert_eq!(share_members(Setting::Bookmarks, &g, 1), vec![1]);
        assert_eq!(share_members(Setting::FocusSelections, &g, 1), vec![1]);
        assert_eq!(share_members(Setting::Playhead, &g, 1), vec![1]);
    }

    #[test]
    fn a_copy_shares_the_view_with_every_channel_of_its_original() {
        let g = groups(&["rec_Ch1.wav", "rec_Ch2.wav", "rec_Ch1 (denoised).wav"]);
        assert_eq!(
            share_members(Setting::HorizontalScroll, &g, 2),
            vec![0, 1, 2]
        );
        assert_eq!(share_members(Setting::VerticalRange, &g, 1), vec![0, 1, 2]);
        // The view history and bookmarks go with the scroll.
        assert_eq!(share_members(Setting::ViewHistory, &g, 0), vec![0, 1, 2]);
        assert_eq!(share_members(Setting::Bookmarks, &g, 2), vec![0, 1, 2]);
        // Channels do not share gain or focus; the pair does.
        assert_eq!(share_members(Setting::Gain, &g, 1), vec![1]);
        assert_eq!(share_members(Setting::Gain, &g, 2), vec![0, 2]);
        assert_eq!(share_members(Setting::FocusSelections, &g, 0), vec![0, 2]);
        // The playhead is shared by the channels and the copy: playback
        // carries on across a switch between any of them.
        assert_eq!(share_members(Setting::Playhead, &g, 1), vec![0, 1, 2]);
    }
}
