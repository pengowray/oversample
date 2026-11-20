//! The Live explorer tab (experimental): `crate::live_explorer` draws into the
//! chart element here. BirdNET runs while this tab is shown and stops when it
//! is closed.

use crate::classifier::{self, catalog, DownloadStatus, ModelChoice};
use crate::live_embedding::{self, LiveStatus, EXPLORER};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;

#[component]
pub fn LiveExplorerPanel() -> impl IntoView {
    let state = expect_context::<AppState>();
    let chart_ref = NodeRef::<leptos::html::Div>::new();
    let drawn = StoredValue::new_local(None::<web_sys::HtmlElement>);
    Effect::new(move |_| {
        let el = chart_ref.get().map(web_sys::HtmlElement::from);
        live_embedding::set_explorer(el.clone());
        if el.is_some() {
            drawn.set_value(el);
            crate::live_explorer::draw(state);
        }
    });
    live_embedding::set_user(state, EXPLORER, true);
    on_cleanup(move || {
        live_embedding::set_explorer(None);
        live_embedding::set_user(state, EXPLORER, false);
        if let Some(el) = drawn.get_value() {
            crate::live_explorer::call("dispose", &[&el]);
        }
    });
    classifier::refresh_downloads(state);
    // Start BirdNET once it has been downloaded.
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
        if ready && waiting {
            live_embedding::restart(state);
        }
    });

    let sound_on = move || {
        state.playback.is_playing().get()
            || state.mic.listening().get()
            || state.mic.recording().get()
    };
    let downloading = move || {
        state.classifier.downloads().with(|m| {
            matches!(
                m.get(catalog::BACKBONE.id),
                Some(DownloadStatus::Downloading { .. })
            )
        })
    };
    let status = move || match state.classifier.live().get() {
        // The live view in the Call detection tab runs Perch: the explorer's
        // views are BirdNET's, so only the spectrum is drawn.
        s if s != LiveStatus::Off
            && live_embedding::active().method != classifier::Method::BirdNet =>
        {
            Some(format!(
                "Only the spectrum updates while the live view in the Call detection tab uses {}.",
                live_embedding::active().name
            ))
        }
        LiveStatus::NeedsDownload if downloading() => {
            Some("Downloading BirdNET\u{2026} The spectrum works without it.".to_string())
        }
        LiveStatus::NeedsDownload => None,
        LiveStatus::Loading => Some("Loading BirdNET\u{2026}".to_string()),
        LiveStatus::Failed(e) => Some(format!("BirdNET stopped: {e}")),
        _ if !sound_on() => Some("Play a file or listen with the microphone.".to_string()),
        LiveStatus::Ready { interval_ms, .. } if interval_ms > 0.0 => Some(format!(
            "BirdNET reads the last 3 s of audio every {:.2} s.",
            interval_ms / 1000.0
        )),
        _ => None,
    };
    let needs_download = move || {
        state.classifier.live().get() == LiveStatus::NeedsDownload
            && !downloading()
            && live_embedding::active().method == classifier::Method::BirdNet
    };

    view! {
        <div class="sidebar-panel live-exp">
            <div class="live-exp-head">
                <span class="live-emb-tag">"Experimental"</span>
                <span class="detection-status-note">"What BirdNET hears, and the spectrum, live."</span>
            </div>
            {move || status().map(|s| view! { <div class="detection-status-note">{s}</div> })}
            <Show when=needs_download>
                <div class="live-exp-download">
                    <span class="detection-status-note">"BirdNET is not downloaded. The spectrum works without it."</span>
                    <button
                        class="psd-btn"
                        on:click=move |_| classifier::download(state, ModelChoice::BirdNet)
                    >"Download BirdNET (67 MB)"</button>
                </div>
            </Show>
            <div class="live-exp-chart" node_ref=chart_ref></div>
        </div>
    }
}
