use crate::components::file_sidebar::file_groups::{self, TrackInfo};
use crate::state::store_fields::*;
use crate::state::{AppState, LoadedFile};
use leptos::prelude::*;

pub use crate::project::Arrangement;

/// Files whose recording times leave more than this many seconds between
/// them are placed one after another when a timeline is first made.
const LONGEST_KEPT_GAP_SECS: f64 = 60.0;

/// One file to place on a timeline. See [`place_clips`].
#[derive(Clone, Debug, PartialEq)]
struct ClipIn {
    /// When the recording started, ms since the epoch, if known.
    start_ms: Option<f64>,
    duration_secs: f64,
    /// The take a channel file belongs to ("260401_0010"). Channel files of
    /// one take were recorded together.
    take: Option<String>,
}

/// Where each clip starts, in seconds from the start of the timeline.
///
/// Returns `(clip index, start)` pairs in timeline order, and the clock time
/// of the timeline's start in ms since the epoch (0.0 when the timeline has
/// no clock time).
///
/// `RecordingTime` puts each clip at its recording time. A clip without one
/// goes after the end of the clips placed before it.
///
/// `InTurn` puts the clips one after another in the order given, with
/// `gap_secs` of silence between them. Channel files of one take start
/// together and take up one place, as long as the longest of them.
///
/// `Together` starts every clip at the start of the timeline.
fn place_clips(clips: &[ClipIn], arrangement: Arrangement) -> (Vec<(usize, f64)>, f64) {
    match arrangement {
        Arrangement::RecordingTime => {
            let mut order: Vec<usize> = (0..clips.len()).collect();
            order.sort_by(|&a, &b| match (clips[a].start_ms, clips[b].start_ms) {
                (Some(ta), Some(tb)) => ta.partial_cmp(&tb).unwrap_or(std::cmp::Ordering::Equal),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            });
            let origin_ms = order.iter().find_map(|&i| clips[i].start_ms).unwrap_or(0.0);
            let mut end_secs = 0.0_f64;
            let placed = order
                .into_iter()
                .map(|i| {
                    let start = match clips[i].start_ms {
                        Some(ms) => (ms - origin_ms) / 1000.0,
                        None => end_secs,
                    };
                    end_secs = end_secs.max(start + clips[i].duration_secs);
                    (i, start)
                })
                .collect();
            (placed, origin_ms)
        }
        Arrangement::Together => ((0..clips.len()).map(|i| (i, 0.0)).collect(), 0.0),
        Arrangement::InTurn { gap_secs } => {
            let gap = if gap_secs.is_finite() {
                gap_secs.max(0.0)
            } else {
                0.0
            };
            // One place per take, or per file that is not a channel file.
            let mut places: Vec<(Option<&str>, f64)> = Vec::new();
            let mut place_of: Vec<usize> = Vec::with_capacity(clips.len());
            for clip in clips {
                let take = clip.take.as_deref();
                let found = take.and_then(|t| places.iter().position(|p| p.0 == Some(t)));
                let at = match found {
                    Some(at) => at,
                    None => {
                        places.push((take, 0.0));
                        places.len() - 1
                    }
                };
                places[at].1 = places[at].1.max(clip.duration_secs);
                place_of.push(at);
            }
            let mut starts = Vec::with_capacity(places.len());
            let mut next = 0.0;
            for &(_, length) in &places {
                starts.push(next);
                next += length + gap;
            }
            let placed = place_of
                .iter()
                .enumerate()
                .map(|(i, &at)| (i, starts[at]))
                .collect();
            (placed, 0.0)
        }
    }
}

/// Two clips that overlap by less than this many seconds can share a row.
/// Recorders write consecutive files whose times overlap by a few
/// milliseconds.
const ROW_OVERLAP_TOLERANCE_SECS: f64 = 0.05;

/// The row each clip is drawn on, so that clips that overlap in time are on
/// different rows. `spans` are `(start, end)` in seconds, in any order; the
/// result has one row index per span, in the same order.
///
/// Clips are taken in the order given and each goes on the first row where
/// it overlaps nothing already there, so a timeline without overlaps has one
/// row, and files that start together get rows in the order given.
fn assign_rows(spans: &[(f64, f64)]) -> Vec<usize> {
    let mut rows: Vec<Vec<(f64, f64)>> = Vec::new();
    spans
        .iter()
        .map(|&(start, end)| {
            let overlaps = |other: &(f64, f64)| {
                start < other.1 - ROW_OVERLAP_TOLERANCE_SECS
                    && other.0 < end - ROW_OVERLAP_TOLERANCE_SECS
            };
            let row = rows
                .iter()
                .position(|taken| !taken.iter().any(overlaps))
                .unwrap_or_else(|| {
                    rows.push(Vec::new());
                    rows.len() - 1
                });
            rows[row].push((start, end));
            row
        })
        .collect()
}

/// The arrangement a new timeline of these clips starts with: recording
/// times when they give a timeline that can be listened to, otherwise one
/// after another.
///
/// Recording times are not used when a clip has none, when two clips that
/// are not channels of one take overlap, or when more than
/// [`LONGEST_KEPT_GAP_SECS`] lies between two clips. A file without recorder
/// metadata gets its time from the file's modified date, so files copied or
/// downloaded together all overlap, and files from different nights are
/// hours apart.
fn suggest_arrangement(clips: &[ClipIn]) -> Arrangement {
    let in_turn = Arrangement::InTurn { gap_secs: 0.0 };
    if clips.iter().any(|c| c.start_ms.is_none()) {
        return in_turn;
    }
    let (placed, _) = place_clips(clips, Arrangement::RecordingTime);
    // The stretch of time each take or single file covers.
    let mut spans: Vec<(Option<&str>, f64, f64)> = Vec::new();
    for &(i, start) in &placed {
        let end = start + clips[i].duration_secs;
        let take = clips[i].take.as_deref();
        match spans.iter_mut().find(|s| take.is_some() && s.0 == take) {
            Some(span) => {
                span.1 = span.1.min(start);
                span.2 = span.2.max(end);
            }
            None => spans.push((take, start, end)),
        }
    }
    spans.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
    let mut end = f64::NEG_INFINITY;
    for &(_, start, span_end) in &spans {
        if end.is_finite() && (start < end - 0.05 || start - end > LONGEST_KEPT_GAP_SECS) {
            return in_turn;
        }
        end = end.max(span_end);
    }
    Arrangement::RecordingTime
}

/// The part of a view `height` px tall that row `row` of `row_count` is
/// drawn in: `(top, height)` in px. Rows are equal and fill the view. The
/// spectrogram, the waveform, the overview and pointer input all use this,
/// so a frequency is at the same place for all of them.
pub fn row_rect(row: usize, row_count: usize, height: f64) -> (f64, f64) {
    let count = row_count.max(1);
    let row_height = height / count as f64;
    (row.min(count - 1) as f64 * row_height, row_height)
}

/// The row at `y` px from the top of a view `height` px tall, and `y`
/// measured from that row's top with the row's height:
/// `(row, y in row, row height)`.
pub fn row_at(y: f64, row_count: usize, height: f64) -> (usize, f64, f64) {
    let count = row_count.max(1);
    let row_height = height / count as f64;
    if row_height <= 0.0 {
        return (0, 0.0, 0.0);
    }
    let row = ((y / row_height).floor().max(0.0) as usize).min(count - 1);
    (row, y - row as f64 * row_height, row_height)
}

/// The pauses the Files menu offers between files placed one after another,
/// in seconds.
pub const PAUSE_CHOICES_SECS: [f64; 3] = [0.5, 1.0, 2.0];

/// A pause length as the Files menu writes it: "0.5", "1", "2".
pub fn pause_text(gap_secs: f64) -> String {
    let text = format!("{gap_secs:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// Compares file names so that numbers inside them count: "rec_9.wav" comes
/// before "rec_10.wav". Letters compare without regard to case.
pub fn natural_name_order(a: &str, b: &str) -> std::cmp::Ordering {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let run = |s: &[u8], from: usize| {
                let end = from + s[from..].iter().take_while(|c| c.is_ascii_digit()).count();
                let digits = &s[from..end];
                let first = digits
                    .iter()
                    .position(|&c| c != b'0')
                    .unwrap_or(digits.len());
                (end, digits[first..].to_vec())
            };
            let (ai, an) = run(a, i);
            let (bj, bn) = run(b, j);
            let by_value = an.len().cmp(&bn.len()).then_with(|| an.cmp(&bn));
            if by_value != std::cmp::Ordering::Equal {
                return by_value;
            }
            i = ai;
            j = bj;
        } else {
            let (ca, cb) = (a[i].to_ascii_lowercase(), b[j].to_ascii_lowercase());
            if ca != cb {
                return ca.cmp(&cb);
            }
            i += 1;
            j += 1;
        }
    }
    (a.len() - i).cmp(&(b.len() - j)).then_with(|| a.cmp(b))
}

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
    /// Epoch ms of the timeline origin (earliest file start). 0.0 when the
    /// clips are placed one after another: the timeline then has no clock time.
    pub origin_epoch_ms: f64,
    /// How the clips are placed.
    pub arrangement: Arrangement,
    /// How many rows the clips take: 1 unless clips overlap in time.
    pub row_count: usize,
    /// The clip whose file has the highest sample rate (the first of them).
    /// The frequency axis covers this file's range, and the zoom level is
    /// counted in this file's spectrogram columns.
    pub primary_file_index: Option<usize>,
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
    /// The row the clip is drawn on, counted from the top. Clips that
    /// overlap in time are on different rows.
    pub row: usize,
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
    /// The row the empty place is on.
    pub row: usize,
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
    /// Build a timeline from the files added to it.
    ///
    /// `arrangement` says where the clips go: see [`place_clips`]. Placed one
    /// after another, they keep the order of `file_indices`.
    ///
    /// `track` is the channel chosen in the Channel menu. Each clip keeps the
    /// place its own file gives it and shows the chosen channel's file of the
    /// same take. A request for a channel the timeline does not offer is
    /// ignored; the result's `track` says which channel was applied.
    pub fn from_files(
        file_indices: &[usize],
        files: &[LoadedFile],
        track: Option<&str>,
        arrangement: Arrangement,
    ) -> Option<Self> {
        if file_indices.len() < 2 {
            return None;
        }

        let channels: Vec<Option<TrackInfo>> = files
            .iter()
            .map(|f| file_groups::channel_of(&f.name))
            .collect();
        let indices: Vec<usize> = file_indices
            .iter()
            .copied()
            .filter(|&i| i < files.len())
            .collect();
        if indices.is_empty() {
            return None;
        }
        let clips = clips_in(&indices, files, &channels);
        let (order, origin_ms) = place_clips(&clips, arrangement);

        // Place every added file. The places do not depend on the chosen
        // channel, so the timeline keeps its length and the clips keep their
        // positions when the channel is switched.
        let placed: Vec<(usize, f64, f64)> = order
            .iter()
            .map(|&(i, start)| (indices[i], start, clips[i].duration_secs))
            .collect();

        // Which file each place shows for the chosen channel.
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
                        row: 0,
                    });
                }
                Shown::Missing { take } => {
                    if !missing.iter().any(|m| &m.take == take) {
                        missing.push(MissingClip {
                            take: take.clone(),
                            timeline_offset_secs: offset,
                            duration_secs: duration,
                            row: 0,
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

        // Clips that overlap in time each get a row. A place left empty by a
        // channel that is not loaded keeps a row too.
        let spans: Vec<(f64, f64)> = segments
            .iter()
            .map(|s| {
                (
                    s.timeline_offset_secs,
                    s.timeline_offset_secs + s.duration_secs,
                )
            })
            .chain(missing.iter().map(|m| {
                (
                    m.timeline_offset_secs,
                    m.timeline_offset_secs + m.duration_secs,
                )
            }))
            .collect();
        let rows = assign_rows(&spans);
        let row_count = rows.iter().max().map_or(1, |r| r + 1);
        for (seg, &row) in segments.iter_mut().zip(&rows) {
            seg.row = row;
        }
        for (m, &row) in missing.iter_mut().zip(&rows[segments.len()..]) {
            m.row = row;
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

        // The first clip with the highest sample rate.
        let mut primary_file_index = None;
        let mut top_rate = 0;
        for seg in &segments {
            let rate = files.get(seg.file_index).map_or(0, |f| f.audio.sample_rate);
            if rate > top_rate {
                top_rate = rate;
                primary_file_index = Some(seg.file_index);
            }
        }

        Some(TimelineView {
            segments,
            total_duration_secs,
            origin_epoch_ms: origin_ms,
            arrangement,
            row_count,
            primary_file_index,
            tracks: plan.tracks,
            take_count: plan.take_count,
            track: plan.track,
            shown_track: plan.shown_track,
            missing,
        })
    }

    /// The arrangement a new timeline of these files starts with. See
    /// [`suggest_arrangement`].
    pub fn suggested_arrangement(file_indices: &[usize], files: &[LoadedFile]) -> Arrangement {
        let channels: Vec<Option<TrackInfo>> = files
            .iter()
            .map(|f| file_groups::channel_of(&f.name))
            .collect();
        let indices: Vec<usize> = file_indices
            .iter()
            .copied()
            .filter(|&i| i < files.len())
            .collect();
        suggest_arrangement(&clips_in(&indices, files, &channels))
    }

    /// The clip that sets the frequency range and the zoom unit: see
    /// `primary_file_index`.
    pub fn primary_segment(&self) -> Option<&TimelineSegment> {
        self.primary_file_index
            .and_then(|i| self.segment_for_file(i))
            .or_else(|| self.segments.first())
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

/// What [`place_clips`] needs to know about each file. `indices` must all be
/// inside `files`.
fn clips_in(
    indices: &[usize],
    files: &[LoadedFile],
    channels: &[Option<TrackInfo>],
) -> Vec<ClipIn> {
    indices
        .iter()
        .map(|&i| ClipIn {
            start_ms: files[i].recording_start_epoch_ms(),
            duration_secs: files[i].audio.duration_secs,
            take: channels[i].as_ref().map(|c| c.group_key.clone()),
        })
        .collect()
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

/// Rebuild the open timeline from the files added to it, the chosen channel
/// and the arrangement. Call after the file list changes. Clears the chosen
/// channel when the timeline no longer offers it.
pub fn rebuild_active(state: &AppState) {
    let sel = state.timeline.clip_file_indices().get_untracked();
    let track = state.timeline.active_track().get_untracked();
    let arrangement = state.timeline.arrangement().get_untracked();
    let rebuilt = state.library.files().with_untracked(|files| {
        TimelineView::from_files(&sel, files, track.as_deref(), arrangement)
    });
    let applied = rebuilt.as_ref().and_then(|t| t.track.clone());
    if applied != track {
        state.timeline.active_track().set(applied);
    }
    state.timeline.active().set(rebuilt);
}

/// Open a timeline of the files at `file_indices` (indices into the loaded
/// files) and show all of it. Returns the timeline, or `None` when fewer
/// than two of the files are loaded.
///
/// `arrangement`: `None` picks one from the files' recording times, and then
/// also puts the files in name order. `definition_id` is the timeline's id in
/// the project, when it is saved there.
pub fn open(
    state: &AppState,
    mut file_indices: Vec<usize>,
    arrangement: Option<Arrangement>,
    definition_id: Option<String>,
) -> Option<TimelineView> {
    let files = state.library.files().get_untracked();
    file_indices.retain(|&i| i < files.len());
    let arrangement = arrangement.unwrap_or_else(|| {
        file_indices.sort_by(|&a, &b| natural_name_order(&files[a].name, &files[b].name));
        TimelineView::suggested_arrangement(&file_indices, &files)
    });
    let tv = TimelineView::from_files(&file_indices, &files, None, arrangement)?;
    // The Files list marks the timeline's files as selected.
    state
        .timeline
        .selected_file_indices()
        .set(file_indices.clone());
    state.timeline.clip_file_indices().set(file_indices);
    state.timeline.arrangement().set(arrangement);
    state.timeline.active_definition_id().set(definition_id);
    state.timeline.only_row().set(None);
    state.timeline.active_track().set(None);
    state.timeline.active().set(Some(tv.clone()));
    state.library.current_index().set(None);
    state.suspend_follow();
    show_whole_timeline(state, &tv);
    Some(tv)
}

/// Close the open timeline.
pub fn close(state: &AppState) {
    state.timeline.active().set(None);
    state.timeline.active_track().set(None);
    state.timeline.active_definition_id().set(None);
    state.timeline.selected_file_indices().set(Vec::new());
    state.timeline.clip_file_indices().set(Vec::new());
}

/// Zoom and scroll so the whole timeline fits the spectrogram.
fn show_whole_timeline(state: &AppState, tv: &TimelineView) {
    let duration = tv.total_duration_secs;
    let time_res = state.library.files().with_untracked(|files| {
        tv.primary_segment()
            .and_then(|s| files.get(s.file_index))
            .map_or(1.0, |f| f.spectrogram.time_resolution)
    });
    let canvas_w = state.viewmode.spectrogram_canvas_width().get_untracked();
    if canvas_w > 0.0 && time_res > 0.0 && duration > 0.0 {
        let zoom = ((canvas_w * time_res) / duration)
            .clamp(crate::viewport::MIN_ZOOM, crate::viewport::MAX_ZOOM);
        state.view.zoom_level().set(zoom);
        let visible_time = crate::viewport::visible_time(canvas_w, zoom, time_res);
        let from_here = state.playback.start_mode().get_untracked().uses_from_here();
        state
            .view
            .scroll_offset()
            .set(crate::viewport::clamp_scroll_for_mode(
                0.0,
                duration,
                visible_time,
                from_here,
            ));
    } else {
        state.view.scroll_offset().set(0.0);
    }
}

/// Place the open timeline's clips another way, and show all of the
/// timeline again, because its length changes. The choice is saved with the
/// timeline in the project. Audio that is playing stops: its position would
/// land in a different clip.
pub fn set_arrangement(state: &AppState, arrangement: Arrangement) {
    if state.timeline.arrangement().get_untracked() == arrangement {
        return;
    }
    crate::audio::playback::stop(state);
    state.timeline.only_row().set(None);
    state.timeline.arrangement().set(arrangement);
    state.interaction.selection().set(None);
    rebuild_active(state);
    if let Some(tv) = state.timeline.active().get_untracked() {
        show_whole_timeline(state, &tv);
    }
    state
        .viewmode
        .tile_ready_signal()
        .update(|n| *n = n.wrapping_add(1));
    save_active_to_project(state);
}

/// Make a timeline of the files at `file_indices`, open it, and add it to
/// the open project when there is one. Returns `false` when fewer than two
/// of the files are loaded.
pub fn create(state: &AppState, file_indices: Vec<usize>) -> bool {
    if open(state, file_indices, None, None).is_none() {
        return false;
    }
    let id = crate::annotations::generate_uuid();
    let mut added = false;
    state.project.current().update(|p| {
        if let Some(proj) = p {
            proj.timelines.push(crate::project::TimelineDefinition {
                id: id.clone(),
                entries: Vec::new(),
                arrangement: None,
                label: None,
            });
            added = true;
        }
    });
    if added {
        state.timeline.active_definition_id().set(Some(id));
        save_active_to_project(state);
    }
    true
}

/// Move the file at position `from` in the open timeline's play order to
/// position `to`. The order decides where the files go when they are placed
/// one after another. Audio that is playing stops.
pub fn move_clip(state: &AppState, from: usize, to: usize) {
    let mut order = state.timeline.clip_file_indices().get_untracked();
    if from == to || from >= order.len() || to >= order.len() {
        return;
    }
    crate::audio::playback::stop(state);
    state.timeline.only_row().set(None);
    let moved = order.remove(from);
    order.insert(to, moved);
    state.timeline.clip_file_indices().set(order);
    state.interaction.selection().set(None);
    rebuild_active(state);
    state
        .viewmode
        .tile_ready_signal()
        .update(|n| *n = n.wrapping_add(1));
    save_active_to_project(state);
}

/// Write the open timeline's files, their order and the arrangement to its
/// definition in the project. Does nothing when the timeline is not in a
/// project. A file of the definition that is not loaded now stays in it,
/// after the loaded ones.
fn save_active_to_project(state: &AppState) {
    let Some(id) = state.timeline.active_definition_id().get_untracked() else {
        return;
    };
    let Some(tv) = state.timeline.active().get_untracked() else {
        return;
    };
    let order = state.timeline.clip_file_indices().get_untracked();
    let files = state.library.files().get_untracked();
    let mut saved = false;
    state.project.current().update(|p| {
        let Some(proj) = p else { return };
        let mut entries: Vec<crate::project::TimelineEntry> = order
            .iter()
            .filter_map(|&i| {
                let loaded = files.get(i)?;
                let file_index = proj.find_file(loaded.identity.as_ref()?)?;
                let offset = tv
                    .segment_for_file(i)
                    .map_or(0.0, |seg| seg.timeline_offset_secs);
                Some(crate::project::TimelineEntry {
                    file_index,
                    start_epoch_ms: tv.origin_epoch_ms + offset * 1000.0,
                    duration_secs: loaded.audio.duration_secs,
                    multitrack_group_id: None,
                })
            })
            .collect();
        let Some(pos) = proj.timelines.iter().position(|t| t.id == id) else {
            return;
        };
        if entries.is_empty() && proj.timelines[pos].entries.is_empty() {
            // None of the files is in the project: nothing to save.
            proj.timelines.remove(pos);
            return;
        }
        let def = &mut proj.timelines[pos];
        let kept: Vec<_> = def
            .entries
            .iter()
            .filter(|old| !entries.iter().any(|e| e.file_index == old.file_index))
            .cloned()
            .collect();
        entries.extend(kept);
        def.entries = entries;
        def.arrangement = Some(tv.arrangement);
        proj.touch();
        saved = true;
    });
    if saved {
        state.project.dirty().set(true);
    } else if state.project.current().with_untracked(|p| {
        p.as_ref()
            .is_some_and(|proj| !proj.timelines.iter().any(|t| t.id == id))
    }) {
        state.timeline.active_definition_id().set(None);
    }
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

    fn clip(start_secs: Option<f64>, duration_secs: f64, take: Option<&str>) -> ClipIn {
        ClipIn {
            start_ms: start_secs.map(|s| 1_700_000_000_000.0 + s * 1000.0),
            duration_secs,
            take: take.map(str::to_string),
        }
    }

    fn starts(clips: &[ClipIn], arrangement: Arrangement) -> Vec<(usize, f64)> {
        place_clips(clips, arrangement).0
    }

    #[test]
    fn in_turn_puts_each_clip_after_the_one_before() {
        // Three files with the same file date, as after a copy.
        let clips = [
            clip(Some(0.0), 2.0, None),
            clip(Some(0.0), 3.0, None),
            clip(Some(0.0), 1.5, None),
        ];
        assert_eq!(
            starts(&clips, Arrangement::InTurn { gap_secs: 0.0 }),
            vec![(0, 0.0), (1, 2.0), (2, 5.0)]
        );
        assert_eq!(
            starts(&clips, Arrangement::InTurn { gap_secs: 0.5 }),
            vec![(0, 0.0), (1, 2.5), (2, 6.0)]
        );
    }

    #[test]
    fn in_turn_has_no_clock_time() {
        let clips = [clip(Some(5.0), 2.0, None), clip(Some(9.0), 2.0, None)];
        assert_eq!(
            place_clips(&clips, Arrangement::InTurn { gap_secs: 1.0 }).1,
            0.0
        );
        assert!(place_clips(&clips, Arrangement::RecordingTime).1 > 0.0);
    }

    #[test]
    fn in_turn_starts_the_channels_of_one_take_together() {
        let clips = [
            clip(None, 2.0, Some("a_0010")),
            clip(None, 2.5, Some("a_0010")),
            clip(None, 1.0, Some("a_0011")),
            clip(None, 4.0, None),
        ];
        // The take's place is as long as its longest channel file.
        assert_eq!(
            starts(&clips, Arrangement::InTurn { gap_secs: 1.0 }),
            vec![(0, 0.0), (1, 0.0), (2, 3.5), (3, 5.5)]
        );
    }

    #[test]
    fn together_starts_every_clip_at_the_start() {
        let clips = [
            clip(Some(0.0), 2.0, None),
            clip(Some(900.0), 3.0, None),
            clip(None, 1.0, None),
        ];
        assert_eq!(
            starts(&clips, Arrangement::Together),
            vec![(0, 0.0), (1, 0.0), (2, 0.0)]
        );
        assert_eq!(place_clips(&clips, Arrangement::Together).1, 0.0);
    }

    #[test]
    fn rows_split_the_view_evenly() {
        assert_eq!(row_rect(0, 1, 600.0), (0.0, 600.0));
        assert_eq!(row_rect(1, 3, 600.0), (200.0, 200.0));
        assert_eq!(row_at(0.0, 3, 600.0), (0, 0.0, 200.0));
        assert_eq!(row_at(250.0, 3, 600.0), (1, 50.0, 200.0));
        // Past either edge, as during a drag: the nearest row.
        assert_eq!(row_at(-5.0, 3, 600.0).0, 0);
        assert_eq!(row_at(700.0, 3, 600.0).0, 2);
        // One row is the whole view.
        assert_eq!(row_at(250.0, 1, 600.0), (0, 250.0, 600.0));
    }

    #[test]
    fn clips_that_do_not_overlap_share_one_row() {
        assert_eq!(
            assign_rows(&[(0.0, 2.0), (2.0, 5.0), (6.0, 7.0)]),
            vec![0, 0, 0]
        );
        // Recorder files that overlap by a few milliseconds still share it.
        assert_eq!(
            assign_rows(&[(0.0, 2.0), (1.99, 4.0), (3.98, 6.0)]),
            vec![0, 0, 0]
        );
        assert_eq!(assign_rows(&[]), Vec::<usize>::new());
    }

    #[test]
    fn clips_that_overlap_get_a_row_each() {
        // Three files that start together.
        assert_eq!(
            assign_rows(&[(0.0, 2.0), (0.0, 3.0), (0.0, 1.5)]),
            vec![0, 1, 2]
        );
        // The third starts after the first has ended, so it goes back to row 0.
        assert_eq!(
            assign_rows(&[(0.0, 2.0), (1.0, 4.0), (2.5, 5.0)]),
            vec![0, 1, 0]
        );
    }

    #[test]
    fn a_bad_gap_counts_as_no_gap() {
        let clips = [clip(None, 2.0, None), clip(None, 2.0, None)];
        for gap_secs in [-3.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                starts(&clips, Arrangement::InTurn { gap_secs }),
                vec![(0, 0.0), (1, 2.0)]
            );
        }
    }

    #[test]
    fn recording_time_orders_by_start_and_puts_untimed_clips_last() {
        let clips = [
            clip(None, 1.0, None),
            clip(Some(10.0), 2.0, None),
            clip(Some(0.0), 2.0, None),
        ];
        assert_eq!(
            starts(&clips, Arrangement::RecordingTime),
            vec![(2, 0.0), (1, 10.0), (0, 12.0)]
        );
    }

    #[test]
    fn a_new_timeline_keeps_recording_times_that_are_close_and_apart() {
        let clips = [
            clip(Some(0.0), 2.0, None),
            clip(Some(10.0), 2.0, None),
            clip(Some(20.0), 2.0, None),
        ];
        assert_eq!(suggest_arrangement(&clips), Arrangement::RecordingTime);
        // Two channels of one take start together: that is not an overlap.
        let takes = [
            clip(Some(0.0), 2.0, Some("a_0010")),
            clip(Some(0.0), 2.0, Some("a_0010")),
            clip(Some(10.0), 2.0, Some("a_0011")),
        ];
        assert_eq!(suggest_arrangement(&takes), Arrangement::RecordingTime);
    }

    #[test]
    fn a_new_timeline_goes_in_turn_when_recording_times_do_not_work() {
        let in_turn = Arrangement::InTurn { gap_secs: 0.0 };
        // Copied together: the same file date.
        let same = [clip(Some(0.0), 2.0, None), clip(Some(-1.0), 3.0, None)];
        assert_eq!(suggest_arrangement(&same), in_turn);
        // Recorded on different nights.
        let far = [clip(Some(0.0), 2.0, None), clip(Some(86_400.0), 2.0, None)];
        assert_eq!(suggest_arrangement(&far), in_turn);
        // One file has no time.
        let untimed = [clip(Some(0.0), 2.0, None), clip(None, 2.0, None)];
        assert_eq!(suggest_arrangement(&untimed), in_turn);
    }

    #[test]
    fn names_sort_with_their_numbers_counted() {
        let mut names = vec![
            "rec_10.wav",
            "Rec_9.wav",
            "rec_009b.wav",
            "alpha.wav",
            "rec_9.wav",
        ];
        names.sort_by(|a, b| natural_name_order(a, b));
        assert_eq!(
            names,
            vec![
                "alpha.wav",
                "Rec_9.wav",
                "rec_9.wav",
                "rec_009b.wav",
                "rec_10.wav"
            ]
        );
    }

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
