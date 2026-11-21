//! Australian Acoustic Observatory (A2O) links.
//!
//! A2O recordings are hours long, so the app never opens a whole one. A link
//! names a clip: an annotated event, or a start and end offset. The clip is
//! downloaded as FLAC and opened like any other file, with a note of where in
//! the recording it came from ([`ClipRef`]). That note lets the app fetch the
//! audio just before or after the clip and put it on the same timeline.
//!
//! Two A2O servers are used:
//!
//! - The search API (`api.search.acousticobservatory.org`) serves clips,
//!   recording details and site details to any website.
//! - The main API (`api.acousticobservatory.org`) serves event details and
//!   tags. It answers pages on localhost and the desktop and Android apps,
//!   and refuses other websites, including app.oversample.com. Only
//!   [`fetch_event`] and [`fetch_tag`] use it.
//!
//! A2O data is licensed CC BY 4.0
//! (acousticobservatory.org/data/data-submission-and-usage-terms).

use crate::components::file_sidebar::{fetch_bytes, fetch_text, load_named_bytes_id};
use crate::source_site::{SiteMetadata, SourceSite};
use crate::state::store_fields::*;
use crate::state::AppState;
use leptos::prelude::*;
use serde::{Deserialize, Serialize};

const SEARCH_API: &str = "https://api.search.acousticobservatory.org/api/v1/a2o";
const MAIN_API: &str = "https://api.acousticobservatory.org";
/// The page for a recording on A2O's own site.
const LISTEN_PAGE: &str = "https://data.acousticobservatory.org/listen";

/// The longest clip A2O serves in one request, in seconds. A request for
/// more is refused (HTTP 422).
pub const MAX_CLIP_SECS: f64 = 300.0;
/// The clip opened for a link that names a recording and no time, and the
/// amount of audio "Add before" and "Add after" fetch, in seconds.
pub const ADD_SECS: f64 = 30.0;

/// What an A2O link names.
#[derive(Clone, Debug, PartialEq)]
pub enum A2oLink {
    /// An annotated event in a recording. Its times come from the main API.
    Event { recording_id: u64, event_id: u64 },
    /// A recording from `start_secs` to `end_secs`. `None` for a link that
    /// leaves one out.
    Clip {
        recording_id: u64,
        start_secs: Option<f64>,
        end_secs: Option<f64>,
    },
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

// ── Links ───────────────────────────────────────────────────────────────────

/// The part of a URL after its host, when the host is one of A2O's:
/// `(host, path, query)`. A host that only contains an A2O name
/// ("acousticobservatory.org.example.com") is not A2O's.
fn split_a2o_url(text: &str) -> Option<(&str, &str, &str)> {
    let rest = text
        .trim()
        .strip_prefix("https://")
        .or_else(|| text.trim().strip_prefix("http://"))?;
    let (host, after) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, ""),
    };
    const HOSTS: [&str; 3] = [
        "data.acousticobservatory.org",
        "api.acousticobservatory.org",
        "api.search.acousticobservatory.org",
    ];
    let host = HOSTS.into_iter().find(|h| host.eq_ignore_ascii_case(h))?;
    let after = after.split('#').next().unwrap_or("");
    let (path, query) = after.split_once('?').unwrap_or((after, ""));
    Some((host, path, query))
}

/// A whole, plain number: digits only, so "12abc" and "-3" are not ids.
fn parse_id(s: &str) -> Option<u64> {
    (!s.is_empty() && s.len() <= 12 && s.bytes().all(|b| b.is_ascii_digit()))
        .then(|| s.parse().ok())
        .flatten()
}

/// A time in seconds from a query value: a finite number, 0 or more.
fn parse_secs(s: &str) -> Option<f64> {
    s.parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0)
}

fn query_value<'a>(query: &'a str, keys: &[&str]) -> Option<&'a str> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        keys.contains(&k).then_some(v)
    })
}

/// Read a pasted A2O link. `None` for anything else, including a link to
/// another host and an A2O page that names no recording.
///
/// Recognised:
/// - `data.acousticobservatory.org/library/<recording>/audio_events/<event>`
/// - `data.acousticobservatory.org/listen/<recording>?start=<s>&end=<s>`
/// - `api.search.acousticobservatory.org/api/v1/a2o/audio_recordings/download/flac/<recording>?start_offset=<s>&end_offset=<s>`
/// - `api.acousticobservatory.org/audio_recordings/<recording>/media.flac?start_offset=<s>&end_offset=<s>`
/// - `api.acousticobservatory.org/audio_recordings/<recording>/audio_events/<event>`
pub fn parse_link(text: &str) -> Option<A2oLink> {
    let (_, path, query) = split_a2o_url(text)?;
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    let clip = |recording_id: u64| {
        let start_secs = query_value(query, &["start_offset", "start"]).and_then(parse_secs);
        let end_secs = query_value(query, &["end_offset", "end"]).and_then(parse_secs);
        A2oLink::Clip {
            recording_id,
            start_secs,
            end_secs,
        }
    };
    // The search API's paths start "api/v1/a2o".
    let parts = match parts.as_slice() {
        ["api", "v1", "a2o", rest @ ..] => rest,
        all => all,
    };
    match parts {
        ["library" | "audio_recordings", recording, "audio_events", event] => {
            let event = event.strip_suffix(".json").unwrap_or(event);
            Some(A2oLink::Event {
                recording_id: parse_id(recording)?,
                event_id: parse_id(event)?,
            })
        }
        ["audio_recordings", "download", _format, recording] => Some(clip(parse_id(recording)?)),
        ["audio_recordings", recording, media] if media.starts_with("media.") => {
            Some(clip(parse_id(recording)?))
        }
        ["listen" | "library", recording] => Some(clip(parse_id(recording)?)),
        _ => None,
    }
}

/// The clip to fetch for a link's start and end: whole seconds are not
/// needed, but the clip must lie inside the recording and be no longer than
/// A2O serves. A link without an end gets [`ADD_SECS`] from its start.
/// `None` when the start is at or past the end of the recording.
pub fn clip_range(
    start_secs: Option<f64>,
    end_secs: Option<f64>,
    recording_secs: f64,
) -> Option<(f64, f64)> {
    let start = start_secs.unwrap_or(0.0).max(0.0);
    if start >= recording_secs {
        return None;
    }
    let end = end_secs
        .filter(|&e| e > start)
        .unwrap_or(start + ADD_SECS)
        .min(start + MAX_CLIP_SECS)
        .min(recording_secs);
    (end > start).then_some((start, end))
}

/// The audio just before (`before`) or just after a stretch of a recording
/// that is already open, `(start, end)` in seconds: up to [`ADD_SECS`] of
/// it. `None` at the start or the end of the recording.
pub fn adjacent_range(
    open_start: f64,
    open_end: f64,
    recording_secs: f64,
    before: bool,
) -> Option<(f64, f64)> {
    let (start, end) = if before {
        ((open_start - ADD_SECS).max(0.0), open_start)
    } else {
        (open_end, (open_end + ADD_SECS).min(recording_secs))
    };
    // Under a tenth of a second is the rounding left at an end, not audio.
    (end - start > 0.1).then_some((start, end))
}

/// A number of seconds as a URL or file name writes it: "1506" or "1506.5".
fn secs_text(secs: f64) -> String {
    let text = format!("{secs:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn clip_url(recording_id: u64, start_secs: f64, end_secs: f64) -> String {
    format!(
        "{SEARCH_API}/audio_recordings/download/flac/{recording_id}?start_offset={}&end_offset={}",
        secs_text(start_secs),
        secs_text(end_secs)
    )
}

/// The clip's file name: the clip's start and end in seconds, then the
/// recording's own name. The times come first because file lists cut long
/// names short, and clips of one recording differ only in their times. The
/// "s" after each number keeps the file list from reading the numbers as a
/// channel or a sequence number.
pub fn clip_file_name(recording_name: &str, start_secs: f64, end_secs: f64) -> String {
    let stem = recording_name
        .rsplit_once('.')
        .map_or(recording_name, |(stem, _)| stem);
    format!(
        "{}s-{}s_{stem}.flac",
        secs_text(start_secs),
        secs_text(end_secs)
    )
}

/// A time in a recording as "h:mm:ss" or "m:ss".
fn clock(secs: f64) -> String {
    let total = secs.round() as u64;
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

// ── A2O's servers ───────────────────────────────────────────────────────────

/// A recording's details from the search API.
#[derive(Clone, Debug, PartialEq)]
pub struct RecordingInfo {
    pub id: u64,
    pub recorded_ms: f64,
    /// The recorded date as A2O writes it (UTC, ISO 8601).
    pub recorded_date: String,
    pub duration_secs: f64,
    pub site_id: Option<u64>,
    pub file_name: String,
    pub timezone: Option<String>,
}

/// An event's place in its recording, from the main API.
#[derive(Clone, Debug, PartialEq)]
pub struct EventInfo {
    pub start_secs: f64,
    pub end_secs: f64,
    pub tag_ids: Vec<u64>,
}

/// The `data` object of an A2O reply.
fn data_of(text: &str) -> Result<serde_json::Value, String> {
    let json: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| format!("A2O sent a reply that is not JSON: {e}"))?;
    match json.get("data") {
        Some(data) if !data.is_null() => Ok(data.clone()),
        _ => Err("A2O sent a reply without data".to_string()),
    }
}

/// Read a recording's details from the search API's reply.
pub fn parse_recording(
    text: &str,
    recorded_ms: impl Fn(&str) -> Option<f64>,
) -> Result<RecordingInfo, String> {
    let data = data_of(text)?;
    let id = data["id"]
        .as_u64()
        .ok_or("A2O's reply has no recording id")?;
    let recorded_date = data["recorded_date"]
        .as_str()
        .ok_or("A2O's reply has no recorded date")?
        .to_string();
    let duration_secs = data["duration_seconds"]
        .as_f64()
        .filter(|d| d.is_finite() && *d > 0.0)
        .ok_or("A2O's reply has no recording length")?;
    Ok(RecordingInfo {
        id,
        recorded_ms: recorded_ms(&recorded_date)
            .ok_or_else(|| format!("A2O's recorded date \"{recorded_date}\" is not a date"))?,
        duration_secs,
        site_id: data["site_id"].as_u64(),
        file_name: data["canonical_file_name"]
            .as_str()
            .or_else(|| data["original_file_name"].as_str())
            .map_or_else(|| format!("A2O_{id}.flac"), str::to_string),
        timezone: data["recorded_date_timezone"].as_str().map(str::to_string),
        recorded_date,
    })
}

/// Read an event's times and tag ids from the main API's reply.
pub fn parse_event(text: &str) -> Result<EventInfo, String> {
    let data = data_of(text)?;
    let secs = |key: &str| data[key].as_f64().filter(|v| v.is_finite() && *v >= 0.0);
    Ok(EventInfo {
        start_secs: secs("start_time_seconds").ok_or("A2O's reply has no event start")?,
        end_secs: secs("end_time_seconds").ok_or("A2O's reply has no event end")?,
        tag_ids: data["taggings"]
            .as_array()
            .map(|tags| tags.iter().filter_map(|t| t["tag_id"].as_u64()).collect())
            .unwrap_or_default(),
    })
}

fn iso_to_ms(date: &str) -> Option<f64> {
    let ms = js_sys::Date::parse(date);
    (!ms.is_nan()).then_some(ms)
}

pub async fn fetch_recording(recording_id: u64) -> Result<RecordingInfo, String> {
    let text = fetch_text(&format!("{SEARCH_API}/audio_recordings/{recording_id}"))
        .await
        .map_err(|e| format!("A2O recording {recording_id}: {e}"))?;
    parse_recording(&text, iso_to_ms)
}

/// The site's name, or `None` when it cannot be read. A clip opens without it.
async fn fetch_site_name(site_id: u64) -> Option<String> {
    let text = fetch_text(&format!("{SEARCH_API}/sites/{site_id}"))
        .await
        .ok()?;
    data_of(&text).ok()?["name"].as_str().map(str::to_string)
}

/// An event's details. Works on localhost and in the desktop and Android
/// apps. On another website the browser blocks the reply, because A2O's main
/// API does not allow that website to read it; the error says so.
pub async fn fetch_event(recording_id: u64, event_id: u64) -> Result<EventInfo, String> {
    let url = format!("{MAIN_API}/audio_recordings/{recording_id}/audio_events/{event_id}");
    match fetch_text(&url).await {
        Ok(text) => parse_event(&text),
        Err(e) if e.starts_with("HTTP ") => Err(format!("A2O event {event_id}: {e}")),
        // A blocked reply shows up as a failed fetch, with no status.
        Err(_) => Err(EVENT_BLOCKED.to_string()),
    }
}

/// Shown when the browser blocks the event lookup.
pub const EVENT_BLOCKED: &str =
    "A2O only gives event details to its own site and to the Oversample \
desktop and Android apps. Open this link in one of those apps, or paste a link with a start and \
end time.";

/// A tag's text ("birds"), or `None` when it cannot be read.
async fn fetch_tag(tag_id: u64) -> Option<String> {
    let text = fetch_text(&format!("{MAIN_API}/tags/{tag_id}"))
        .await
        .ok()?;
    data_of(&text).ok()?["text"].as_str().map(str::to_string)
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

/// Open what an A2O link names. Returns the new file's id.
pub async fn load_link(state: AppState, link: &A2oLink) -> Result<u64, String> {
    match *link {
        A2oLink::Event {
            recording_id,
            event_id,
        } => {
            let event = fetch_event(recording_id, event_id).await?;
            let recording = fetch_recording(recording_id).await?;
            let (start, end) = clip_range(
                Some(event.start_secs),
                Some(event.end_secs),
                recording.duration_secs,
            )
            .ok_or("The event is past the end of its recording")?;
            let mut tags = Vec::new();
            for &id in event.tag_ids.iter().take(8) {
                if let Some(text) = fetch_tag(id).await {
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
        adjacent_range(self.start_secs, self.end_secs, self.recording_secs, before).is_some()
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
pub async fn add_adjacent(state: AppState, before: bool) -> Result<(), String> {
    let stretch = open_stretch_untracked(&state).ok_or("The open audio is not an A2O clip")?;
    let (start, end) = adjacent_range(
        stretch.start_secs,
        stretch.end_secs,
        stretch.recording_secs,
        before,
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
        crate::components::file_sidebar::add_open_files_to_project(state);
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

    fn event(recording_id: u64, event_id: u64) -> Option<A2oLink> {
        Some(A2oLink::Event {
            recording_id,
            event_id,
        })
    }

    #[test]
    fn reads_library_event_links() {
        assert_eq!(
            parse_link("https://data.acousticobservatory.org/library/21934/audio_events/23087"),
            event(21934, 23087)
        );
        assert_eq!(
            parse_link("https://data.acousticobservatory.org/library/40863/audio_events/26264"),
            event(40863, 26264)
        );
        assert_eq!(
            parse_link("https://data.acousticobservatory.org/library/42262/audio_events/26725"),
            event(42262, 26725)
        );
        assert_eq!(
            parse_link(" https://data.acousticobservatory.org/library/55743/audio_events/30169 "),
            event(55743, 30169)
        );
        assert_eq!(
            parse_link("https://api.acousticobservatory.org/audio_recordings/55743/audio_events/30169.json"),
            event(55743, 30169)
        );
    }

    #[test]
    fn reads_download_links_with_a_start_and_end() {
        let clip = Some(A2oLink::Clip {
            recording_id: 256800,
            start_secs: Some(4035.0),
            end_secs: Some(4040.0),
        });
        assert_eq!(
            parse_link("https://api.search.acousticobservatory.org/api/v1/a2o/audio_recordings/download/flac/256800?start_offset=4035&end_offset=4040"),
            clip
        );
        assert_eq!(
            parse_link("https://api.acousticobservatory.org/audio_recordings/256800/media.flac?start_offset=4035&end_offset=4040"),
            clip
        );
        assert_eq!(
            parse_link("https://data.acousticobservatory.org/listen/256800?start=4035&end=4040"),
            clip
        );
        // No times: the recording from its start.
        assert_eq!(
            parse_link("https://data.acousticobservatory.org/listen/256800"),
            Some(A2oLink::Clip {
                recording_id: 256800,
                start_secs: None,
                end_secs: None,
            })
        );
    }

    #[test]
    fn other_links_are_not_a2o_links() {
        for text in [
            "",
            "21934",
            "https://data.acousticobservatory.org/",
            "https://data.acousticobservatory.org/projects/1",
            "https://data.acousticobservatory.org/library/abc/audio_events/1",
            "https://data.acousticobservatory.org/library/21934/audio_events/-1",
            // Hosts that only look like A2O's.
            "https://data.acousticobservatory.org.example.com/library/21934/audio_events/23087",
            "https://example.com/data.acousticobservatory.org/library/21934/audio_events/23087",
            "https://evil.example/?u=https://data.acousticobservatory.org/library/21934/audio_events/23087",
            "ftp://data.acousticobservatory.org/library/21934/audio_events/23087",
            "https://xeno-canto.org/928094",
        ] {
            assert_eq!(parse_link(text), None, "{text}");
        }
    }

    #[test]
    fn a_clip_stays_inside_the_recording_and_under_the_limit() {
        // An event's own times.
        assert_eq!(
            clip_range(Some(1506.0), Some(1516.0), 14389.685),
            Some((1506.0, 1516.0))
        );
        // No end: 30 s from the start.
        assert_eq!(
            clip_range(Some(100.0), None, 14389.685),
            Some((100.0, 130.0))
        );
        assert_eq!(clip_range(None, None, 14389.685), Some((0.0, 30.0)));
        // Longer than A2O serves: the first 300 s.
        assert_eq!(
            clip_range(Some(0.0), Some(900.0), 14389.685),
            Some((0.0, 300.0))
        );
        // Past the end of the recording: cut at the end.
        assert_eq!(
            clip_range(Some(14380.0), Some(14400.0), 14389.685),
            Some((14380.0, 14389.685))
        );
        assert_eq!(clip_range(Some(14400.0), Some(14410.0), 14389.685), None);
        // An end before the start is ignored.
        assert_eq!(
            clip_range(Some(50.0), Some(10.0), 14389.685),
            Some((50.0, 80.0))
        );
    }

    #[test]
    fn audio_before_and_after_stops_at_the_ends_of_the_recording() {
        assert_eq!(
            adjacent_range(1506.0, 1516.0, 14389.685, true),
            Some((1476.0, 1506.0))
        );
        assert_eq!(
            adjacent_range(1506.0, 1516.0, 14389.685, false),
            Some((1516.0, 1546.0))
        );
        assert_eq!(
            adjacent_range(10.0, 20.0, 14389.685, true),
            Some((0.0, 10.0))
        );
        assert_eq!(adjacent_range(0.0, 10.0, 14389.685, true), None);
        assert_eq!(
            adjacent_range(14370.0, 14380.0, 14389.685, false),
            Some((14380.0, 14389.685))
        );
        assert_eq!(adjacent_range(14380.0, 14389.685, 14389.685, false), None);
    }

    #[test]
    fn clip_names_are_not_read_as_channels_or_a_sequence() {
        use crate::components::file_sidebar::file_groups;
        let recording = "20200331T190000+1100_Tarcutta-Hills-Wet-A_55743.flac";
        let names = vec![
            clip_file_name(recording, 1476.0, 1506.0),
            clip_file_name(recording, 1506.0, 1516.0),
            clip_file_name(recording, 1516.0, 1546.5),
        ];
        assert_eq!(
            names[1],
            "1506s-1516s_20200331T190000+1100_Tarcutta-Hills-Wet-A_55743.flac"
        );
        assert_eq!(
            names[2],
            "1516s-1546.5s_20200331T190000+1100_Tarcutta-Hills-Wet-A_55743.flac"
        );
        // In name order the clips are in time order, also past 9999 s.
        let mut sorted = vec![
            clip_file_name(recording, 10000.0, 10030.0),
            clip_file_name(recording, 990.0, 1020.0),
        ];
        sorted.sort_by(|a, b| crate::timeline::natural_name_order(a, b));
        assert!(sorted[0].starts_with("990s"));
        for name in &names {
            assert_eq!(file_groups::channel_of(name), None, "{name}");
        }
        assert!(file_groups::compute_file_groups(&names)
            .iter()
            .all(Option::is_none));
    }

    #[test]
    fn reads_a_recording_reply() {
        let text = r#"{"meta":{"status":200},"data":{"id":55743,"recorded_date":"2020-03-31T08:00:00.000Z","site_id":58,"duration_seconds":14389.685,"sample_rate_hertz":22050,"canonical_file_name":"20200331T190000+1100_Tarcutta-Hills-Wet-A_55743.flac","recorded_date_timezone":"Australia/Sydney"}}"#;
        let info = parse_recording(text, |_| Some(1_585_641_600_000.0)).unwrap();
        assert_eq!(info.id, 55743);
        assert_eq!(info.duration_secs, 14389.685);
        assert_eq!(info.site_id, Some(58));
        assert_eq!(info.recorded_ms, 1_585_641_600_000.0);
        assert_eq!(info.timezone.as_deref(), Some("Australia/Sydney"));
        assert!(parse_recording(r#"{"data":null}"#, |_| Some(0.0)).is_err());
        assert!(parse_recording("<html>", |_| Some(0.0)).is_err());
    }

    #[test]
    fn reads_an_event_reply() {
        let text = r#"{"meta":{"status":200},"data":{"id":30169,"audio_recording_id":55743,"start_time_seconds":1506.0,"end_time_seconds":1516.0,"taggings":[{"id":31171,"tag_id":105},{"id":31170,"tag_id":114}]}}"#;
        assert_eq!(
            parse_event(text).unwrap(),
            EventInfo {
                start_secs: 1506.0,
                end_secs: 1516.0,
                tag_ids: vec![105, 114],
            }
        );
        assert!(parse_event(r#"{"data":{"id":1}}"#).is_err());
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

    #[test]
    fn times_in_a_recording_read_as_clock_times() {
        assert_eq!(clock(1506.0), "25:06");
        assert_eq!(clock(14389.685), "3:59:50");
        assert_eq!(clock(5.0), "0:05");
        assert_eq!(secs_text(1506.0), "1506");
        assert_eq!(secs_text(14389.685), "14389.685");
    }
}
