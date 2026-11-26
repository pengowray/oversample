//! The "Add N annotations" fold of Birds and bats: the boxes it found
//! (`classifier::combined::found_boxes`) added as annotations, each with its
//! time and frequency range, tagged `#auto=birds-and-bats` so the next add
//! replaces them.

use super::parts::Fold;
use super::{count, event_target_checked, undo_keys};
use crate::annotations::{now_iso8601, Annotation, AnnotationId, AnnotationKind, AnnotationSet, Region};
use crate::call_annotations::remove_annotations_where;
use crate::canvas::box_class::BoxClass;
use crate::classifier::combined::{found_boxes, FoundBox};
use crate::classifier::summary::has_auto_tag;
use crate::state::store_fields::*;
use crate::state::AppState;
use leptos::prelude::*;
use oversample_core::annotation_tags::{Tag, AUTO};

/// The `auto` tag's value on the annotations added here.
const TAG: &str = "birds-and-bats";

fn wanted(boxes: &[FoundBox], noise: bool) -> Vec<FoundBox> {
    boxes
        .iter()
        .filter(|b| noise || b.class == BoxClass::Animal)
        .cloned()
        .collect()
}

fn build(boxes: &[FoundBox], now: &str) -> Vec<Annotation> {
    let tag = Tag::with_value(AUTO, TAG).to_stored();
    boxes
        .iter()
        .enumerate()
        .map(|(order, b)| Annotation {
            id: AnnotationId::new(),
            kind: AnnotationKind::Region(Region {
                time_start: b.t0,
                time_end: b.t1,
                freq_low: Some(b.f0),
                freq_high: Some(b.f1),
                label: Some(b.label.clone()),
                color: None,
                locked: None,
            }),
            created_at: now.to_string(),
            modified_at: now.to_string(),
            notes: b.scientific.clone(),
            parent_id: None,
            sort_order: Some(order as f64),
            tags: vec![tag.clone()],
            label_default: None,
            play: None,
        })
        .collect()
}

/// Add `boxes` to file `file_id`'s annotations, replacing those added here
/// before.
fn add(state: AppState, file_id: u64, boxes: Vec<FoundBox>) {
    if boxes.is_empty() {
        return;
    }
    let annotations = build(&boxes, &now_iso8601());
    let new_set = state.library.files().with_untracked(|files| {
        files.iter().find(|f| f.id == file_id).map(|f| {
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
        removed = remove_annotations_where(&mut set.annotations, |a| has_auto_tag(a, TAG));
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

/// The fold, for the current file; nothing before anything is found.
pub(super) fn annotate_fold(state: AppState, current_file: Memo<Option<u64>>) -> impl IntoView {
    let noise = RwSignal::new(false);
    let boxes = Memo::new(move |_| {
        current_file
            .get()
            .map(|f| found_boxes(state, f))
            .unwrap_or_default()
    });
    let to_add = Memo::new(move |_| boxes.with(|b| wanted(b, noise.get()).len()));
    let existing = Memo::new(move |_| {
        let Some(file) = current_file.get() else {
            return 0;
        };
        state.annotations.store().with(|store| {
            store
                .get(file)
                .map(|set| set.annotations.iter().filter(|a| has_auto_tag(a, TAG)).count())
                .unwrap_or(0)
        })
    });
    let title = Signal::derive(move || match to_add.get() {
        0 => "Add annotations".to_string(),
        n => format!("Add {}", count(n, "annotation", "annotations")),
    });
    let body = move || {
        view! {
            <div class="detection-annotate-row">
                <label class="setting-label">
                    <input
                        type="checkbox"
                        prop:checked=move || noise.get()
                        on:change=move |ev| noise.set(event_target_checked(&ev))
                    />
                    " Include noise"
                </label>
            </div>
            {move || {
                let (e, n) = (existing.get(), to_add.get());
                (e > 0 && n > 0).then(|| view! {
                    <div class="detection-status-note detection-annotate-row">{format!(
                        "Replaces {} tagged #auto={TAG}, including any you edited. Undo with {}.",
                        count(e, "annotation", "annotations"),
                        undo_keys()
                    )}</div>
                })
            }}
            <button
                class="psd-btn cd-add-btn"
                disabled=move || to_add.get() == 0
                on:click=move |_| {
                    if let Some(file) = current_file.get_untracked() {
                        add(state, file, boxes.with_untracked(|b| wanted(b, noise.get_untracked())));
                    }
                }
            >{move || title.get()}</button>
        }
    };
    move || {
        boxes.with(|b| !b.is_empty()).then(|| {
            view! {
                <div class="cd-folds">
                    <Fold name="combined-annotate" title=title>{body()}</Fold>
                </div>
            }
        })
    }
}
