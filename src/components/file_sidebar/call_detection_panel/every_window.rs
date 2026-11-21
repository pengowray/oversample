//! The "Every window" fold: the windows at or above the minimum confidence,
//! in time order.

use super::rows::{class_name, flag_cell, flag_header, show_window, species_swatch};
use crate::classifier::summary::{self, WindowKind};
use crate::classifier::{self, Method};
use crate::state::store_fields::*;
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;

/// One row of the "Every window" list.
#[derive(Clone, PartialEq)]
struct WindowRow {
    /// The window's number, when windows do not overlap.
    number: Option<usize>,
    start: f64,
    end: f64,
    label: Option<classifier::SpeciesLabel>,
    /// The species' place in `summary::colour_order`; `None` for a class
    /// that is not a species.
    slot: Option<usize>,
    other: bool,
    confidence: f32,
}

impl WindowRow {
    /// Everything the row shows, so a row is drawn again only when it changes.
    fn key(&self) -> (u64, Option<usize>, Option<usize>, bool, u32, String) {
        (
            self.start.to_bits(),
            self.number,
            self.slot,
            self.other,
            self.confidence.to_bits(),
            self.label
                .as_ref()
                .map_or_else(String::new, |l| l.scientific.clone()),
        )
    }
}

/// The "Every window" list: windows at or above the minimum, in time order.
/// The rows are worked out in a memo and drawn by key, so readings that add
/// no row (below the minimum) redraw nothing, and a new row adds one row.
pub(super) fn window_table(
    state: AppState,
    method: Method,
    current_file: Memo<Option<u64>>,
) -> impl IntoView {
    let rows = Memo::new(move |_| {
        let min = classifier::min_confidence(state, method);
        state.classifier.results().with(|m| {
            let Some((file, run)) = current_file
                .get()
                .and_then(|f| m.get(&(f, method)).map(|r| (f, r)))
            else {
                return (true, Vec::new());
            };
            // With overlapping windows the numbers change as a run adds
            // windows between others, so the column is left out.
            let numbered = !run.overlapping();
            let order = classifier::kept_order::colour_order(file, method, run, min);
            let rows = run
                .windows
                .iter()
                .enumerate()
                .filter_map(|(i, w)| {
                    let kind = summary::window_kind(run, i, min);
                    if kind == WindowKind::Below {
                        return None;
                    }
                    let top = w.top[0];
                    let other = kind == WindowKind::NotSpecies;
                    Some(WindowRow {
                        number: numbered.then_some(i + 1),
                        start: w.start_s,
                        end: w.end_s,
                        label: run.label(&top).cloned(),
                        slot: (!other)
                            .then(|| order.iter().position(|&c| c == top.class))
                            .flatten(),
                        other,
                        confidence: top.confidence,
                    })
                })
                .collect::<Vec<_>>();
            (numbered, rows)
        })
    });
    let numbered = move || rows.with(|r| r.0);
    let row = move |r: WindowRow| {
        let (start, end, other) = (r.start, r.end, r.other);
        let class = move || {
            let mut c = String::from("detection-row");
            if other {
                c.push_str(" detection-row-other");
            }
            if state.call_detection.selected_window().get() == Some(start) {
                c.push_str(" selected");
            }
            c
        };
        let scientific = r
            .label
            .as_ref()
            .map(|l| l.scientific.clone())
            .unwrap_or_default();
        let flag = flag_cell(
            state,
            method,
            current_file.get_untracked(),
            r.label.as_ref(),
        );
        view! {
            <tr class=class title=scientific on:click=move |_| show_window(state, start, end)>
                {r.number.map(|n| view! { <td class="detection-col-num">{n}</td> })}
                <td>{crate::format_time::format_time_display(start, 1)}</td>
                <td class="detection-col-kind">
                    <span class="cd-name-line">{species_swatch(r.slot)}<span>{class_name(r.label.as_ref())}</span></span>
                </td>
                {flag}
                <td class="detection-col-num">{format!("{:.2}", r.confidence)}</td>
            </tr>
        }
    };
    view! {
        <div class="detection-list">
            <table class="detection-table">
                <thead>
                    <tr>
                        {move || numbered().then(|| view! { <th class="detection-col-num">"#"</th> })}
                        <th>"Start"</th>
                        <th>"Species"</th>
                        {flag_header(method)}
                        <th class="detection-col-num" title="Confidence">"Conf."</th>
                    </tr>
                </thead>
                <tbody>
                    <For each=move || rows.with(|r| r.1.clone()) key=WindowRow::key children=row />
                </tbody>
            </table>
        </div>
    }
}
