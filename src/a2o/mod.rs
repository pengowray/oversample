//! Australian Acoustic Observatory (A2O) links.
//!
//! A2O recordings are hours long, so the app never opens a whole one. A link
//! names a clip: an annotated event, or a start and end offset. The clip is
//! downloaded as FLAC and opened like any other file, with a note of where in
//! the recording it came from ([`ClipRef`]). That note lets the app fetch the
//! audio just before or after the clip and put it on the same timeline.
//!
//! - `link`: reading a pasted link, clip times and clip names.
//! - `api`: requests to A2O's servers and to the Oversample server.
//! - this file: opening a clip, the project it goes in, and adding the audio
//!   before and after it.
//!
//! A2O data is licensed CC BY 4.0
//! (acousticobservatory.org/data/data-submission-and-usage-terms).

mod api;
mod link;

pub use api::{fetch_event, fetch_recording, EventInfo, RecordingInfo};
pub use link::{adjacent_range, clip_file_name, clip_range, length_text, parse_link, A2oLink};

use crate::components::file_sidebar::{fetch_bytes, load_named_bytes_id};
use crate::source_site::{SiteMetadata, SourceSite};
use crate::state::store_fields::*;
use crate::state::AppState;
use api::{clip_url, fetch_site_name, fetch_tag};
use leptos::prelude::*;
use link::{clock, secs_text};
use serde::{Deserialize, Serialize};

/// The page for a recording on A2O's own site.
const LISTEN_PAGE: &str = "https://data.acousticobservatory.org/listen";

/// The longest clip A2O serves in one request, in seconds. A request for
/// more is refused (HTTP 422).
pub const MAX_CLIP_SECS: f64 = 300.0;
/// The clip opened for a link that names a recording and no time, and the
/// amount of audio "Add before" and "Add after" fetch, in seconds.
pub const ADD_SECS: f64 = 30.0;
/// The amounts of audio "Add before" and "Add after" can fetch, in seconds.
/// The longest is the most A2O serves in one request.
pub const ADD_CHOICES_SECS: [f64; 4] = [10.0, 30.0, 60.0, MAX_CLIP_SECS];

/// The amount "Add before" and "Add after" fetch: the user's choice from the
/// length menu, kept between sessions, or [`ADD_SECS`].
pub fn add_secs() -> f64 {
    crate::settings::get_raw(crate::settings::keys::A2O_ADD_SECS)
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|secs| ADD_CHOICES_SECS.contains(secs))
        .unwrap_or(ADD_SECS)
}

pub fn set_add_secs(secs: f64) {
    if ADD_CHOICES_SECS.contains(&secs) {
        crate::settings::set_raw(crate::settings::keys::A2O_ADD_SECS, &secs.to_string());
    }
}

/// Where a file came from in an A2O recording. Kept on the open file and
/// saved in a project, so the clip can be fetched again and the audio around
/// it can be added.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClipRef {
    pub recording_id: u64,
    /// The clip's start and end, in seconds from the start of the recording.
    pub start_secs: f64,
    pub end_secs: f64,
    /// When the recording started, in ms since the epoch.
    pub recording_start_ms: f64,
    /// The whole recording's length in seconds.
    pub recording_secs: f64,
}

impl ClipRef {
    /// When the clip itself started, in ms since the epoch.
    pub fn start_ms(&self) -> f64 {
        self.recording_start_ms + self.start_secs * 1000.0
    }
}

// ── Opening a clip ──────────────────────────────────────────────────────────

/// The rows shown for an A2O clip in the Info panel and the licence dialog.
fn clip_fields(
    recording: &RecordingInfo,
    clip: &ClipRef,
    site_name: Option<String>,
    tags: &[String],
) -> Vec<(String, String)> {
    let mut fields = Vec::new();
    if let Some(name) = site_name {
        fields.push(("Site".to_string(), name));
    }
    if !tags.is_empty() {
        fields.push(("Tags".to_string(), tags.join(", ")));
    }
    fields.push((
        "Recording started".to_string(),
        recording.recorded_date.clone(),
    ));
    if let Some(tz) = &recording.timezone {
        fields.push(("Time zone".to_string(), tz.clone()));
    }
    fields.push((
        "Clip".to_string(),
        format!(
            "{} to {} of {}",
            clock(clip.start_secs),
            clock(clip.end_secs),
            clock(clip.recording_secs)
        ),
    ));
    fields.push(("Recording ID".to_string(), recording.id.to_string()));
    fields.push((
        "Author".to_string(),
        "Australian Acoustic Observatory".to_string(),
    ));
    fields.push(("License".to_string(), "CC BY 4.0".to_string()));
    fields.push((
        "License URL".to_string(),
        "https://creativecommons.org/licenses/by/4.0/".to_string(),
    ));
    fields.push((
        "URL".to_string(),
        format!(
            "{LISTEN_PAGE}/{}?start={}&end={}",
            recording.id,
            secs_text(clip.start_secs.floor()),
            secs_text(clip.end_secs.ceil())
        ),
    ));
    fields
}

/// Download one clip and open it as a file. Returns the new file's id.
async fn open_clip(
    state: AppState,
    recording: &RecordingInfo,
    start_secs: f64,
    end_secs: f64,
    tags: &[String],
) -> Result<u64, String> {
    let clip = ClipRef {
        recording_id: recording.id,
        start_secs,
        end_secs,
        recording_start_ms: recording.recorded_ms,
        recording_secs: recording.duration_secs,
    };
    // The same clip is open already: show that file. A second copy would
    // sit on top of the first on a timeline.
    let open_already = state.library.files().with_untracked(|files| {
        files
            .iter()
            .position(|f| f.origin_clip.as_ref() == Some(&clip))
            .map(|i| (i, files[i].id))
    });
    if let Some((index, id)) = open_already {
        if state.timeline.active().with_untracked(|t| t.is_none()) {
            state.library.current_index().set(Some(index));
        }
        return Ok(id);
    }
    let name = clip_file_name(&recording.file_name, start_secs, end_secs);
    let load_id = state.loading_start(&name);
    let result = async {
        let bytes = fetch_bytes(&clip_url(recording.id, start_secs, end_secs))
            .await
            .map_err(|e| format!("A2O did not send the clip: {e}"))?;
        let site_name = match recording.site_id {
            Some(id) => fetch_site_name(id).await,
            None => None,
        };
        let fields = clip_fields(recording, &clip, site_name, tags);
        let id = load_named_bytes_id(
            name.clone(),
            &bytes,
            Some(SiteMetadata::new(SourceSite::AcousticObservatory, fields)),
            None,
            None,
            state,
            load_id,
            false,
        )
        .await?;
        // Mark where the file came from: its start time on a timeline, and
        // what "Add before" and "Add after" fetch.
        state.library.files().update(|files| {
            if let Some(file) = files.iter_mut().find(|f| f.id == id) {
                file.origin_clip = Some(clip.clone());
            }
        });
        Ok(id)
    }
    .await;
    state.loading_done(load_id);
    result
}

/// Fetch a clip a project saved, and open it. Returns the new file's id.
pub async fn open_saved_clip(state: AppState, clip: &ClipRef) -> Result<u64, String> {
    let recording = fetch_recording(clip.recording_id).await?;
    let (start, end) = clip_range(
        Some(clip.start_secs),
        Some(clip.end_secs),
        recording.duration_secs,
    )
    .ok_or("The saved clip is past the end of its recording")?;
    open_clip(state, &recording, start, end, &[]).await
}

/// Wait until the file's identity (its name, size and hashes) is worked
/// out, which a project needs to list the file. Gives up after 5 s.
async fn wait_for_identity(state: AppState, file_id: u64) {
    for _ in 0..50 {
        let ready = state.library.files().with_untracked(|files| {
            files
                .iter()
                .find(|f| f.id == file_id)
                .is_none_or(|f| f.identity.is_some())
        });
        if ready {
            return;
        }
        crate::web_util::sleep_ms(100).await;
    }
}

/// Open what an A2O link names, and put the clip in a project, so that
/// audio added before and after it is saved with it (see
/// `put_a2o_clip_in_project` for which project). Returns the file's id.
pub async fn load_link(state: AppState, link: &A2oLink) -> Result<u64, String> {
    let open_before: Vec<u64> = state
        .library
        .files()
        .with_untracked(|files| files.iter().map(|f| f.id).collect());
    let id = open_link(state, link).await?;
    // A clip that was open already keeps the project it is in, if any.
    if !open_before.contains(&id) {
        wait_for_identity(state, id).await;
        let name = state.library.files().with_untracked(|files| {
            let file = files.iter().find(|f| f.id == id)?;
            let site = file
                .site_metadata
                .as_ref()?
                .fields
                .iter()
                .find(|(key, _)| key == "Site")
                .map(|(_, value)| value.clone());
            let recording = file.origin_clip.as_ref()?.recording_id;
            Some(project_name(site.as_deref(), recording))
        });
        if let Some(name) = name {
            crate::components::file_sidebar::put_a2o_clip_in_project(state, id, name);
        }
    }
    Ok(id)
}

/// The name of the project made for an A2O link: the site's name, or the
/// recording's number when the site is not known.
pub fn project_name(site: Option<&str>, recording_id: u64) -> String {
    match site.map(str::trim).filter(|s| !s.is_empty()) {
        Some(site) => format!("A2O {site}"),
        None => format!("A2O recording {recording_id}"),
    }
}

async fn open_link(state: AppState, link: &A2oLink) -> Result<u64, String> {
    match *link {
        A2oLink::Event {
            recording_id,
            event_id,
        } => {
            let event = fetch_event(&state, recording_id, event_id).await?;
            let recording = fetch_recording(recording_id).await?;
            let (start, end) = clip_range(
                Some(event.start_secs),
                Some(event.end_secs),
                recording.duration_secs,
            )
            .ok_or("The event is past the end of its recording")?;
            let mut tags = Vec::new();
            for &id in event.tag_ids.iter().take(8) {
                if let Some(text) = fetch_tag(&state, id).await {
                    tags.push(text);
                }
            }
            open_clip(state, &recording, start, end, &tags).await
        }
        A2oLink::Clip {
            recording_id,
            start_secs,
            end_secs,
        } => {
            let recording = fetch_recording(recording_id).await?;
            let (start, end) = clip_range(start_secs, end_secs, recording.duration_secs)
                .ok_or("The link's start time is past the end of the recording")?;
            open_clip(state, &recording, start, end, &[]).await
        }
    }
}

/// Open a pasted link or URL-hash target, and report a failure as a toast.
pub async fn load_from_input(state: AppState, input: &str) {
    let Some(link) = parse_link(input) else {
        state.show_error_toast("Not an A2O recording link");
        return;
    };
    if let Err(e) = load_link(state, &link).await {
        log::error!("A2O link {input:?} failed: {e}");
        state.show_error_toast(e);
    }
}

// ── Audio before and after ──────────────────────────────────────────────────

/// The A2O audio that is open now, as one stretch of one recording: the
/// clips on the open timeline, or the one open file. `None` when what is
/// open is not from A2O, or the timeline's clips are from more than one
/// recording.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenStretch {
    pub recording_id: u64,
    pub start_secs: f64,
    pub end_secs: f64,
    pub recording_secs: f64,
}

impl OpenStretch {
    pub fn can_add(&self, before: bool) -> bool {
        // Whether any audio is left on that side: the amount does not matter.
        adjacent_range(
            self.start_secs,
            self.end_secs,
            self.recording_secs,
            before,
            ADD_SECS,
        )
        .is_some()
    }
}

/// The stretch the given clips cover, when they are all from one recording.
fn stretch_of(clips: &[&ClipRef]) -> Option<OpenStretch> {
    let first = clips.first()?;
    if clips.iter().any(|c| c.recording_id != first.recording_id) {
        return None;
    }
    Some(OpenStretch {
        recording_id: first.recording_id,
        start_secs: clips
            .iter()
            .map(|c| c.start_secs)
            .fold(f64::INFINITY, f64::min),
        end_secs: clips.iter().map(|c| c.end_secs).fold(0.0, f64::max),
        recording_secs: first.recording_secs,
    })
}

/// See [`OpenStretch`]. Reads the open timeline, the open file and the file
/// list as signals, so a view that calls it follows them.
pub fn open_stretch(state: &AppState) -> Option<OpenStretch> {
    let on_timeline = state.timeline.active().with(|t| t.is_some());
    let indices: Vec<usize> = if on_timeline {
        state.timeline.clip_file_indices().get()
    } else {
        state.library.current_index().get().into_iter().collect()
    };
    state.library.files().with(|files| {
        let clips: Option<Vec<&ClipRef>> = indices
            .iter()
            .map(|&i| files.get(i).and_then(|f| f.origin_clip.as_ref()))
            .collect();
        stretch_of(&clips?)
    })
}

/// Fetch the audio just before or just after what is open, and put it on
/// the timeline beside it. One open file and the new clip become a timeline.
pub async fn add_adjacent(state: AppState, before: bool, add_secs: f64) -> Result<(), String> {
    let stretch = open_stretch_untracked(&state).ok_or("The open audio is not an A2O clip")?;
    let (start, end) = adjacent_range(
        stretch.start_secs,
        stretch.end_secs,
        stretch.recording_secs,
        before,
        add_secs,
    )
    .ok_or(if before {
        "This is the start of the recording"
    } else {
        "This is the end of the recording"
    })?;
    // The files that are open now, by id: indices change while a file loads.
    let open_ids: Vec<u64> = state.library.files().with_untracked(|files| {
        let indices = if state.timeline.active().with_untracked(|t| t.is_some()) {
            state.timeline.clip_file_indices().get_untracked()
        } else {
            state
                .library
                .current_index()
                .get_untracked()
                .into_iter()
                .collect()
        };
        indices
            .iter()
            .filter_map(|&i| files.get(i).map(|f| f.id))
            .collect()
    });
    let recording = fetch_recording(stretch.recording_id).await?;
    let new_id = open_clip(state, &recording, start, end, &[]).await?;
    // With a project open, the new clip joins it, so the timeline saved in
    // the project lists every clip.
    if state.project.current().with_untracked(|p| p.is_some()) {
        wait_for_identity(state, new_id).await;
        crate::components::file_sidebar::add_open_files_to_project(state, Some(new_id));
    }

    let mut ids = open_ids;
    if before {
        ids.insert(0, new_id);
    } else {
        ids.push(new_id);
    }
    let indices: Vec<usize> = state.library.files().with_untracked(|files| {
        ids.iter()
            .filter_map(|id| files.iter().position(|f| f.id == *id))
            .collect()
    });
    crate::timeline::set_files(&state, indices);
    Ok(())
}

fn open_stretch_untracked(state: &AppState) -> Option<OpenStretch> {
    untrack(|| open_stretch(state))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_link_project_is_named_after_the_site() {
        assert_eq!(
            project_name(Some("Tarcutta Hills Wet A"), 55743),
            "A2O Tarcutta Hills Wet A"
        );
        assert_eq!(project_name(Some("  "), 55743), "A2O recording 55743");
        assert_eq!(project_name(None, 55743), "A2O recording 55743");
    }

    #[test]
    fn open_clips_of_one_recording_make_one_stretch() {
        let clip = |recording_id, start_secs, end_secs| ClipRef {
            recording_id,
            start_secs,
            end_secs,
            recording_start_ms: 0.0,
            recording_secs: 1000.0,
        };
        let (a, b, other) = (clip(7, 30.0, 40.0), clip(7, 0.0, 30.0), clip(8, 0.0, 30.0));
        let stretch = stretch_of(&[&a, &b]).unwrap();
        assert_eq!((stretch.start_secs, stretch.end_secs), (0.0, 40.0));
        assert!(!stretch.can_add(true));
        assert!(stretch.can_add(false));
        assert_eq!(stretch_of(&[&a, &other]), None);
        assert_eq!(stretch_of(&[]), None);
    }
}
