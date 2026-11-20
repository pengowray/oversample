//! Species panel: the selected bat book species' facts, and its typical calls
//! drawn on the spectrogram at the recording's own time scale (px per second)
//! and frequency scale, so call lengths, spacing and frequencies can be
//! compared directly.
//!
//! The calls are drawn at the left edge of the spectrogram (or just after the
//! end of the recording when there's room). When the spectrogram is big
//! enough, the species text sits in a column over the drawing, with a clear
//! window over the drawn calls: the drawing stays put and the text scrolls
//! over it, so scrolling the text covers the calls and scrolling back to the
//! top shows them again. On a small spectrogram (phones) the drawing is
//! narrower and has no text; the text is in the species card, a sheet opened
//! from the bat book's More info button, with the calls drawn large.
//!
//! The drawing uses a different colour scheme from the recording (greyscale,
//! or cividis when the recording itself is greyscale) so it is never read as
//! part of the recording. It redraws only on view-scale changes, never on
//! playback position.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::bat_book::auto_resolve;
use crate::bat_book::call_sketch::{
    self, CallSketch, Form, FormKind, Range, SketchCall, StripLayout, StripWidth, STRIP_PAD,
};
use crate::bat_book::data::get_manifest;
use crate::bat_book::types::BatBookEntry;
use crate::canvas::spectrogram_renderer::{freq_to_y, Colormap};
use crate::state::store_fields::*;
use crate::state::{AppState, SpectSize};

/// Widest the drawing may get, as a fraction of the spectrogram's width.
const MAX_WIDTH_FRACTION: f64 = 0.35;
/// Width of the text column over the drawing, CSS px.
const COLUMN_WIDTH: f64 = 270.0;
/// Height of the column's top block (navigation, name, scientific name)
/// until it has been measured, CSS px.
const TOP_BLOCK_GUESS_PX: f64 = 70.0;
/// Clear space above and below the drawn calls in the window, CSS px.
const WINDOW_MARGIN: f64 = 12.0;
/// Below this spectrogram size the text goes in the species card: the
/// column would cover too much of the recording.
const COLUMN_MIN_VIEW_W: f64 = 2.0 * COLUMN_WIDTH;
const COLUMN_MIN_VIEW_H: f64 = 230.0;
/// Without the column (phones), the drawing is at least this wide, CSS px:
/// half the column-less width it had before 0.5.156, which was mostly empty.
const NARROW_MIN_WIDTH: f64 = 75.0;
/// Without the column, more calls of the train are drawn only while they fit
/// in this fraction of the spectrogram's width (or `NARROW_MIN_WIDTH`).
const NARROW_FIT_FRACTION: f64 = 0.2;
/// The card's drawing: height, the left margin for kHz labels, and the
/// bottom margin for the time scale, CSS px.
const CARD_SKETCH_H: f64 = 150.0;
const CARD_AXIS_W: f64 = 52.0;
const CARD_SCALE_H: f64 = 18.0;
/// The card stretches time so the calls fill its width, but keeps each call
/// at least this wide, CSS px, by drawing fewer calls of the train.
const CARD_MIN_CALL_PX: f64 = 10.0;

/// The main view's scale, which the drawing copies.
#[derive(Clone, Copy, Debug, PartialEq)]
struct ViewScale {
    /// CSS px per second.
    px_per_s: f64,
    /// Visible frequency range, Hz.
    min_hz: f64,
    max_hz: f64,
    /// Highest frequency in the recording (half its sample rate), Hz.
    file_max_hz: f64,
    /// Spectrogram size, CSS px.
    css_w: f64,
    css_h: f64,
}

/// Where the drawn calls are when none of them is in the visible frequency
/// range.
#[derive(Clone, Copy, Debug, PartialEq)]
enum OutOfView {
    /// Above the view; zooming out on frequency shows them.
    Above,
    /// Above the recording's highest frequency: the recording can't show them.
    AboveRecording {
        lo_khz: f64,
        hi_khz: f64,
        top_khz: f64,
    },
    /// Below the view; zooming out on frequency shows them.
    Below,
}

/// The species the panel shows: its bat book entry and its sketch, if any.
#[derive(Clone, Debug, PartialEq)]
struct Shown {
    entry: BatBookEntry,
    sketch: Option<&'static CallSketch>,
}

/// The id of the species the panel shows: the chosen one of several
/// selected, else the last selected.
fn shown_id(state: AppState) -> Option<String> {
    let ids = state.bat_book.selected_ids().get();
    let chosen = state.bat_book.shown_id().get();
    match chosen {
        Some(id) if ids.contains(&id) => Some(id),
        _ => ids.last().cloned(),
    }
}

/// The species panel is open over the spectrogram (not hidden by clean view).
fn panel_open(state: AppState) -> bool {
    state.bat_book.open().get()
        && state.bat_book.ref_open().get()
        && !state.viewmode.clean_view().get()
}

/// The species text goes in the species card instead of a column over the
/// drawn calls: the column would cover too much of the recording.
pub fn text_below(state: AppState) -> bool {
    let v = state.bat_book.spect_size().get();
    panel_open(state) && v.w > 0.0 && (v.w < COLUMN_MIN_VIEW_W || v.h < COLUMN_MIN_VIEW_H)
}

/// The bat book shows its More info button: a species is shown and its text
/// has no column over the spectrogram (a small spectrogram, another view, or
/// no recording open).
pub fn info_button_shown(state: AppState) -> bool {
    let column =
        panel_open(state) && state.bat_book.spect_size().get().w > 0.0 && !text_below(state);
    state.bat_book.ref_open().get()
        && !state.bat_book.selected_ids().with(|ids| ids.is_empty())
        && !column
}

fn entry_for(state: AppState, id: &str) -> Option<BatBookEntry> {
    let region = state.bat_book.region().get();
    get_manifest(region)
        .entries
        .into_iter()
        .find(|e| e.id == id)
        .or_else(|| auto_resolve::find_entry_any_book(id))
}

/// The form chosen in the panel, or search calls when the species lacks it.
fn chosen_form(sketch: &'static CallSketch, state: AppState) -> Form<'static> {
    state
        .bat_book
        .call_form()
        .with(|key| sketch.form(key))
        .unwrap_or_else(|| sketch.search())
}

/// Lowest and highest frequency drawn, kHz: the calls and their harmonics.
fn drawn_khz_extent(form: Form<'static>) -> Option<(f64, f64)> {
    let (lo, hi) = form.khz_extent()?;
    let multiples = || {
        form.sequence()
            .into_iter()
            .flat_map(|c| c.harmonics.iter().map(|h| h.multiple))
    };
    Some((
        lo * multiples().fold(1.0, f64::min),
        hi * multiples().fold(1.0, f64::max),
    ))
}

/// Colour scheme for the drawing: one the recording isn't using.
fn sketch_colormap(state: AppState) -> Colormap {
    let main = state.spect.colormap_preference().get();
    let hfr = state.spect.hfr_colormap_preference().get();
    if main == Colormap::Greyscale || hfr == Colormap::Greyscale {
        Colormap::Cividis
    } else {
        Colormap::Greyscale
    }
}

fn rgba(c: [u8; 3], a: f64) -> String {
    format!("rgba({},{},{},{a:.3})", c[0], c[1], c[2])
}

fn fmt(v: f64) -> String {
    if (v - v.round()).abs() < 0.05 {
        format!("{}", v.round() as i64)
    } else {
        format!("{v:.1}")
    }
}

fn range_text(r: Option<&Range>, unit: &str) -> Option<String> {
    let r = r?;
    let v = r.value()?;
    let typical = format!("{} {unit}", fmt(v));
    match (r.min, r.max) {
        (Some(a), Some(b)) if a < b => Some(format!("{typical} ({} to {} {unit})", fmt(a), fmt(b))),
        _ => Some(typical),
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// "300 ms" under a second, "1.2 s" from a second up.
fn duration_text(s: f64) -> String {
    if s < 1.0 {
        format!("{} ms", fmt(s * 1000.0))
    } else {
        format!("{} s", fmt(s))
    }
}

/// The facts of one form, one line each, in the order a reader compares
/// them with the recording: frequency, call length, spacing.
fn form_facts(form: Form<'static>) -> Vec<String> {
    let one = |field: &str| {
        if form.is_one_source(field) {
            ", 1 source"
        } else {
            ""
        }
    };
    // Each kind of call once, in train order: two for a species that
    // alternates high and low calls, first and last for an approach.
    let mut kinds: Vec<&SketchCall> = Vec::new();
    for c in form.sequence().into_iter().chain(form.calls.iter()) {
        if !kinds.iter().any(|k| k.name == c.name) {
            kinds.push(c);
        }
    }
    if form.kind == FormKind::Search || form.is_ramp() {
        // Search lists only the calls of its train; an approach its two ends.
        let seq = form.sequence();
        kinds.retain(|k| seq.iter().any(|c| c.name == k.name));
    }
    let several = kinds.len() > 1;
    let prefix = |c: &SketchCall| {
        if several {
            format!("{} call: ", capitalize(&c.name))
        } else {
            String::new()
        }
    };
    let mut facts = Vec::new();
    for c in &kinds {
        let freq = range_text(c.peak_khz.as_ref(), "kHz")
            .map(|t| ("Peak frequency", "peak_khz", t))
            .or_else(|| {
                range_text(c.characteristic_khz.as_ref(), "kHz")
                    .map(|t| ("Characteristic frequency", "characteristic_khz", t))
            });
        let sweep = match (
            c.start_khz.as_ref().and_then(Range::value),
            c.end_khz.as_ref().and_then(Range::value),
        ) {
            (Some(a), Some(b)) => Some(format!("from {} to {} kHz", fmt(a), fmt(b))),
            _ => None,
        };
        match (freq, sweep) {
            (Some((label, field, t)), _) => {
                let label = if several {
                    label.to_lowercase()
                } else {
                    label.to_string()
                };
                facts.push(format!("{}{label} {t}{}", prefix(c), one(field)));
            }
            (None, Some(sw)) => {
                let label = if several { "sweeps" } else { "Sweeps" };
                facts.push(format!("{}{label} {sw}{}", prefix(c), one("start_khz")));
            }
            _ => {}
        }
    }
    let lengths: Vec<Option<String>> = kinds
        .iter()
        .map(|c| range_text(c.duration_ms.as_ref(), "ms"))
        .collect();
    if form.is_ramp() && kinds.len() == 2 {
        let v = |c: &SketchCall| c.duration_ms.as_ref().and_then(Range::value);
        if let (Some(a), Some(b)) = (v(kinds[0]), v(kinds[1])) {
            facts.push(format!(
                "Call length {} ms, shortening to {} ms{}",
                fmt(a),
                fmt(b),
                one("duration_ms")
            ));
        }
    } else if !lengths.is_empty() && lengths.iter().all(|l| *l == lengths[0]) {
        if let Some(t) = &lengths[0] {
            facts.push(format!("Call length {t}{}", one("duration_ms")));
        }
    } else {
        for (c, l) in kinds.iter().zip(&lengths) {
            if let Some(t) = l {
                facts.push(format!("{}length {t}{}", prefix(c), one("duration_ms")));
            }
        }
    }
    let iv = form.train.and_then(|t| t.interval_ms.as_ref());
    if form.is_ramp() {
        if let Some((a, b)) = iv.and_then(|r| Some((r.max?, r.min?))) {
            facts.push(format!(
                "Call interval {} ms, shrinking to {} ms{}",
                fmt(a),
                fmt(b),
                one("interval_ms")
            ));
        }
    } else if let Some(t) = range_text(iv, "ms") {
        let rate = match (form.kind, form.interval_s()) {
            (FormKind::Buzz, Some(s)) if s > 0.0 => format!(", {} calls per second", fmt(1.0 / s)),
            _ => String::new(),
        };
        facts.push(format!("Call interval {t}{rate}{}", one("interval_ms")));
    }
    if let Some(s) = form.length_s() {
        facts.push(format!("Lasts {}{}", duration_text(s), one("length_ms")));
    }
    facts
}

/// Why a form isn't drawn, when it isn't.
fn not_drawn_reason(form: Form<'static>) -> Option<&'static str> {
    if form.drawable() {
        return None;
    }
    let seq = form.sequence();
    if seq.iter().any(|c| c.shape.len() < 2) {
        Some("Not drawn: no published shape for this call.")
    } else {
        Some("Not drawn: no published call length.")
    }
}

fn out_of_view_text(o: OutOfView) -> String {
    match o {
        OutOfView::Above => "Calls are above the top of the spectrogram. Zoom out on frequency \
                             (Shift + scroll wheel) to see them."
            .to_string(),
        OutOfView::AboveRecording {
            lo_khz,
            hi_khz,
            top_khz,
        } => format!(
            "Calls are at {} to {} kHz, above the highest frequency in this recording ({} kHz). \
             Zooming will not show them.",
            fmt(lo_khz),
            fmt(hi_khz),
            fmt(top_khz)
        ),
        OutOfView::Below => "Calls are below the bottom of the spectrogram. Zoom out on frequency \
                             (Shift + scroll wheel) to see them."
            .to_string(),
    }
}

/// Step to the previous (-1) or next (+1) species: through the selected
/// species when there are several, else through the region's book (which
/// also selects it and focuses its frequency range).
fn step(state: AppState, delta: i32) {
    let ids = state.bat_book.selected_ids().get_untracked();
    if ids.len() > 1 {
        let cur = state
            .bat_book
            .shown_id()
            .get_untracked()
            .and_then(|id| ids.iter().position(|i| *i == id))
            .unwrap_or(ids.len() - 1) as i32;
        let next = (cur + delta).clamp(0, ids.len() as i32 - 1) as usize;
        state.bat_book.shown_id().set(Some(ids[next].clone()));
        return;
    }
    let region = state.bat_book.region().get_untracked();
    let manifest = get_manifest(region);
    let n = manifest.entries.len();
    if n == 0 {
        return;
    }
    let cur = ids
        .last()
        .and_then(|id| manifest.entries.iter().position(|e| e.id == id.as_str()));
    let next = match cur {
        Some(c) => {
            let nx = c as i32 + delta;
            if nx < 0 || nx >= n as i32 {
                return;
            }
            nx as usize
        }
        // A species from another region's book: step into this one.
        None if delta > 0 => 0,
        None => n - 1,
    };
    let entry = &manifest.entries[next];
    state
        .bat_book
        .selected_ids()
        .set(vec![entry.id.to_string()]);
    state
        .bat_book
        .last_clicked_id()
        .set(Some(entry.id.to_string()));
    if state.bat_book.auto_focus().get_untracked() {
        if let Some(idx) = state.library.current_index().get_untracked() {
            let nyquist = state
                .library
                .files()
                .with_untracked(|f| f.get(idx).map(|f| f.audio.sample_rate as f64 / 2.0));
            if let Some(nyquist) = nyquist {
                if entry.freq_lo_hz < nyquist {
                    state.push_bat_book_ff(entry.freq_lo_hz, entry.freq_hi_hz.min(nyquist));
                }
            }
        }
    }
}

/// "‹ Europe 12 / 38 › ×": where the shown species is, the step buttons and
/// the close button, which calls `close`.
fn nav_row(state: AppState, close: impl Fn() + 'static) -> impl IntoView {
    let position = move || {
        let ids = state.bat_book.selected_ids().get();
        if ids.len() > 1 {
            let shown = shown_id(state);
            let pos = shown
                .and_then(|id| ids.iter().position(|i| *i == id))
                .map(|i| i + 1)
                .unwrap_or(ids.len());
            return format!("{pos} of {} selected", ids.len());
        }
        let region = state.bat_book.region().get();
        let manifest = get_manifest(region);
        let pos = ids
            .first()
            .and_then(|id| manifest.entries.iter().position(|e| e.id == id.as_str()));
        match pos {
            Some(p) => format!(
                "{}: {} of {}",
                region.short_label(),
                p + 1,
                manifest.entries.len()
            ),
            // A species from another region's book has no place in this one.
            None => region.short_label().to_string(),
        }
    };
    view! {
        <div class="species-nav">
            <button class="species-step" title="Previous species" aria-label="Previous species"
                on:click=move |ev| { ev.stop_propagation(); step(state, -1); }>"\u{2039}"</button>
            <span class="species-pos">{position}</span>
            <button class="species-step" title="Next species" aria-label="Next species"
                on:click=move |ev| { ev.stop_propagation(); step(state, 1); }>"\u{203a}"</button>
            <button class="species-close" title="Close (Esc)" aria-label="Close"
                on:click=move |ev| { ev.stop_propagation(); close(); }>
                "\u{00d7}"
            </button>
        </div>
    }
}

/// Name and scientific name.
fn name_block(shown: Memo<Option<Shown>>) -> impl IntoView {
    move || {
        shown.get().map(|s| {
            let sci = s.entry.scientific_name;
            view! {
                <div class="species-names">
                    <div class="species-name">{s.entry.name}</div>
                    {(!sci.is_empty()).then(|| view! { <div class="species-sci"><i>{sci}</i></div> })}
                </div>
            }
        })
    }
}

/// Kinds of call: one button each, when the species has more than search
/// calls. The chosen one is drawn.
fn forms_row(
    state: AppState,
    shown: Memo<Option<Shown>>,
    form: Memo<Option<Form<'static>>>,
) -> impl IntoView {
    move || {
        let sketch = shown.get()?.sketch?;
        let forms = sketch.forms();
        if forms.len() < 2 {
            return None;
        }
        let current = form.get().map(|f| f.key());
        Some(view! {
            <div class="species-forms" role="radiogroup" aria-label="Kind of call">
                {forms.into_iter().map(|f| {
                    let key = f.key();
                    let selected = current.as_ref() == Some(&key);
                    let drawn = f.drawable();
                    view! {
                        <button
                            class="species-form-btn"
                            class:sel=selected
                            class:undrawn=!drawn
                            role="radio"
                            aria-checked=selected.to_string()
                            title=if drawn { "" } else { "Listed, not drawn" }
                            on:click=move |ev| {
                                ev.stop_propagation();
                                state.bat_book.call_form().set(key.clone());
                            }
                        >{f.name()}</button>
                    }
                }).collect_view()}
            </div>
        })
    }
}

/// Everything below the name and kinds of call: the shown kind's facts,
/// warnings, the description, and sources.
fn body(
    shown: Memo<Option<Shown>>,
    form: Memo<Option<Form<'static>>>,
    warning: Signal<Option<String>>,
) -> impl IntoView {
    let details_open = RwSignal::new(false);
    let facts = move || {
        let Some(s) = shown.get() else { return None };
        let Some(f) = form.get() else {
            let msg = if s.entry.echolocates {
                "No sketch of this species' calls yet."
            } else {
                "Doesn't echolocate."
            };
            return Some(view! { <div class="species-muted">{msg}</div> }.into_any());
        };
        let lines = form_facts(f);
        let reason = not_drawn_reason(f);
        // Names the drawing (and which kind of call it is), for a reader
        // who takes it for part of the recording.
        let caption = f.drawable().then(|| {
            if f.kind == FormKind::Search {
                "Typical search calls".to_string()
            } else {
                format!("Typical {}", f.name().to_lowercase())
            }
        });
        Some(
            view! {
                {caption.map(|c| view! { <div class="species-muted">{c}</div> })}
                <div class="species-facts">
                    {lines.into_iter().map(|l| view! { <div>{l}</div> }).collect_view()}
                    {reason.map(|r| view! { <div class="species-muted">{r}</div> })}
                </div>
            }
            .into_any(),
        )
    };
    let about = move || {
        let s = shown.get()?;
        let e = &s.entry;
        let kind = [e.family, e.call_type]
            .iter()
            .filter(|t| !t.is_empty())
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        Some(view! {
            <div class="species-desc">{e.description}</div>
            {(!kind.is_empty()).then(|| view! { <div class="species-muted">{kind}</div> })}
        })
    };
    let sources = move || {
        let sketch = shown.get()?.sketch?;
        let f = form.get()?;
        let list = f.sources(&sketch.sources);
        let notes = f.notes.or(if f.kind == FormKind::Search {
            sketch.notes.as_deref()
        } else {
            None
        });
        let n = list.len();
        let toggle = move |ev: web_sys::MouseEvent| {
            ev.stop_propagation();
            details_open.update(|v| *v = !*v);
        };
        Some(view! {
            <button class="species-details-btn" on:click=toggle aria-expanded=move || details_open.get().to_string()>
                {move || if details_open.get() { "\u{25be} " } else { "\u{25b8} " }}
                {format!("{n} source{} and notes", if n == 1 { "" } else { "s" })}
            </button>
            {move || details_open.get().then(|| view! {
                <div class="species-details">
                    {notes.map(|t| view! { <p>{t}</p> })}
                    <ol class="species-sources">
                        {list.iter().map(|src| {
                            let cite = src.citation.clone();
                            match &src.url {
                                Some(u) => view! {
                                    <li><a href=u.clone() target="_blank" rel="noopener">{cite}</a></li>
                                }.into_any(),
                                None => view! { <li>{cite}</li> }.into_any(),
                            }
                        }).collect_view()}
                    </ol>
                </div>
            })}
        })
    };
    view! {
        {facts}
        {move || warning.get().map(|w| view! { <div class="species-warn">{w}</div> })}
        {about}
        {sources}
        <div class="species-muted species-draft">"Draft. May contain errors."</div>
    }
}

/// Draws the calls of `lay` with a soft glow, harmonics fainter by their
/// level. `y_of` maps kHz to a y position, CSS px.
fn draw_calls(
    ctx: &web_sys::CanvasRenderingContext2d,
    lay: &StripLayout<'_>,
    y_of: impl Fn(f64) -> f64,
    line: [u8; 3],
) {
    ctx.set_line_cap("round");
    ctx.set_line_join("round");
    for placed in &lay.calls {
        if lay.too_small {
            // Too small to draw the shape: mark where each call is, so the
            // spacing of the train still shows.
            let (lo, hi) = (0..=8).fold((f64::MAX, f64::MIN), |(lo, hi), i| {
                let k = placed.khz_at(i as f64 / 8.0);
                (lo.min(k), hi.max(k))
            });
            ctx.set_stroke_style_str(&rgba(line, 0.9));
            ctx.set_line_width(1.5);
            ctx.begin_path();
            ctx.move_to(placed.x, y_of(hi));
            ctx.line_to(placed.x, y_of(lo));
            ctx.stroke();
            continue;
        }
        let n = ((placed.width * 2.0).ceil() as usize).clamp(12, 400);
        let mut layers: Vec<(f64, f64)> = vec![(1.0, 1.0)];
        for h in &placed.call.harmonics {
            layers.push((
                h.multiple,
                10f64.powf(h.relative_db / 20.0).clamp(0.05, 1.0),
            ));
        }
        for (mult, gain) in layers {
            ctx.set_shadow_color(&rgba(line, 0.6 * gain));
            ctx.set_shadow_blur(6.0);
            ctx.set_line_width(2.5);
            for i in 0..n {
                let t0 = i as f64 / n as f64;
                let t1 = (i + 1) as f64 / n as f64;
                // Softer at both ends of the call, as a call fades in and out.
                let mid = (t0 + t1) / 2.0;
                let env = (std::f64::consts::PI * mid).sin().powf(0.6).max(0.25);
                ctx.set_stroke_style_str(&rgba(line, env * gain));
                ctx.begin_path();
                ctx.move_to(placed.x + t0 * placed.width, y_of(placed.khz_at(t0) * mult));
                ctx.line_to(placed.x + t1 * placed.width, y_of(placed.khz_at(t1) * mult));
                ctx.stroke();
            }
        }
        ctx.set_shadow_blur(0.0);
    }
}

/// A canvas's 2D context, scaled for the device's pixel ratio, after sizing
/// the canvas to `w` by `h` CSS px.
fn sized_context(
    el: &web_sys::HtmlCanvasElement,
    w: f64,
    h: f64,
) -> Option<web_sys::CanvasRenderingContext2d> {
    let dpr = web_sys::window()
        .map(|w| w.device_pixel_ratio())
        .unwrap_or(1.0);
    el.set_width((w * dpr).round() as u32);
    el.set_height((h * dpr).round() as u32);
    let ctx = el
        .get_context("2d")
        .ok()
        .flatten()
        .and_then(|c| c.dyn_into::<web_sys::CanvasRenderingContext2d>().ok())?;
    let _ = ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0);
    Some(ctx)
}

/// Drawn calls at the spectrogram's left edge, with the species text in a
/// column over them when the spectrogram is big enough.
#[component]
pub fn SpeciesCalls(canvas_ref: NodeRef<leptos::html::Canvas>) -> impl IntoView {
    let state = expect_context::<AppState>();
    let draw_ref = NodeRef::<leptos::html::Canvas>::new();
    let scroll_ref = NodeRef::<leptos::html::Div>::new();

    let open = Memo::new(move |_| panel_open(state));
    let shown = Memo::new(move |_| {
        if !open.get() {
            return None;
        }
        let id = shown_id(state)?;
        let entry = entry_for(state, &id)?;
        Some(Shown {
            entry,
            sketch: call_sketch::for_species(&id),
        })
    });
    let form = Memo::new(move |_| shown.get()?.sketch.map(|s| chosen_form(s, state)));

    // The spectrogram's size, CSS px. Its height changes without any signal
    // (the bat book strip opening shrinks it), so watch it.
    let view_size = state.bat_book.spect_size();
    Effect::new(move |_| {
        let Some(el) = canvas_ref.get() else { return };
        let canvas: &web_sys::HtmlCanvasElement = el.as_ref();
        let read = {
            let canvas = canvas.clone();
            move || SpectSize {
                w: canvas.client_width() as f64,
                h: canvas.client_height() as f64,
            }
        };
        view_size.set(read());
        let cb = wasm_bindgen::closure::Closure::<dyn Fn(js_sys::Array)>::new(
            move |_e: js_sys::Array| {
                let now = read();
                // try_*: the observer may fire after this component is gone.
                if view_size.try_get_untracked().is_some_and(|old| old != now) {
                    view_size.try_set(now);
                }
            },
        );
        if let Ok(obs) = web_sys::ResizeObserver::new(cb.as_ref().unchecked_ref()) {
            obs.observe(canvas);
            on_cleanup(move || obs.disconnect());
        }
        cb.forget();
    });

    // None until the main canvas has a size.
    let scale = Memo::new(move |_| {
        let SpectSize {
            w: css_w, h: css_h, ..
        } = view_size.get();
        let zoom = state.view.zoom_level().get();
        let idx = state.library.current_index().get()?;
        let (time_res, file_max) = state.library.files().with(|files| {
            files
                .get(idx)
                .map(|f| (f.spectrogram.time_resolution, f.spectrogram.max_freq))
        })?;
        let max_f = state.view.max_display_freq().get().unwrap_or(file_max);
        let min_f = state.view.min_display_freq().get().unwrap_or(0.0);
        let _ = state.viewmode.spectrogram_canvas_width().get();
        let canvas = canvas_ref.get()?;
        let el: &web_sys::HtmlCanvasElement = canvas.as_ref();
        let backing_w = el.width() as f64;
        if css_w <= 0.0 || css_h <= 0.0 || backing_w <= 0.0 || time_res <= 0.0 {
            return None;
        }
        // zoom is backing px per spectrogram column.
        let px_per_s = zoom / time_res * (css_w / backing_w);
        Some(ViewScale {
            px_per_s,
            min_hz: min_f,
            max_hz: max_f,
            file_max_hz: file_max,
            css_w,
            css_h,
        })
    });

    let below = Memo::new(move |_| text_below(state));

    let layout = Memo::new(move |_| {
        let f = form.get()?;
        if !f.drawable() {
            return None;
        }
        let sc = scale.get()?;
        let cap = sc.css_w * MAX_WIDTH_FRACTION;
        let size = if below.get() {
            StripWidth {
                fit: (sc.css_w * NARROW_FIT_FRACTION).max(NARROW_MIN_WIDTH),
                min: NARROW_MIN_WIDTH,
                cap: cap.max(NARROW_MIN_WIDTH),
            }
        } else {
            StripWidth::up_to(cap.max(COLUMN_WIDTH))
        };
        Some(call_sketch::layout_in(f, sc.px_per_s, size))
    });

    // Width of the whole thing: the drawing alone, or the text column (which
    // grows to hold a wider drawing).
    let width = Memo::new(move |_| {
        if shown.get().is_none() || scale.get().is_none() {
            return None;
        }
        let draw_w = layout.get().map(|l| l.width);
        if below.get() {
            draw_w
        } else {
            Some(draw_w.unwrap_or(0.0).max(COLUMN_WIDTH))
        }
    });

    let out_of_view = Memo::new(move |_| {
        let f = form.get()?;
        let (lo, hi) = f.khz_extent()?;
        // A harmonic in view counts as the calls being in view.
        let (drawn_lo, drawn_hi) = drawn_khz_extent(f)?;
        layout.get()?;
        let sc = scale.get()?;
        let lo_hz = lo * 1000.0;
        if lo_hz > sc.file_max_hz {
            Some(OutOfView::AboveRecording {
                lo_khz: lo,
                hi_khz: hi,
                top_khz: sc.file_max_hz / 1000.0,
            })
        } else if drawn_lo * 1000.0 > sc.max_hz {
            Some(OutOfView::Above)
        } else if drawn_hi * 1000.0 < sc.min_hz {
            Some(OutOfView::Below)
        } else {
            None
        }
    });

    // One warning at a time: when the calls are outside the frequency range,
    // their width doesn't matter.
    let warning = Signal::derive(move || {
        if let Some(o) = out_of_view.get() {
            return Some(out_of_view_text(o));
        }
        let lay = layout.get()?;
        if lay.too_small {
            Some("Calls are under 2 pixels wide at this zoom. Zoom in on time to see them.".into())
        } else if lay.cut_off {
            let len = form.get().and_then(|f| f.length_s()).map(duration_text);
            Some(match len {
                Some(l) => {
                    format!("Only the start fits at this zoom; the whole sequence lasts {l}.")
                }
                None => "Only the start fits at this zoom.".into(),
            })
        } else {
            None
        }
    });
    state.bat_book.panel_warning().set(None);
    Effect::new(move |_| {
        let w = warning.get();
        if state.bat_book.panel_warning().get_untracked() != w {
            state.bat_book.panel_warning().set(w);
        }
    });

    // Left edge, CSS px. At the left of the view, unless the recording ends
    // inside the view with room after it: then it goes there, so it covers
    // none of the recording. Only this memo follows scrolling, and it only
    // moves the element.
    let left_px = Memo::new(move |_| {
        let w = width.get()?;
        let sc = scale.get()?;
        let idx = state.library.current_index().get()?;
        let duration = state
            .library
            .files()
            .with(|files| files.get(idx).map(|f| f.audio.duration_secs))?;
        let scroll = state.view.scroll_offset().get();
        let end_px = (duration - scroll) * sc.px_per_s;
        const GAP: f64 = 8.0;
        if end_px >= 0.0 && sc.css_w - end_px >= w + 2.0 * GAP {
            Some(end_px + GAP)
        } else {
            Some(0.0)
        }
    });

    // Tell canvas overlays (the "..." menus) how much of the spectrogram's
    // left edge is covered.
    let cover = state.bat_book.sketch_cover_px();
    Effect::new(move |_| {
        let px = match (width.get(), left_px.get()) {
            (Some(w), Some(left)) if left <= 0.0 => w,
            _ => 0.0,
        };
        if cover.get_untracked() != px {
            cover.set(px);
        }
    });
    on_cleanup(move || {
        cover.set(0.0);
        // No spectrogram, no measurement: the panel below closes with it.
        view_size.try_set(SpectSize::default());
    });

    // The clear window over the drawn calls and their harmonics: top and
    // height, CSS px.
    let window = Memo::new(move |_| {
        let sc = scale.get()?;
        let f = form.get()?;
        layout.get()?;
        let (lo, hi) = drawn_khz_extent(f)?;
        let y = |khz: f64| freq_to_y(khz * 1000.0, sc.min_hz, sc.max_hz, sc.css_h);
        let top = (y(hi) - WINDOW_MARGIN).clamp(0.0, sc.css_h);
        let bottom = (y(lo) + WINDOW_MARGIN).clamp(0.0, sc.css_h);
        Some((top, (bottom - top).max(0.0)))
    });

    // Back to the top (window over the calls) for each species and kind.
    Effect::new(move |_| {
        let _ = shown.get();
        let _ = form.get();
        if let Some(el) = scroll_ref.get() {
            el.set_scroll_top(0);
        }
    });

    Effect::new(move |_| {
        let (Some(w), Some(sc)) = (width.get(), scale.get()) else {
            return;
        };
        let lay = layout.get();
        let (min_f, max_f, css_h) = (sc.min_hz, sc.max_hz, sc.css_h);
        let cmap = sketch_colormap(state);
        let Some(canvas) = draw_ref.get() else {
            return;
        };
        let el: &web_sys::HtmlCanvasElement = canvas.as_ref();
        let Some(ctx) = sized_context(el, w, css_h) else {
            return;
        };
        ctx.set_fill_style_str(&rgba(cmap.apply(16), 1.0));
        ctx.fill_rect(0.0, 0.0, w, css_h);
        let Some(lay) = lay else { return };
        draw_calls(
            &ctx,
            &lay,
            |khz| freq_to_y(khz * 1000.0, min_f, max_f, css_h),
            cmap.apply(240),
        );
    });

    // The name block's height, measured: it grows with a long name that
    // wraps and with the kinds-of-call row. Where the block goes (above or
    // under the window) depends on it, and it doesn't depend on where the
    // block goes, so measuring can't loop.
    let top_ref = NodeRef::<leptos::html::Div>::new();
    let top_block_h = RwSignal::new(TOP_BLOCK_GUESS_PX);
    Effect::new(move |_| {
        let Some(el) = top_ref.get() else { return };
        let el: web_sys::HtmlElement = el.into();
        let read = {
            let el = el.clone();
            move || {
                let h = el.offset_height() as f64;
                // try_*: the observer may fire after this component is gone.
                if h > 0.0 && top_block_h.try_get_untracked().is_some_and(|old| old != h) {
                    top_block_h.try_set(h);
                }
            }
        };
        read();
        let cb = wasm_bindgen::closure::Closure::<dyn Fn(js_sys::Array)>::new(
            move |_e: js_sys::Array| read(),
        );
        if let Ok(obs) = web_sys::ResizeObserver::new(cb.as_ref().unchecked_ref()) {
            obs.observe(&el);
            on_cleanup(move || obs.disconnect());
        }
        cb.forget();
    });

    // The column's scrolled content: the name block sits at the top of the
    // column when it fits above the window, else just below the window; the
    // rest follows.
    let column = move || {
        let (top, h) = window.get().unwrap_or((0.0, 0.0));
        let top_h = top_block_h.get();
        let name_first = top_h <= top;
        // Under the window, the name block must still be in view: when the
        // calls reach the bottom of the spectrogram, the window ends early.
        let h = if name_first {
            h
        } else {
            let view_h = scale.get().map_or(0.0, |sc| sc.css_h);
            h.min((view_h - top - top_h).max(0.0))
        };
        let spacer = if name_first { top - top_h } else { top };
        let spacer_div =
            view! { <div class="species-spacer" style:height=format!("{spacer}px")></div> };
        let top_block = view! {
            <div class="species-top species-text" node_ref=top_ref>
                {nav_row(state, move || state.bat_book.ref_open().set(false))}
                {name_block(shown)}
                {forms_row(state, shown, form)}
            </div>
        };
        let window_div = view! { <div class="species-window" style:height=format!("{h}px")></div> };
        view! {
            {if name_first {
                view! { {top_block} {spacer_div} {window_div} }.into_any()
            } else {
                view! { {spacer_div} {window_div} {top_block} }.into_any()
            }}
            <div class="species-bottom species-text">
                {body(shown, form, warning)}
            </div>
        }
    };

    move || {
        let w = width.get()?;
        let is_below = below.get();
        Some(view! {
            <div
                class="species-calls"
                class=("species-calls-floating", move || left_px.get().unwrap_or(0.0) > 0.0)
                class=("species-calls-drawing-only", is_below)
                style:width=format!("{w}px")
                style:left=move || format!("{}px", left_px.get().unwrap_or(0.0))
            >
                <canvas node_ref=draw_ref class="species-calls-canvas"/>
                {(!is_below).then(|| view! {
                    <div class="species-scroll" node_ref=scroll_ref>{column}</div>
                })}
            </div>
        })
    }
}

/// The calls of `form` stretched in time to fill `w` CSS px, and the scale
/// that takes, CSS px per second. A train whose calls would be under
/// `CARD_MIN_CALL_PX` wide shows fewer calls instead.
fn card_layout(form: Form<'static>, w: f64) -> Option<(StripLayout<'static>, f64)> {
    let unbounded = StripWidth {
        fit: f64::INFINITY,
        min: 0.0,
        cap: f64::INFINITY,
    };
    // At 1000 px per second, 1 px is 1 ms.
    let probe = call_sketch::layout_in(form, 1000.0, unbounded);
    let last = probe.calls.last()?;
    let span_s = (last.x + last.width - STRIP_PAD) / 1000.0;
    if span_s <= 0.0 || w <= 2.0 * STRIP_PAD {
        return None;
    }
    let narrowest_s = probe
        .calls
        .iter()
        .map(|c| c.width)
        .fold(f64::INFINITY, f64::min)
        / 1000.0;
    let mut px_per_s = (w - 2.0 * STRIP_PAD) / span_s;
    if narrowest_s > 0.0 && narrowest_s * px_per_s < CARD_MIN_CALL_PX {
        px_per_s = CARD_MIN_CALL_PX / narrowest_s;
    }
    // Half a pixel spare, so rounding doesn't drop the last call.
    let size = StripWidth {
        fit: w + 0.5,
        min: 0.0,
        cap: w,
    };
    Some((call_sketch::layout_in(form, px_per_s, size), px_per_s))
}

/// Gap between kHz gridlines: the smallest round step that gives at most
/// `max_lines` lines over `range_khz`.
fn khz_step(range_khz: f64, max_lines: f64) -> f64 {
    [1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0]
        .into_iter()
        .find(|s| range_khz / s <= max_lines)
        .unwrap_or(200.0)
}

/// Length of the time scale bar, ms: the longest round length up to a third
/// of `span_ms`.
fn scale_bar_ms(span_ms: f64) -> f64 {
    [
        1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0,
    ]
    .into_iter()
    .rev()
    .find(|ms| *ms <= span_ms / 3.0)
    .unwrap_or(1.0)
}

/// Draws `form` large in the species card: frequency gridlines with kHz
/// labels on the left, the calls stretched in time to fill the width, and a
/// time scale bar, since the time scale isn't the recording's.
fn draw_card_sketch(el: &web_sys::HtmlCanvasElement, form: Form<'static>, w: f64, cmap: Colormap) {
    let h = CARD_SKETCH_H;
    let Some(ctx) = sized_context(el, w, h) else {
        return;
    };
    let plot_w = w - CARD_AXIS_W;
    let Some((lay, px_per_s)) = card_layout(form, plot_w) else {
        return;
    };
    let Some((lo, hi)) = drawn_khz_extent(form) else {
        return;
    };
    let pad = ((hi - lo) * 0.15).max(3.0);
    let (lo, hi) = ((lo - pad).max(0.0), hi + pad);
    let (top, bottom) = (8.0, h - CARD_SCALE_H);
    let y_of = move |khz: f64| bottom - (khz - lo) / (hi - lo) * (bottom - top);

    ctx.set_fill_style_str(&rgba(cmap.apply(16), 1.0));
    ctx.fill_rect(CARD_AXIS_W, 0.0, plot_w, h);

    // Frequency gridlines and their labels.
    ctx.set_font("11px sans-serif");
    ctx.set_text_align("right");
    ctx.set_text_baseline("middle");
    ctx.set_line_width(1.0);
    let step = khz_step(hi - lo, 4.0);
    let mut k = (lo / step).ceil() * step;
    while k <= hi {
        let y = y_of(k).round() + 0.5;
        ctx.set_stroke_style_str("rgba(255,255,255,0.14)");
        ctx.begin_path();
        ctx.move_to(CARD_AXIS_W, y);
        ctx.line_to(w, y);
        ctx.stroke();
        ctx.set_fill_style_str("#9a9a9a");
        let _ = ctx.fill_text(&format!("{} kHz", fmt(k)), CARD_AXIS_W - 6.0, y);
        k += step;
    }

    let _ = ctx.translate(CARD_AXIS_W, 0.0);
    draw_calls(&ctx, &lay, y_of, cmap.apply(240));

    // Time scale bar at the bottom right, with its length.
    let span_ms = (plot_w - 2.0 * STRIP_PAD) / px_per_s * 1000.0;
    let bar_ms = scale_bar_ms(span_ms);
    let bar_px = bar_ms / 1000.0 * px_per_s;
    let x1 = plot_w - STRIP_PAD;
    let y = h - CARD_SCALE_H / 2.0;
    ctx.set_stroke_style_str("#bbb");
    ctx.set_line_width(2.0);
    ctx.begin_path();
    ctx.move_to(x1 - bar_px, y);
    ctx.line_to(x1, y);
    ctx.stroke();
    ctx.set_fill_style_str("#bbb");
    let _ = ctx.fill_text(&duration_text(bar_ms / 1000.0), x1 - bar_px - 6.0, y);
}

/// The species card: a sheet with the shown species' facts, description,
/// sources and its calls drawn large. Opened from the bat book's More info
/// button when the spectrogram has no room for the text column (phones).
#[component]
pub fn SpeciesInfoCard() -> impl IntoView {
    let state = expect_context::<AppState>();
    let open = state.bat_book.info_open();
    let shown = Memo::new(move |_| {
        if !state.bat_book.ref_open().get() {
            return None;
        }
        let id = shown_id(state)?;
        let entry = entry_for(state, &id)?;
        Some(Shown {
            entry,
            sketch: call_sketch::for_species(&id),
        })
    });
    let form = Memo::new(move |_| shown.get()?.sketch.map(|s| chosen_form(s, state)));
    let warning = Signal::derive(move || state.bat_book.panel_warning().get());
    let has_species = Memo::new(move |_| shown.get().is_some());
    let drawable = Memo::new(move |_| form.get().is_some_and(|f| f.drawable()));

    // Close when there's nothing to show, or the text column has room again.
    Effect::new(move |_| {
        if open.get() && !info_button_shown(state) {
            open.set(false);
        }
    });

    let close = move || open.set(false);

    // The drawing fills the card's width; follow it.
    let sketch_ref = NodeRef::<leptos::html::Canvas>::new();
    let sketch_w = RwSignal::new(0.0);
    Effect::new(move |_| {
        let Some(el) = sketch_ref.get() else { return };
        let canvas: web_sys::HtmlCanvasElement = el.into();
        let read = {
            let canvas = canvas.clone();
            move || {
                let w = canvas.client_width() as f64;
                // try_*: the observer may fire after this component is gone.
                if sketch_w.try_get_untracked().is_some_and(|old| old != w) {
                    sketch_w.try_set(w);
                }
            }
        };
        read();
        let cb = wasm_bindgen::closure::Closure::<dyn Fn(js_sys::Array)>::new(
            move |_e: js_sys::Array| read(),
        );
        if let Ok(obs) = web_sys::ResizeObserver::new(cb.as_ref().unchecked_ref()) {
            obs.observe(&canvas);
            on_cleanup(move || obs.disconnect());
        }
        cb.forget();
    });
    Effect::new(move |_| {
        let w = sketch_w.get();
        let (Some(f), Some(el)) = (form.get(), sketch_ref.get()) else {
            return;
        };
        if w > 0.0 && f.drawable() {
            draw_card_sketch(el.as_ref(), f, w, sketch_colormap(state));
        }
    });

    view! {
        <div
            class=move || if open.get() { "species-card-backdrop open" } else { "species-card-backdrop" }
            on:click=move |_| close()
        ></div>
        <div
            class=move || if open.get() { "species-card species-text open" } else { "species-card species-text" }
            role="dialog"
            aria-label="Species"
            inert=move || !open.get()
        >
            {move || has_species.get().then(|| view! {
                <div class="species-card-head">
                    {nav_row(state, close)}
                    {name_block(shown)}
                    {forms_row(state, shown, form)}
                </div>
                {move || drawable.get().then(|| view! {
                    <canvas class="species-card-sketch" node_ref=sketch_ref></canvas>
                })}
                {body(shown, form, warning)}
            })}
        </div>
    }
}
