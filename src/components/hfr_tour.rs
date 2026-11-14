// HFR tutorial and the "playing without HFR" card.
//
// Bat calls sit above human hearing, so a bat recording played 1:1 sounds
// nearly silent. Two ways in:
//   • The tutorial walks through turning HFR on, dragging the frequency
//     gutter (left of the spectrogram) to pick the band, dragging the time
//     gutter (under it) to pick the stretch to play, then Play. Each step
//     highlights the real control and moves on by itself once the user has
//     done it there. The overlay and popovers are driver.js, through
//     js/hfr-tour.js.
//   • The card shows above Play when an ultrasonic recording starts playing
//     1:1 with HFR off, at most once per session. It offers HFR or the
//     tutorial; "Don't show again" clears a setting in Settings > Bats.
//
// Entry points for the tutorial: the card, the Band presets menu beside HFR,
// and Settings > Bats.

use crate::components::popup::{Align, PopupPanel, Side};
use crate::state::store_fields::*;
use crate::state::{AppState, MainView, PlaybackMode};
use js_sys::{Array, Object, Reflect};
use leptos::prelude::*;
use std::cell::RefCell;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};

// ── Strings ─────────────────────────────────────────────────────────────

/// Label of the buttons and menu items that start the tutorial.
pub const TOUR_NAME: &str = "HFR tutorial";
/// Tooltips on the tutorial buttons while the tutorial can't run.
const TOUR_NEEDS_FILE: &str = "Open a recording first";
const TOUR_STOP_LISTENING: &str = "Stop listening first";
const TOUR_STOP_RECORDING: &str = "Stop recording first";

const T_HFR: &str = "Turn on HFR";
const D_HFR: &str = "HFR lowers the pitch of bat calls into the range you can hear.";
const T_BAND: &str = "Choose frequencies";
const D_BAND: &str = "Bat calls show as bright marks in the spectrogram. \
    Drag up or down the frequency strip to select their frequencies.";
const T_TIME: &str = "Choose a time range";
const D_TIME: &str = "Drag along the time strip to choose the part of the recording to play.";
const T_PLAY: &str = "Press Play";
const D_PLAY: &str = "The frequency band you chose plays at a lower pitch.";

const L_NEXT: &str = "Next";
const L_DONE: &str = "Done";
const L_CLOSE: &str = "Close tutorial";
const L_PROGRESS: &str = "{{current}} of {{total}}";

const NUDGE_TEXT_LIVE: &str = "Listening 1:1. Sound above 20 kHz is too high to hear.";
const NUDGE_TEXT: &str = "Playing 1:1. Sound above 20 kHz is too high to hear.";
const NUDGE_HFR: &str = "Turn on HFR";
const NUDGE_NEVER: &str = "Don't show again";
const NUDGE_NEVER_TIP: &str = "Turn back on in Settings > Bats";
const NUDGE_CLOSE: &str = "Close";

/// Settings > Bats checkbox that allows the card.
pub const NUDGE_SETTING: &str = "Suggest HFR when playing or listening\u{a0}1:1";

// ── Steps ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TourStep {
    /// The HFR button in the Hear bar.
    Hfr,
    /// The frequency gutter beside the spectrogram.
    BandGutter,
    /// The time gutter under the spectrogram.
    TimeGutter,
    Play,
}

impl TourStep {
    /// (CSS selector, title, description, popover side, popover align).
    /// js/hfr-tour.js highlights the first match that is on screen.
    fn spec(
        self,
    ) -> (
        &'static str,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    ) {
        match self {
            TourStep::Hfr => (
                ".hearing-bar .band-cell-hfr",
                T_HFR,
                D_HFR,
                "bottom",
                "start",
            ),
            TourStep::BandGutter => (".band-gutter", T_BAND, D_BAND, "right", "start"),
            TourStep::TimeGutter => (".time-gutter", T_TIME, D_TIME, "top", "center"),
            TourStep::Play => (".play-main-btn", T_PLAY, D_PLAY, "top", "start"),
        }
    }
}

/// The steps for this screen. The HFR steps are left out when HFR is on.
fn plan(state: AppState) -> Vec<TourStep> {
    let mut steps = Vec::new();
    if !state.viewmode.hfr_enabled().get_untracked() {
        steps.push(TourStep::Hfr);
    }
    steps.extend([TourStep::BandGutter, TourStep::TimeGutter, TourStep::Play]);
    steps
}

/// JS callbacks handed to js/hfr-tour.js. Kept alive until the next
/// tutorial starts: driver.js calls `on_end` while it tears down, so the
/// closures must outlive the tutorial they belong to.
struct TourCallbacks {
    _on_step: Closure<dyn Fn(u32)>,
    _on_end: Closure<dyn Fn()>,
}

thread_local! {
    static CALLBACKS: RefCell<Option<TourCallbacks>> = const { RefCell::new(None) };
}

fn tour_api() -> Option<JsValue> {
    Reflect::get(&js_sys::global(), &JsValue::from_str("oversampleTour"))
        .ok()
        .filter(|v| v.is_object())
}

fn call_api(name: &str) {
    let Some(api) = tour_api() else { return };
    if let Ok(f) = Reflect::get(&api, &JsValue::from_str(name)) {
        if let Some(f) = f.dyn_ref::<js_sys::Function>() {
            let _ = f.call0(&api);
        }
    }
}

fn set(obj: &Object, key: &str, val: &JsValue) {
    let _ = Reflect::set(obj, &JsValue::from_str(key), val);
}

/// Why the tutorial can't run now, or None when it can. It needs an open
/// recording, and its last step (press Play) has no meaning while the mic
/// is live.
pub fn tour_unavailable_reason(state: AppState) -> Option<&'static str> {
    if state.mic.recording().get() {
        Some(TOUR_STOP_RECORDING)
    } else if state.mic.listening().get() {
        Some(TOUR_STOP_LISTENING)
    } else if state.library.current_index().get().is_none()
        || state.timeline.active().get().is_some()
    {
        Some(TOUR_NEEDS_FILE)
    } else {
        None
    }
}

pub fn tour_available(state: AppState) -> bool {
    tour_unavailable_reason(state).is_none()
}

/// Start (or restart) the tutorial.
pub fn start_tour(state: AppState) {
    if !tour_available(state) {
        return;
    }
    state.dialogs.hfr_nudge().set(false);
    state.panels.layer_panel_open().set(None);
    // The last step is pressing Play, which would stop a play in progress.
    if state.playback.is_playing().get_untracked() {
        crate::audio::playback::stop(&state);
    }
    // The chromagram has no frequency gutter.
    if state.viewmode.main_view().get_untracked() == MainView::Chromagram {
        state.viewmode.main_view().set(MainView::Spectrogram);
    }
    // End a running tutorial while its callbacks still exist.
    call_api("stop");

    let steps = plan(state);
    let js_steps = Array::new();
    for step in &steps {
        let (selector, title, description, side, align) = step.spec();
        let o = Object::new();
        set(&o, "selector", &selector.into());
        set(&o, "title", &title.into());
        set(&o, "description", &description.into());
        set(&o, "side", &side.into());
        set(&o, "align", &align.into());
        js_steps.push(&o);
    }
    let labels = Object::new();
    set(&labels, "next", &L_NEXT.into());
    set(&labels, "done", &L_DONE.into());
    set(&labels, "close", &L_CLOSE.into());
    set(&labels, "progress", &L_PROGRESS.into());

    let steps_for_cb = steps.clone();
    let on_step = Closure::<dyn Fn(u32)>::new(move |i: u32| {
        let Some(&step) = steps_for_cb.get(i as usize) else {
            return;
        };
        enter_step(state, step);
    });
    let on_end = Closure::<dyn Fn()>::new(move || {
        state.dialogs.hfr_tour_step().try_set(None);
    });

    let Some(api) = tour_api() else {
        state.show_error_toast("Couldn't start the tutorial: hfr-tour.js is missing");
        return;
    };
    let Ok(start) = Reflect::get(&api, &JsValue::from_str("start")) else {
        return;
    };
    let Some(start) = start.dyn_ref::<js_sys::Function>() else {
        return;
    };
    let args = Array::of4(
        &js_steps,
        &labels,
        on_step.as_ref().unchecked_ref(),
        on_end.as_ref().unchecked_ref(),
    );
    CALLBACKS.with(|c| {
        *c.borrow_mut() = Some(TourCallbacks {
            _on_step: on_step,
            _on_end: on_end,
        })
    });
    match start.apply(&api, &args) {
        Ok(promise) => {
            if let Ok(promise) = promise.dyn_into::<js_sys::Promise>() {
                wasm_bindgen_futures::spawn_local(async move {
                    if wasm_bindgen_futures::JsFuture::from(promise).await.is_err() {
                        state.show_error_toast("Couldn't load the tutorial");
                    }
                });
            }
        }
        Err(_) => state.show_error_toast("Couldn't start the tutorial"),
    }
}

/// Runs as a step comes on screen: record it, and close any menu and the
/// phone Hear sheet so the highlighted control is visible.
fn enter_step(state: AppState, step: TourStep) {
    state.dialogs.hfr_tour_step().try_set(Some(step));
    state.panels.layer_panel_open().try_set(None);
    if state.panels.hear_sheet_open().try_get_untracked() == Some(true) {
        state.panels.hear_sheet_open().try_set(false);
    }
}

/// Move the tutorial on after the user did `step`. Waits a moment so the
/// result (HFR lit, band drawn) shows before the popover moves, and does
/// nothing if the user has already moved on with Next.
fn advance_from(state: AppState, step: TourStep) {
    wasm_bindgen_futures::spawn_local(async move {
        crate::web_util::sleep_ms(250).await;
        if state.dialogs.hfr_tour_step().try_get_untracked() == Some(Some(step)) {
            call_api("next");
        }
    });
}

/// Mounted once in `App`: moves the tutorial on when the user does each step.
#[component]
pub fn HfrTour() -> impl IntoView {
    let state = expect_context::<AppState>();

    // HFR turned on.
    Effect::new(move || {
        let step = state.dialogs.hfr_tour_step().get();
        let on = state.viewmode.hfr_enabled().get();
        if on && step == Some(TourStep::Hfr) {
            advance_from(state, TourStep::Hfr);
        }
    });

    // A drag on a gutter finished. Both gutters set `is_dragging` for the
    // length of a drag; a tap never sets it.
    Effect::new(move |was_dragging: Option<bool>| {
        let dragging = state.interaction.is_dragging().get();
        if was_dragging == Some(true) && !dragging {
            match state.dialogs.hfr_tour_step().get_untracked() {
                Some(TourStep::BandGutter) => {
                    let lo = state.filter.band_ff_freq_lo().get_untracked();
                    let hi = state.filter.band_ff_freq_hi().get_untracked();
                    if state.viewmode.hfr_enabled().get_untracked() && hi > lo {
                        advance_from(state, TourStep::BandGutter);
                    }
                }
                Some(TourStep::TimeGutter) => {
                    let has_range = state
                        .interaction
                        .selection()
                        .get_untracked()
                        .is_some_and(|s| s.time_end > s.time_start);
                    if has_range {
                        advance_from(state, TourStep::TimeGutter);
                    }
                }
                _ => {}
            }
        }
        dragging
    });

    // Playback started on the last step.
    Effect::new(move || {
        let playing = state.playback.is_playing().get();
        if playing && state.dialogs.hfr_tour_step().get_untracked() == Some(TourStep::Play) {
            advance_from(state, TourStep::Play);
        }
    });

    // Closing the recording ends the tutorial.
    Effect::new(move || {
        if !tour_available(state) && state.dialogs.hfr_tour_step().get_untracked().is_some() {
            call_api("stop");
        }
    });
}

// ── The card ────────────────────────────────────────────────────────────

/// True when the recording now playing can hold sound above hearing range
/// and is being played unchanged, so the user hears little or nothing of it.
fn playing_ultrasound_at_1_1(state: AppState) -> bool {
    if state.viewmode.hfr_enabled().get_untracked()
        || state.playback.mode().get_untracked() != PlaybackMode::Normal
        // The app turned HFR off itself for a 1:1 play of a selection.
        || state.playback.paused_hfr_for_normal().get_untracked()
        || state.timeline.active().get_untracked().is_some()
        || state.mic.listening().get_untracked()
        || state.mic.recording().get_untracked()
    {
        return false;
    }
    let rate = state.library.files().with_untracked(|files| {
        state
            .library
            .current_index()
            .get_untracked()
            .and_then(|i| files.get(i))
            .map(|f| f.audio.sample_rate)
    });
    rate.is_some_and(crate::dsp::ultrasonic::is_ultrasonic_rate)
}

/// True while live listening sends the mic to the speakers unchanged and the
/// mic's sample rate can hold sound above hearing range.
fn listening_ultrasound_at_1_1(state: AppState) -> bool {
    state.mic.listening().get_untracked()
        && !state.mic.mute_output().get_untracked()
        && !state.viewmode.hfr_enabled().get_untracked()
        // Time expansion also plays live audio 1:1, but it has its own toast.
        && state.playback.mode().get_untracked() == PlaybackMode::Normal
        && crate::dsp::ultrasonic::is_ultrasonic_rate(state.mic.sample_rate().get_untracked())
}

/// Whether the card may show now: turned on, not shown yet this session,
/// and no tutorial running.
fn nudge_allowed(state: AppState) -> bool {
    state.dialogs.hfr_nudge_enabled().get_untracked()
        && !state.dialogs.hfr_nudge_shown().get_untracked()
        && state.dialogs.hfr_tour_step().get_untracked().is_none()
}

fn show_nudge(state: AppState) {
    state.dialogs.hfr_nudge_shown().set(true);
    state.dialogs.hfr_nudge().set(true);
}

fn hide_nudge(state: AppState) {
    state.dialogs.hfr_nudge().set(false);
}

/// The card above Play. Mounted in the bottom toolbar.
#[component]
pub fn HfrNudge() -> impl IntoView {
    let state = expect_context::<AppState>();

    // Show on the start of playback, once per session.
    Effect::new(move |was_playing: Option<bool>| {
        let playing = state.playback.is_playing().get();
        if playing && was_playing == Some(false) && nudge_allowed(state) && playing_ultrasound_at_1_1(state)
        {
            show_nudge(state);
        }
        playing
    });

    // Live listening: show when the speakers start getting the mic (Listen
    // pressed, or unmuted), once per session; hide when that stops. The
    // sample rate is part of the edge in case it arrives after the flag.
    Effect::new(move |was_out: Option<bool>| {
        let out = state.mic.listening().get()
            && !state.mic.mute_output().get()
            && state.mic.sample_rate().get() > 0;
        if out && was_out == Some(false) && nudge_allowed(state) && listening_ultrasound_at_1_1(state) {
            show_nudge(state);
        } else if !out && was_out == Some(true) {
            hide_nudge(state);
        }
        out
    });

    // HFR turned on some other way, or another recording opened. Starting
    // to listen opens the live recording after the card shows, so a switch
    // to the live recording leaves the card up.
    Effect::new(move |prev_idx: Option<Option<usize>>| {
        let idx = state.library.current_index().get();
        let hfr = state.viewmode.hfr_enabled().get();
        let to_live = idx.is_some() && idx == state.mic.live_file_idx().get_untracked();
        if hfr || (prev_idx.is_some_and(|p| p != idx) && !to_live) {
            hide_nudge(state);
        }
        idx
    });

    let turn_on_hfr = move |_| {
        hide_nudge(state);
        if !state.viewmode.hfr_enabled().get_untracked() {
            state.toggle_hfr();
        }
    };
    let show_me = move |_| {
        hide_nudge(state);
        start_tour(state);
    };
    let never = move |_| {
        hide_nudge(state);
        state.dialogs.hfr_nudge_enabled().set(false);
        crate::settings::set_bool(crate::settings::keys::HFR_NUDGE_ENABLED, false);
    };

    // An empty box at the toolbar's top edge, in line with Play (see
    // .hfr-nudge-anchor). The card sits above it and is portalled, so a
    // resized (scrolling) toolbar cannot clip it.
    let anchor = NodeRef::<leptos::html::Div>::new();
    // Hidden while the phone Hear sheet is open: it would show through it.
    let is_open = Signal::derive(move || {
        state.dialogs.hfr_nudge().get() && !state.panels.hear_sheet_open().get()
    });

    view! {
        <div class="hfr-nudge-anchor" node_ref=anchor></div>
        <PopupPanel
            is_open=is_open
            anchor=anchor
            preferred_side=Side::Above
            preferred_align=Align::Start
            class="layer-panel hfr-nudge"
        >
            <div role="status" on:click=|ev: web_sys::MouseEvent| ev.stop_propagation()>
                <button class="hfr-nudge-close" title=NUDGE_CLOSE aria-label=NUDGE_CLOSE
                    on:click=move |_| hide_nudge(state)
                >"\u{00D7}"</button>
                <p class="hfr-nudge-text">
                    {move || if state.mic.listening().get() { NUDGE_TEXT_LIVE } else { NUDGE_TEXT }}
                </p>
                <div class="hfr-nudge-actions">
                    <button class="hfr-nudge-btn primary" on:click=turn_on_hfr>{NUDGE_HFR}</button>
                    <Show when=move || tour_available(state)>
                        <button class="hfr-nudge-btn" on:click=show_me>{TOUR_NAME}</button>
                    </Show>
                    <button class="hfr-nudge-link" title=NUDGE_NEVER_TIP on:click=never>{NUDGE_NEVER}</button>
                </div>
            </div>
        </PopupPanel>
    }
}
