// The "quiet playback" card: shows above Play when what is playing, with
// gain off, is very quiet, and offers to turn on Gain. At most once per
// session; "Don't show again" clears a setting in Settings > Playback.
//
// The level comes from the playback stream itself
// (`streaming_playback::OutputLevelReport`): the peak of the first 3 s of
// output after the filters and the playback mode (HFR, heterodyne, ...), so
// a recording that is loud only outside the band being played still counts
// as quiet.
//
// The card only shows when turning on gain would make it clearly louder.
// The Gain switch turns on the last auto mode. Peak mode measures the raw
// recording by default, which can be loud while the HFR band is quiet; then
// the button turns on AGC instead, which works on what is played.
//
// It is a `PlayCard`, like the HFR card (hfr_tour.rs), and never shows while
// that card does.

use crate::audio::streaming_playback::{self, OutputLevelReport};
use crate::components::play_card::{PlayCard, PlayCardBody};
use crate::state::store_fields::*;
use crate::state::{AppState, GainMode};
use leptos::prelude::*;
use std::rc::Rc;

// ── Strings ─────────────────────────────────────────────────────────────

const NUDGE_GAIN: &str = "Turn on Gain";
const NUDGE_NEVER: &str = "Don't show again";
const NUDGE_NEVER_TIP: &str = "Turn back on in Settings > Playback";

/// Settings > Playback checkbox that allows the card.
pub const NUDGE_SETTING: &str = "Suggest Gain when playback is very quiet";

fn nudge_text(peak_db: f64) -> String {
    format!(
        "Playback is very quiet (peak level \u{2212}{:.0}\u{a0}dB). Gain is off.",
        -peak_db
    )
}

// ── Thresholds ──────────────────────────────────────────────────────────

/// Output peak (dBFS) below which playback counts as very quiet.
const QUIET_DB: f64 = -30.0;
/// The peak the gain must bring playback up to for the card to offer it.
const LOUD_ENOUGH_DB: f64 = -20.0;
/// AGC does not boost sound below its noise gate (-50 dBFS, `AgcConfig`),
/// so it is only offered above this.
const AGC_FLOOR_DB: f64 = -45.0;

/// What the card's button does.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TurnOn {
    /// The same as pressing the Gain switch.
    Switch,
    /// AGC: the switch's Peak mode would barely raise this playback.
    Agc,
}

/// How to make this playback louder, if gain can.
fn choose(state: AppState, peak_db: f64) -> Option<TurnOn> {
    match state.gain.mode_last_auto().get_untracked() {
        GainMode::Adaptive => (peak_db >= AGC_FLOOR_DB).then_some(TurnOn::Switch),
        _ => {
            let boost =
                state.compute_auto_gain_untracked() + state.gain.db().get_untracked().max(0.0);
            if peak_db + boost >= LOUD_ENOUGH_DB {
                Some(TurnOn::Switch)
            } else if peak_db >= AGC_FLOOR_DB {
                Some(TurnOn::Agc)
            } else {
                None
            }
        }
    }
}

/// Whether the card may show for this report, and what its button does.
fn should_show(state: AppState, report: OutputLevelReport) -> Option<(f64, TurnOn)> {
    let d = &state.dialogs;
    if !d.gain_nudge_enabled().get_untracked()
        || d.gain_nudge_shown().get_untracked()
        || d.hfr_nudge().get_untracked()
        || d.hfr_tour_step().get_untracked().is_some()
        || !state.playback.is_playing().get_untracked()
        || state.gain.mode().get_untracked() != GainMode::Off
        || state.mic.listening().get_untracked()
        || state.mic.recording().get_untracked()
        || state.timeline.active().get_untracked().is_some()
        // Sound above hearing range: louder won't help, the HFR card will.
        || crate::components::hfr_tour::playing_ultrasound_at_1_1(state)
    {
        return None;
    }
    // Digital silence: gain can't help.
    let peak_db = report.peak_db?;
    if peak_db >= QUIET_DB {
        return None;
    }
    Some((peak_db, choose(state, peak_db)?))
}

fn hide(state: AppState) {
    state.dialogs.gain_nudge().set(false);
}

/// The card above Play. Mounted in the bottom toolbar beside the HFR card.
#[component]
pub fn GainNudge() -> impl IntoView {
    let state = expect_context::<AppState>();
    let peak_db = RwSignal::new(0.0f64);
    let turn_on = StoredValue::new(TurnOn::Switch);

    streaming_playback::set_output_level_hook(Some(Rc::new(move |report| {
        if let Some((peak, how)) = should_show(state, report) {
            peak_db.set(peak);
            turn_on.set_value(how);
            state.dialogs.gain_nudge_shown().set(true);
            state.dialogs.gain_nudge().set(true);
        }
    })));
    on_cleanup(|| streaming_playback::set_output_level_hook(None));

    // The measurement no longer applies: gain turned on some other way,
    // another file, HFR or the playback mode changed, or the HFR card opened.
    Effect::new(move |prev: Option<(Option<usize>, bool, crate::state::PlaybackMode)>| {
        let now = (
            state.library.current_index().get(),
            state.viewmode.hfr_enabled().get(),
            state.playback.mode().get(),
        );
        let gain_on = state.gain.mode().get() != GainMode::Off;
        let hfr_card = state.dialogs.hfr_nudge().get();
        if gain_on || hfr_card || prev.is_some_and(|p| p != now) {
            hide(state);
        }
        now
    });

    let turn_on_gain = move |_| {
        hide(state);
        match turn_on.get_value() {
            TurnOn::Switch => {
                if state.gain.mode().get_untracked() == GainMode::Off {
                    state.toggle_gain();
                }
            }
            TurnOn::Agc => state.set_gain_mode(GainMode::Adaptive),
        }
    };
    let never = move |_| {
        hide(state);
        state.dialogs.gain_nudge_enabled().set(false);
        crate::settings::set_bool(crate::settings::keys::GAIN_NUDGE_ENABLED, false);
    };

    // Hidden while the phone Hear sheet is open: it would show through it.
    let showing = Signal::derive(move || {
        state.dialogs.gain_nudge().get() && !state.panels.hear_sheet_open().get()
    });

    view! {
        <PlayCard is_open=showing class="layer-panel play-card gain-nudge">
            <PlayCardBody
                text=Signal::derive(move || nudge_text(peak_db.get()))
                on_close=Callback::new(move |()| hide(state))
            >
                <button class="play-card-btn primary" on:click=turn_on_gain>{NUDGE_GAIN}</button>
                <button class="play-card-link" title=NUDGE_NEVER_TIP on:click=never>{NUDGE_NEVER}</button>
            </PlayCardBody>
        </PlayCard>
    }
}
