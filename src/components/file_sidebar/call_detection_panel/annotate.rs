//! The "Add N annotations" fold: the windows Analyse found, added as
//! annotations.

use super::{count, undo_keys};
use crate::annotations::{now_iso8601, AnnotationId, AnnotationSet};
use crate::call_annotations::remove_annotations_where;
use crate::classifier::{self, location, summary, Method};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;

/// Add the windows at or above the minimum confidence as annotations,
/// replacing the ones this method (and region) added before.
fn add_annotations(state: AppState, file_id: u64, method: Method, merge: bool) {
    let min = classifier::min_confidence_untracked(state, method);
    let now = now_iso8601();
    let built = state.classifier.results().with_untracked(|m| {
        m.get(&(file_id, method)).map(|run| {
            let spans = summary::spans(run, min, merge);
            (
                summary::build_annotations(run, &spans, &now, AnnotationId::new, |l| {
                    untrack(|| location::unlikely_here(state, file_id, method, l))
                }),
                summary::auto_tag_value(run),
            )
        })
    });
    let Some((annotations, tag)) = built else {
        return;
    };
    if annotations.is_empty() {
        return;
    }
    let Some(idx) = state.library.current_index().get_untracked() else {
        return;
    };
    let new_set = state.library.files().with_untracked(|files| {
        files.get(idx).map(|f| {
            let id = f.identity.clone().unwrap_or_else(|| {
                crate::file_identity::identity_layer1(&f.name, f.audio.metadata.file_size as u64)
            });
            AnnotationSet::new_with_metadata(id, &f.audio, f.cached_peak_db, f.cached_full_peak_db)
        })
    });
    let Some(new_set) = new_set else {
        return;
    };
    let added = annotations.len();
    state.snapshot_annotations();
    let mut removed = 0;
    let mut remaining_ids: Vec<AnnotationId> = Vec::new();
    state.annotations.store().update(|store| {
        let set = store.entry_or_insert_with(file_id, || new_set);
        removed =
            remove_annotations_where(&mut set.annotations, |a| summary::has_auto_tag(a, &tag));
        set.annotations.extend(annotations);
        remaining_ids = set.annotations.iter().map(|a| a.id.clone()).collect();
    });
    if removed > 0 {
        state
            .annotations
            .selected_ids()
            .update(|ids| ids.retain(|id| remaining_ids.contains(id)));
    }
    state.annotations.dirty().set(true);
    state.annotations.visible().set(true);
    let mut toast = format!("{} added", count(added, "annotation", "annotations"));
    if removed > 0 {
        toast.push_str(&format!(
            ". {} removed.",
            count(removed, "previous annotation", "previous annotations")
        ));
    }
    state.show_info_toast(toast);
}

/// The inside of the "Add N annotations" fold: one per window or merged,
/// what adding replaces, and the button.
pub(super) fn annotate_body(
    state: AppState,
    method: Method,
    current_file: Memo<Option<u64>>,
    merge: RwSignal<bool>,
    to_add: Memo<(usize, String)>,
) -> impl IntoView {
    let note = move || {
        let file_id = current_file.get()?;
        let (n, tag) = to_add.get();
        let existing = state.annotations.store().with(|store| {
            store
                .get(file_id)
                .map(|set| {
                    set.annotations
                        .iter()
                        .filter(|a| summary::has_auto_tag(a, &tag))
                        .count()
                })
                .unwrap_or(0)
        });
        (existing > 0 && n > 0).then(|| {
            let text = format!(
                "Replaces {} tagged #auto={tag}, including any you edited. Undo with {}.",
                count(existing, "annotation", "annotations"),
                undo_keys()
            );
            view! { <div class="detection-status-note detection-annotate-row">{text}</div> }
        })
    };
    let option = move |value: bool, label: &'static str| {
        view! {
            <label class="setting-label detection-radio">
                <input
                    type="radio"
                    name="classifier-annotate-merge"
                    prop:checked=move || merge.get() == value
                    on:change=move |_| merge.set(value)
                />
                {label}
            </label>
        }
    };
    view! {
        <div class="detection-annotate-row detection-radio-group">
            {option(false, "One per window")}
            {option(true, "Merge neighbours")}
        </div>
        {note}
        <button
            class="psd-btn cd-add-btn"
            disabled=move || to_add.get().0 == 0
            on:click=move |_| {
                if let Some(file_id) = current_file.get_untracked() {
                    add_annotations(state, file_id, method, merge.get_untracked());
                }
            }
        >{move || match to_add.get().0 {
            0 => "Add annotations".to_string(),
            n => format!("Add {}", count(n, "annotation", "annotations")),
        }}</button>
    }
}
