use super::rate_notes_view::{note_selector, GuanoControls, RateNotes, TimeExpansionRow};
use crate::rate_notes::{file_rate_notes, NoteKind};
use crate::state::store_fields::*;
use crate::state::{AppState, MetadataView, RightSidebarTab};
use crate::types::AudioFormat;
use leptos::prelude::*;
use leptos::task::spawn_local;
use oversample_core::audio::source::ChannelView;
use oversample_core::dsp::chime_scan::{
    self, ChimeScanResult, ChimeScanner, FoundChime, RemoteCommand,
};
use oversample_core::position::parse_lat_lon;

/// Returns (section, display_key) for a GUANO field.
/// Known fields return "GUANO" as section; unknown pipe-separated keys
/// return the prefix (e.g. "BatGizmo App") as section and the last segment as display key.
fn categorize_guano_key(key: &str) -> (String, String) {
    let known = match key {
        // Pipe-separated variants
        "Loc|Lat" => Some("Latitude"),
        "Loc|Lon" => Some("Longitude"),
        "Loc|Elev" => Some("Elevation"),
        "Filter|HP" => Some("High-pass Filter"),
        "Filter|LP" => Some("Low-pass Filter"),
        "Species|Auto" => Some("Species (Auto)"),
        "Species|Manual" => Some("Species (Manual)"),
        // Standard GUANO space-separated field names
        "Loc Position" => Some("GPS Position"),
        "Loc Elevation" => Some("Elevation"),
        "Filter HP" => Some("High-pass Filter (kHz)"),
        "Filter LP" => Some("Low-pass Filter (kHz)"),
        "Species Auto ID" => Some("Species (Auto)"),
        "Species Manual ID" => Some("Species (Manual)"),
        "Temperature Int" => Some("Internal Temp"),
        "Temperature Ext" => Some("External Temp"),
        "Model" => Some("Model"),
        "Serial" => Some("Serial"),
        "Microphone" => Some("Microphone"),
        // Common fields
        "TE" => Some("Time Expansion"),
        "Samplerate" => Some("Sample Rate"),
        "Length" => Some("Length"),
        _ => None,
    };
    if let Some(display) = known {
        return ("GUANO".into(), display.into());
    }
    if let Some(pos) = key.rfind('|') {
        let prefix = &key[..pos];
        let short = &key[pos + 1..];
        (prefix.replace('|', " "), short.into())
    } else {
        ("GUANO".into(), key.into())
    }
}

/// Classify a metadata field by its display label. Drives formatted-mode
/// transforms (humanized dates, °F conversion, coordinate parsing, JSON
/// expansion) and is robust to both GUANO and XC field name variants.
#[derive(Clone, Copy, PartialEq)]
enum FieldKind {
    Date,
    Temperature,
    GpsPosition,
    /// A position the recorder's owner typed in (a D500X has no GPS).
    TypedPosition,
    License,
    /// Markdown text (Xeno-canto remarks). `classify` never returns this: the
    /// caller picks it for the one field it knows is Markdown.
    Markdown,
    None,
}

fn classify(label: &str) -> FieldKind {
    let l = label.to_ascii_lowercase();
    if l == "date" || l == "recorded" || l == "timestamp" || l == "datetime" {
        FieldKind::Date
    } else if l.contains("temp") {
        FieldKind::Temperature
    } else if l == "gps position"
        || l == "coordinates"
        || l == "gps"
        || l == "loc position"
        || l == "position"
    {
        FieldKind::GpsPosition
    } else if l == "license" || l == "licence" || l == "lic" {
        FieldKind::License
    } else {
        FieldKind::None
    }
}

/// Parse a temperature value (string) into degrees Celsius.
/// Accepts e.g. "25", "25.5", "25C", "25 °C", "25°C". If the value
/// already contains "F", we assume Fahrenheit was intended and skip
/// the conversion.
fn parse_temp_c(value: &str) -> Option<f64> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    let lower = v.to_ascii_lowercase();
    if lower.contains('f') && !lower.contains('c') {
        return None;
    }
    // Strip non-numeric trailing characters
    let num: String = v
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-' || *c == '.' || *c == '+')
        .collect();
    num.parse().ok()
}

/// Parse an ISO-ish date/time string into a JS epoch ms. Falls back to
/// js_sys::Date::parse which handles ISO 8601, RFC 2822, and a number
/// of looser shapes (incl. "2024-03-15").
fn parse_date_ms(value: &str) -> Option<f64> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    let ms = js_sys::Date::parse(v);
    if ms.is_nan() {
        None
    } else {
        Some(ms)
    }
}

/// Extract the UTC offset (in minutes) explicitly written in the
/// original timestamp string. Recognizes trailing `Z`, `+HH:MM`,
/// `-HH:MM`, `+HHMM`, `-HHMM`. Returns None if no offset is present.
fn extract_tz_offset_minutes(value: &str) -> Option<i32> {
    let v = value.trim();
    if v.is_empty() {
        return None;
    }
    if v.ends_with('Z') || v.ends_with('z') {
        return Some(0);
    }
    // A timezone offset can only appear in the *time* portion. If there is no
    // time portion at all (a bare date like "2025-05-30"), there is no offset —
    // and scanning the whole string would mis-read the date's own "-" separators
    // (this is exactly how "2025-05-30" turned into the nonsensical "UTC-30:00":
    // the "30" after the last "-" was parsed as 30 hours). So bail out unless a
    // `T` or space separator marks where the time begins.
    let time_start = match v.find('T').or_else(|| v.find(' ')) {
        Some(i) => i + 1,
        None => return None,
    };
    let tail = &v[time_start..];
    let sign_idx = tail.rfind(|c: char| c == '+' || c == '-')?;
    let sign_char = tail.as_bytes()[sign_idx] as char;
    let off = &tail[sign_idx + 1..];
    let (h, m) = if let Some((h, m)) = off.split_once(':') {
        (h.parse::<i32>().ok()?, m.parse::<i32>().ok()?)
    } else if off.len() == 4 {
        (off[..2].parse::<i32>().ok()?, off[2..].parse::<i32>().ok()?)
    } else if off.len() == 2 {
        (off.parse::<i32>().ok()?, 0)
    } else {
        return None;
    };
    // Reject implausible offsets. Real UTC offsets span −12:00..+14:00 with
    // minutes in 0..59 (e.g. +05:45 Nepal, +12:45 Chatham), so anything outside
    // that is a parse artefact rather than a real zone.
    if !(0..=14).contains(&h) || !(0..60).contains(&m) {
        return None;
    }
    let total = h * 60 + m;
    Some(if sign_char == '-' { -total } else { total })
}

fn format_tz_offset(minutes: i32) -> String {
    if minutes == 0 {
        return "UTC".into();
    }
    let sign = if minutes < 0 { '-' } else { '+' };
    let m = minutes.unsigned_abs();
    format!("UTC{sign}{:02}:{:02}", m / 60, m % 60)
}

/// Render a JS Date in a long, locale-sensitive format.
/// `tz_minutes`: if Some, render the date in that fixed UTC offset (so
/// the displayed wall-clock time matches the original timestamp string);
/// if None, render in the user's local time zone.
fn format_date_long(ms: f64, had_time: bool, tz_minutes: Option<i32>) -> String {
    let opts = js_sys::Object::new();
    if had_time {
        let _ = js_sys::Reflect::set(&opts, &"dateStyle".into(), &"long".into());
        let _ = js_sys::Reflect::set(&opts, &"timeStyle".into(), &"medium".into());
    } else {
        let _ = js_sys::Reflect::set(&opts, &"dateStyle".into(), &"long".into());
    }
    let shifted_ms = match tz_minutes {
        Some(off) => {
            // Adjust the epoch so that the UTC reading at this offset
            // matches the wall-clock time in the original zone, then
            // force the formatter into UTC.
            let _ = js_sys::Reflect::set(&opts, &"timeZone".into(), &"UTC".into());
            ms + (off as f64) * 60_000.0
        }
        None => ms,
    };
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(shifted_ms));
    d.to_locale_string("default", &opts)
        .as_string()
        .unwrap_or_default()
}

/// "4 days ago" / "in 2 hours" style relative time.
fn humanize_relative(ms: f64) -> String {
    let now = js_sys::Date::now();
    let delta_secs = (now - ms) / 1000.0;
    let abs = delta_secs.abs();
    let future = delta_secs < 0.0;
    let (n, unit) = if abs < 60.0 {
        (abs as i64, "second")
    } else if abs < 3600.0 {
        ((abs / 60.0) as i64, "minute")
    } else if abs < 86_400.0 {
        ((abs / 3600.0) as i64, "hour")
    } else if abs < 30.0 * 86_400.0 {
        ((abs / 86_400.0) as i64, "day")
    } else if abs < 365.0 * 86_400.0 {
        ((abs / (30.0 * 86_400.0)) as i64, "month")
    } else {
        ((abs / (365.0 * 86_400.0)) as i64, "year")
    };
    let plural = if n == 1 { "" } else { "s" };
    if future {
        format!("in {n} {unit}{plural}")
    } else if n == 0 {
        "just now".into()
    } else {
        format!("{n} {unit}{plural} ago")
    }
}

/// Try to extract a balanced JSON array/object from a string that may
/// contain a prefix or trailing junk. Returns the parsed Value if the
/// extracted slice round-trips through serde_json.
fn extract_json(value: &str) -> Option<serde_json::Value> {
    let s = value.trim();
    let start = s.find(|c: char| c == '[' || c == '{')?;
    // Find the matching closing bracket using simple depth tracking
    // (good enough for the well-behaved JSON we see in GUANO).
    let bytes = s.as_bytes();
    let open = bytes[start] as char;
    let close = if open == '[' { ']' } else { '}' };
    let mut depth = 0i32;
    let mut end = None;
    let mut in_str = false;
    let mut esc = false;
    for (i, b) in bytes.iter().enumerate().skip(start) {
        let c = *b as char;
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        match c {
            '"' => in_str = true,
            x if x == open => depth += 1,
            x if x == close => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    serde_json::from_str(&s[start..end]).ok()
}

/// Render an extracted JSON value as a small nested "k: v" block.
fn json_block(json: &serde_json::Value) -> impl IntoView {
    fn line(key: String, val: String) -> impl IntoView {
        view! {
            <div class="metadata-json-line">
                <span class="metadata-json-key">{key}</span>
                <span class="metadata-json-sep">": "</span>
                <span class="metadata-json-val">{val}</span>
            </div>
        }
    }
    fn flatten_value(v: &serde_json::Value) -> String {
        match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => b.to_string(),
            serde_json::Value::Null => "null".into(),
            other => other.to_string(),
        }
    }
    let entries: Vec<_> = match json {
        serde_json::Value::Array(arr) => arr
            .iter()
            .flat_map(|v| match v {
                serde_json::Value::Object(map) => map
                    .iter()
                    .map(|(k, v)| line(k.clone(), flatten_value(v)).into_any())
                    .collect::<Vec<_>>(),
                other => vec![line(String::new(), flatten_value(other)).into_any()],
            })
            .collect(),
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| line(k.clone(), flatten_value(v)).into_any())
            .collect(),
        other => vec![line(String::new(), flatten_value(other)).into_any()],
    };
    view! { <div class="metadata-json-block">{entries}</div> }
}

/// Parse a temperature value and return (°C string, °F string).
/// Both formatted to 0 dp when near-integer, otherwise 1 dp.
fn format_temp_c_f(value: &str) -> Option<(String, String)> {
    let c = parse_temp_c(value)?;
    let f = c * 9.0 / 5.0 + 32.0;
    let fmt = |n: f64| {
        if (n - n.round()).abs() < 0.05 {
            format!("{n:.0}")
        } else {
            format!("{n:.1}")
        }
    };
    Some((
        format!("{}\u{00B0}C", fmt(c)),
        format!("{}\u{00B0}F", fmt(f)),
    ))
}

/// Inline (single-line) metadata row — used by the File section.
fn inline_row(label: String, value: String, label_title: Option<String>) -> impl IntoView {
    let value_for_copy = value.clone();
    let value_for_title = value.clone();
    let on_copy = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        super::copy_to_clipboard(&value_for_copy);
    };
    view! {
        <div class="setting-row metadata-row metadata-row-inline">
            <span class="setting-label" title=label_title.unwrap_or_default()>{label}</span>
            <span class="setting-value metadata-value" title=value_for_title>{value}</span>
            <button class="copy-btn" on:click=on_copy title="Copy">{"\u{2398}"}</button>
        </div>
    }
}

/// Spacious row (key on its own line, value beneath). Used for all
/// sections other than the File summary. Honors the Formatted/Original
/// view mode for value rendering.
fn spacious_row(
    label: String,
    value: String,
    label_title: Option<String>,
    view_mode: MetadataView,
) -> impl IntoView {
    let kind = classify(&label);
    let open = open_link(kind, &value, None);
    spacious_row_as(label, value, label_title, view_mode, kind, open)
}

/// A spacious row whose value is formatted as `kind`, whatever its label, and
/// whose open-in-new-tab button opens `open`.
fn spacious_row_as(
    label: String,
    value: String,
    label_title: Option<String>,
    view_mode: MetadataView,
    kind: FieldKind,
    open: Option<String>,
) -> impl IntoView {
    let value_for_copy = value.clone();
    let on_copy = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        super::copy_to_clipboard(&value_for_copy);
    };

    let body = render_value_body(&label, &value, kind, view_mode);
    let open = open.map(|href| {
        view! {
            <a class="copy-btn copy-btn-spacious open-link-btn" href=href target="_blank"
                rel="noopener noreferrer" title="Open in new tab">{"\u{2197}"}</a>
        }
    });

    view! {
        <div class="metadata-row metadata-row-spacious">
            <div class="metadata-key-line">
                <span class="metadata-key" title=label_title.unwrap_or_default()>{label}</span>
                {open}
                <button class="copy-btn copy-btn-spacious" on:click=on_copy title="Copy">{"\u{2398}"}</button>
            </div>
            {body}
        </div>
    }
}

/// The address a row's open-in-new-tab button opens: the value, when it is
/// one web address. A License row opens the licence's page from
/// `license_link`, so Xeno-canto's "//creativecommons.org/..." values, short
/// CC names and Commons' "License URL" field get a button too.
fn open_link(kind: FieldKind, value: &str, license_url: Option<&str>) -> Option<String> {
    match kind {
        FieldKind::License => crate::license::license_link(value, license_url),
        _ => web_link(value),
    }
}

/// The value as a link to open, when the whole value is one http or https URL.
fn web_link(value: &str) -> Option<String> {
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    ((lower.starts_with("https://") || lower.starts_with("http://"))
        && !v.contains(char::is_whitespace))
    .then(|| v.to_string())
}

/// Render the value portion of a spacious row according to view mode
/// and detected field kind.
fn render_value_body(
    label: &str,
    value: &str,
    kind: FieldKind,
    view_mode: MetadataView,
) -> leptos::tachys::view::any_view::AnyView {
    // Values with line breaks (Xeno-canto remarks) keep them.
    let raw_class = if value.contains('\n') {
        "metadata-value-block metadata-value-lines"
    } else {
        "metadata-value-block"
    };
    let raw = || {
        view! {
            <div class=raw_class title=value.to_string()>{value.to_string()}</div>
        }
        .into_any()
    };

    if view_mode == MetadataView::Original {
        return raw();
    }

    match kind {
        FieldKind::Date => date_block(value).into_any(),
        FieldKind::Temperature => {
            if let Some((c, f)) = format_temp_c_f(value) {
                view! {
                    <div class="metadata-value-block">
                        {c}
                        " "
                        <span class="metadata-temp-f">"("{f}")"</span>
                    </div>
                }
                .into_any()
            } else {
                raw()
            }
        }
        FieldKind::GpsPosition | FieldKind::TypedPosition => {
            let note = (kind == FieldKind::TypedPosition).then_some("Typed into the recorder");
            if let Some((lat, lon)) = parse_lat_lon(value) {
                gps_block(lat, lon, value.to_string(), note).into_any()
            } else {
                raw()
            }
        }
        FieldKind::License => {
            // The short label, with the value as sent below it when it differs
            // by more than letter case ("CC0 1.0" over "CC0"; "CC BY-SA 4.0"
            // and "CC BY-SA 3.0 de" once).
            let short = crate::license::parse_cc_license(value)
                .filter(|s| !s.eq_ignore_ascii_case(value.trim()));
            if let Some(short) = short {
                view! {
                    <div class="metadata-value-block">{short}</div>
                    <div class="metadata-value-subtle" title=value.to_string()>{value.to_string()}</div>
                }.into_any()
            } else {
                raw()
            }
        }
        // Markdown is turned into elements, never inserted as HTML, so raw
        // HTML in the text shows as text and only http(s) links are links.
        FieldKind::Markdown => {
            let blocks = crate::markdown::parse(value);
            // Text with nothing to show once parsed (only a link reference
            // definition, say) keeps its raw form rather than a blank row.
            if blocks.is_empty() {
                return raw();
            }
            view! {
                <div class="metadata-value-block metadata-markdown">
                    {crate::markdown::markdown_view(&blocks)}
                </div>
            }
            .into_any()
        }
        FieldKind::None => {
            // Try JSON expansion (e.g. Wildlife Acoustics "Audio settings")
            if value.trim_start().starts_with('[') || value.trim_start().starts_with('{') {
                if let Some(j) = extract_json(value) {
                    return view! {
                        <div class="metadata-value-block metadata-value-json">
                            {json_block(&j)}
                        </div>
                    }
                    .into_any();
                }
            }
            let _ = label;
            raw()
        }
    }
}

/// Render a parsed date with the original UTC offset, a localized
/// "Local: ..." line if the local interpretation differs, and a
/// humanized relative-time hint.
fn date_block(value: &str) -> impl IntoView {
    let Some(ms) = parse_date_ms(value) else {
        return view! {
            <div class="metadata-value-block" title=value.to_string()>{value.to_string()}</div>
        }
        .into_any();
    };
    let had_time = value.contains('T') || value.contains(':');
    let orig_tz = extract_tz_offset_minutes(value);
    let rel = humanize_relative(ms);

    let primary_str = match orig_tz {
        Some(off) => format!(
            "{} {}",
            format_date_long(ms, had_time, Some(off)),
            format_tz_offset(off)
        ),
        // A bare date with no explicit zone (e.g. "2025-05-30") is parsed as UTC
        // midnight; render it in UTC too so it doesn't slip to the previous day
        // for viewers west of Greenwich.
        None if !had_time => format_date_long(ms, had_time, Some(0)),
        None => format_date_long(ms, had_time, None),
    };

    // A distinct "Local:" interpretation exists only when the original timestamp
    // pinned a specific tz AND that tz differs from the viewer's local tz.
    let local_line = orig_tz.and_then(|off| {
        let local_offset =
            -(js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(ms)).get_timezone_offset() as i32);
        if local_offset == off {
            return None;
        }
        Some(format!(
            "Local: {} {}",
            format_date_long(ms, had_time, None),
            format_tz_offset(local_offset),
        ))
    });

    let Some(local_str) = local_line else {
        return view! {
            <div class="metadata-value-block">{primary_str}</div>
            <div class="metadata-value-relative">{rel}</div>
        }
        .into_any();
    };

    // The local line is shown by default only for recent recordings (within the
    // last week); older ones start hidden. Either way the primary date is a
    // toggle — click it to show/hide the local interpretation.
    let within_week = {
        let age = js_sys::Date::now() - ms; // ms; positive = in the past
        age.abs() <= 7.0 * 86_400_000.0
    };
    let show_local = RwSignal::new(within_week);

    view! {
        <div
            class="metadata-value-block metadata-date-toggle"
            title="Click to toggle local time"
            on:click=move |_| show_local.update(|v| *v = !*v)
        >{primary_str}</div>
        {move || show_local.get().then({
            let local_str = local_str.clone();
            move || view! { <div class="metadata-value-local">{local_str}</div> }
        })}
        <div class="metadata-value-relative">{rel}</div>
    }
    .into_any()
}

/// Parse an 8- or 14-digit `YYYYMMDD[HHMMSS]` run into an ISO string, with
/// sanity bounds on each field. Returns None if it's not a plausible date.
fn parse_ymd_run(run: &str) -> Option<String> {
    if run.len() != 8 && run.len() != 14 {
        return None;
    }
    let y: i32 = run[0..4].parse().ok()?;
    let m: u32 = run[4..6].parse().ok()?;
    let d: u32 = run[6..8].parse().ok()?;
    if !(1990..=2100).contains(&y) || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    if run.len() == 8 {
        return Some(format!("{y:04}-{m:02}-{d:02}"));
    }
    let hh: u32 = run[8..10].parse().ok()?;
    let mm: u32 = run[10..12].parse().ok()?;
    let ss: u32 = run[12..14].parse().ok()?;
    if hh > 23 || mm > 59 || ss > 59 {
        return None;
    }
    Some(format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}"))
}

/// Extract an ISO-ish datestamp embedded in a filename. Recognizes the common
/// recorder patterns `YYYYMMDD`, `YYYYMMDDHHMMSS`, and `YYYYMMDD[_-T ]HHMMSS`
/// (e.g. "PREFIX_20210630_223015.wav", "20240115.WAV"). Field bounds reject
/// most non-date digit runs. Returns the first plausible match.
fn date_from_filename(name: &str) -> Option<String> {
    let chars: Vec<char> = name.chars().collect();
    let n = chars.len();
    let is_d = |c: char| c.is_ascii_digit();
    let mut i = 0;
    while i < n {
        if !is_d(chars[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && is_d(chars[i]) {
            i += 1;
        }
        let run_len = i - start;
        if run_len == 8 || run_len == 14 {
            let run: String = chars[start..i].iter().collect();
            if let Some(iso) = parse_ymd_run(&run) {
                // A bare YYYYMMDD may be followed by a separator + HHMMSS.
                if run_len == 8 && i < n && matches!(chars[i], '_' | '-' | 'T' | ' ') {
                    let t_start = i + 1;
                    let mut t_end = t_start;
                    while t_end < n && is_d(chars[t_end]) {
                        t_end += 1;
                    }
                    if t_end - t_start == 6 {
                        let t: String = chars[t_start..t_end].iter().collect();
                        let hh: u32 = t[0..2].parse().ok()?;
                        let mm: u32 = t[2..4].parse().ok()?;
                        let ss: u32 = t[4..6].parse().ok()?;
                        if hh <= 23 && mm <= 59 && ss <= 59 {
                            return Some(format!("{iso}T{hh:02}:{mm:02}:{ss:02}"));
                        }
                    }
                }
                return Some(iso);
            }
        }
    }
    None
}

/// A spacious row whose value is always rendered as a date (so the source label
/// can be descriptive, e.g. "GUANO timestamp", without relying on `classify`).
fn date_row(label: String, value: String, view_mode: MetadataView) -> impl IntoView {
    let value_for_copy = value.clone();
    let on_copy = move |ev: web_sys::MouseEvent| {
        ev.stop_propagation();
        super::copy_to_clipboard(&value_for_copy);
    };
    let body = if view_mode == MetadataView::Original {
        view! { <div class="metadata-value-block" title=value.clone()>{value.clone()}</div> }
            .into_any()
    } else {
        date_block(&value).into_any()
    };
    view! {
        <div class="metadata-row metadata-row-spacious">
            <div class="metadata-key-line">
                <span class="metadata-key">{label}</span>
                <button class="copy-btn copy-btn-spacious" on:click=on_copy title="Copy">{"\u{2398}"}</button>
            </div>
            {body}
        </div>
    }
}

/// One note about a WAV file's structure: amber when it changes which audio is
/// read, grey otherwise. A note with more detail opens on a click or tap to
/// show it (a tooltip would not show on a phone).
pub(super) fn wav_note_view(n: &crate::types::WavNote) -> impl IntoView {
    let class = if n.warning {
        "analysis-warning"
    } else {
        "metadata-note"
    };
    match n.detail.clone() {
        None => view! { <div class=class>{n.text.clone()}</div> }.into_any(),
        Some(detail) => view! {
            <details class=format!("{class} note-with-detail")>
                <summary>{n.text.clone()}</summary>
                <div class="note-detail">{detail}</div>
            </details>
        }
        .into_any(),
    }
}

/// A recorder's own metadata block (a D500X header, a Wildlife Acoustics `wamd`
/// chunk) as a section titled "<recorder> metadata". Kaleidoscope copies most `wamd`
/// fields into the file's GUANO block, so rows with the same value as `guano` are
/// folded into one closed "Same values as Guano metadata (N fields)" row, which keeps
/// each value on screen once and still lets the user check the chunk's full contents.
fn recorder_section(
    block: Option<crate::types::RecorderBlock>,
    guano: &[(String, String)],
    view_mode: MetadataView,
) -> impl IntoView {
    let Some(block) = block else {
        return view! { <span></span> }.into_any();
    };
    let (same, differ): (Vec<_>, Vec<_>) = block
        .fields
        .into_iter()
        .partition(|(k, v)| crate::audio::wamd::same_in_guano(k, v, guano));
    // A D500X writes its position as two rows of degrees, minutes and seconds.
    // The map goes in a Position row straight after them.
    let position = oversample_core::audio::wav::d500x_position(&differ);
    let position_after = differ
        .iter()
        .rposition(|(k, _)| k.eq_ignore_ascii_case("LAT") || k.eq_ignore_ascii_case("LON"));
    let mut items: Vec<_> = Vec::with_capacity(differ.len() + 1);
    for (i, (k, v)) in differ.into_iter().enumerate() {
        items.push(spacious_row(k, v, None, view_mode).into_any());
        if let (Some((lat, lon)), true) = (position, position_after == Some(i)) {
            // The owner types the position into a D500X (it has no GPS), so
            // the row says so under the coordinates.
            items.push(
                spacious_row_as(
                    "Position".into(),
                    format!("{lat:.5}, {lon:.5}"),
                    Some("From the LAT and LON rows".into()),
                    view_mode,
                    FieldKind::TypedPosition,
                    None,
                )
                .into_any(),
            );
        }
    }
    let folded = (!same.is_empty()).then(|| {
        let n = same.len();
        let label = format!(
            "Same values as Guano metadata ({n} field{})",
            if n == 1 { "" } else { "s" }
        );
        let rows: Vec<_> = same
            .into_iter()
            .map(|(k, v)| spacious_row(k, v, None, view_mode).into_any())
            .collect();
        view! {
            <details class="metadata-same">
                <summary>{label}</summary>
                {rows}
            </details>
        }
    });
    view! {
        <div class="setting-group">
            <div class="setting-group-title setting-group-title-major">
                {format!("{} metadata", block.recorder)}
            </div>
            {items}
            {folded}
        </div>
    }
    .into_any()
}

/// The Sample rate row's value: "384 kHz", "44.1 kHz", "38.4 kHz". The row never wraps,
/// so the space is an ordinary one, and the row's Copy button copies it as shown.
/// (`file_badges`, `suggestions` and `xc_browser` each have their own version; they
/// differ in spacing and precision.)
fn sample_rate_row_value(hz: u32) -> String {
    format!("{} kHz", crate::format_units::sample_rate_khz_value(hz))
}

/// Open the right sidebar on the Info tab, as the "Heterodyne recording" and
/// "Corrected ×10" badges do. On a phone the two sidebars are overlays, so
/// the Files sidebar is closed, as the toolbar's Show info panel button does.
/// If the panel is scrolled down, the note of kind `note` under the Sample
/// rate row is scrolled into view once drawn.
pub(crate) fn open_info_panel(state: AppState, note: NoteKind) {
    state.panels.right_tab().set(RightSidebarTab::Metadata);
    state.panels.right_collapsed().set(false);
    if state.status.is_mobile().get_untracked() {
        state.panels.left_collapsed().set(true);
    }
    request_animation_frame(move || {
        if let Some(el) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.query_selector(&note_selector(note)).ok().flatten())
        {
            let opts = web_sys::ScrollIntoViewOptions::new();
            opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
            // The note annotates the Sample rate row above it, so bring that
            // row into view first. Then the note: "nearest" leaves it alone
            // when the first scroll already showed it.
            if let Some(row) = el.previous_element_sibling() {
                row.scroll_into_view_with_scroll_into_view_options(&opts);
            }
            el.scroll_into_view_with_scroll_into_view_options(&opts);
        }
    });
}

/// Format row: the container name, plus RF64 when the file uses 64-bit sizes.
fn format_text(meta: &crate::types::FileMetadata) -> String {
    match &meta.wav {
        Some(w) if w.rf64 => format!("{} (RF64)", meta.format),
        _ => meta.format.to_string(),
    }
}

/// Bit depth row: "16-bit", "32-bit float", "Lossy", or "12-bit,
/// stored as 16-bit" when the file declares fewer significant bits than the
/// sample width.
fn bit_depth_text(meta: &crate::types::FileMetadata) -> String {
    let float = if meta.is_float { " float" } else { "" };
    match (
        meta.pcm_bits(),
        meta.wav.as_ref().and_then(|w| w.valid_bits),
    ) {
        (Some(width), Some(valid)) => format!("{valid}-bit{float}, stored as {width}-bit"),
        _ => meta.bit_depth_label(),
    }
}

/// D500X "YYMMDD hh:mm:ss" as ISO 8601, assuming 20YY.
fn d500x_time_iso(v: &str) -> Option<String> {
    let (date, time) = v.trim().split_once(' ')?;
    if date.len() != 6 || !date.bytes().all(|b| b.is_ascii_digit()) || time.len() != 8 {
        return None;
    }
    Some(format!(
        "20{}-{}-{}T{}",
        &date[0..2],
        &date[2..4],
        &date[4..6],
        time
    ))
}

/// "Date" section: every date we can pull from the file's metadata, in rough
/// order of trust. Filesystem (created/modified) timestamps and the XC download
/// date need platform plumbing and are a follow-up.
fn date_section(f: &crate::state::LoadedFile, view_mode: MetadataView) -> impl IntoView {
    let mut rows: Vec<(String, String)> = Vec::new();

    // 1. Datestamp embedded in the filename.
    if let Some(d) = date_from_filename(&f.name) {
        rows.push(("Filename".to_string(), d));
    }

    // 2. GUANO timestamp + any other date/time fields.
    if let Some(guano) = &f.audio.metadata.guano {
        for (k, v) in &guano.fields {
            if v.trim().is_empty() {
                continue;
            }
            let kl = k.to_ascii_lowercase();
            if kl == "timestamp" {
                rows.push(("GUANO timestamp".to_string(), v.clone()));
            } else if kl.contains("date") {
                rows.push((format!("GUANO {k}"), v.clone()));
            }
        }
    }

    // 2b. Recording time from a recorder's own metadata block (D500X writes
    // "File Time: YYMMDD hh:mm:ss").
    if let Some(block) = f
        .audio
        .metadata
        .wav
        .as_ref()
        .and_then(|w| w.recorder_block.as_ref())
    {
        if let Some((_, v)) = block.fields.iter().find(|(k, _)| k == "File Time") {
            let value = d500x_time_iso(v).unwrap_or_else(|| v.clone());
            rows.push((format!("{} file time", block.recorder), value));
        }
    }

    // 2c. Wildlife Acoustics timestamp ("2019-08-15 22:12:11-05:00"), unless the
    // GUANO timestamp above is the same.
    if let Some(wamd) = f.audio.metadata.wav.as_ref().and_then(|w| w.wamd.as_ref()) {
        let guano = f
            .audio
            .metadata
            .guano
            .as_ref()
            .map(|g| g.fields.as_slice())
            .unwrap_or_default();
        if let Some((_, v)) = wamd.fields.iter().find(|(k, _)| k == "Timestamp") {
            if !crate::audio::wamd::same_in_guano("Timestamp", v, guano) {
                rows.push((
                    format!("{} timestamp", wamd.recorder),
                    v.replacen(' ', "T", 1),
                ));
            }
        }
    }

    // 3. Recording date (+ time) and upload date from the site the file came
    // from (Xeno-canto, Wikimedia Commons, ...).
    if let Some(site_meta) = &f.site_metadata {
        let get = |key: &str| {
            site_meta
                .fields
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(key))
                .map(|(_, v)| v.clone())
                .filter(|v| !v.trim().is_empty())
        };
        let date_label = format!("{} date", site_meta.site.name());
        match (get("Date"), get("Time")) {
            (Some(d), Some(t)) => rows.push((date_label, format!("{d}T{t}"))),
            (Some(d), None) => rows.push((date_label, d)),
            _ => {}
        }
        if let Some(u) = get("Uploaded").or_else(|| get("Upload date")) {
            rows.push(("Uploaded".to_string(), u));
        }
    }

    let any = !rows.is_empty();
    let items: Vec<_> = rows
        .into_iter()
        .map(|(label, value)| date_row(label, value, view_mode).into_any())
        .collect();
    if any {
        view! {
            <div class="setting-group">
                <div class="setting-group-title setting-group-title-major">"Date"</div>
                {items}
            </div>
        }
        .into_any()
    } else {
        view! { <span></span> }.into_any()
    }
}

/// World-map block with a pin and out-links. Coordinates are in WGS84
/// decimal degrees. The map asset lives at `world-map.png` (declared
/// as a Trunk `copy-file` in index.html, relative path so it works
/// under both the root domain and the /oversample/ subpath build) and
/// is expected to be an equirectangular projection (lon −180→+180 maps
/// linearly across width, lat +90→−90 across height).
fn gps_block(lat: f64, lon: f64, raw_value: String, note: Option<&'static str>) -> impl IntoView {
    let pin_left_pct = (lon + 180.0) / 360.0 * 100.0;
    let pin_top_pct = (90.0 - lat) / 180.0 * 100.0;
    let osm = format!("https://www.openstreetmap.org/?mlat={lat}&mlon={lon}#map=10/{lat}/{lon}");
    let gmaps = format!("https://www.google.com/maps?q={lat},{lon}");
    let coord_text = format!("{lat:.5}, {lon:.5}");
    view! {
        <div class="metadata-value-block">{coord_text}</div>
        {note.map(|n| view! { <div class="metadata-value-relative">{n}</div> })}
        <div class="metadata-map-wrap">
            <img class="metadata-map-img" src="world-map.png" alt="World map" />
            <div class="metadata-map-pin"
                 style=format!("left: {pin_left_pct:.3}%; top: {pin_top_pct:.3}%;")
                 title=raw_value.clone()></div>
        </div>
        <div class="metadata-map-links">
            <a href=osm target="_blank" rel="noopener noreferrer">"OpenStreetMap"</a>
            <span class="metadata-map-link-sep">"\u{00B7}"</span>
            <a href=gmaps target="_blank" rel="noopener noreferrer">"Google Maps"</a>
        </div>
    }
}

/// Characters of a hash shown before and after the "…" that stands for its
/// middle.
const HASH_SHOWN_EACH_END: usize = 10;
/// The same, for the two hashes of a mismatch, which each follow a name
/// ("This file", "Xeno-canto") on the same line.
const HASH_SHOWN_EACH_END_IN_PAIR: usize = 8;
/// Characters of a read error shown in a hash row. The whole error is in the
/// line's tooltip. A browser's error can run to a stack trace of many lines.
const READ_ERROR_SHOWN: usize = 80;

/// A hash on one line: its first and last characters with "…" between them.
///
/// The whole hash is in the text, so selecting the line and copying gives all
/// of it. The middle is in a span that takes no room (`.hash-mid`, font size
/// 0), and the "…" is drawn by CSS, so it is not part of the copied text. A
/// `copy` listener in index.html covers browsers that leave zero-size text
/// out of the clipboard: it puts `data-full` there.
fn hash_value(hash: &str, n: usize) -> impl IntoView {
    let full = hash.to_string();
    if !hash.is_ascii() || hash.len() <= 2 * n + 1 {
        let text = full.clone();
        return view! { <span class="hash-value" data-full=full>{text}</span> }.into_any();
    }
    let head = hash[..n].to_string();
    let mid = hash[n..hash.len() - n].to_string();
    let tail = hash[hash.len() - n..].to_string();
    view! {
        <span class="hash-value" data-full=full.clone() title=full>
            {head}<span class="hash-mid">{mid}</span>{tail}
        </span>
    }
    .into_any()
}

/// The name of the place a saved hash was read from, as the status lines of
/// the hash rows give it.
fn saved_source_name(source: crate::file_identity::SavedSource) -> &'static str {
    match source {
        crate::file_identity::SavedSource::XenoCanto => "Xeno-canto",
        crate::file_identity::SavedSource::Annotations => "saved annotations",
    }
}

/// One kind of hash in the File Identity group: its name, the buttons, the
/// hash on one line, and a status line saying where the hash is from and
/// whether it was compared with a saved one.
///
/// `can_compute`: the file can be read again. `auto`: the file is small, so
/// its hashes are computed without a button. `coming`: this hash is being
/// computed as the file opens.
fn hash_row(
    state: AppState,
    file_id: u64,
    row: crate::file_identity::HashRow,
    can_compute: bool,
    auto: bool,
    coming: bool,
) -> impl IntoView {
    use crate::file_identity::{HashKind, HashState, SavedSource};

    let kind = row.kind;
    let row_state = row.state();
    let has_saved = row.saved.is_some();
    let source = row.saved.as_ref().map(|(_, s)| *s);
    let jobs = crate::file_identity::hash_jobs();

    // The hash the Copy button copies: the file's own when it is known.
    let shown = row
        .computed
        .clone()
        .or_else(|| row.saved.as_ref().map(|(h, _)| h.clone()));
    let copy = shown.clone().map(|hash| {
        let on_copy = move |ev: web_sys::MouseEvent| {
            ev.stop_propagation();
            super::copy_to_clipboard(&hash);
        };
        view! {
            <button class="copy-btn copy-btn-spacious" on:click=on_copy title="Copy full hash">
                {"\u{2398}"}
            </button>
        }
    });

    // Check (compare a saved hash with the file) or Compute (a hash the file
    // has no value of). A small file's saved hashes are compared without a
    // button, so its Check button is there only after a read failed.
    let action = {
        let jobs = jobs.clone();
        move || {
            let (running, failed) = jobs.with(|j| {
                (
                    j.running(file_id, kind).is_some(),
                    j.failure(file_id, kind).is_some(),
                )
            });
            if running || !can_compute || (coming && !failed) {
                return None;
            }
            let (label, title) = match row_state {
                Some(HashState::Saved) if !auto || failed => (
                    "Check",
                    if kind == HashKind::Spot {
                        "Compute this hash from the file and compare it with the saved hash."
                    } else {
                        "Compute this hash from the file and compare it with the saved hash. \
                         Reads the whole file, which can take a while."
                    },
                ),
                None => (
                    "Compute",
                    "Compute this hash from the file. Reads the whole file, which can take a while.",
                ),
                _ => return None,
            };
            let on_click = move |ev: web_sys::MouseEvent| {
                ev.stop_propagation();
                crate::file_identity::compute_hashes(state, file_id, vec![kind]);
            };
            Some(view! {
                <button class="hash-calc-btn" on:click=on_click title=title>{label}</button>
            })
        }
    };

    let status = move || {
        let (running, failure) = jobs.with(|j| {
            (
                j.running(file_id, kind)
                    .map(|job| (job.percent(), job.kinds != [HashKind::Spot])),
                j.failure(file_id, kind).map(str::to_string),
            )
        });
        let verb = if has_saved { "compare" } else { "compute" };
        if let Some((percent, whole_file)) = running {
            let text = if whole_file {
                format!("Reading file to {verb}: {percent}%")
            } else {
                format!("Reading file to {verb}")
            };
            return view! { <div class="hash-status hash-status-busy">{text}</div> }.into_any();
        }
        if let Some(error) = failure {
            let first_line = error.lines().next().unwrap_or_default();
            let short: String = first_line.chars().take(READ_ERROR_SHOWN).collect();
            let cut = short.len() < error.len();
            return view! {
                <div class="hash-status hash-status-failed" title=error.clone()>
                    {format!("Could not read the file: {short}{}", if cut { "…" } else { "" })}
                </div>
            }
            .into_any();
        }
        if coming {
            return view! {
                <div class="hash-status hash-status-busy">{format!("Reading file to {verb}")}</div>
            }
            .into_any();
        }
        let source = source.unwrap_or(SavedSource::Annotations);
        match row_state {
            Some(HashState::Computed) => {
                view! { <div class="hash-status">"Computed from this file"</div> }.into_any()
            }
            Some(HashState::Saved) => view! {
                <div class="hash-status">
                    {match (source, can_compute) {
                        (SavedSource::XenoCanto, true) => "From Xeno-canto. Not compared yet.",
                        (SavedSource::Annotations, true) => {
                            "From saved annotations. Not compared yet."
                        }
                        // No Check button: say why.
                        (SavedSource::XenoCanto, false) => {
                            "From Xeno-canto. Not compared: the file can't be read again."
                        }
                        (SavedSource::Annotations, false) => {
                            "From saved annotations. Not compared: the file can't be read again."
                        }
                    }}
                </div>
            }
            .into_any(),
            Some(HashState::Match) => view! {
                <div class="hash-status hash-status-match">
                    <span class="hash-mark">{"\u{2713}"}</span>
                    {match source {
                        SavedSource::XenoCanto => "Matches the Xeno-canto hash",
                        SavedSource::Annotations => "Matches the hash in saved annotations",
                    }}
                </div>
            }
            .into_any(),
            // The mismatch line is above the two hashes (see below).
            Some(HashState::Mismatch) | None => ().into_any(),
        }
    };

    let body = match (row_state, &row.computed, &row.saved) {
        // Both hashes, each named, under a line that says they differ.
        (Some(HashState::Mismatch), Some(computed), Some((saved, source))) => view! {
            <div class="hash-status hash-status-mismatch">
                <span class="hash-mark">{"\u{2717}"}</span>
                {match source {
                    SavedSource::XenoCanto => "Does not match the Xeno-canto hash",
                    SavedSource::Annotations => "Does not match the hash in saved annotations",
                }}
            </div>
            <div class="hash-pair">
                <span class="hash-pair-label">"This file"</span>
                {hash_value(computed, HASH_SHOWN_EACH_END_IN_PAIR)}
                <span class="hash-pair-label">
                    {match source {
                        SavedSource::XenoCanto => "Xeno-canto",
                        SavedSource::Annotations => "Saved",
                    }}
                </span>
                {hash_value(saved, HASH_SHOWN_EACH_END_IN_PAIR)}
            </div>
            {(*source == SavedSource::Annotations).then(|| {
                let on_click = move |ev: web_sys::MouseEvent| {
                    ev.stop_propagation();
                    crate::file_identity::save_file_hash(state, file_id, kind);
                };
                view! {
                    <button class="hash-calc-btn hash-save-btn" on:click=on_click
                        title="Replace the hash in saved annotations with this file's hash.">
                        "Save this file's hash"
                    </button>
                }
            })}
        }
        .into_any(),
        _ => match &shown {
            Some(hash) => {
                view! { <div class="metadata-value-block">{hash_value(hash, HASH_SHOWN_EACH_END)}</div> }.into_any()
            }
            None if coming => ().into_any(),
            None => view! {
                <div class="metadata-value-block hash-not-computed">"Not computed"</div>
            }
            .into_any(),
        },
    };

    let state_name = match row_state {
        Some(HashState::Computed) => "computed",
        Some(HashState::Saved) => "saved",
        Some(HashState::Match) => "match",
        Some(HashState::Mismatch) => "mismatch",
        None => "none",
    };
    view! {
        <div class="metadata-row metadata-row-spacious hash-row"
             data-hash=kind.key() data-state=state_name>
            <div class="metadata-key-line">
                <span class="metadata-key">{kind.label()}</span>
                {action}
                {copy}
            </div>
            {body}
            {status}
        </div>
    }
}

fn zc_header_section(f: &crate::state::LoadedFile, view_mode: MetadataView) -> impl IntoView {
    let Some(zc) = f.audio.metadata.zc_data.as_ref() else {
        return view! { <span></span> }.into_any();
    };
    let md = &zc.metadata;

    let mut rows: Vec<(String, String)> = Vec::new();
    if !md.location.is_empty() {
        rows.push(("Location".into(), md.location.clone()));
    }
    if !md.species.is_empty() {
        rows.push(("Species".into(), md.species.clone()));
    }
    if !md.tape.is_empty() {
        rows.push(("Tape".into(), md.tape.clone()));
    }
    if !md.date.is_empty() {
        rows.push(("Date".into(), md.date.clone()));
    }
    if !md.spec.is_empty() {
        rows.push(("Spec".into(), md.spec.clone()));
    }
    if !md.note1.is_empty() {
        rows.push(("Note 1".into(), md.note1.clone()));
    }
    if !md.note2.is_empty() {
        rows.push(("Note 2".into(), md.note2.clone()));
    }
    if !md.id_code.is_empty() {
        rows.push(("ID".into(), md.id_code.clone()));
    }
    if !md.gps.is_empty() {
        rows.push(("GPS".into(), md.gps.clone()));
        if let Some((lat, lon)) = oversample_core::audio::zc::gps_position(&md.gps) {
            rows.push(("Position".into(), format!("{lat:.5}, {lon:.5}")));
        }
    }
    if let Some(ts) = md.timestamp {
        rows.push((
            "Recorded".into(),
            format!(
                "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:06}",
                ts.year, ts.month, ts.day, ts.hour, ts.minute, ts.second, ts.microseconds_total
            ),
        ));
    }

    if rows.is_empty() {
        return view! { <span></span> }.into_any();
    }

    let items: Vec<_> = rows
        .into_iter()
        .map(|(k, v)| spacious_row(k, v, None, view_mode).into_any())
        .collect();

    view! {
        <div class="setting-group">
            <div class="setting-group-title setting-group-title-major"
                 title="Fixed-header text fields embedded in the Anabat .zc binary.">
                "Header metadata"
            </div>
            {items}
        </div>
    }
    .into_any()
}

fn format_file_size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

thread_local! {
    /// Files (by `LoadedFile.id`) whose "Other hashes" list the user opened.
    /// Kept here because the File Identity group is built again whenever the
    /// file changes, which includes the end of a hash computation started
    /// from inside the list.
    static OTHER_HASHES_OPEN: std::cell::RefCell<std::collections::HashSet<u64>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// The File Identity group: the file's size in bytes, its main hash, and its
/// other hashes in a list that opens. Which hash is the main one, and what
/// each row's state is, come from `crate::file_identity`.
fn file_identity_section(f: &crate::state::LoadedFile) -> impl IntoView {
    use crate::file_identity::{HashKind, HashState, SavedSource};

    let state = expect_context::<AppState>();
    let file_id = f.id;
    let rows = crate::file_identity::file_hash_rows(f);
    let can_compute = crate::file_identity::can_compute_hashes(f);

    let actual_size = f.identity.as_ref().map(|id| id.file_size);
    let saved_size = f
        .xc_hashes
        .as_ref()
        .and_then(|h| h.file_size.map(|s| (s, SavedSource::XenoCanto)))
        .or_else(|| {
            f.saved_hashes
                .as_ref()
                .and_then(|h| h.file_size.map(|s| (s, SavedSource::Annotations)))
        });
    let size = actual_size.or(saved_size.map(|(s, _)| s)).unwrap_or(0);
    let is_small = size < crate::file_identity::SMALL_FILE_THRESHOLD;

    // A hash that is on its way as the file opens: the spot hash until it is
    // known, then, for a small file, the others.
    let spot_pending = crate::file_identity::spot_hash_pending(file_id);
    let jobs = crate::file_identity::hash_jobs();
    let coming = |kind: HashKind| {
        let row_empty = rows.iter().any(|r| r.kind == kind && r.computed.is_none());
        let (failed, running) = jobs.with_untracked(|j| {
            (
                j.failure(file_id, kind).is_some(),
                j.running(file_id, kind).is_some(),
            )
        });
        let at_open = spot_pending && (kind == HashKind::Spot || is_small);
        row_empty && !failed && f.identity.is_some() && (at_open || running)
    };

    let mut items: Vec<leptos::tachys::view::any_view::AnyView> = Vec::new();

    if size > 0 {
        let status = match (actual_size, saved_size) {
            (Some(actual), Some((saved, source))) if actual == saved => Some(
                view! {
                    <div class="hash-status hash-status-match">
                        <span class="hash-mark">{"\u{2713}"}</span>
                        {match source {
                            SavedSource::XenoCanto => "Matches the Xeno-canto size",
                            SavedSource::Annotations => "Matches the size in saved annotations",
                        }}
                    </div>
                }
                .into_any(),
            ),
            (Some(_), Some((saved, source))) => Some(
                view! {
                    <div class="hash-status hash-status-mismatch">
                        <span class="hash-mark">{"\u{2717}"}</span>
                        {format!("Different size in {}: {saved} bytes", saved_source_name(source))}
                    </div>
                }
                .into_any(),
            ),
            _ => None,
        };
        let size_text = size.to_string();
        let on_copy = {
            let size_text = size_text.clone();
            move |ev: web_sys::MouseEvent| {
                ev.stop_propagation();
                super::copy_to_clipboard(&size_text);
            }
        };
        items.push(
            view! {
                <div class="metadata-row metadata-row-spacious hash-row" data-hash="size">
                    <div class="metadata-key-line">
                        <span class="metadata-key">"File size (bytes)"</span>
                        <button class="copy-btn copy-btn-spacious" on:click=on_copy title="Copy">
                            {"\u{2398}"}
                        </button>
                    </div>
                    <div class="metadata-value-block hash-value">{size_text}</div>
                    {status}
                </div>
            }
            .into_any(),
        );
    }

    let main = crate::file_identity::main_hash(
        size,
        &rows,
        coming(if is_small {
            HashKind::Blake3
        } else {
            HashKind::Spot
        }),
    );
    let shown = |kind: HashKind| {
        rows.iter()
            .find(|r| r.kind == kind)
            .filter(|r| r.state().is_some() || can_compute || coming(kind))
            .cloned()
    };
    if let Some(row) = shown(main) {
        items.push(hash_row(state, file_id, row, can_compute, is_small, coming(main)).into_any());
    }

    if crate::file_identity::only_audio_matches(&rows) {
        items.push(
            view! {
                <div class="metadata-row metadata-row-spacious">
                    <div class="hash-note">
                        "The audio is the same. Only the file's header or embedded metadata is different."
                    </div>
                </div>
            }
            .into_any(),
        );
    }

    let others: Vec<_> = HashKind::ALL
        .into_iter()
        .filter(|kind| *kind != main)
        .filter_map(shown)
        .collect();
    if !others.is_empty() {
        // A hash in the list that differs from the saved one opens the list.
        let mismatch = others
            .iter()
            .any(|r| r.state() == Some(HashState::Mismatch));
        let open = mismatch || OTHER_HASHES_OPEN.with(|o| o.borrow().contains(&file_id));
        let on_toggle = move |ev: web_sys::Event| {
            let now_open = ev
                .target()
                .and_then(|t| js_sys::Reflect::get(&t, &"open".into()).ok())
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            OTHER_HASHES_OPEN.with(|o| {
                if now_open {
                    o.borrow_mut().insert(file_id);
                } else {
                    o.borrow_mut().remove(&file_id);
                }
            });
        };
        let label = format!("Other hashes ({})", others.len());
        let other_rows: Vec<_> = others
            .into_iter()
            .map(|row| {
                let coming = coming(row.kind);
                hash_row(state, file_id, row, can_compute, is_small, coming).into_any()
            })
            .collect();
        items.push(
            view! {
                <details class="metadata-same hash-others" open=open on:toggle=on_toggle>
                    <summary>{label}</summary>
                    {other_rows}
                </details>
            }
            .into_any(),
        );
    }

    // A small file's saved hashes are compared with the file as soon as they
    // are shown. Started after this view is built, since it changes signals.
    if is_small {
        spawn_local(async move {
            crate::file_identity::check_saved_hashes_of_small_file(state, file_id);
        });
    }

    if items.is_empty() {
        view! { <span></span> }.into_any()
    } else {
        view! {
            <div class="setting-group file-identity">
                <div class="setting-group-title setting-group-title-major">"File Identity"</div>
                {items}
            </div>
        }
        .into_any()
    }
}

/// Seconds at each end of a file that are searched for chimes. The X8 phone remote's start
/// chime is in the first few seconds of a take, and its stop chime in the last few.
const CHIME_EDGE_SECS: f64 = 30.0;
/// Seconds of audio decoded between yields to the browser, per channel.
const CHIME_PIECE_SECS: f64 = 1.0;
/// More channels than this are left out of the search.
const CHIME_MAX_CHANNELS: u32 = 8;

/// The chime search for one file.
#[derive(Clone, PartialEq)]
enum ChimeSearch {
    Searching {
        file_id: u64,
    },
    Done {
        file_id: u64,
        result: ChimeScanResult,
    },
}

/// Search the first and last `CHIME_EDGE_SECS` of the current file for chimes, in pieces, with
/// a yield to the browser after each.
fn start_chime_search(
    state: AppState,
    search: RwSignal<Option<ChimeSearch>>,
    generation: RwSignal<u32>,
) {
    let current = state.library.current_index().get().and_then(|i| {
        state.library.files().with(|files| {
            files.get(i).map(|f| {
                let searchable =
                    f.audio.metadata.zc_data.is_none() && !f.is_live_listen && !f.is_recording;
                (f.id, searchable, f.audio.source.clone(), f.audio.channels)
            })
        })
    });
    let Some((file_id, searchable, source, channels)) = current else {
        return;
    };
    let already = search.with_untracked(|s| match s {
        Some(ChimeSearch::Searching { file_id: id } | ChimeSearch::Done { file_id: id, .. }) => {
            *id == file_id
        }
        None => false,
    });
    if already {
        return;
    }
    let this_run = generation.get_untracked().wrapping_add(1);
    generation.set(this_run);
    if !searchable {
        search.set(Some(ChimeSearch::Done {
            file_id,
            result: ChimeScanResult::default(),
        }));
        return;
    }
    search.set(Some(ChimeSearch::Searching { file_id }));
    spawn_local(async move {
        // The panel can close while this runs, which disposes these signals, so reads use try_*.
        let is_latest = move || generation.try_get_untracked() == Some(this_run);
        let sample_rate = source.sample_rate();
        let channels = channels.clamp(1, CHIME_MAX_CHANNELS);
        let piece = (CHIME_PIECE_SECS * sample_rate as f64) as u64;
        let mut results = Vec::new();
        for (start, frames) in
            chime_scan::edge_ranges(source.total_samples(), sample_rate, CHIME_EDGE_SECS)
        {
            let Some(mut scanner) = ChimeScanner::new(sample_rate, channels as usize, start) else {
                break;
            };
            let mut at = start;
            while at < start + frames {
                let len = piece.min(start + frames - at) as usize;
                crate::audio::streaming_source::prefetch_streaming(&*source, at, len).await;
                if !is_latest() {
                    return;
                }
                for channel in 0..channels {
                    let view = if channels == 1 {
                        ChannelView::MonoMix
                    } else {
                        ChannelView::Channel(channel)
                    };
                    scanner.push(channel as usize, &source.read_region(view, at, len));
                }
                at += len as u64;
                crate::web_util::yield_now().await;
                if !is_latest() {
                    return;
                }
            }
            results.push(scanner.finish());
        }
        let _ = search.try_set(Some(ChimeSearch::Done {
            file_id,
            result: chime_scan::merge(results),
        }));
    });
}

/// A position in the file as minutes and seconds.
fn file_position(secs: f64) -> String {
    let whole = secs.max(0.0) as u64;
    format!("{}:{:02}", whole / 60, whole % 60)
}

/// "1 and 2", "1, 2 and 3", counting from 1.
fn channel_list(channels: &[u32]) -> String {
    let names: Vec<String> = channels.iter().map(|c| (c + 1).to_string()).collect();
    match names.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    }
}

fn chime_heading(chime: &FoundChime) -> String {
    let what = match chime.remote().map(|remote| remote.command()) {
        // With or without a choice for the X8's Mute while recording switch.
        Some(Some(RemoteCommand::Start | RemoteCommand::StartWithMute(_))) => {
            CHIME_START.to_string()
        }
        Some(Some(RemoteCommand::Stop)) => CHIME_STOP.to_string(),
        Some(Some(RemoteCommand::SetClock)) => CHIME_SET_CLOCK.to_string(),
        // Mode name first, so "1:1" doesn't sit next to the file position.
        Some(Some(RemoteCommand::Listen(mode))) => {
            format!("{CHIME_REMOTE_PREFIX}{} listening mode", mode.name())
        }
        Some(None) => CHIME_UNKNOWN.to_string(),
        None => CHIME_OTHER.to_string(),
    };
    // Format version 2: the phone page had the X8's code typed in.
    let coded = chime
        .remote()
        .is_some_and(|remote| remote.check().is_some());
    let code = if coded { CHIME_WITH_CODE } else { "" };
    format!("{what}{code} at {}", file_position(chime.start_secs))
}

fn chime_signal_text(chime: &FoundChime) -> String {
    let snr = match chime.snr_db {
        Some(snr) => format!("SNR {snr:.0} dB"),
        None => "SNR unknown".to_string(),
    };
    let channel_word = if chime.channels.len() == 1 {
        "channel"
    } else {
        "channels"
    };
    format!(
        "Carrier {:.0} dBFS, {snr}, {channel_word} {}",
        chime.level_dbfs,
        channel_list(&chime.channels)
    )
}

const CHIME_TITLE: &str = "AudioMoth chimes";
const CHIME_START: &str = "Phone remote: start";
const CHIME_STOP: &str = "Phone remote: stop";
const CHIME_SET_CLOCK: &str = "Phone remote: set clock";
/// Followed by the X8 mode button's name ("HET", "1:1") and " listening mode".
const CHIME_REMOTE_PREFIX: &str = "Phone remote: ";
/// A command from a newer phone page than this version of Oversample reads.
const CHIME_UNKNOWN: &str = "Phone remote: unknown command";
const CHIME_OTHER: &str = "Chime";
const CHIME_WITH_CODE: &str = ", sent with a code,";
const CHIME_TIME_LABEL: &str = "Phone date and time";
/// Not "phone location": the AudioMoth app can send a location typed in by hand.
const CHIME_LOCATION_LABEL: &str = "Chime location";
const CHIME_SIGNAL_LABEL: &str = "Signal";
const CHIME_ID_LABEL: &str = "Deployment ID";
const CHIME_SEARCHING: &str = "Searching for AudioMoth chimes\u{2026}";

/// What the chimes in the file say, or nothing if there are none.
fn chime_section(
    search: Option<ChimeSearch>,
    f: &crate::state::LoadedFile,
    view_mode: MetadataView,
) -> impl IntoView {
    let result = match search {
        Some(ChimeSearch::Done { file_id, result }) if file_id == f.id => result,
        Some(ChimeSearch::Searching { file_id })
            if file_id == f.id && f.audio.sample_rate >= 40_000 =>
        {
            return view! {
                <div class="setting-group">
                    <div class="metadata-note">{CHIME_SEARCHING}</div>
                </div>
            }
            .into_any();
        }
        _ => return view! { <span></span> }.into_any(),
    };
    if result.chimes.is_empty() && result.unreadable == 0 {
        return view! { <span></span> }.into_any();
    }
    let mut items: Vec<leptos::tachys::view::any_view::AnyView> = Vec::new();
    for chime in &result.chimes {
        items.push(view! { <div class="setting-group-title setting-group-title-sub">{chime_heading(chime)}</div> }.into_any());
        items.push(
            date_row(
                CHIME_TIME_LABEL.to_string(),
                chime.local_time_iso(),
                view_mode,
            )
            .into_any(),
        );
        if let Some((lat, lon)) = chime.location {
            let raw = format!("{lat:.6}, {lon:.6}");
            items.push(
                view! {
                    <div class="metadata-row metadata-row-spacious">
                        <div class="metadata-key-line">
                            <span class="metadata-key">{CHIME_LOCATION_LABEL}</span>
                        </div>
                        {gps_block(lat, lon, raw, None)}
                    </div>
                }
                .into_any(),
            );
        }
        if chime.remote_command().is_none() {
            if let Some(hex) = chime.deployment_id_hex() {
                items.push(inline_row(CHIME_ID_LABEL.to_string(), hex, None).into_any());
            }
        }
        items.push(
            spacious_row(
                CHIME_SIGNAL_LABEL.to_string(),
                chime_signal_text(chime),
                None,
                view_mode,
            )
            .into_any(),
        );
    }
    if result.unreadable > 0 {
        let line = if result.unreadable == 1 {
            "1 more chime was detected but could not be read.".to_string()
        } else {
            format!(
                "{} more chimes were detected but could not be read.",
                result.unreadable
            )
        };
        items.push(view! { <div class="metadata-note">{line}</div> }.into_any());
    }
    if f.audio.duration_secs > 2.0 * CHIME_EDGE_SECS {
        items.push(view! { <div class="metadata-note">{CHIME_SEARCHED_EDGES}</div> }.into_any());
    }
    view! {
        <div class="setting-group">
            <div class="setting-group-title setting-group-title-major">{CHIME_TITLE}</div>
            {items}
        </div>
    }
    .into_any()
}

const CHIME_SEARCHED_EDGES: &str = "Searched only the first 30 s and the last 30 s of the file.";

#[component]
pub(crate) fn MetadataPanel() -> impl IntoView {
    let state = expect_context::<AppState>();
    let view_mode = state.panels.metadata_view();
    let chime_search: RwSignal<Option<ChimeSearch>> = RwSignal::new(None);
    let chime_generation = RwSignal::new(0u32);
    Effect::new(move || start_chime_search(state, chime_search, chime_generation));

    view! {
        <div class="sidebar-panel">
            <div class="metadata-view-toggle">
                <button
                    class=move || if view_mode.get() == MetadataView::Formatted {
                        "psd-btn psd-btn-active"
                    } else {
                        "psd-btn"
                    }
                    on:click=move |_| view_mode.set(MetadataView::Formatted)
                    title="Pretty-print JSON, localize dates, show \u{00B0}F"
                >
                    "Formatted"
                </button>
                <button
                    class=move || if view_mode.get() == MetadataView::Original {
                        "psd-btn psd-btn-active"
                    } else {
                        "psd-btn"
                    }
                    on:click=move |_| view_mode.set(MetadataView::Original)
                    title="Show raw values as stored in the file"
                >
                    "Original"
                </button>
            </div>
            {move || {
                let files = state.library.files().get();
                let idx = state.library.current_index().get();
                let file = idx.and_then(|i| files.get(i));
                let view_mode = view_mode.get();

                match file {
                    None => view! {
                        <div class="sidebar-panel-empty">"No file selected"</div>
                    }.into_any(),
                    Some(f) => {
                        let meta = &f.audio.metadata;
                        let (size_str, size_label) = if meta.file_size > 0 {
                            (format_file_size(meta.file_size), "File size".to_string())
                        } else if f.audio.duration_secs > 0.0 {
                            let bytes_per_sample = (meta.bits_per_sample as usize).max(16) / 8;
                            let num_samples = (f.audio.duration_secs * f.audio.sample_rate as f64).ceil() as usize;
                            let estimated = 44 + num_samples * f.audio.channels as usize * bytes_per_sample;
                            (format!("~{}", format_file_size(estimated)), "File size (est.)".to_string())
                        } else {
                            ("0 B".to_string(), "File size".to_string())
                        };
                        let site = f.site_metadata.as_ref().map(|m| m.site.clone());
                        let is_xc = f.site_metadata.as_ref().is_some_and(|m| m.is_xeno_canto());
                        let (xc_fields, source_label) = f.site_metadata.as_ref()
                            .map(|m| (m.fields.clone(), m.site.name().to_string()))
                            .unwrap_or_default();
                        let has_xc = !xc_fields.is_empty();
                        let guano_fields: Vec<_> = meta.guano.as_ref()
                            .map(|g| g.fields.clone())
                            .unwrap_or_default();
                        let has_guano = !guano_fields.is_empty();
                        let wav = meta.wav.clone().unwrap_or_default();
                        // Only the notes that change which audio is read. Data
                        // analysis lists every note, under File structure.
                        let structure_warnings: Vec<_> = wav.notes.iter()
                            .filter(|n| n.warning)
                            .map(wav_note_view)
                            .collect();
                        let recorder_block = wav.recorder_block.clone();
                        let wamd = wav.wamd.clone();
                        let guano_fields_all = guano_fields.clone();

                        view! {
                            <div class="setting-group">
                                <div class="setting-group-title setting-group-title-major">"File"</div>
                                {inline_row("Name".into(), f.name.clone(), None)}
                                {inline_row("Format".into(), format_text(meta), None)}
                                {inline_row("Duration".into(), crate::format_time::format_duration(f.audio.duration_secs, 3), None)}
                                // Not for a live recording: its length is still growing, and
                                // its rate is the microphone's.
                                {(f.audio.can_set_time_expansion() && !f.is_recording && !f.is_live_listen).then(|| view! {
                                    <TimeExpansionRow
                                        file_id=f.id
                                        factor=f.audio.metadata.time_expansion.map_or(1.0, |te| te.factor)
                                        stored_rate=f.audio.stored_rate()
                                        loading=f.loading_id.is_some()
                                        saved=!f.read_only
                                    />
                                })}
                                {inline_row("Sample rate".into(), sample_rate_row_value(f.audio.sample_rate), None)}
                                <RateNotes
                                    notes=file_rate_notes(f)
                                    file_id=f.id
                                    stored_rate=f.audio.stored_rate()
                                    loading=f.loading_id.is_some()
                                    kept=crate::rate_notes::Kept::of(f)
                                />
                                <GuanoControls
                                    file_id=f.id
                                    name=f.name.clone()
                                    action=crate::guano_write::action_for(&crate::guano_write::GuanoFacts::of(
                                        // The desktop app only: Android reaches
                                        // recordings through the media store.
                                        state.is_tauri && !state.status.is_mobile().get_untracked(),
                                        f,
                                        state.annotations.store().with(|store| {
                                            store.get(f.id).and_then(|set| set.file_edits.last().cloned())
                                        }),
                                    ))
                                    loading=f.loading_id.is_some()
                                />
                                {inline_row("Channels".into(), f.audio.channels.to_string(), None)}
                                {inline_row("Bit depth".into(), bit_depth_text(meta), None)}
                                {inline_row(size_label, size_str, None)}
                                {structure_warnings}
                            </div>
                            {date_section(f, view_mode)}
                            {chime_section(chime_search.get(), f, view_mode)}
                            {zc_header_section(f, view_mode)}
                            {if has_xc {
                                let license_url = super::file_badges::get_xc_field(&xc_fields, "License URL");
                                let items: Vec<_> = xc_fields.into_iter().map(|(key, value)| {
                                    let label = site.as_ref()
                                        .map_or(key.as_str(), |s| s.field_label(&key))
                                        .to_string();
                                    // Xeno-canto recordists write their remarks in Markdown.
                                    let kind = if is_xc && key == "Remarks" {
                                        FieldKind::Markdown
                                    } else {
                                        classify(&label)
                                    };
                                    let open = open_link(kind, &value, license_url.as_deref());
                                    spacious_row_as(label, value, None, view_mode, kind, open).into_any()
                                }).collect();
                                view! {
                                    <div class="setting-group">
                                        <div class="setting-group-title setting-group-title-major">{source_label}</div>
                                        {items}
                                    </div>
                                }.into_any()
                            } else {
                                view! { <span></span> }.into_any()
                            }}
                            {if has_guano {
                                let is_guano_source =
                                    matches!(meta.format, AudioFormat::Wav | AudioFormat::W4v);
                                let default_section: &str = if is_guano_source {
                                    "Guano metadata"
                                } else {
                                    "Audio file metadata"
                                };
                                let mut items: Vec<leptos::tachys::view::any_view::AnyView> = Vec::new();
                                let mut current_section: Option<String> = None;
                                let mut first_heading = true;
                                for (k, v) in guano_fields {
                                    let (section, display_key) = if is_guano_source {
                                        let (s, d) = categorize_guano_key(&k);
                                        let s = if s == "GUANO" { default_section.to_string() } else { s };
                                        (s, d)
                                    } else {
                                        (default_section.to_string(), k.clone())
                                    };
                                    if current_section.as_ref() != Some(&section) {
                                        let heading = section.clone();
                                        let show_badge = is_guano_source && heading != default_section;
                                        let heading_class = if first_heading {
                                            "setting-group-title setting-group-title-major"
                                        } else {
                                            "setting-group-title setting-group-title-sub"
                                        };
                                        items.push(view! {
                                            <div class=heading_class>
                                                {heading}
                                                {if show_badge {
                                                    view! { <span class="metadata-source-badge">"GUANO"</span> }.into_any()
                                                } else {
                                                    view! { <span></span> }.into_any()
                                                }}
                                            </div>
                                        }.into_any());
                                        current_section = Some(section);
                                        first_heading = false;
                                    }
                                    items.push(spacious_row(display_key, v, Some(k), view_mode).into_any());
                                }
                                view! {
                                    <div class="setting-group">
                                        {items}
                                    </div>
                                }.into_any()
                            } else {
                                view! { <span></span> }.into_any()
                            }}
                            {recorder_section(recorder_block, &[], view_mode)}
                            {recorder_section(wamd, &guano_fields_all, view_mode)}
                            {if !f.is_recording {
                                file_identity_section(f).into_any()
                            } else {
                                view! { <span></span> }.into_any()
                            }}
                        }.into_any()
                    }
                }
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{
        channel_list, date_from_filename, extract_tz_offset_minutes, file_position,
        format_tz_offset, open_link, sample_rate_row_value, FieldKind,
    };

    #[test]
    fn license_rows_link_to_the_licence() {
        let gnu = "http://www.gnu.org/copyleft/fdl.html";
        for (value, license_url, href) in [
            // Xeno-canto sends addresses without "https:".
            (
                "//creativecommons.org/licenses/by-nc-sa/4.0/",
                None,
                Some("https://creativecommons.org/licenses/by-nc-sa/4.0/"),
            ),
            // Short CC names from the search server and Commons.
            (
                "CC BY-SA 4.0",
                None,
                Some("https://creativecommons.org/licenses/by-sa/4.0/"),
            ),
            // Commons' License URL field.
            ("GFDL", Some(gnu), Some(gnu)),
            ("Fair use", None, None),
        ] {
            assert_eq!(
                open_link(FieldKind::License, value, license_url).as_deref(),
                href,
                "{value}"
            );
        }
        // Other rows open only a value that is a web address.
        assert_eq!(
            open_link(FieldKind::None, "https://xeno-canto.org/1064993", None).as_deref(),
            Some("https://xeno-canto.org/1064993")
        );
        assert_eq!(
            open_link(
                FieldKind::None,
                "//creativecommons.org/licenses/by/4.0/",
                None
            ),
            None
        );
        assert_eq!(open_link(FieldKind::None, "CC BY-SA 4.0", Some(gnu)), None);
    }

    #[test]
    fn the_sample_rate_row_has_an_ordinary_space() {
        assert_eq!(sample_rate_row_value(384_000), "384 kHz");
        assert_eq!(sample_rate_row_value(38_400), "38.4 kHz");
        assert_eq!(sample_rate_row_value(44_100), "44.1 kHz");
        assert_eq!(sample_rate_row_value(500_000), "500 kHz");
    }

    #[test]
    fn chime_positions_and_channels_read_naturally() {
        assert_eq!(file_position(252.9), "4:12");
        assert_eq!(file_position(5.0), "0:05");
        assert_eq!(channel_list(&[0]), "1");
        assert_eq!(channel_list(&[0, 1]), "1 and 2");
        assert_eq!(channel_list(&[0, 1, 3]), "1, 2 and 4");
    }

    #[test]
    fn filename_date_patterns() {
        assert_eq!(
            date_from_filename("20240115.WAV").as_deref(),
            Some("2024-01-15")
        );
        assert_eq!(
            date_from_filename("PREFIX_20210630_223015.wav").as_deref(),
            Some("2021-06-30T22:30:15"),
        );
        assert_eq!(
            date_from_filename("20210630223015.wav").as_deref(),
            Some("2021-06-30T22:30:15"),
        );
        assert_eq!(
            date_from_filename("AUDIOMOTH-20231101T080000.wav").as_deref(),
            Some("2023-11-01T08:00:00"),
        );
    }

    #[test]
    fn filename_date_rejects_non_dates() {
        // Too-large month/day, out-of-range year, random digit runs.
        assert_eq!(date_from_filename("track123456789.wav"), None);
        assert_eq!(date_from_filename("99999999.wav"), None);
        assert_eq!(date_from_filename("18000101.wav"), None); // year < 1990
        assert_eq!(date_from_filename("XC1008337.wav"), None); // XC id, not a date
        assert_eq!(date_from_filename("recording.wav"), None);
    }

    #[test]
    fn tz_offset_bare_date_is_none() {
        // The XC1008337 regression: a bare date must NOT yield a timezone
        // offset (the "-30" in "2025-05-30" was being read as UTC-30:00).
        assert_eq!(extract_tz_offset_minutes("2025-05-30"), None);
        assert_eq!(extract_tz_offset_minutes("1999-12-31"), None);
        assert_eq!(extract_tz_offset_minutes("2025-01-05"), None);
    }

    #[test]
    fn tz_offset_zulu() {
        assert_eq!(extract_tz_offset_minutes("2025-05-30T10:30:45Z"), Some(0));
        assert_eq!(extract_tz_offset_minutes("2025-05-30T10:30:45z"), Some(0));
    }

    #[test]
    fn tz_offset_colon_forms() {
        assert_eq!(
            extract_tz_offset_minutes("2025-05-30T10:30:45+05:00"),
            Some(300)
        );
        assert_eq!(
            extract_tz_offset_minutes("2025-05-30T10:30:45-05:00"),
            Some(-300)
        );
        assert_eq!(
            extract_tz_offset_minutes("2025-05-30T10:30:45+05:30"),
            Some(330)
        );
        assert_eq!(
            extract_tz_offset_minutes("2025-05-30T10:30:45-09:30"),
            Some(-570)
        );
        // Space separator instead of 'T'.
        assert_eq!(
            extract_tz_offset_minutes("2025-05-30 10:30:45-05:00"),
            Some(-300)
        );
        // Odd-but-real three-quarter-hour zone (Chatham Islands, +12:45).
        assert_eq!(
            extract_tz_offset_minutes("2025-05-30T10:30:45+12:45"),
            Some(765)
        );
    }

    #[test]
    fn tz_offset_compact_forms() {
        assert_eq!(
            extract_tz_offset_minutes("2025-05-30T10:30:45+0530"),
            Some(330)
        );
        assert_eq!(
            extract_tz_offset_minutes("2025-05-30T10:30:45-0530"),
            Some(-330)
        );
        assert_eq!(
            extract_tz_offset_minutes("2025-05-30T10:30:45+05"),
            Some(300)
        );
        assert_eq!(
            extract_tz_offset_minutes("2025-05-30T10:30:45-08"),
            Some(-480)
        );
    }

    #[test]
    fn tz_offset_no_offset_present() {
        assert_eq!(extract_tz_offset_minutes("2025-05-30T10:30:45"), None);
        assert_eq!(extract_tz_offset_minutes("2025-05-30T10:30"), None);
    }

    #[test]
    fn tz_offset_invalid_or_empty() {
        assert_eq!(extract_tz_offset_minutes(""), None);
        assert_eq!(extract_tz_offset_minutes("   "), None);
        assert_eq!(extract_tz_offset_minutes("not a date"), None);
        // Implausible offsets are rejected rather than displayed.
        assert_eq!(extract_tz_offset_minutes("2025-05-30T10:30:45+30:00"), None);
        assert_eq!(extract_tz_offset_minutes("2025-05-30T10:30:45-99:99"), None);
        assert_eq!(extract_tz_offset_minutes("2025-05-30T10:30:45+05:99"), None);
    }

    #[test]
    fn format_offset_roundtrips() {
        assert_eq!(format_tz_offset(0), "UTC");
        assert_eq!(format_tz_offset(300), "UTC+05:00");
        assert_eq!(format_tz_offset(-300), "UTC-05:00");
        assert_eq!(format_tz_offset(330), "UTC+05:30");
        assert_eq!(format_tz_offset(-570), "UTC-09:30");
        assert_eq!(format_tz_offset(765), "UTC+12:45");
    }
}
