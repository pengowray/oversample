// "Fine output adjustment": a section of the HET, TE, PS and ZC settings
// panels (mode_button.rs). It draws the recording's band on the left, the
// band heard on the right, and a ribbon joining them, and lets the heard
// band be dragged by its low, middle and high handles.
//
// It replaced the Hear bar's OUT button and its "Output range" popup
// (2026-11-20, the owner's request: no divide-factor buttons, labels on the
// handles, Snap "Free" by default).
//
// The mappings:
//   TE, ZC   heard = recording ÷ factor
//   PS       heard = recording ÷ factor − shift   (`ps_shift_hz`)
//   HET      heard = |recording − carrier|, low-passed at the LP cutoff. With
//            several carriers every carrier's band lands on 0 to the cutoff.
//
// A drag does not write the playback settings on every pointer move. The
// diagram follows the pointer from a draft value; the settings are written
// when the pointer rests for `COMMIT_IDLE_MS` and when it is released. Each
// write redraws every XForm tile, so writing on every move would blank the
// spectrogram for the whole drag.

use crate::audio::streaming_playback::{het_carriers_for, selection_freq_band};
use crate::components::hfr_switch::nyquist_for_current;
use crate::components::icons::Icon;
use crate::components::mode_button::format_factor_value;
use crate::dsp::pitch_shift::PitchFactor;
use crate::state::store_fields::*;
use crate::state::{AppState, OutputSnap, PlaybackMode};
use leptos::prelude::*;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::spawn_local;

// ── Diagram geometry, in viewBox units ──────────────────────────────────────

const VB_W: f64 = 330.0;
const VB_H: f64 = 232.0;
const TOP: f64 = 18.0;
const BOT: f64 = 208.0;
const IN_X: f64 = 50.0;
const IN_W: f64 = 24.0;
const OUT_X: f64 = 198.0;
const OUT_W: f64 = 56.0;

/// The pointer has to rest this long during a drag before the settings are
/// written (and the spectrogram and playback follow).
const COMMIT_IDLE_MS: i32 = 140;
/// How long XForm stays on after a drag that turned it on.
const PREVIEW_HOLD_MS: i32 = 1500;

/// Tops the heard axis can have, in Hz. The smallest that leaves room above
/// the heard band is used.
const HEARD_TOPS: [f64; 6] = [10_000.0, 20_000.0, 50_000.0, 100_000.0, 200_000.0, 500_000.0];

const STANDARD_FACTORS: [f64; 8] = [2.0, 4.0, 8.0, 10.0, 16.0, 32.0, 64.0, 128.0];
/// Powers of 2: each halving is one octave, so musical intervals are kept.
const CHROMA_FACTORS: [f64; 7] = [2.0, 4.0, 8.0, 16.0, 32.0, 64.0, 128.0];
const HET_SNAP_STEP_HZ: f64 = 5_000.0;
/// Colours of the HET carriers' bands, lowest carrier first.
const HET_COLOURS: [&str; 5] = ["#4cc9f0", "#80ed99", "#ffd166", "#f79d65", "#c77dff"];

fn y_of(hz: f64, top_hz: f64) -> f64 {
    BOT - (hz / top_hz.max(1.0)).clamp(0.0, 1.0) * (BOT - TOP)
}

fn heard_top_for(heard_hi: f64) -> f64 {
    HEARD_TOPS
        .into_iter()
        .find(|top| heard_hi <= top * 0.9)
        .unwrap_or(HEARD_TOPS[HEARD_TOPS.len() - 1])
}

/// "5.0 kHz", or "800 Hz" under 1 kHz.
fn freq_label(hz: f64) -> String {
    if hz < 995.0 {
        format!("{:.0}\u{00A0}Hz", (hz / 10.0).round() * 10.0)
    } else {
        format!("{:.1}\u{00A0}kHz", hz / 1000.0)
    }
}

/// A whole number of kHz without decimals, others with one: "40", "37.5".
fn khz_number(hz: f64) -> String {
    let khz = hz / 1000.0;
    if (khz - khz.round()).abs() < 0.05 {
        format!("{:.0}", khz)
    } else {
        format!("{khz:.1}")
    }
}

/// The ribbon from a band of the recording column to a band of the heard
/// column: an S-curve along each edge.
fn ribbon_path(in_lo_y: f64, in_hi_y: f64, out_lo_y: f64, out_hi_y: f64) -> String {
    let (x0, x1) = (IN_X + IN_W, OUT_X);
    let m = (x0 + x1) / 2.0;
    format!(
        "M{x0},{in_hi_y:.1} C{m},{in_hi_y:.1} {m},{out_hi_y:.1} {x1},{out_hi_y:.1} \
         L{x1},{out_lo_y:.1} C{m},{out_lo_y:.1} {m},{in_lo_y:.1} {x0},{in_lo_y:.1} Z"
    )
}

// ── The divide mapping (TE, PS, ZC) ─────────────────────────────────────────

/// heard = recording × `scale` − `shift`. `shift` is 0 outside PS.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Mapping {
    scale: f64,
    shift: f64,
}

impl Mapping {
    fn heard(self, hz: f64) -> f64 {
        (hz * self.scale - self.shift).max(0.0)
    }
}

/// Which handle of the heard band is dragged.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Anchor {
    Lo,
    Mid,
    Hi,
}

/// The PS output shift playback applies: the stored one, limited so the low
/// edge of the band is not shifted below 0 Hz. Playback, the heard axis and
/// this section all use it. With no band (`band_lo` 0) the stored value is
/// used as it is.
pub(crate) fn effective_ps_shift(stored: f64, band_lo: f64, factor: f64) -> f64 {
    let s = stored.max(0.0);
    let f = factor.abs().max(1.0);
    if band_lo <= 0.0 {
        return s;
    }
    s.min(band_lo / f)
}

/// A divide factor moved to one the mode can use: TE and PS take 1.5 to 64
/// (or the Snap setting's nearest factor), ZC a whole number from 2 to 32.
fn usable_factor(raw: f64, mode: PlaybackMode, snap: OutputSnap) -> f64 {
    if mode == PlaybackMode::ZeroCrossing {
        return raw.round().clamp(2.0, 32.0);
    }
    let nearest = |list: &[f64]| {
        list.iter()
            .copied()
            .min_by(|a, b| (a - raw).abs().total_cmp(&(b - raw).abs()))
            .unwrap_or(8.0)
    };
    match snap {
        OutputSnap::Free => ((raw * 10.0).round() / 10.0).clamp(1.5, 64.0),
        OutputSnap::Standard => nearest(&STANDARD_FACTORS),
        OutputSnap::EqualChroma => nearest(&CHROMA_FACTORS),
    }
}

/// The mapping that puts the dragged handle of the heard band at `target` Hz.
///
/// `start` is the mapping when the drag began. With `allow_shift` (PS,
/// "Divide and shift") the low and high handles move one end of the heard
/// band and keep the other where it was, which takes a factor and a shift
/// together. The shift can only lower the output, so the heard band cannot be
/// made narrower than plain division gives: past that point the shift is 0
/// and the other end follows. The middle handle moves the whole band and
/// keeps the shift at the same share of its largest value.
fn solve_drag(
    anchor: Anchor,
    target: f64,
    band: (f64, f64),
    start: Mapping,
    allow_shift: bool,
    usable: impl Fn(f64) -> f64,
) -> Mapping {
    let (lo, hi) = band;
    let target = target.max(1.0);
    let shifting = allow_shift && lo > 0.0 && hi > lo;
    let max_shift_share = if shifting && start.scale > 0.0 {
        (start.shift / (lo * start.scale)).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let plain = |anchor_hz: f64| target / anchor_hz.max(1.0);
    let mid = (lo + hi) / 2.0;

    // The scale asked for, and the heard frequency of the end that stays put.
    let (raw_scale, pinned): (f64, Option<(f64, f64)>) = match anchor {
        Anchor::Mid => (plain(mid - max_shift_share * lo), None),
        Anchor::Hi if shifting => {
            let heard_lo = start.heard(lo);
            let scale = (target - heard_lo) / (hi - lo);
            if scale > 0.0 && lo * scale >= heard_lo {
                (scale, Some((lo, heard_lo)))
            } else {
                (plain(hi), None)
            }
        }
        Anchor::Lo if shifting => {
            let heard_hi = start.heard(hi);
            let scale = (heard_hi - target) / (hi - lo);
            if scale > 0.0 && lo * scale >= target {
                (scale, Some((hi, heard_hi)))
            } else {
                (plain(lo), None)
            }
        }
        Anchor::Hi => (plain(hi), None),
        Anchor::Lo => (plain(if lo > 0.0 { lo } else { hi }), None),
    };

    let scale = 1.0 / usable(1.0 / raw_scale.max(1e-9));
    let largest_shift = if shifting { lo * scale } else { 0.0 };
    let shift = match (anchor, pinned) {
        (Anchor::Mid, _) => max_shift_share * largest_shift,
        (_, Some((pinned_hz, pinned_heard))) => pinned_hz * scale - pinned_heard,
        (_, None) => 0.0,
    };
    Mapping {
        scale,
        shift: ((shift.clamp(0.0, largest_shift)) / 10.0).round() * 10.0,
    }
}

/// The mapping the playback settings give for `mode`.
fn read_mapping(state: &AppState, mode: PlaybackMode, band_lo: f64, tracked: bool) -> Mapping {
    macro_rules! read {
        ($signal:expr) => {
            if tracked {
                $signal.get()
            } else {
                $signal.get_untracked()
            }
        };
    }
    let from_signed = |f: f64| {
        if f > 1.0 {
            1.0 / f
        } else if f < -1.0 {
            f.abs()
        } else {
            1.0
        }
    };
    match mode {
        PlaybackMode::TimeExpansion => Mapping {
            scale: from_signed(read!(state.transform.te_factor())),
            shift: 0.0,
        },
        PlaybackMode::ZeroCrossing => Mapping {
            scale: 1.0 / read!(state.transform.zc_factor()).max(1.0),
            shift: 0.0,
        },
        _ => {
            let factor = read!(state.transform.ps_factor()).signed();
            let stored = read!(state.transform.ps_shift_hz());
            Mapping {
                scale: from_signed(factor),
                shift: effective_ps_shift(stored, band_lo, factor),
            }
        }
    }
}

/// Write `m` to the playback settings of `mode`, turning the factor's Auto
/// off. Shows the old picture until the new one is drawn.
fn write_mapping(state: &AppState, mode: PlaybackMode, m: Mapping) {
    let factor = 1.0 / m.scale.max(1e-9);
    freeze_spectrogram(state);
    match mode {
        PlaybackMode::TimeExpansion => {
            state.transform.te_factor_auto().set(false);
            state.transform.te_factor().set(factor);
        }
        PlaybackMode::ZeroCrossing => state.transform.zc_factor().set(factor),
        PlaybackMode::PitchShift => {
            state.transform.ps_factor_auto().set(false);
            state.transform.ps_factor().set(PitchFactor::from_signed(factor));
            state.transform.ps_shift_hz().set(m.shift);
        }
        _ => {}
    }
}

// ── XForm while dragging, and the picture kept until the new one is drawn ──

thread_local! {
    /// (this section turned XForm on, number of the latest drag).
    static PREVIEW: Cell<(bool, u32)> = const { Cell::new((false, 0)) };
    /// Number of the latest freeze. An older freeze's watcher stops.
    static FREEZE: Cell<u32> = const { Cell::new(0) };
}

/// A drag began: show the XForm view, where the spectrogram is of the audio
/// heard, if it is not on already.
fn begin_preview(state: &AppState) {
    let (mut ours, n) = PREVIEW.get();
    if !state.display.xform_enabled().get_untracked()
        && state.viewmode.main_view().get_untracked().is_spectrogram()
    {
        crate::components::app::enable_xform(state);
        ours = true;
    }
    PREVIEW.set((ours, n.wrapping_add(1)));
}

/// A drag ended: turn XForm off again after `PREVIEW_HOLD_MS`, if this
/// section turned it on and no other drag has begun since.
fn end_preview(state: &AppState) {
    let state = *state;
    let (_, n) = PREVIEW.get();
    spawn_local(async move {
        crate::web_util::sleep_ms(PREVIEW_HOLD_MS).await;
        let (ours, now) = PREVIEW.get();
        if ours && now == n {
            PREVIEW.set((false, now));
            crate::components::app::disable_xform(&state);
        }
    });
}

/// Keep the spectrogram's current picture on screen while its tiles are
/// redrawn for a new setting: a copy of the canvas is laid over it and
/// removed once the new tiles have stopped arriving. Without it the view goes
/// blank between a change and its redraw. Does nothing unless XForm shows the
/// playback mode, which is the only time a setting here redraws the tiles.
///
/// The copy is a DOM overlay (see "DOM overlay pattern" in CLAUDE.md): no
/// canvas Effect subscribes to anything for it.
fn freeze_spectrogram(state: &AppState) {
    const CLASS: &str = "fine-out-freeze";
    if !state.display.transform().get_untracked() {
        return;
    }
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let n = FREEZE.get().wrapping_add(1);
    FREEZE.set(n);
    // A copy from an earlier change that is still up is the last complete
    // picture, so it stays.
    if doc.query_selector(&format!(".{CLASS}")).ok().flatten().is_none() {
        let Some(src) = doc
            .query_selector(".spectrogram-container > canvas")
            .ok()
            .flatten()
            .and_then(|el| el.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        else {
            return;
        };
        let Ok(copy) = doc
            .create_element("canvas")
            .map(|el| el.unchecked_into::<web_sys::HtmlCanvasElement>())
        else {
            return;
        };
        copy.set_width(src.width());
        copy.set_height(src.height());
        copy.set_class_name(CLASS);
        let _ = copy.set_attribute(
            "style",
            &format!(
                "position:absolute;left:{}px;top:{}px;width:{}px;height:{}px;pointer-events:none;",
                src.offset_left(),
                src.offset_top(),
                src.client_width(),
                src.client_height()
            ),
        );
        if let Some(ctx) = copy
            .get_context("2d")
            .ok()
            .flatten()
            .and_then(|c| c.dyn_into::<web_sys::CanvasRenderingContext2d>().ok())
        {
            let _ = ctx.draw_image_with_html_canvas_element(&src, 0.0, 0.0);
        }
        let _ = src.insert_adjacent_element("afterend", &copy);
    }
    let state = *state;
    spawn_local(async move {
        // Removed when at least two tile updates have come (the clear is the
        // first) and none for 250 ms, or after 3 s.
        let mut last = state.viewmode.tile_ready_signal().get_untracked();
        let (mut updates, mut quiet_ms, mut total_ms) = (0u32, 0, 0);
        loop {
            crate::web_util::sleep_ms(50).await;
            if FREEZE.get() != n {
                return;
            }
            total_ms += 50;
            let now = state.viewmode.tile_ready_signal().get_untracked();
            if now != last {
                last = now;
                updates += 1;
                quiet_ms = 0;
            } else {
                quiet_ms += 50;
            }
            if (updates >= 2 && quiet_ms >= 250) || total_ms >= 3000 {
                break;
            }
        }
        if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
            if let Ok(Some(el)) = doc.query_selector(&format!(".{CLASS}")) {
                el.remove();
            }
        }
    });
}

/// Follow the pointer until it is released: `on_move` gets each clientY,
/// `on_end` runs once at release or cancel.
fn drag_on_window(on_move: impl Fn(f64) + 'static, on_end: impl Fn() + 'static) {
    type Listener = Rc<RefCell<Option<Closure<dyn FnMut(web_sys::PointerEvent)>>>>;
    let Some(win) = web_sys::window() else {
        return;
    };
    let move_slot: Listener = Rc::new(RefCell::new(None));
    let end_slot: Listener = Rc::new(RefCell::new(None));
    let move_cb = Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |e: web_sys::PointerEvent| {
        on_move(e.client_y() as f64);
    });
    let (win2, move_slot2, end_slot2) = (win.clone(), move_slot.clone(), end_slot.clone());
    let end_cb = Closure::<dyn FnMut(web_sys::PointerEvent)>::new(move |_: web_sys::PointerEvent| {
        if let Some(m) = move_slot2.borrow_mut().take() {
            let _ = win2.remove_event_listener_with_callback("pointermove", m.as_ref().unchecked_ref());
        }
        if let Some(u) = end_slot2.borrow_mut().take() {
            for name in ["pointerup", "pointercancel"] {
                let _ = win2.remove_event_listener_with_callback(name, u.as_ref().unchecked_ref());
            }
            on_end();
        }
    });
    let _ = win.add_event_listener_with_callback("pointermove", move_cb.as_ref().unchecked_ref());
    for name in ["pointerup", "pointercancel"] {
        let _ = win.add_event_listener_with_callback(name, end_cb.as_ref().unchecked_ref());
    }
    *move_slot.borrow_mut() = Some(move_cb);
    *end_slot.borrow_mut() = Some(end_cb);
}

/// Hz of the heard axis at a clientY, for a diagram whose wrapper is `el`.
fn heard_hz_at(el: &web_sys::HtmlElement, top_hz: f64) -> impl Fn(f64) -> f64 + Copy {
    let rect = el.get_bounding_client_rect();
    let per_px = VB_H / rect.height().max(1.0);
    let top_px = rect.top();
    move |client_y: f64| {
        let y = (client_y - top_px) * per_px;
        ((BOT - y) / (BOT - TOP)).clamp(0.0, 1.0) * top_hz
    }
}

// ── Shared pieces of the two diagrams ───────────────────────────────────────

/// Column headings, the two columns' frames and their tick labels.
fn axes_view(nyquist: Signal<f64>, top: Signal<f64>) -> impl IntoView {
    let in_ticks = move || {
        let nyq = nyquist.get();
        let step = [5_000.0, 10_000.0, 25_000.0, 50_000.0, 100_000.0]
            .into_iter()
            .find(|s| nyq / s <= 4.5)
            .unwrap_or(100_000.0);
        let mut ticks = vec![];
        let mut hz = 0.0;
        while hz < nyq * 0.9 {
            ticks.push(hz);
            hz += step;
        }
        ticks.push(nyq);
        ticks
            .into_iter()
            .map(|hz| {
                let y = y_of(hz, nyq);
                view! {
                    <line x1=IN_X - 4.0 x2=IN_X y1=y y2=y stroke="#555"/>
                    <text class="fine-out-tick" x=IN_X - 7.0 y=y + 3.0 text-anchor="end">{khz_number(hz)}</text>
                }
            })
            .collect::<Vec<_>>()
    };
    let out_ticks = move || {
        let top = top.get();
        let parts = if (top - 20_000.0).abs() < 1.0 { 4 } else { 5 };
        (0..=parts)
            .map(|i| {
                let hz = top * i as f64 / parts as f64;
                let y = y_of(hz, top);
                view! {
                    <line x1=OUT_X x2=OUT_X + OUT_W y1=y y2=y stroke="#2c2c2c"/>
                    <line x1=OUT_X + OUT_W x2=OUT_X + OUT_W + 4.0 y1=y y2=y stroke="#555"/>
                    <text class="fine-out-tick" x=OUT_X + OUT_W + 8.0 y=y + 3.0>{khz_number(hz)}</text>
                }
            })
            .collect::<Vec<_>>()
    };
    view! {
        <text class="fine-out-col" x=IN_X + IN_W / 2.0 y="10" text-anchor="middle">"RECORDING"</text>
        <text class="fine-out-col" x=OUT_X + OUT_W / 2.0 y="10" text-anchor="middle">"HEARD"</text>
        <rect x=IN_X y=TOP width=IN_W height=BOT - TOP fill="#1f1f1f" stroke="#333"/>
        <rect x=OUT_X y=TOP width=OUT_W height=BOT - TOP fill="#1f1f1f" stroke="#333"/>
        {in_ticks}
        {out_ticks}
        <text class="fine-out-tick" x=IN_X - 7.0 y=VB_H - 6.0 text-anchor="end">"kHz"</text>
        <text class="fine-out-tick" x=OUT_X + OUT_W + 8.0 y=VB_H - 6.0>"kHz"</text>
    }
}

/// The Snap setting. `options` are (value, label) pairs.
fn snap_select(state: AppState, options: &'static [(OutputSnap, &'static str)]) -> impl IntoView {
    let on_change = move |ev: web_sys::Event| {
        let value = event_target_value(&ev);
        if let Some((snap, _)) = options.iter().find(|(_, label)| *label == value) {
            state.viewmode.output_snap().set(*snap);
        }
    };
    view! {
        <label class="fine-out-snap">
            "Snap"
            <select on:change=on_change
                prop:value=move || {
                    let snap = state.viewmode.output_snap().get();
                    options.iter().find(|(s, _)| *s == snap).unwrap_or(&options[0]).1
                }
            >
                {options.iter().map(|(_, label)| view! { <option value=*label>{*label}</option> }).collect::<Vec<_>>()}
            </select>
        </label>
    }
}

/// The section: a heading that opens and closes it, and the diagram of the
/// mode that is playing.
#[component]
pub fn FineOutputSection(mode: PlaybackMode) -> impl IntoView {
    let state = expect_context::<AppState>();
    let open = move || state.viewmode.fine_output_open().get();
    view! {
        <div class="fine-out">
            <button class="fine-out-head"
                class:open=open
                aria-expanded=move || open().to_string()
                on:click=move |_| state.viewmode.fine_output_open().update(|v| *v = !*v)
            >
                <Icon kind=Icon::ChevronRight />
                "Fine output adjustment"
            </button>
            <Show when=open>
                {move || if mode == PlaybackMode::Heterodyne {
                    view! { <HetDiagram/> }.into_any()
                } else {
                    view! { <DivideDiagram mode/> }.into_any()
                }}
            </Show>
        </div>
    }
}

// ── TE, PS, ZC ──────────────────────────────────────────────────────────────

#[component]
fn DivideDiagram(mode: PlaybackMode) -> impl IntoView {
    let state = expect_context::<AppState>();
    let wrap = NodeRef::<leptos::html::Div>::new();
    let is_ps = mode == PlaybackMode::PitchShift;

    // The mapping under the pointer during a drag, before it is written.
    let draft = RwSignal::new(None::<Mapping>);
    let dragging = RwSignal::new(false);
    let allow_shift =
        RwSignal::new(is_ps && state.transform.ps_shift_hz().get_untracked() >= 1.0);

    let nyquist = Memo::new(move |_| {
        let _ = state.library.current_index().get();
        state.library.files().track();
        nyquist_for_current(state).max(1_000.0)
    });
    // The HFR band, or the whole recording when no band is set.
    let band = Memo::new(move |_| {
        let (lo, hi) = (
            state.filter.band_ff_freq_lo().get(),
            state.filter.band_ff_freq_hi().get(),
        );
        if hi > lo {
            (lo, hi)
        } else {
            (0.0, nyquist.get())
        }
    });
    let mapping = Memo::new(move |_| {
        let set = read_mapping(&state, mode, band.get().0, true);
        draft.get().unwrap_or(set)
    });
    let heard = Memo::new(move |_| {
        let (lo, hi) = band.get();
        let m = mapping.get();
        (m.heard(lo), m.heard(hi))
    });
    // The heard axis keeps its top during a drag, so the handle stays under
    // the pointer.
    let top = Memo::new(move |prev: Option<&f64>| match prev {
        Some(prev) if dragging.get() => *prev,
        _ => heard_top_for(heard.get().1),
    });

    let commit = move |m: Mapping| {
        let set = read_mapping(&state, mode, band.get_untracked().0, false);
        if m != set {
            write_mapping(&state, mode, m);
        }
    };

    let start_drag = move |ev: web_sys::PointerEvent, anchor: Anchor| {
        ev.prevent_default();
        ev.stop_propagation();
        let Some(el) = wrap.get_untracked() else {
            return;
        };
        let top_hz = top.get_untracked();
        let hz_at = heard_hz_at(&el, top_hz);
        let band = band.get_untracked();
        let start = mapping.get_untracked();
        let (heard_lo, heard_hi) = (start.heard(band.0), start.heard(band.1));
        let handle_hz = match anchor {
            Anchor::Lo => heard_lo,
            Anchor::Hi => heard_hi,
            Anchor::Mid => (heard_lo + heard_hi) / 2.0,
        };
        // The handle keeps its distance from the pointer.
        let grab = handle_hz - hz_at(ev.client_y() as f64);
        let allow = is_ps && allow_shift.get_untracked();
        let snap = state.viewmode.output_snap().get_untracked();
        dragging.set(true);
        begin_preview(&state);

        let moves = Rc::new(Cell::new(0u32));
        let moves_end = moves.clone();
        let on_move = move |client_y: f64| {
            let target = (hz_at(client_y) + grab).clamp(0.0, top_hz);
            let m = solve_drag(anchor, target, band, start, allow, |f| {
                usable_factor(f, mode, snap)
            });
            if draft.try_set(Some(m)).is_some() {
                return; // the panel closed
            }
            let n = moves.get().wrapping_add(1);
            moves.set(n);
            let moves = moves.clone();
            spawn_local(async move {
                crate::web_util::sleep_ms(COMMIT_IDLE_MS).await;
                if moves.get() == n {
                    commit(m);
                }
            });
        };
        let on_end = move || {
            moves_end.set(moves_end.get().wrapping_add(1));
            if let Some(Some(m)) = draft.try_get_untracked() {
                commit(m);
            }
            draft.try_set(None);
            dragging.try_set(false);
            end_preview(&state);
        };
        drag_on_window(on_move, on_end);
    };

    let band_y = move || {
        let (lo, hi) = band.get();
        let nyq = nyquist.get();
        (y_of(lo, nyq), y_of(hi, nyq))
    };
    let heard_y = move || {
        let (lo, hi) = heard.get();
        let top = top.get();
        (y_of(lo, top), y_of(hi, top))
    };
    let factor_text = move || {
        let scale = mapping.get().scale;
        if scale > 1.0 {
            format!("\u{00d7}{}", format_factor_value(scale))
        } else {
            format!("\u{00f7}{}", format_factor_value(1.0 / scale.max(1e-9)))
        }
    };
    let shift_text = move || {
        let shift = mapping.get().shift;
        (shift >= 1.0).then(|| format!("\u{2212}{}", freq_label(shift)))
    };

    let on_shift_typed = move |ev: web_sys::Event| {
        let input: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(khz) = input.value().parse::<f64>() {
            freeze_spectrogram(&state);
            state.transform.ps_shift_hz().set((khz * 1000.0).max(0.0));
        }
        // Show the shift in use, which the band's low edge can limit.
        input.set_value(&format!("{:.2}", mapping.get_untracked().shift / 1000.0));
    };

    const FACTOR_SNAPS: &[(OutputSnap, &str)] = &[
        (OutputSnap::Free, "Free"),
        (OutputSnap::Standard, "Standard factors"),
        (OutputSnap::EqualChroma, "Powers of 2"),
    ];

    view! {
        <div class="fine-out-body">
            {(mode != PlaybackMode::ZeroCrossing).then(|| view! {
                <div class="fine-out-row">
                    {is_ps.then(|| view! {
                        <div class="fine-out-seg">
                            <button class:sel=move || !allow_shift.get()
                                title="Every frequency is divided by the factor"
                                on:click=move |_| {
                                    allow_shift.set(false);
                                    if state.transform.ps_shift_hz().get_untracked() != 0.0 {
                                        freeze_spectrogram(&state);
                                        state.transform.ps_shift_hz().set(0.0);
                                    }
                                }
                            >"Divide only"</button>
                            <button class:sel=move || allow_shift.get()
                                title="Divide by a smaller factor, then lower the result by a fixed number of kHz. The low and high handles then move separately."
                                on:click=move |_| allow_shift.set(true)
                            >"Divide and shift"</button>
                        </div>
                    })}
                    <span class="fine-out-gap"></span>
                    {snap_select(state, FACTOR_SNAPS)}
                </div>
            })}
            <div class="fine-out-diagram" node_ref=wrap>
                <svg viewBox=format!("0 0 {VB_W} {VB_H}") class="fine-out-svg">
                    {axes_view(nyquist.into(), top.into())}
                    <rect x=IN_X width=IN_W fill="#e0a030" opacity="0.85"
                        y=move || band_y().1
                        height=move || (band_y().0 - band_y().1).max(1.0)
                    />
                    <path class="fine-out-ribbon"
                        d=move || {
                            let (in_lo, in_hi) = band_y();
                            let (out_lo, out_hi) = heard_y();
                            ribbon_path(in_lo, in_hi, out_lo, out_hi)
                        }
                    />
                    <text class="fine-out-in" x=IN_X + IN_W + 4.0 y=move || (band_y().1 - 3.0).max(TOP + 9.0)>
                        {move || format!("{}\u{00A0}kHz", khz_number(band.get().1))}
                    </text>
                    <text class="fine-out-in" x=IN_X + IN_W + 4.0 y=move || (band_y().0 + 11.0).min(VB_H - 2.0)>
                        {move || format!("{}\u{00A0}kHz", khz_number(band.get().0))}
                    </text>
                    // The heard band: the middle handle.
                    <rect class="fine-out-band" x=OUT_X width=OUT_W
                        y=move || heard_y().1
                        height=move || (heard_y().0 - heard_y().1).max(1.0)
                        on:pointerdown=move |ev| start_drag(ev, Anchor::Mid)
                    >
                        <title>"Drag to move the whole heard band"</title>
                    </rect>
                    <g class="fine-out-grip"
                        style:display={move || if heard_y().0 - heard_y().1 < 26.0 { "none" } else { "" }}
                        transform=move || format!("translate(0,{:.1})", (heard_y().0 + heard_y().1) / 2.0)
                    >
                        <line x1=OUT_X + 20.0 x2=OUT_X + 36.0 y1="-3" y2="-3"/>
                        <line x1=OUT_X + 20.0 x2=OUT_X + 36.0 y1="0" y2="0"/>
                        <line x1=OUT_X + 20.0 x2=OUT_X + 36.0 y1="3" y2="3"/>
                    </g>
                    // High handle and its label above it.
                    <g class="fine-out-handle"
                        transform=move || format!("translate(0,{:.1})", heard_y().1)
                        on:pointerdown=move |ev| start_drag(ev, Anchor::Hi)
                    >
                        <title>"Drag to set the highest frequency heard"</title>
                        <rect class="fine-out-hit" x=OUT_X y="-18" width=OUT_W height="24"/>
                        <rect class="fine-out-knob" x=OUT_X + 14.0 y="-4" width="28" height="8" rx="3"/>
                    </g>
                    <text class="fine-out-value" x=OUT_X + OUT_W / 2.0 text-anchor="middle"
                        y=move || { let y = heard_y().1; if y < 20.0 { y + 17.0 } else { y - 8.0 } }
                    >{move || freq_label(heard.get().1)}</text>
                    // Low handle and its label below it. With no band the low
                    // edge is 0 Hz of the recording, which every factor
                    // leaves at 0.
                    <g class="fine-out-handle"
                        style:display={move || if band.get().0 <= 0.0 { "none" } else { "" }}
                        transform=move || format!("translate(0,{:.1})", heard_y().0)
                        on:pointerdown=move |ev| start_drag(ev, Anchor::Lo)
                    >
                        <title>"Drag to set the lowest frequency heard"</title>
                        <rect class="fine-out-hit" x=OUT_X y="-6" width=OUT_W height="24"/>
                        <rect class="fine-out-knob" x=OUT_X + 14.0 y="-4" width="28" height="8" rx="3"/>
                    </g>
                    <text class="fine-out-value" x=OUT_X + OUT_W / 2.0 text-anchor="middle"
                        y=move || heard_y().0 + 16.0
                    >{move || freq_label(heard.get().0)}</text>
                    // The factor, beside the heard band.
                    <text class="fine-out-factor" x=OUT_X + OUT_W + 30.0
                        y=move || (heard_y().0 + heard_y().1) / 2.0 + 4.0
                    >{factor_text}</text>
                    <text class="fine-out-shift" x=OUT_X + OUT_W + 30.0
                        y=move || (heard_y().0 + heard_y().1) / 2.0 + 17.0
                    >{shift_text}</text>
                </svg>
            </div>
            {is_ps.then(|| view! {
                <Show when=move || allow_shift.get()>
                    <div class="fine-out-row">
                        <span>{move || format!("Divide by {}, then lower by", format_factor_value(1.0 / mapping.get().scale.max(1e-9)))}</span>
                        <input type="number" class="factor-input fine-out-num" min="0" step="0.1"
                            aria-label="Lower the output by, in kHz"
                            prop:value=move || format!("{:.2}", mapping.get().shift / 1000.0)
                            on:change=on_shift_typed
                        />
                        <span>"kHz"</span>
                    </div>
                </Show>
            })}
        </div>
    }
}

// ── HET ─────────────────────────────────────────────────────────────────────

/// A typed HET carrier limited to 0..`nyquist`, the frequencies the audio can
/// hold. A carrier above Nyquist aliases. NaN gives 0.
fn clamp_carrier_hz(hz: f64, nyquist: f64) -> f64 {
    // `f64::max` and `f64::min` return the other value for NaN.
    hz.max(0.0).min(nyquist.max(0.0))
}

fn snap_carrier(hz: f64, snap: OutputSnap) -> f64 {
    let step = match snap {
        OutputSnap::Free => 100.0,
        // Carriers have no musical steps, so both snaps use 5 kHz.
        OutputSnap::Standard | OutputSnap::EqualChroma => HET_SNAP_STEP_HZ,
    };
    (hz.max(0.0) / step).round() * step
}

/// (carrier setting, LP cutoff), both in Hz.
type HetSetting = (f64, f64);

#[component]
fn HetDiagram() -> impl IntoView {
    let state = expect_context::<AppState>();
    let wrap = NodeRef::<leptos::html::Div>::new();
    let draft = RwSignal::new(None::<HetSetting>);
    let dragging = RwSignal::new(false);

    let nyquist = Memo::new(move |_| {
        let _ = state.library.current_index().get();
        state.library.files().track();
        nyquist_for_current(state).max(1_000.0)
    });
    let setting = Memo::new(move |_| {
        let set = (
            state.transform.het_frequency().get(),
            state.transform.het_cutoff().get(),
        );
        draft.get().unwrap_or(set)
    });
    // A selection with a frequency band retunes HET to that band, in
    // playback and in the XForm picture.
    let selection_band = Memo::new(move |_| {
        let _ = state.interaction.selection().get();
        let _ = state.interaction.active_focus().get();
        let nyq = nyquist.get();
        crate::audio::playback::effective_selection(&state).and_then(|sel| {
            selection_freq_band(
                sel.freq_low.unwrap_or(0.0),
                sel.freq_high.unwrap_or(nyq),
                true,
                nyq,
            )
        })
    });
    let carriers = Memo::new(move |_| {
        het_carriers_for(
            setting.get().0,
            state.transform.het_comb_count().get(),
            state.transform.het_comb_spacing().get(),
            selection_band.get(),
        )
    });
    let top = Memo::new(move |prev: Option<&f64>| match prev {
        Some(prev) if dragging.get() => *prev,
        _ => heard_top_for(setting.get().1),
    });

    let commit = move |(carrier, cutoff): HetSetting| {
        let set_carrier = state.transform.het_frequency().get_untracked();
        let set_cutoff = state.transform.het_cutoff().get_untracked();
        if carrier == set_carrier && cutoff == set_cutoff {
            return;
        }
        freeze_spectrogram(&state);
        if carrier != set_carrier {
            state.transform.het_freq_auto().set(false);
            state.transform.het_frequency().set(carrier);
        }
        if cutoff != set_cutoff {
            state.transform.het_cutoff_auto().set(false);
            state.transform.het_cutoff().set(cutoff);
        }
    };

    // `to_setting` turns a clientY into the setting the drag asks for.
    let run_drag = move |to_setting: Rc<dyn Fn(f64) -> HetSetting>| {
        dragging.set(true);
        begin_preview(&state);
        let moves = Rc::new(Cell::new(0u32));
        let moves_end = moves.clone();
        let on_move = move |client_y: f64| {
            let s = to_setting(client_y);
            if draft.try_set(Some(s)).is_some() {
                return; // the panel closed
            }
            let n = moves.get().wrapping_add(1);
            moves.set(n);
            let moves = moves.clone();
            spawn_local(async move {
                crate::web_util::sleep_ms(COMMIT_IDLE_MS).await;
                if moves.get() == n {
                    commit(s);
                }
            });
        };
        let on_end = move || {
            moves_end.set(moves_end.get().wrapping_add(1));
            if let Some(Some(s)) = draft.try_get_untracked() {
                commit(s);
            }
            draft.try_set(None);
            dragging.try_set(false);
            end_preview(&state);
        };
        drag_on_window(on_move, on_end);
    };

    // The heard column's handle sets the LP cutoff, within the same 1 to
    // 30 kHz as the band edges on the spectrogram.
    let start_cutoff_drag = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        ev.stop_propagation();
        let Some(el) = wrap.get_untracked() else {
            return;
        };
        let hz_at = heard_hz_at(&el, top.get_untracked());
        let (carrier, cutoff) = setting.get_untracked();
        let grab = cutoff - hz_at(ev.client_y() as f64);
        run_drag(Rc::new(move |client_y| {
            let hz = ((hz_at(client_y) + grab) / 100.0).round() * 100.0;
            (carrier, hz.clamp(1_000.0, 30_000.0))
        }));
    };
    // Dragging the recording column moves the carriers.
    let start_carrier_drag = move |ev: web_sys::PointerEvent| {
        ev.prevent_default();
        ev.stop_propagation();
        if selection_band.get_untracked().is_some() {
            return;
        }
        let Some(el) = wrap.get_untracked() else {
            return;
        };
        let nyq = nyquist.get_untracked();
        let hz_at = heard_hz_at(&el, nyq);
        let (carrier, cutoff) = setting.get_untracked();
        let grab = carrier - hz_at(ev.client_y() as f64);
        let snap = state.viewmode.output_snap().get_untracked();
        run_drag(Rc::new(move |client_y| {
            let hz = snap_carrier(hz_at(client_y) + grab, snap);
            (clamp_carrier_hz(hz, nyq), cutoff)
        }));
    };

    let bands = move || {
        let nyq = nyquist.get();
        let cutoff = setting.get().1;
        let top = top.get();
        let (out_lo, out_hi) = (y_of(0.0, top), y_of(cutoff, top));
        carriers
            .get()
            .into_iter()
            .enumerate()
            .map(|(i, carrier)| {
                let colour = HET_COLOURS[i % HET_COLOURS.len()];
                let in_lo = y_of((carrier - cutoff).max(0.0), nyq);
                let in_hi = y_of((carrier + cutoff).min(nyq), nyq);
                let yc = y_of(carrier, nyq);
                let (x0, x1) = (IN_X + IN_W, OUT_X);
                let m = (x0 + x1) / 2.0;
                // The line from each carrier to 0 Hz: a call at the carrier
                // itself is heard at 0 Hz, and one at either edge of the
                // band at the cutoff.
                let centre = format!("M{x0},{yc:.1} C{m},{yc:.1} {m},{out_lo:.1} {x1},{out_lo:.1}");
                view! {
                    <rect x=IN_X width=IN_W y=in_hi height=(in_lo - in_hi).max(1.0) fill=colour opacity="0.8"/>
                    <path d=ribbon_path(in_lo, in_hi, out_lo, out_hi) fill=colour opacity="0.22"/>
                    <path d=centre fill="none" stroke=colour stroke-width="1.2"/>
                    <text class="fine-out-in" x=IN_X + IN_W + 4.0 y=yc + 3.0>{khz_number(carrier)}</text>
                }
            })
            .collect::<Vec<_>>()
    };
    let cutoff_y = move || y_of(setting.get().1, top.get());

    let on_carrier_input = move |ev: web_sys::Event| {
        let el: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        if let Ok(khz) = el.value().parse::<f64>() {
            let nyquist = nyquist_for_current(state);
            commit((
                clamp_carrier_hz(khz * 1000.0, nyquist),
                state.transform.het_cutoff().get_untracked(),
            ));
        }
    };
    // On Enter or when the field loses focus, write the carrier in use back
    // into the field. `prop:value` does not update when the limited carrier
    // equals the previous one, so a typed 44.1 kHz would otherwise stay on
    // screen for a 44.1 kHz file.
    let on_carrier_change = move |ev: web_sys::Event| {
        let el: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        el.set_value(&format!(
            "{:.1}",
            state.transform.het_frequency().get_untracked() / 1000.0
        ));
    };
    let several = move || carriers.get().len() > 1;

    const CARRIER_SNAPS: &[(OutputSnap, &str)] = &[
        (OutputSnap::Free, "Free"),
        (OutputSnap::Standard, "5 kHz steps"),
        (OutputSnap::EqualChroma, "5 kHz steps"),
    ];

    view! {
        <div class="fine-out-body">
            <div class="fine-out-diagram" node_ref=wrap>
                <svg viewBox=format!("0 0 {VB_W} {VB_H}") class="fine-out-svg">
                    {axes_view(nyquist.into(), top.into())}
                    {bands}
                    <rect class="fine-out-carriers" x=IN_X y=TOP width=IN_W height=BOT - TOP
                        class:locked=move || selection_band.get().is_some()
                        on:pointerdown=start_carrier_drag
                    >
                        <title>{move || if selection_band.get().is_some() {
                            "The carrier follows the selection's frequency band"
                        } else {
                            "Drag to move the carrier"
                        }}</title>
                    </rect>
                    <rect class="fine-out-band still" x=OUT_X width=OUT_W
                        y=cutoff_y
                        height=move || (BOT - cutoff_y()).max(1.0)
                    />
                    <g class="fine-out-handle"
                        transform=move || format!("translate(0,{:.1})", cutoff_y())
                        on:pointerdown=start_cutoff_drag
                    >
                        <title>"Drag to set the LP cutoff: the highest frequency heard"</title>
                        <rect class="fine-out-hit" x=OUT_X y="-18" width=OUT_W height="24"/>
                        <rect class="fine-out-knob" x=OUT_X + 14.0 y="-4" width="28" height="8" rx="3"/>
                    </g>
                    <text class="fine-out-value" x=OUT_X + OUT_W / 2.0 text-anchor="middle"
                        y=move || cutoff_y() - 8.0
                    >{move || freq_label(setting.get().1)}</text>
                </svg>
            </div>
            <div class="fine-out-row">
                <span>{move || if several() { "Lowest carrier" } else { "Carrier" }}</span>
                <input type="number" class="factor-input fine-out-num" min="0" max="500" step="0.5"
                    aria-label="Carrier frequency in kHz"
                    prop:value=move || format!("{:.1}", setting.get().0 / 1000.0)
                    on:input=on_carrier_input
                    on:change=on_carrier_change
                />
                <span>"kHz"</span>
                <button
                    class=move || if state.transform.het_freq_auto().get() { "auto-toggle on" } else { "auto-toggle" }
                    on:click=move |_| state.transform.het_freq_auto().update(|v| *v = !*v)
                    title="Auto: set the carrier from the HFR band"
                >"A"</button>
                <span class="fine-out-gap"></span>
                {move || several().then(|| view! {
                    <span class="fine-out-dim">
                        {format!("{}\u{00A0}kHz apart", khz_number(state.transform.het_comb_spacing().get()))}
                    </span>
                })}
            </div>
            <div class="fine-out-row">
                <span class="fine-out-gap"></span>
                {snap_select(state, CARRIER_SNAPS)}
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free(f: f64) -> f64 {
        usable_factor(f, PlaybackMode::PitchShift, OutputSnap::Free)
    }
    const BAND: (f64, f64) = (40_000.0, 60_000.0);

    #[test]
    fn typed_carrier_stays_inside_the_file() {
        let nyq = 22_050.0;
        assert_eq!(clamp_carrier_hz(19_000.0, nyq), 19_000.0);
        assert_eq!(clamp_carrier_hz(44_100.0, nyq), nyq);
        assert_eq!(clamp_carrier_hz(1e9, nyq), nyq);
        assert_eq!(clamp_carrier_hz(f64::INFINITY, nyq), nyq);
        assert_eq!(clamp_carrier_hz(-5_000.0, nyq), 0.0);
        assert_eq!(clamp_carrier_hz(f64::NAN, nyq), 0.0);
        assert_eq!(clamp_carrier_hz(80_000.0, 96_000.0), 80_000.0);
    }

    #[test]
    fn divide_only_moves_both_ends() {
        let start = Mapping { scale: 0.1, shift: 0.0 };
        let m = solve_drag(Anchor::Hi, 12_000.0, BAND, start, false, free);
        assert!((m.scale - 0.2).abs() < 1e-9);
        assert_eq!(m.shift, 0.0);
        assert!((m.heard(BAND.0) - 8_000.0).abs() < 1.0);
    }

    #[test]
    fn low_handle_keeps_the_high_end_in_divide_and_shift() {
        // ÷10 gives 4 to 6 kHz. Dragging the low end to 1 kHz with the high
        // end kept at 6 kHz takes ÷4 and a 9 kHz shift.
        let start = Mapping { scale: 0.1, shift: 0.0 };
        let m = solve_drag(Anchor::Lo, 1_000.0, BAND, start, true, free);
        assert!((1.0 / m.scale - 4.0).abs() < 1e-9);
        assert!((m.shift - 9_000.0).abs() < 1.0);
        assert!((m.heard(BAND.0) - 1_000.0).abs() < 1.0);
        assert!((m.heard(BAND.1) - 6_000.0).abs() < 1.0);
    }

    #[test]
    fn high_handle_keeps_the_low_end_in_divide_and_shift() {
        let start = Mapping { scale: 0.2, shift: 7_000.0 }; // 1 to 5 kHz
        let m = solve_drag(Anchor::Hi, 9_000.0, BAND, start, true, free);
        assert!((m.heard(BAND.0) - 1_000.0).abs() < 1.0);
        assert!((m.heard(BAND.1) - 9_000.0).abs() < 1.0);
    }

    #[test]
    fn a_band_narrower_than_division_gives_is_plain_division() {
        // 4 to 6 kHz at ÷10. Raising the low end to 5 kHz would need the
        // output shifted up, which playback cannot do.
        let start = Mapping { scale: 0.1, shift: 0.0 };
        let m = solve_drag(Anchor::Lo, 5_000.0, BAND, start, true, free);
        assert_eq!(m.shift, 0.0);
        assert!((1.0 / m.scale - 8.0).abs() < 1e-9);
    }

    #[test]
    fn the_shift_never_lowers_the_band_below_zero() {
        let start = Mapping { scale: 0.2, shift: 7_000.0 };
        for target in [0.0, 50.0, 500.0, 3_000.0, 9_500.0] {
            for anchor in [Anchor::Lo, Anchor::Mid, Anchor::Hi] {
                let m = solve_drag(anchor, target, BAND, start, true, free);
                assert!(m.shift >= 0.0 && m.shift <= BAND.0 * m.scale + 10.0, "{anchor:?} {target}");
            }
        }
    }

    #[test]
    fn zc_takes_whole_factors() {
        let zc = |f| usable_factor(f, PlaybackMode::ZeroCrossing, OutputSnap::Free);
        assert_eq!(zc(9.6), 10.0);
        assert_eq!(zc(1.0), 2.0);
        assert_eq!(zc(90.0), 32.0);
    }

    #[test]
    fn heard_axis_top_leaves_room_above_the_band() {
        assert_eq!(heard_top_for(5_000.0), 10_000.0);
        assert_eq!(heard_top_for(9_600.0), 20_000.0);
        assert_eq!(heard_top_for(15_000.0), 20_000.0);
        assert_eq!(heard_top_for(1e9), 500_000.0);
    }
}
