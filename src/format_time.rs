// SPDX-License-Identifier: GPL-3.0-only OR MIT OR Apache-2.0
//! Centralized time formatting for the entire app.
//!
//! All time values are in seconds. Two main entry points:
//!
//! - **`format_time_label`** — adaptive precision for canvas timeline labels (compact)
//! - **`format_time_display`** — fixed precision for UI text (annotations, metadata, etc.)

// ── Adaptive-precision (canvas timeline) ────────────────────────────────

/// Format a time value as a compact label for canvas timeline ticks.
///
/// Precision adapts to the tick `interval` (in seconds) so labels aren't
/// over- or under-specified.  `use_ms` forces millisecond notation for
/// sub-second values (should be true only when the max visible time ≤ 100 ms).
pub fn format_time_label(seconds: f64, interval: f64, use_ms: bool) -> String {
    let abs = seconds.abs();

    if abs < 1.0 {
        if use_ms {
            let ms = seconds * 1000.0;
            return if interval < 0.01 {
                format!("{:.1} ms", ms)
            } else {
                format!("{:.0} ms", ms)
            };
        }
        return format_seconds_adaptive(seconds, interval);
    }

    if abs < 60.0 {
        return format_seconds_adaptive(seconds, interval);
    }

    if abs < 3600.0 {
        return format_minutes_seconds_adaptive(seconds, interval);
    }

    format_hours_minutes_seconds_adaptive(seconds, interval)
}

/// An offset from a reference time: `+200 ms` under a second, `+1.5 s` from
/// a second up, `−50 ms` before it. `interval` is the tick spacing (or the
/// precision wanted), in seconds. Empty when the offset is 0.
///
/// Every offset in the app uses this, so the main timeline and the 3D view
/// label offsets the same way.
pub fn format_relative_label(offset: f64, interval: f64) -> String {
    let abs = offset.abs();
    if abs < interval * 1e-3 {
        return String::new();
    }
    let sign = if offset >= 0.0 { "+" } else { "\u{2212}" }; // − (Unicode minus)
    let ms_decimals: i32 = if interval >= 0.001 { 0 } else { 1 };
    let ms = abs * 1000.0;
    let scale = 10f64.powi(ms_decimals);
    if (ms * scale).round() / scale < 1000.0 {
        format!("{sign}{ms:.prec$} ms", prec = ms_decimals as usize)
    } else {
        // Rounded to 1 ms; Display drops trailing zeros ("+1 s", "+1.5 s").
        format!("{sign}{} s", ms.round() / 1000.0)
    }
}

// ── Fixed-precision (UI display) ────────────────────────────────────────

/// Format a time position with fixed decimal precision for UI display.
///
/// Examples (precision=3): `5.250s`, `1m30.500s`, `1h05m30.000s`
pub fn format_time_display(seconds: f64, precision: u8) -> String {
    let sign = if seconds < 0.0 { "-" } else { "" };
    let abs = seconds.abs();

    if abs < 60.0 {
        format!("{}{:.prec$}s", sign, abs, prec = precision as usize)
    } else if abs < 3600.0 {
        let mins = (abs / 60.0).floor() as u32;
        let secs = abs - mins as f64 * 60.0;
        format!(
            "{}{}m{:0>width$.prec$}s",
            sign,
            mins,
            secs,
            width = seconds_width(precision),
            prec = precision as usize
        )
    } else {
        let hours = (abs / 3600.0).floor() as u32;
        let rem = abs - hours as f64 * 3600.0;
        let mins = (rem / 60.0).floor() as u32;
        let secs = rem - mins as f64 * 60.0;
        format!(
            "{}{}h{:02}m{:0>width$.prec$}s",
            sign,
            hours,
            mins,
            secs,
            width = seconds_width(precision),
            prec = precision as usize
        )
    }
}

/// Width of the zero-padded seconds after minutes: "05" with no decimals, "05.250"
/// (3 + decimals) with them.
fn seconds_width(precision: u8) -> usize {
    if precision == 0 {
        2
    } else {
        3 + precision as usize
    }
}

/// Format a duration (always positive) with fixed precision.
///
/// Same as `format_time_display` but takes the absolute value.
pub fn format_duration(seconds: f64, precision: u8) -> String {
    format_time_display(seconds.abs(), precision)
}

/// Format a duration compactly for UI lists (file lengths, gaps, etc.).
///
/// - Under 90s: `45.1s`
/// - 90s–1h: `5m30s`
/// - Over 1h: `9h44m22s`
pub fn format_duration_compact(seconds: f64) -> String {
    let abs = seconds.abs();
    if abs < 90.0 {
        format!("{abs:.1}s")
    } else if abs < 3600.0 {
        let mins = (abs / 60.0).floor() as u32;
        let secs = (abs - mins as f64 * 60.0).round() as u32;
        if secs == 60 {
            format!("{}m00s", mins + 1)
        } else {
            format!("{mins}m{secs:02}s")
        }
    } else {
        let hours = (abs / 3600.0).floor() as u32;
        let rem = abs - hours as f64 * 3600.0;
        let mins = (rem / 60.0).floor() as u32;
        let secs = (rem - mins as f64 * 60.0).round() as u32;
        if secs == 60 {
            format!("{hours}h{:02}m00s", mins + 1)
        } else {
            format!("{hours}h{mins:02}m{secs:02}s")
        }
    }
}

/// Format a time range as "start–end" (en-dash separated).
///
/// Example: `5.000–10.500s`
pub fn format_time_range(start: f64, end: f64, precision: u8) -> String {
    format!(
        "{}–{}",
        format_time_display(start, precision),
        format_time_display(end, precision)
    )
}

// ── Offset axis (3D view) ───────────────────────────────────────────────

/// Ticks for a time axis that starts at `start` seconds in the recording and
/// is `span` seconds long. The first tick is the time in the recording
/// (`20m12.32s`); the rest are offsets from it (`+200 ms`, `+1.5 s`).
/// Positions are offsets in seconds. At most `max_ticks` ticks.
pub fn offset_axis_ticks(start: f64, span: f64, max_ticks: usize) -> (Vec<f64>, Vec<String>) {
    let step = nice_step(span / max_ticks.saturating_sub(1).max(1) as f64);
    let n = (span / step + 1e-9).floor().max(0.0) as usize;
    // Milliseconds, like the selection times elsewhere; finer for tiny steps.
    let precision = if step >= 0.01 { 3 } else { 4 };
    let vals: Vec<f64> = (0..=n).map(|i| i as f64 * step).collect();
    let text = vals
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            if i == 0 {
                format_time_display(start, precision)
            } else {
                format_relative_label(v, step)
            }
        })
        .collect();
    (vals, text)
}

/// Smallest 1, 2 or 5 times a power of ten that is at least `raw`.
fn nice_step(raw: f64) -> f64 {
    if raw <= 0.0 || !raw.is_finite() {
        return 1.0;
    }
    let mag = 10f64.powf(raw.log10().floor());
    [1.0, 2.0, 5.0, 10.0]
        .iter()
        .map(|m| m * mag)
        .find(|&s| s >= raw * (1.0 - 1e-9))
        .unwrap_or(10.0 * mag)
}

// ── GUANO timestamp helper ─────────────────────────────────────────────

/// Build an ISO 8601 timestamp with local timezone offset for a recording
/// that started `duration_secs` ago.
/// Returns e.g. "2024-03-15T10:30:00+10:00".
pub fn recording_timestamp(duration_secs: f64) -> String {
    let now = js_sys::Date::new_0();
    let start_ms = now.get_time() - (duration_secs * 1000.0);
    let start = js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(start_ms));

    let tz_offset_minutes = start.get_timezone_offset() as i32;
    let offset_total = -tz_offset_minutes;
    let offset_sign = if offset_total < 0 { '-' } else { '+' };
    let offset_abs = offset_total.unsigned_abs();
    let offset_h = offset_abs / 60;
    let offset_m = offset_abs % 60;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}{}{:02}:{:02}",
        start.get_full_year(),
        start.get_month() + 1,
        start.get_date(),
        start.get_hours(),
        start.get_minutes(),
        start.get_seconds(),
        offset_sign,
        offset_h,
        offset_m,
    )
}

// ── Private adaptive helpers ────────────────────────────────────────────

fn format_seconds_adaptive(seconds: f64, interval: f64) -> String {
    if interval >= 1.0 {
        format!("{:.0}s", seconds)
    } else if interval >= 0.1 {
        // Drop ".0" when seconds are whole
        let rounded = (seconds * 10.0).round() / 10.0;
        if (rounded - rounded.round()).abs() < 0.01 {
            format!("{:.0}s", rounded)
        } else {
            format!("{:.1}s", seconds)
        }
    } else if interval >= 0.01 {
        format!("{:.2}s", seconds)
    } else {
        format!("{:.3}s", seconds)
    }
}

fn format_minutes_seconds_adaptive(seconds: f64, interval: f64) -> String {
    let sign = if seconds < 0.0 { "-" } else { "" };
    let abs = seconds.abs();
    let mins = (abs / 60.0).floor() as u32;
    let secs = abs - mins as f64 * 60.0;
    if interval >= 1.0 {
        format!("{}{}m{:02.0}s", sign, mins, secs)
    } else if interval >= 0.1 {
        // Drop ".0" when seconds are whole
        let rounded = (secs * 10.0).round() / 10.0;
        if (rounded - rounded.round()).abs() < 0.01 {
            format!("{}{}m{:02.0}s", sign, mins, rounded)
        } else {
            format!("{}{}m{:04.1}s", sign, mins, secs)
        }
    } else {
        format!("{}{}m{:06.3}s", sign, mins, secs)
    }
}

fn format_hours_minutes_seconds_adaptive(seconds: f64, interval: f64) -> String {
    let sign = if seconds < 0.0 { "-" } else { "" };
    let abs = seconds.abs();
    let hours = (abs / 3600.0).floor() as u32;
    let rem = abs - hours as f64 * 3600.0;
    let mins = (rem / 60.0).floor() as u32;
    let secs = rem - mins as f64 * 60.0;
    if interval >= 1.0 {
        format!("{}{}h{:02}m{:02.0}s", sign, hours, mins, secs)
    } else {
        format!("{}{}h{:02}m{:04.1}s", sign, hours, mins, secs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_seconds_pad_to_two_digits() {
        assert_eq!(format_duration(134.0, 0), "2m14s");
        assert_eq!(format_duration(65.0, 0), "1m05s");
        assert_eq!(format_duration(8364.0, 0), "2h19m24s");
        assert_eq!(format_duration(65.25, 3), "1m05.250s");
        assert_eq!(format_duration(9.0, 0), "9s");
    }

    #[test]
    fn offset_axis_starts_with_recording_time() {
        let (vals, text) = offset_axis_ticks(1212.32, 1.6, 6);
        assert_eq!(vals, vec![0.0, 0.5, 1.0, 1.5]);
        assert_eq!(text, vec!["20m12.320s", "+500 ms", "+1 s", "+1.5 s"]);
    }

    #[test]
    fn offset_axis_short_and_long_spans() {
        let (_, text) = offset_axis_ticks(5.0, 0.012, 6);
        assert_eq!(text, vec!["5.0000s", "+5 ms", "+10 ms"]);
        let (_, text) = offset_axis_ticks(0.384, 0.9, 6);
        assert_eq!(
            text,
            vec!["0.384s", "+200 ms", "+400 ms", "+600 ms", "+800 ms"]
        );
        let (vals, text) = offset_axis_ticks(3700.0, 10.0, 6);
        assert_eq!(vals.len(), 6);
        assert_eq!(text[0], "1h01m40.000s");
        assert_eq!(text[5], "+10 s");
    }

    #[test]
    fn offset_hover_precision() {
        assert_eq!(format_relative_label(0.1184, 0.0001), "+118.4 ms");
        assert_eq!(format_relative_label(1.2345, 0.0001), "+1.235 s");
    }

    #[test]
    fn relative_labels_on_the_main_timeline() {
        assert_eq!(format_relative_label(0.05, 0.01), "+50 ms");
        assert_eq!(format_relative_label(0.4, 0.2), "+400 ms");
        assert_eq!(format_relative_label(1.5, 0.5), "+1.5 s");
        assert_eq!(format_relative_label(2.0, 1.0), "+2 s");
        assert_eq!(format_relative_label(-0.05, 0.01), "\u{2212}50 ms");
        assert_eq!(format_relative_label(0.9996, 0.001), "+1 s");
        assert_eq!(format_relative_label(0.0, 0.1), "");
        assert_eq!(format_time_label(0.05, 0.01, true), "50 ms");
    }
}
