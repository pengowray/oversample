//! The experimental live view in the Call detection tab, for BirdNET or Perch
//! (the method picked in the tab): a switch, a status line, and the chart that
//! `crate::live_embedding` draws into.
//!
//! The worker runs only while the switch is on and this section is shown;
//! leaving the tab stops it, coming back starts it again, and picking the
//! other method restarts it with that model.

use crate::classifier::{DownloadStatus, Method};
use crate::live_embedding::{self, LiveStatus};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;

#[component]
pub(super) fn LiveView(
    /// BirdNET or Perch: the method picked in the tab.
    #[prop(into)]
    method: Signal<Method>,
) -> impl IntoView {
    let state = expect_context::<AppState>();
    let on = move || state.classifier.live_on().get();
    let model = move || live_embedding::model_for(method.get());
    live_embedding::set_picked(method.get_untracked());
    if state.classifier.live_on().get_untracked() {
        live_embedding::set_user(state, live_embedding::LIVE_VIEW, true);
    }
    // Leaving the tab stops this view's readings; the switch is remembered.
    on_cleanup(move || live_embedding::set_user(state, live_embedding::LIVE_VIEW, false));
    // The other method picked: its model replaces the running one.
    Effect::new(move |_| live_embedding::set_model(state, method.get()));

    // Start once the model has been downloaded, if the switch was turned on before.
    Effect::new(move |_| {
        let m = model();
        let ready = state.classifier.downloads().with(|d| {
            m.parts
                .iter()
                .chain([&m.labels])
                .all(|f| matches!(d.get(f.id), Some(DownloadStatus::Downloaded)))
        });
        let waiting = state
            .classifier
            .live()
            .with_untracked(|s| *s == LiveStatus::NeedsDownload);
        if ready && waiting {
            live_embedding::restart(state);
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

    // "Find recordings" beside a species in the map's list: live-embedding.js
    // sends the species' scientific name in an `oversample-find-species` event.
    let on_find = window_event_listener(
        leptos::ev::Custom::<web_sys::CustomEvent>::new("oversample-find-species"),
        move |ev| {
            if let Some(name) = ev.detail().as_string() {
                crate::components::find_recordings::open_with_search(state, &name);
            }
        },
    );
    on_cleanup(move || on_find.remove());

    let sound_on = move || {
        state.playback.is_playing().get()
            || state.mic.listening().get()
            || state.mic.recording().get()
    };
    let status = move || match state.classifier.live().get() {
        LiveStatus::Off => None,
        LiveStatus::NeedsDownload => Some(format!(
            "Download {} below to use the live view.",
            model().name
        )),
        LiveStatus::Loading => Some(format!("Loading {}\u{2026}", model().name)),
        LiveStatus::Ready { interval_ms, .. } if interval_ms > 0.0 && sound_on() => Some(format!(
            "The last {} s of audio, updated every {:.2} s",
            model().window_s,
            interval_ms / 1000.0
        )),
        LiveStatus::Ready { .. } if sound_on() => Some("Starting\u{2026}".to_string()),
        // A file is open and paused: the readout and the map show the 3 s
        // Play would start with (live_embedding's paused reading). Not for
        // a timeline, which gets no paused reading.
        LiveStatus::Ready { .. }
            if state.library.current_index().get().is_some()
                && state.timeline.active().with(Option::is_none) =>
        {
            Some(format!(
                "The {} s from where Play starts (blue triangle on the map)",
                model().window_s
            ))
        }
        LiveStatus::Ready { .. } => Some("Play a file or listen with the microphone.".to_string()),
        LiveStatus::Failed(e) => Some(format!("Live view stopped: {e}")),
    };
    let model_time = move || match state.classifier.live().get() {
        LiveStatus::Ready { model_ms, .. } if model_ms > 0.0 => {
            format!(
                "{} takes {model_ms:.0} ms for each reading on this device.",
                model().name
            )
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
                // Hidden until the model is loaded: before that it would show
                // an empty frame under "Download ... below". Hidden, not
                // removed, so the element registered with live_embedding
                // stays the same; its ResizeObserver redraws it when shown.
                <div
                    class="live-emb-chart"
                    node_ref=chart_ref
                    style:display=move || {
                        if matches!(state.classifier.live().get(), LiveStatus::Ready { .. }) {
                            ""
                        } else {
                            "none"
                        }
                    }
                ></div>
            </Show>
        </div>
    }
}
