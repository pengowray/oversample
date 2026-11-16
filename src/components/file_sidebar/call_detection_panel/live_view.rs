//! The experimental live BirdNET view in the Call detection tab: a switch, a
//! status line, and the chart that `crate::live_embedding` draws into.
//!
//! The worker runs only while the switch is on and this section is shown;
//! leaving the tab stops it, coming back starts it again.

use crate::classifier::{catalog, DownloadStatus};
use crate::live_embedding::{self, LiveStatus};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;

#[component]
pub(super) fn LiveView() -> impl IntoView {
    let state = expect_context::<AppState>();
    let on = move || state.classifier.live_on().get();
    if state.classifier.live_on().get_untracked() {
        live_embedding::set_enabled(state, true);
    }
    on_cleanup(move || {
        if state.classifier.live_on().get_untracked() {
            // Stop the worker but remember the switch for when the tab opens again.
            live_embedding::set_enabled(state, false);
            state.classifier.live_on().set(true);
        }
    });

    // Start once BirdNET has been downloaded, if the switch was turned on before.
    Effect::new(move |_| {
        let ready = state.classifier.downloads().with(|m| {
            [&catalog::BACKBONE, &catalog::BIRDNET_LABELS]
                .iter()
                .all(|f| matches!(m.get(f.id), Some(DownloadStatus::Downloaded)))
        });
        let waiting = state
            .classifier
            .live()
            .with_untracked(|s| *s == LiveStatus::NeedsDownload);
        if ready && waiting && state.classifier.live_on().get_untracked() {
            live_embedding::set_enabled(state, true);
        }
    });

    let chart_ref = NodeRef::<leptos::html::Div>::new();
    let drawn = StoredValue::new_local(None::<web_sys::HtmlElement>);
    Effect::new(move |_| {
        let el = chart_ref.get().map(web_sys::HtmlElement::from);
        live_embedding::set_chart(el.clone());
        if el.is_some() {
            drawn.set_value(el);
        }
    });
    on_cleanup(move || {
        live_embedding::set_chart(None);
        if let Some(el) = drawn.get_value() {
            live_embedding::call("dispose", &[&el]);
        }
    });

    let sound_on = move || {
        state.playback.is_playing().get()
            || state.mic.listening().get()
            || state.mic.recording().get()
    };
    let status = move || match state.classifier.live().get() {
        LiveStatus::Off => None,
        LiveStatus::NeedsDownload => {
            Some("Download BirdNET below to use the live view.".to_string())
        }
        LiveStatus::Loading => Some("Loading BirdNET\u{2026}".to_string()),
        LiveStatus::Ready { interval_ms, .. } if interval_ms > 0.0 && sound_on() => Some(format!(
            "The last 3 s of audio, updated every {:.2} s",
            interval_ms / 1000.0
        )),
        LiveStatus::Ready { .. } if sound_on() => Some("Starting\u{2026}".to_string()),
        LiveStatus::Ready { .. } => Some("Play a file or listen with the microphone.".to_string()),
        LiveStatus::Failed(e) => Some(format!("Live view stopped: {e}")),
    };
    let model_time = move || match state.classifier.live().get() {
        LiveStatus::Ready { model_ms, .. } if model_ms > 0.0 => {
            format!("BirdNET takes {model_ms:.0} ms for each reading on this device.")
        }
        _ => String::new(),
    };

    view! {
        <div class="live-emb">
            <label class="setting-label live-emb-switch">
                <input
                    type="checkbox"
                    prop:checked=on
                    on:change=move |ev| live_embedding::set_enabled(state, event_target_checked(&ev))
                />
                <span>"Live view"</span>
                <span class="live-emb-tag">"Experimental"</span>
            </label>
            <Show when=on>
                {move || status().map(|s| view! { <div class="detection-status-note" title=model_time>{s}</div> })}
                <div class="live-emb-chart" node_ref=chart_ref></div>
            </Show>
        </div>
    }
}
