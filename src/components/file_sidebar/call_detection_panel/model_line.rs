//! The line about the stored model, with Delete, and the attribution and
//! Credits the model's licence asks for.

use super::bird_boxes;
use super::classifier_section::mb;
use crate::classifier::{self, catalog, DownloadStatus, Method, MethodFacts};
use crate::state::store_fields::*;
use crate::state::AppState;
use leptos::prelude::*;

/// The files that Delete removes.
fn deletable_files(method: Method) -> Vec<&'static catalog::ModelFile> {
    match method {
        Method::BirdNet | Method::Bats => catalog::birdnet_files().collect(),
        Method::Perch => catalog::PERCH_FILES.to_vec(),
        Method::BirdNetV3 => vec![&catalog::BIRDNET_V3],
    }
}

/// The line about the stored model (its size and Delete) and the
/// attribution the model's licence asks for, with Credits.
pub(super) fn model_line(
    state: AppState,
    method: Method,
    model_size: u64,
    backbone: Memo<DownloadStatus>,
    show_credits: RwSignal<bool>,
) -> impl IntoView {
    let downloaded = backbone.get() == DownloadStatus::Downloaded;
    let a = catalog::attribution(method);
    let byline = method.info().byline;
    // What can't run once the model is deleted, and whether the model is
    // shared (BirdNET's backbone is shared with BattyBirdNET).
    let (cant_run, shared) = match method {
        Method::BirdNet | Method::Bats => ("BirdNET and BattyBirdNET", true),
        Method::Perch => ("Perch", false),
        Method::BirdNetV3 => ("BirdNET+ V3.0", false),
    };
    // BirdBox's model, beside BirdNET+ V3.0: its size, Delete and its
    // licence (AGPL-3.0) go on the same lines.
    let v3 = method == Method::BirdNetV3;
    let boxes_stored = v3 && bird_boxes::model_stored(state);
    let boxes_credited = v3 && (boxes_stored || state.call_detection.bird_boxes().get());
    let boxes_size = catalog::JUST_BIRD.size;
    let b = catalog::JUST_BIRD_ATTRIBUTION;
    // The link text is the site, e.g. "birdnet.cornell.edu".
    let site = a
        .powered_by_url
        .trim_start_matches("https://")
        .trim_start_matches("www.")
        .trim_end_matches('/');
    view! {
        <div class="cd-model">
            {downloaded.then(|| view! {
                <div>
                    // Deleting BirdNET's backbone stops BattyBirdNET too, so
                    // the line says so where Delete is.
                    {if shared {
                        format!("Model on this device, {}, used by BirdNET and BattyBirdNET. ", mb(model_size))
                    } else if boxes_stored {
                        format!("Models on this device, {} and {}. ", mb(model_size), mb(boxes_size))
                    } else {
                        format!("Model on this device, {}. ", mb(model_size))
                    }}
                    <button
                        class="detection-link-btn"
                        on:click=move |_| {
                            let question = if boxes_stored {
                                format!(
                                    "Delete BirdNET+ V3.0 ({}) and BirdBox Just-Bird ({}) from this device? Neither model can run until it is downloaded again. Results already shown stay.",
                                    mb(model_size),
                                    mb(boxes_size)
                                )
                            } else {
                                format!(
                                    "Delete the model ({}) from this device? {cant_run} can't run until it is downloaded again. Results already shown stay.",
                                    mb(model_size)
                                )
                            };
                            let ok = web_sys::window()
                                .and_then(|w| w.confirm_with_message(&question).ok())
                                .unwrap_or(false);
                            if ok {
                                classifier::cancel(state);
                                let mut files = deletable_files(method);
                                if boxes_stored {
                                    files.push(&catalog::JUST_BIRD);
                                }
                                classifier::delete_files(state, files);
                            }
                        }
                    >"Delete"</button>
                </div>
            })}
            <div>
                {byline}" "
                <a href=a.licence_url target="_blank" rel="noopener">{a.licence}</a>
                ". "
                {boxes_credited.then(|| view! {
                    "Boxes by BirdBox Just-Bird. "
                    <a href=b.licence_url target="_blank" rel="noopener">{b.licence}</a>
                    ". "
                })}
                <button
                    class="detection-link-btn"
                    aria-expanded=move || show_credits.get().to_string()
                    on:click=move |_| show_credits.update(|v| *v = !*v)
                >"Credits"</button>
            </div>
            {move || show_credits.get().then(|| view! {
                <div class="detection-credits">
                    {a.credits.iter().map(|c| view! { <p>{*c}</p> }).collect::<Vec<_>>()}
                    <p>{a.citation}</p>
                    <p>
                        <a href=a.powered_by_url target="_blank" rel="noopener">{site}</a>
                    </p>
                    {boxes_credited.then(|| view! {
                        {b.credits.iter().map(|c| view! { <p>{*c}</p> }).collect::<Vec<_>>()}
                        <p>{b.citation}</p>
                        <p>
                            <a href=b.source_url target="_blank" rel="noopener">"github.com/birdnet-team/BirdBox"</a>
                            " "
                            <a href=b.licence_url target="_blank" rel="noopener">"AGPL-3.0 licence text"</a>
                        </p>
                    })}
                </div>
            })}
        </div>
    }
}
