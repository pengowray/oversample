use crate::components::file_sidebar::file_groups::{self, TrackInfo};
use crate::state::store_fields::*;
use crate::state::{AppState, LoadedFile};
use leptos::prelude::*;

/// Runtime view of a timeline — maps timeline-local time to file data.
/// Built from selected files and their recording timestamps.
#[derive(Clone, Debug)]
pub struct TimelineView {
    /// Segments sorted by timeline offset (earliest first). Each one names the
    /// file that is drawn and played, which is the chosen channel's file when
    /// a channel is chosen.
    pub segments: Vec<TimelineSegment>,
    /// Total span in seconds (from origin to end of last segment).
    pub total_duration_secs: f64,
    /// Epoch ms of the timeline origin (earliest file start).
    pub origin_epoch_ms: f64,
    /// The channels the timeline can be switched to, in menu order. Empty
    /// unless the takes on the timeline have two or more channels loaded.
    pub tracks: Vec<TrackOption>,
    /// How many takes on the timeline are channels of a multitrack recording.
    pub take_count: usize,
    /// The channel chosen in the Channel menu. `None` shows the files as they
    /// were added to the timeline.
    pub track: Option<String>,
    /// The channel every multitrack clip shows. `None` when the clips show
    /// different channels.
    pub shown_track: Option<String>,
    /// Takes whose file for the chosen channel is not loaded. Their place on
    /// the timeline is empty and silent.
    pub missing: Vec<MissingClip>,
}

/// A single file's placement on the timeline.
#[derive(Clone, Debug)]
pub struct TimelineSegment {
    /// Index into `AppState::files`.
    pub file_index: usize,
    /// Offset in seconds from the timeline origin.
    pub timeline_offset_secs: f64,
    /// Duration of this file in seconds.
    pub duration_secs: f64,
    /// True if this segment overlaps with another segment.
    pub has_overlap: bool,
}

/// One channel in the Channel menu of a multitrack timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackOption {
    /// The channel's label from the file names ("1", "Ch2", "1-2", "MIX").
    pub label: String,
    /// How many takes on the timeline have this channel's file loaded.
    pub takes_loaded: usize,
}

/// A take's place on the timeline that is empty, because the file for the
/// chosen channel is not loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct MissingClip {
    /// The take's name: the file name without its channel ("260401_0011").
    pub take: String,
    pub timeline_offset_secs: f64,
    pub duration_secs: f64,
}

/// What one file added to the timeline shows for the chosen channel.
#[derive(Clone, Debug, PartialEq)]
enum Shown {
    /// The file at this index.
    File(usize),
    /// Nothing: this take's file for the chosen channel is not loaded.
    Missing { take: String },
}

/// The channel choice worked out for a timeline. See [`plan_tracks`].
#[derive(Clone, Debug, PartialEq)]
struct TrackPlan {
    tracks: Vec<TrackOption>,
    take_count: usize,
    /// The requested channel, or `None` if the timeline does not offer it.
    track: Option<String>,
    /// One entry per file added to the timeline, in the same order.
    shown: Vec<Shown>,
    shown_track: Option<String>,
}

/// Sort key that puts channel labels in counting order: "1", "1-2", "2",
/// "Ch3", "10", then labels without a number ("MIX").
fn track_sort_key(label: &str) -> (u32, String) {
    let lower = label.to_ascii_lowercase();
    let digits: String = lower
        .trim_start_matches("ch")
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    (digits.parse().unwrap_or(u32::MAX), lower)
}

/// Work out which file each timeline file shows for the chosen channel.
///
/// `added` is the files added to the timeline (indices into `channels`).
/// `channels` has one entry per loaded file: the take and channel its name
/// gives, from `file_groups::channel_of`.
///
/// A file that is not a channel of anything always shows itself. A channel
/// file shows the loaded file of the same take with the chosen channel.
fn plan_tracks(added: &[usize], channels: &[Option<TrackInfo>], track: Option<&str>) -> TrackPlan {
    let channel_of = |i: usize| channels.get(i).and_then(|c| c.as_ref());
    let file_for = |take: &str, label: &str| {
        channels.iter().position(|c| {
            c.as_ref()
                .is_some_and(|c| c.group_key == take && c.label == label)
        })
    };

    // The takes on the timeline, each once.
    let mut takes: Vec<&str> = Vec::new();
    for &i in added {
        if let Some(c) = channel_of(i) {
            if !takes.contains(&c.group_key.as_str()) {
                takes.push(&c.group_key);
            }
        }
    }

    // Every channel that any of those takes has loaded.
    let mut labels: Vec<&str> = Vec::new();
    for c in channels.iter().flatten() {
        if takes.contains(&c.group_key.as_str()) && !labels.contains(&c.label.as_str()) {
            labels.push(&c.label);
        }
    }
    labels.sort_by_key(|l| track_sort_key(l));
    let tracks: Vec<TrackOption> = if labels.len() < 2 {
        Vec::new()
    } else {
        labels
            .iter()
            .map(|label| TrackOption {
                label: label.to_string(),
                takes_loaded: takes
                    .iter()
                    .filter(|take| file_for(take, label).is_some())
                    .count(),
            })
            .collect()
    };

    let track = track.filter(|t| tracks.iter().any(|o| o.label == *t));

    let shown: Vec<Shown> = added
        .iter()
        .map(|&i| match (track, channel_of(i)) {
            (Some(t), Some(c)) if c.label != t => match file_for(&c.group_key, t) {
                Some(j) => Shown::File(j),
                None => Shown::Missing {
                    take: c.group_key.clone(),
                },
            },
            _ => Shown::File(i),
        })
        .collect();

    let shown_track = match track {
        Some(t) => Some(t.to_string()),
        None if tracks.is_empty() => None,
        None => {
            let mut shown_labels = added
                .iter()
                .filter_map(|&i| channel_of(i))
                .map(|c| &c.label);
            let first = shown_labels.next();
            first
                .filter(|f| shown_labels.all(|l| l == *f))
                .map(|l| l.to_string())
        }
    };

    TrackPlan {
        tracks,
        take_count: takes.len(),
        track: track.map(str::to_string),
        shown,
        shown_track,
    }
}

impl TimelineView {
    /// Build a timeline from selected file indices.
    ///
    /// Files are positioned by their `recording_start_epoch_ms`. Files without
    /// timestamps are placed sequentially after the last positioned file.
    ///
    /// `track` is the channel chosen in the Channel menu. Each clip keeps the
    /// place its own file gives it and shows the chosen channel's file of the
    /// same take. A request for a channel the timeline does not offer is
    /// ignored; the result's `track` says which channel was applied.
    pub fn from_files(
        file_indices: &[usize],
        files: &[LoadedFile],
        track: Option<&str>,
    ) -> Option<Self> {
        if file_indices.len() < 2 {
            return None;
        }

        // Collect (file_index, start_epoch_ms_or_none, duration)
        let mut entries: Vec<(usize, Option<f64>, f64)> = file_indices
            .iter()
            .filter_map(|&i| {
                let f = files.get(i)?;
                Some((i, f.recording_start_epoch_ms(), f.audio.duration_secs))
            })
            .collect();

        if entries.is_empty() {
            return None;
        }

        // Sort: files with timestamps first (by timestamp), then files without
        entries.sort_by(|a, b| match (&a.1, &b.1) {
            (Some(ta), Some(tb)) => ta.partial_cmp(tb).unwrap_or(std::cmp::Ordering::Equal),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => std::cmp::Ordering::Equal,
        });

        // Determine the origin (earliest start time)
        let origin_ms = entries
            .iter()
            .filter_map(|(_, start, _)| *start)
            .next()
            .unwrap_or(0.0);

        // Place every added file. The places do not depend on the chosen
        // channel, so the timeline keeps its length and the clips keep their
        // positions when the channel is switched.
        let mut placed: Vec<(usize, f64, f64)> = Vec::with_capacity(entries.len());
        let mut cursor_secs = 0.0; // For files without timestamps
        for (file_idx, start_ms, duration) in &entries {
            let offset = if let Some(ms) = start_ms {
                (ms - origin_ms) / 1000.0
            } else {
                // No timestamp: place after the current cursor
                cursor_secs
            };
            placed.push((*file_idx, offset, *duration));
            let end = offset + duration;
            if end > cursor_secs {
                cursor_secs = end;
            }
        }

        // Which file each place shows for the chosen channel.
        let channels: Vec<Option<TrackInfo>> = files
            .iter()
            .map(|f| file_groups::channel_of(&f.name))
            .collect();
        let added: Vec<usize> = placed.iter().map(|p| p.0).collect();
        let plan = plan_tracks(&added, &channels, track);

        let mut segments: Vec<TimelineSegment> = Vec::new();
        let mut missing: Vec<MissingClip> = Vec::new();
        for (&(_, offset, duration), shown) in placed.iter().zip(&plan.shown) {
            match shown {
                Shown::File(j) => {
                    // Two channels of one take were both added and now show
                    // the same file: keep one clip.
                    if plan.track.is_some() && segments.iter().any(|s| s.file_index == *j) {
                        continue;
                    }
                    let Some(file) = files.get(*j) else { continue };
                    segments.push(TimelineSegment {
                        file_index: *j,
                        timeline_offset_secs: offset,
                        duration_secs: file.audio.duration_secs,
                        has_overlap: false,
                    });
                }
                Shown::Missing { take } => {
                    if !missing.iter().any(|m| &m.take == take) {
                        missing.push(MissingClip {
                            take: take.clone(),
                            timeline_offset_secs: offset,
                            duration_secs: duration,
                        });
                    }
                }
            }
        }

        // Detect overlaps
        for i in 0..segments.len() {
            for j in (i + 1)..segments.len() {
                let a_start = segments[i].timeline_offset_secs;
                let a_end = a_start + segments[i].duration_secs;
                let b_start = segments[j].timeline_offset_secs;
                let b_end = b_start + segments[j].duration_secs;

                if a_start < b_end && b_start < a_end {
                    segments[i].has_overlap = true;
                    segments[j].has_overlap = true;
                }
            }
        }

        // Total duration: every place counts, including the empty ones.
        let total_duration_secs = placed
            .iter()
            .map(|&(_, offset, duration)| offset + duration)
            .chain(
                segments
                    .iter()
                    .map(|s| s.timeline_offset_secs + s.duration_secs),
            )
            .fold(0.0_f64, f64::max);

        Some(TimelineView {
            segments,
            total_duration_secs,
            origin_epoch_ms: origin_ms,
            tracks: plan.tracks,
            take_count: plan.take_count,
            track: plan.track,
            shown_track: plan.shown_track,
            missing,
        })
    }

    /// Find which file (if any) is at the given timeline-local time.
    /// Returns `(file_index, offset_within_file)`.
    pub fn file_at_time(&self, t: f64) -> Option<(usize, f64)> {
        // Return the first segment that contains this time
        for seg in &self.segments {
            let seg_end = seg.timeline_offset_secs + seg.duration_secs;
            if t >= seg.timeline_offset_secs && t < seg_end {
                return Some((seg.file_index, t - seg.timeline_offset_secs));
            }
        }
        None
    }

    /// Get all segments that are visible in the given time range.
    pub fn segments_in_range(&self, start: f64, end: f64) -> Vec<&TimelineSegment> {
        self.segments
            .iter()
            .filter(|s| {
                let seg_end = s.timeline_offset_secs + s.duration_secs;
                s.timeline_offset_secs < end && seg_end > start
            })
            .collect()
    }

    /// Get the segment for a specific file index.
    pub fn segment_for_file(&self, file_index: usize) -> Option<&TimelineSegment> {
        self.segments.iter().find(|s| s.file_index == file_index)
    }

    /// The `(start, end)` in seconds of every place on the timeline, in time
    /// order: the clips and the places left empty by a channel that is not
    /// loaded. The gaps between these are the gaps between recordings.
    pub fn clip_spans(&self) -> Vec<(f64, f64)> {
        let mut spans: Vec<(f64, f64)> = self
            .segments
            .iter()
            .map(|s| {
                (
                    s.timeline_offset_secs,
                    s.timeline_offset_secs + s.duration_secs,
                )
            })
            .chain(self.missing.iter().map(|m| {
                (
                    m.timeline_offset_secs,
                    m.timeline_offset_secs + m.duration_secs,
                )
            }))
            .collect();
        spans.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
        spans
    }
}

/// A channel's name in the Channel menu and in messages: "Channel 2" for the
/// file-name labels "2" and "Ch2", "Channel 1-2" for "1-2". Other labels
/// ("MIX") are shown as they are in the file name.
pub fn track_name(label: &str) -> String {
    let short = track_short_name(label);
    if short.starts_with(|c: char| c.is_ascii_digit()) {
        format!("Channel {short}")
    } else {
        short
    }
}

/// A channel's name on the Channel chip, which already says "Ch": "2" for
/// the file-name labels "2" and "Ch2".
pub fn track_short_name(label: &str) -> String {
    let lower = label.to_ascii_lowercase();
    match lower.strip_prefix("ch") {
        Some(n) if !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()) => n.to_string(),
        _ => label.to_string(),
    }
}

/// Rebuild the open timeline from the files added to it and the chosen
/// channel. Call after the file list changes. Clears the chosen channel when
/// the timeline no longer offers it.
pub fn rebuild_active(state: &AppState) {
    let sel = state.timeline.selected_file_indices().get_untracked();
    let track = state.timeline.active_track().get_untracked();
    let rebuilt = state
        .library
        .files()
        .with_untracked(|files| TimelineView::from_files(&sel, files, track.as_deref()));
    let applied = rebuilt.as_ref().and_then(|t| t.track.clone());
    if applied != track {
        state.timeline.active_track().set(applied);
    }
    state.timeline.active().set(rebuilt);
}

/// Switch the open timeline to a channel (`None`: the files as they were
/// added). The spectrogram, waveform, overview and playback all read the
/// timeline's segments, so they follow. Audio that is playing carries on from
/// the same position on the new channel.
pub fn set_track(state: &AppState, track: Option<String>) {
    state.timeline.active_track().set(track);
    rebuild_active(state);
    state
        .viewmode
        .tile_ready_signal()
        .update(|n| *n = n.wrapping_add(1));
    crate::audio::playback::schedule_replay_live(state);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn channels(names: &[&str]) -> Vec<Option<TrackInfo>> {
        names.iter().map(|n| file_groups::channel_of(n)).collect()
    }

    const NAMES: [&str; 6] = [
        "260401_0010_1.wav", // 0
        "260401_0010_2.wav", // 1
        "260401_0011_1.wav", // 2, channel 2 not loaded
        "260401_0012_1.wav", // 3
        "260401_0012_2.wav", // 4
        "notes.wav",         // 5, not a channel of anything
    ];

    #[test]
    fn no_channel_chosen_shows_the_files_as_added() {
        let plan = plan_tracks(&[0, 2, 3], &channels(&NAMES), None);
        assert_eq!(
            plan.shown,
            vec![Shown::File(0), Shown::File(2), Shown::File(3)]
        );
        assert_eq!(plan.track, None);
        // All three are channel 1, so the menu marks channel 1.
        assert_eq!(plan.shown_track.as_deref(), Some("1"));
        assert_eq!(plan.take_count, 3);
        assert_eq!(
            plan.tracks,
            vec![
                TrackOption {
                    label: "1".into(),
                    takes_loaded: 3
                },
                TrackOption {
                    label: "2".into(),
                    takes_loaded: 2
                },
            ]
        );
    }

    #[test]
    fn chosen_channel_swaps_each_take_to_its_file_for_that_channel() {
        let plan = plan_tracks(&[0, 2, 3], &channels(&NAMES), Some("2"));
        assert_eq!(
            plan.shown,
            vec![
                Shown::File(1),
                Shown::Missing {
                    take: "260401_0011".into()
                },
                Shown::File(4),
            ]
        );
        assert_eq!(plan.track.as_deref(), Some("2"));
        assert_eq!(plan.shown_track.as_deref(), Some("2"));
    }

    #[test]
    fn a_file_that_is_not_a_channel_stays_on_every_channel() {
        let plan = plan_tracks(&[0, 5], &channels(&NAMES), Some("2"));
        assert_eq!(plan.shown, vec![Shown::File(1), Shown::File(5)]);
    }

    #[test]
    fn mixed_channels_have_no_single_shown_channel() {
        let plan = plan_tracks(&[0, 4], &channels(&NAMES), None);
        assert_eq!(plan.shown_track, None);
        // Choosing one makes both takes show it.
        let plan = plan_tracks(&[0, 4], &channels(&NAMES), Some("1"));
        assert_eq!(plan.shown, vec![Shown::File(0), Shown::File(3)]);
    }

    #[test]
    fn a_channel_the_timeline_does_not_have_is_ignored() {
        let plan = plan_tracks(&[0, 2, 3], &channels(&NAMES), Some("7"));
        assert_eq!(plan.track, None);
        assert_eq!(
            plan.shown,
            vec![Shown::File(0), Shown::File(2), Shown::File(3)]
        );
    }

    #[test]
    fn takes_with_one_channel_loaded_offer_no_choice() {
        let names = ["260401_0010_1.wav", "260401_0011_1.wav"];
        let plan = plan_tracks(&[0, 1], &channels(&names), None);
        assert!(plan.tracks.is_empty());
        assert_eq!(plan.shown_track, None);
    }

    #[test]
    fn channels_are_listed_in_counting_order() {
        let names = [
            "a_0001_MIX.wav",
            "a_0001_10.wav",
            "a_0001_2.wav",
            "a_0001_1-2.wav",
            "a_0001_Ch3.wav",
        ];
        let plan = plan_tracks(&[0], &channels(&names), None);
        let labels: Vec<&str> = plan.tracks.iter().map(|t| t.label.as_str()).collect();
        assert_eq!(labels, vec!["1-2", "2", "Ch3", "10", "MIX"]);
    }

    #[test]
    fn channel_names_for_the_menu_and_the_chip() {
        assert_eq!(track_name("2"), "Channel 2");
        assert_eq!(track_name("Ch2"), "Channel 2");
        assert_eq!(track_name("1-2"), "Channel 1-2");
        assert_eq!(track_name("MIX"), "MIX");
        assert_eq!(track_short_name("ch12"), "12");
        assert_eq!(track_short_name("MIX"), "MIX");
    }
}
