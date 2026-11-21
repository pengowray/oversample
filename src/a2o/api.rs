//! Requests to A2O's two servers, and reading their replies.
//!
//! - The search API serves clips, recording details and site details to any
//!   website.
//! - The main API serves event details and tags. It answers pages on
//!   localhost and the desktop and Android apps, and refuses other websites,
//!   including app.oversample.com. Those ask the Oversample server instead,
//!   which passes the question on ([`main_api_text`]).

use super::link::secs_text;
use crate::components::file_sidebar::fetch_text;
use crate::state::AppState;

const SEARCH_API: &str = "https://api.search.acousticobservatory.org/api/v1/a2o";
const MAIN_API: &str = "https://api.acousticobservatory.org";

pub(super) fn clip_url(recording_id: u64, start_secs: f64, end_secs: f64) -> String {
    format!(
        "{SEARCH_API}/audio_recordings/download/flac/{recording_id}?start_offset={}&end_offset={}",
        secs_text(start_secs),
        secs_text(end_secs)
    )
}

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
pub(super) fn data_of(text: &str) -> Result<serde_json::Value, String> {
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
pub(super) async fn fetch_site_name(site_id: u64) -> Option<String> {
    let text = fetch_text(&format!("{SEARCH_API}/sites/{site_id}"))
        .await
        .ok()?;
    data_of(&text).ok()?["name"].as_str().map(str::to_string)
}

/// Whether this page is one A2O's main API answers: the desktop or
/// Android app, or a page on this computer. Any other website gets no reply
/// it can read.
fn main_api_answers_this_page(state: &AppState) -> bool {
    if state.is_tauri {
        return true;
    }
    let host = web_sys::window()
        .and_then(|w| w.location().hostname().ok())
        .unwrap_or_default();
    host == "localhost" || host == "127.0.0.1" || host == "[::1]" || host.ends_with(".localhost")
}

/// A reply from A2O's main API for `path` ("/tags/105"). A page the main API
/// answers asks it directly. Any other website asks the Oversample server,
/// which passes the question on (`/a2o/*` in the oversample-api repo). If
/// the direct request fails without a status (no network, or a browser that
/// blocks it after all), the Oversample server is asked as well.
///
/// The error is the HTTP status as "HTTP 404", or `None` when neither
/// server could be reached.
async fn main_api_text(state: &AppState, path: &str) -> Result<String, Option<u16>> {
    let status_of = |e: &str| e.strip_prefix("HTTP ").and_then(|s| s.trim().parse().ok());
    if main_api_answers_this_page(state) {
        match fetch_text(&format!("{MAIN_API}{path}")).await {
            Ok(text) => return Ok(text),
            Err(e) => {
                if let Some(status) = status_of(&e) {
                    return Err(Some(status));
                }
            }
        }
    }
    let relay = format!("{}/a2o{path}", crate::xc_client::proxy_base());
    fetch_text(&relay).await.map_err(|e| status_of(&e))
}

/// An event's times and tags.
pub async fn fetch_event(
    state: &AppState,
    recording_id: u64,
    event_id: u64,
) -> Result<EventInfo, String> {
    let path = format!("/audio_recordings/{recording_id}/audio_events/{event_id}");
    match main_api_text(state, &path).await {
        Ok(text) => parse_event(&text),
        Err(Some(404)) => Err(format!(
            "A2O has no event {event_id} in recording {recording_id}"
        )),
        // A2O answers this way for a private event, and for an event that
        // is not in the recording the link names.
        Err(Some(401 | 403)) => Err(format!(
            "A2O does not share event {event_id} in recording {recording_id}"
        )),
        Err(Some(status)) => Err(format!("A2O event {event_id}: HTTP {status}")),
        Err(None) => Err(format!(
            "Could not reach A2O for event {event_id}. Check the internet connection."
        )),
    }
}

/// A tag's text ("birds"), or `None` when it cannot be read.
pub(super) async fn fetch_tag(state: &AppState, tag_id: u64) -> Option<String> {
    let text = main_api_text(state, &format!("/tags/{tag_id}"))
        .await
        .ok()?;
    data_of(&text).ok()?["text"].as_str().map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
