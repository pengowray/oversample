//! A2O links and clip times: what a pasted link names, which stretch of a
//! recording to fetch for it, and how clips are named. Nothing here asks a
//! server.

use super::{ADD_SECS, MAX_CLIP_SECS};

/// An amount of audio as the length menu writes it: "10 s", "1 min".
pub fn length_text(secs: f64) -> String {
    let whole = secs.round() as u32;
    if whole >= 60 && whole % 60 == 0 {
        format!("{}\u{a0}min", whole / 60)
    } else {
        format!("{whole}\u{a0}s")
    }
}

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
/// that is already open, `(start, end)` in seconds: up to `add_secs` of it
/// (and never more than A2O serves in one request). `None` at the start or
/// the end of the recording.
pub fn adjacent_range(
    open_start: f64,
    open_end: f64,
    recording_secs: f64,
    before: bool,
    add_secs: f64,
) -> Option<(f64, f64)> {
    let add_secs = add_secs.clamp(1.0, MAX_CLIP_SECS);
    let (start, end) = if before {
        ((open_start - add_secs).max(0.0), open_start)
    } else {
        (open_end, (open_end + add_secs).min(recording_secs))
    };
    // Under a tenth of a second is the rounding left at an end, not audio.
    (end - start > 0.1).then_some((start, end))
}

/// A number of seconds as a URL or file name writes it: "1506" or "1506.5".
pub(super) fn secs_text(secs: f64) -> String {
    let text = format!("{secs:.3}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
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
pub(super) fn clock(secs: f64) -> String {
    let total = secs.round() as u64;
    let (h, m, s) = (total / 3600, (total / 60) % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
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
            adjacent_range(1506.0, 1516.0, 14389.685, true, 30.0),
            Some((1476.0, 1506.0))
        );
        assert_eq!(
            adjacent_range(1506.0, 1516.0, 14389.685, false, 30.0),
            Some((1516.0, 1546.0))
        );
        assert_eq!(
            adjacent_range(10.0, 20.0, 14389.685, true, 30.0),
            Some((0.0, 10.0))
        );
        assert_eq!(adjacent_range(0.0, 10.0, 14389.685, true, 30.0), None);
        assert_eq!(
            adjacent_range(14370.0, 14380.0, 14389.685, false, 30.0),
            Some((14380.0, 14389.685))
        );
        assert_eq!(
            adjacent_range(14380.0, 14389.685, 14389.685, false, 30.0),
            None
        );
        // A longer step, and a step longer than A2O serves in one request.
        assert_eq!(
            adjacent_range(1506.0, 1516.0, 14389.685, false, 60.0),
            Some((1516.0, 1576.0))
        );
        assert_eq!(
            adjacent_range(1506.0, 1516.0, 14389.685, true, 300.0),
            Some((1206.0, 1506.0))
        );
        assert_eq!(
            adjacent_range(1506.0, 1516.0, 14389.685, false, 900.0),
            Some((1516.0, 1816.0))
        );
    }

    #[test]
    fn lengths_read_as_seconds_or_minutes() {
        assert_eq!(length_text(10.0), "10\u{a0}s");
        assert_eq!(length_text(30.0), "30\u{a0}s");
        assert_eq!(length_text(60.0), "1\u{a0}min");
        assert_eq!(length_text(300.0), "5\u{a0}min");
        assert_eq!(length_text(90.0), "90\u{a0}s");
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
    fn times_in_a_recording_read_as_clock_times() {
        assert_eq!(clock(1506.0), "25:06");
        assert_eq!(clock(14389.685), "3:59:50");
        assert_eq!(clock(5.0), "0:05");
        assert_eq!(secs_text(1506.0), "1506");
        assert_eq!(secs_text(14389.685), "14389.685");
    }
}
