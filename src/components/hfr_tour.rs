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
//     tutorial; "Don't show again" clears a setting in Settings > Playback.
//     "Turn on HFR" leaves the card up for NUDGE_ON_MS as "HFR is on", with
//     the tutorial button. That card moves to just under the HFR switch,
//     filled blue, and the button gets a blue ring (body.hfr-nudge-on).
//     The card pulses twice as it opens (style.css, .play-card): it appears
//     while sound plays and was easy to miss.
//
// Entry points for the tutorial: the card, the Band presets menu beside HFR,
// and Settings > Playback.

use crate::components::play_card::{PlayCard, PlayCardBody};
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

// The card spells out "HFR (High Frequency Range)" once, so the name on the
// HFR switch means something to a new user. The second sentence is D_HFR's.
const NUDGE_TEXT_LIVE: &str = "Listening 1:1. Sound above 20\u{a0}kHz is too high to hear. \
    HFR (High Frequency Range) lowers the pitch of bat calls into the range you can hear.";
const NUDGE_TEXT: &str = "Playing 1:1. Sound above 20\u{a0}kHz is too high to hear. \
    HFR (High Frequency Range) lowers the pitch of bat calls into the range you can hear.";
/// The card after "Turn on HFR": it stays a few seconds so the tutorial is
/// still one press away. It does not say the calls can now be heard: the
/// recording may have none, and the band HFR picks may miss them.
const NUDGE_ON_TEXT: &str =
    "HFR is on. To turn it off, press the HFR switch above the spectrogram.";
/// The same card when it sits just under the HFR switch.
const NUDGE_ON_TEXT_UNDER_BTN: &str = "HFR is on. To turn it off, press the HFR switch.";
/// How long the "HFR is on" card stays while the pointer is not on it.
const NUDGE_ON_MS: i32 = 10_000;
const NUDGE_HFR: &str = "Turn on HFR";
const NUDGE_NEVER: &str = "Don't show again";
const NUDGE_NEVER_TIP: &str = "Turn back on in Settings > Playback";

/// Settings > Playback checkbox that allows the card.
pub const NUDGE_SETTING: &str = "Suggest HFR when playing or listening\u{a0}1:1";

// ── Steps ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TourStep {
    /// The HFR switch in the Hear bar.
    Hfr,
    /// The frequency gutter beside the spectrogram.
    BandGutter,
    /// The time gutter under the spectrogram.
    TimeGutter,
    Play,
}

/// What js/hfr-tour.js needs to show one step.
struct StepSpec {
    /// CSS selector of the control. The first match that is on screen gets
    /// the highlight.
    selector: &'static str,
    title: &'static str,
    description: &'static str,
    /// Where the popover sits beside the control (driver.js names).
    side: &'static str,
    align: &'static str,
    /// The side to use instead when the control is short (a phone, a small
    /// window): a popover beside a short strip covers most of it.
    short_side: Option<&'static str>,
    /// The drag a hand acts out over the control while the step shows:
    /// "drag-up" or "drag-right". None: no hand.
    gesture: Option<&'static str>,
}

impl StepSpec {
    fn to_js(&self) -> Object {
        let o = Object::new();
        set(&o, "selector", &self.selector.into());
        set(&o, "title", &self.title.into());
        set(&o, "description", &self.description.into());
        set(&o, "side", &self.side.into());
        set(&o, "align", &self.align.into());
        if let Some(short_side) = self.short_side {
            set(&o, "short_side", &short_side.into());
        }
        if let Some(gesture) = self.gesture {
            set(&o, "gesture", &gesture.into());
        }
        o
    }
}

impl TourStep {
    fn spec(self) -> StepSpec {
        match self {
            TourStep::Hfr => StepSpec {
                selector: ".hearing-bar .hfr-switch .switch-btn-top",
                title: T_HFR,
                description: D_HFR,
                side: "bottom",
                align: "start",
                short_side: None,
                gesture: None,
            },
            TourStep::BandGutter => StepSpec {
                selector: ".band-gutter",
                title: T_BAND,
                description: D_BAND,
                side: "right",
                align: "start",
                // Above the strip, the popover leaves the whole strip and
                // the calls beside it in view.
                short_side: Some("top"),
                gesture: Some("drag-up"),
            },
            // The popover sits over the right end of the strip, so the hand
            // has the left end to drag along.
            TourStep::TimeGutter => StepSpec {
                selector: ".time-gutter",
                title: T_TIME,
                description: D_TIME,
                side: "top",
                align: "end",
                short_side: None,
                gesture: Some("drag-right"),
            },
            TourStep::Play => StepSpec {
                selector: ".play-main-btn",
                title: T_PLAY,
                description: D_PLAY,
                side: "top",
                align: "start",
                short_side: None,
                gesture: None,
            },
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
    // On a phone the sidebars lie over the view, and the steps point at
    // controls under them (the tutorial button is in Settings).
    if state.status.is_mobile().get_untracked() {
        state.panels.left_collapsed().set(true);
        state.panels.right_collapsed().set(true);
    }
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
        js_steps.push(&step.spec().to_js());
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
pub(crate) fn playing_ultrasound_at_1_1(state: AppState) -> bool {
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
        if playing
            && was_playing == Some(false)
            && nudge_allowed(state)
            && playing_ultrasound_at_1_1(state)
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
        if out
            && was_out == Some(false)
            && nudge_allowed(state)
            && listening_ultrasound_at_1_1(state)
        {
            show_nudge(state);
        } else if !out && was_out == Some(true) {
            hide_nudge(state);
        }
        out
    });

    // True while the card shows "HFR is on" after its "Turn on HFR" button.
    let hfr_on_state = RwSignal::new(false);
    // A mouse pointer is over the card: the "HFR is on" card waits instead
    // of closing under it. Mouse only: a tap sends pointerenter too, and on
    // touch screens the matching pointerleave is not reliable.
    let held = RwSignal::new(false);
    // The card's body, to see whether keyboard focus is inside it. Read on
    // each timer tick: focusout does not fire when the focused "Turn on HFR"
    // button is removed, so a flag set by focus events could stay set.
    let body_ref = NodeRef::<leptos::html::Div>::new();
    let focus_inside = move || {
        let Some(Some(el)) = body_ref.try_get_untracked() else {
            return false;
        };
        web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.active_element())
            .is_some_and(|a| el.contains(Some(a.as_ref())))
    };
    // Where the HFR switch is on screen (left, top, width, height), while
    // the "HFR is on" card shows: that card opens under the button. `None`
    // when the button is not fully on screen (the Hear bar scrolls sideways
    // on phones); the card then stays above Play.
    let btn_rect = RwSignal::new(None::<(f64, f64, f64, f64)>);
    let measure_btn = move || {
        let rect = (|| {
            let win = web_sys::window()?;
            let el = win
                .document()?
                .query_selector(".hearing-bar .hfr-switch")
                .ok()??;
            let r = el.get_bounding_client_rect();
            let v_w = win.inner_width().ok()?.as_f64()?;
            (r.width() > 0.0 && r.left() >= 0.0 && r.right() <= v_w)
                .then(|| (r.left(), r.top(), r.width(), r.height()))
        })();
        btn_rect.try_set(rect);
    };
    // The popup re-places itself a frame after a window resize, so the
    // button's new place is known by then.
    let resize_handle = window_event_listener(leptos::ev::resize, move |_| {
        if hfr_on_state.try_get_untracked() == Some(true) {
            measure_btn();
        }
    });
    on_cleanup(move || resize_handle.remove());
    // Bumped on every "Turn on HFR", so an older close timer does nothing.
    let timer_gen = StoredValue::new(0u32);

    // The card closed (any way): leave the "HFR is on" state.
    Effect::new(move || {
        if !state.dialogs.hfr_nudge().get() {
            hfr_on_state.set(false);
            held.set(false);
        }
    });

    // HFR turned on some other way, or another recording opened. Starting
    // to listen opens the live recording after the card shows, so a switch
    // to the live recording leaves the card up. In the "HFR is on" state the
    // card stays while HFR is on, and closes if HFR is turned off again.
    Effect::new(move |prev_idx: Option<Option<usize>>| {
        let idx = state.library.current_index().get();
        let hfr = state.viewmode.hfr_enabled().get();
        let to_live = idx.is_some() && idx == state.mic.live_file_idx().get_untracked();
        let hfr_changed_card = if hfr_on_state.get_untracked() {
            !hfr
        } else {
            hfr
        };
        if hfr_changed_card || (prev_idx.is_some_and(|p| p != idx) && !to_live) {
            hide_nudge(state);
        }
        idx
    });

    // Pulse the HFR switch while the card says "press the HFR switch".
    Effect::new(move || {
        let on = hfr_on_state.get();
        if let Some(body) = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.body())
        {
            let _ = body.class_list().toggle_with_force("hfr-nudge-on", on);
        }
    });

    let turn_on_hfr = move |_| {
        // While listening there is no tutorial to offer, so the card closes.
        let offer_tour = tour_available(state);
        if offer_tour {
            // The card moves from under the pointer, so no pointerleave
            // comes: without this the close timer would never count down.
            held.set(false);
            measure_btn();
            hfr_on_state.set(true);
        } else {
            hide_nudge(state);
        }
        if !state.viewmode.hfr_enabled().get_untracked() {
            state.toggle_hfr();
        }
        if !offer_tour {
            return;
        }
        timer_gen.update_value(|g| *g = g.wrapping_add(1));
        let gen = timer_gen.get_value();
        wasm_bindgen_futures::spawn_local(async move {
            const STEP_MS: i32 = 250;
            let mut left = NUDGE_ON_MS;
            while left > 0 {
                crate::web_util::sleep_ms(STEP_MS).await;
                if timer_gen.try_get_value() != Some(gen)
                    || hfr_on_state.try_get_untracked() != Some(true)
                {
                    return;
                }
                if held.try_get_untracked() != Some(true) && !focus_inside() {
                    left -= STEP_MS;
                }
            }
            state.dialogs.hfr_nudge().try_set(false);
        });
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

    // An empty box over the HFR switch, for the "HFR is on" card.
    let btn_anchor = NodeRef::<leptos::html::Div>::new();
    // Hidden while the phone Hear sheet is open: it would show through it.
    // Hidden while the Band presets menu is open: the menu opens from the
    // same control and covered half the card.
    let showing = Signal::derive(move || {
        state.dialogs.hfr_nudge().get()
            && !state.panels.hear_sheet_open().get()
            && state.panels.layer_panel_open().get() != Some(crate::state::LayerPanel::BandPresets)
    });
    let under_btn = Signal::derive(move || hfr_on_state.get() && btn_rect.get().is_some());
    let open_above_play = Signal::derive(move || showing.get() && !under_btn.get());
    let open_under_btn = Signal::derive(move || showing.get() && under_btn.get());

    let text = Signal::derive(move || {
        if under_btn.get() {
            NUDGE_ON_TEXT_UNDER_BTN
        } else if hfr_on_state.get() {
            NUDGE_ON_TEXT
        } else if state.mic.listening().get() {
            NUDGE_TEXT_LIVE
        } else {
            NUDGE_TEXT
        }
        .to_string()
    });
    // The card's contents: the same in both places.
    let card = move || {
        view! {
            <PlayCardBody text=text on_close=Callback::new(move |()| hide_nudge(state))
                body_ref=body_ref hover=held
            >
                <Show when=move || !hfr_on_state.get()>
                    <button class="play-card-btn primary" on:click=turn_on_hfr>{NUDGE_HFR}</button>
                </Show>
                // Stays a plain button once HFR is on: filled, it read as
                // a required next step rather than an offer.
                <Show when=move || tour_available(state)>
                    <button class="play-card-btn" on:click=show_me>{TOUR_NAME}</button>
                </Show>
                <Show when=move || !hfr_on_state.get()>
                    <button class="play-card-link" title=NUDGE_NEVER_TIP on:click=never>{NUDGE_NEVER}</button>
                </Show>
            </PlayCardBody>
        }
    };

    view! {
        <div class="hfr-nudge-btn-anchor" node_ref=btn_anchor
            style=move || {
                let (l, t, w, h) = btn_rect.get().unwrap_or_default();
                format!("left: {l}px; top: {t}px; width: {w}px; height: {h}px;")
            }
        ></div>
        <PlayCard is_open=open_above_play class="layer-panel play-card hfr-nudge">
            {card()}
        </PlayCard>
        <PopupPanel
            is_open=open_under_btn
            anchor=btn_anchor
            preferred_side=Side::Below
            preferred_align=Align::Start
            class="layer-panel play-card hfr-nudge hfr-nudge-under-btn"
        >
            {card()}
        </PopupPanel>
    }
}
