//! The search under the "In this recording" list: type part of a name (or
//! "PC"), then pick a match to add its row above the spectrogram.

use super::event_target_value;
use super::rows::{component_picked, strip_picked, toggle_component, toggle_strip};
use crate::classifier::{self, Method, MethodFacts};
use crate::state::{AppState, ClassifierStateStoreFields};
use leptos::prelude::*;

/// Most matches the search lists at once.
const SEARCH_MATCHES: usize = 8;

/// A run's label list, compared by identity: a memo of it changes only when
/// another run's labels come in, not each time a window's scores do.
#[derive(Clone)]
struct LabelList(std::sync::Arc<Vec<classifier::SpeciesLabel>>);

impl LabelList {
    fn key(&self) -> usize {
        std::sync::Arc::as_ptr(&self.0) as usize
    }
}

/// Each label's common and scientific name, folded for matching
/// (`classifier::strips::fold_labels`).
type FoldedNames = std::sync::Arc<Vec<(String, String)>>;

/// One row the search offers: a species, or one of the model's
/// components.
#[derive(Clone, PartialEq)]
struct SearchMatch {
    name: String,
    /// Empty for a component.
    scientific: String,
    component: Option<usize>,
    added: bool,
}

/// The components a query asks for: "pc" lists them all, "pc1" those whose
/// number starts with 1. Empty for any other query.
fn component_matches(query: &str, n: usize) -> Vec<usize> {
    let q = query.trim().to_lowercase();
    let Some(rest) = q.strip_prefix("pc") else {
        return Vec::new();
    };
    let rest = rest.trim();
    if !rest.chars().all(|c| c.is_ascii_digit()) {
        return Vec::new();
    }
    (0..n)
        .filter(|k| (k + 1).to_string().starts_with(rest))
        .collect()
}

/// The last row of the list: type part of a common or scientific name (or
/// "PC" for a model's components), then pick a match (or press Enter
/// for the highlighted one) to add its row above the spectrogram. Searches
/// every class of the current file's run for this method, so a species the
/// run never scored highly can be added. Matches are listed in the panel
/// under the field rather than in a popup, which the narrow sidebar would
/// clip.
pub(super) fn strip_search(
    state: AppState,
    method: Method,
    current_file: Memo<Option<u64>>,
) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let focused = RwSignal::new(false);
    let highlight = RwSignal::new(0usize);
    let with_components = method.info().component_rows;
    let labels = Memo::new_with_compare(
        move |_| {
            let file = current_file.get()?;
            state
                .classifier
                .results()
                .with(|m| m.get(&(file, method)).map(|r| LabelList(r.labels.clone())))
        },
        |a, b| {
            a.map(|a| a.as_ref().map(LabelList::key)) != b.map(|b| b.as_ref().map(LabelList::key))
        },
    );
    // Folded names of the label list, built on the first search of each list
    // (Perch has 14,795 classes): (list key, names).
    let folded: StoredValue<Option<(usize, FoldedNames)>> = StoredValue::new(None);
    let matches = Memo::new(move |_| -> Vec<SearchMatch> {
        let q = query.get();
        if q.trim().is_empty() {
            return Vec::new();
        }
        let mut out = Vec::new();
        if with_components {
            let n = crate::components::score_strips::components_for(method)
                .map_or(0, |p| p.variance.len());
            out.extend(component_matches(&q, n).into_iter().map(|k| SearchMatch {
                name: format!("PC{}", k + 1),
                scientific: String::new(),
                component: Some(k),
                added: component_picked(state, k),
            }));
        }
        let Some(list) = labels.get() else {
            return out;
        };
        let cached = folded.with_value(|f| {
            f.as_ref()
                .filter(|(k, _)| *k == list.key())
                .map(|(_, n)| n.clone())
        });
        let names = cached.unwrap_or_else(|| {
            let n = std::sync::Arc::new(classifier::strips::fold_labels(&list.0));
            folded.set_value(Some((list.key(), n.clone())));
            n
        });
        let room = SEARCH_MATCHES.saturating_sub(out.len());
        out.extend(
            classifier::strips::find_classes(&list.0, &names, &q, room)
                .into_iter()
                .filter_map(|c| list.0.get(c as usize))
                .map(|l| SearchMatch {
                    name: classifier::strips::display_name(l),
                    scientific: l.scientific.clone(),
                    component: None,
                    added: strip_picked(state, method, &l.scientific),
                }),
        );
        out
    });
    let open = move || focused.get() && !query.with(|q| q.trim().is_empty());
    let choose = move |m: &SearchMatch| {
        if !m.added {
            match m.component {
                Some(k) => toggle_component(state, k),
                None => toggle_strip(state, method, m.scientific.clone()),
            }
        }
        query.set(String::new());
        highlight.set(0);
    };
    let list_id = format!("strip-search-list-{method:?}");
    let option_id = move |i: usize| format!("strip-search-{method:?}-{i}");
    let on_keydown = move |ev: web_sys::KeyboardEvent| {
        let n = matches.with_untracked(Vec::len);
        match ev.key().as_str() {
            "ArrowDown" => {
                ev.prevent_default();
                highlight.update(|h| *h = (*h + 1).min(n.saturating_sub(1)));
            }
            "ArrowUp" => {
                ev.prevent_default();
                highlight.update(|h| *h = h.saturating_sub(1));
            }
            "Enter" => {
                ev.prevent_default();
                let m = matches.with_untracked(|v| v.get(highlight.get_untracked()).cloned());
                if let Some(m) = m {
                    choose(&m);
                }
            }
            "Escape" if !query.with_untracked(String::is_empty) => {
                ev.prevent_default();
                ev.stop_propagation();
                query.set(String::new());
                highlight.set(0);
            }
            _ => {}
        }
    };
    let list_view = {
        let list_id = list_id.clone();
        move || {
            if !open() {
                return None;
            }
            let found = matches.get();
            if found.is_empty() {
                return Some(
                    view! { <div class="strip-search-empty">"No species found"</div> }.into_any(),
                );
            }
            let rows = found
                .into_iter()
                .enumerate()
                .map(|(i, m)| {
                    let name = m.name.clone();
                    let scientific = (!m.scientific.is_empty() && m.scientific != m.name)
                        .then(|| m.scientific.clone());
                    let added = m.added;
                    let class = move || {
                        if highlight.get() == i {
                            "strip-search-option highlight"
                        } else {
                            "strip-search-option"
                        }
                    };
                    view! {
                        <div
                            id=option_id(i)
                            role="option"
                            class=class
                            aria-selected=move || (highlight.get() == i).to_string()
                            on:mousedown=move |ev: web_sys::MouseEvent| {
                                // Keeps the focus in the field, for another search.
                                ev.prevent_default();
                                choose(&m);
                            }
                            on:mousemove=move |_| {
                                if highlight.get_untracked() != i {
                                    highlight.set(i);
                                }
                            }
                        >
                            <span class="strip-search-names">
                                <span>{name}</span>
                                {scientific.map(|s| view! { <span class="detection-scientific">{s}</span> })}
                            </span>
                            {added.then(|| view! { <span class="strip-search-added">"Added"</span> })}
                        </div>
                    }
                })
                .collect::<Vec<_>>();
            Some(
                view! { <div id=list_id.clone() role="listbox" class="strip-search-list">{rows}</div> }
                    .into_any(),
            )
        }
    };
    let placeholder = if with_components {
        "Add a species or component"
    } else {
        "Add a species"
    };
    move || {
        labels
            .with(|l| l.as_ref().is_some_and(|l| !l.0.is_empty()))
            .then(|| {
                let list_id = list_id.clone();
                let list_view = list_view.clone();
                view! {
                    <div class="strip-search">
                        <input
                            type="text"
                            class="strip-search-input"
                            placeholder=placeholder
                            aria-label="Add a row above the spectrogram"
                            role="combobox"
                            aria-autocomplete="list"
                            aria-controls=list_id
                            aria-expanded=move || open().to_string()
                            aria-activedescendant=move || {
                                (open() && matches.with(|v| !v.is_empty()))
                                    .then(|| option_id(highlight.get()))
                            }
                            autocomplete="off"
                            spellcheck="false"
                            prop:value=move || query.get()
                            on:input=move |ev| {
                                query.set(event_target_value(&ev));
                                highlight.set(0);
                            }
                            on:focus=move |_| focused.set(true)
                            on:blur=move |_| focused.set(false)
                            on:keydown=on_keydown
                        />
                        {list_view}
                    </div>
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::component_matches;

    #[test]
    fn components_by_name() {
        assert_eq!(component_matches("PC", 12).len(), 12);
        assert_eq!(component_matches("pc1", 12), vec![0, 9, 10, 11]);
        assert_eq!(component_matches(" pc 3", 12), vec![2]);
        assert!(component_matches("pc1x", 12).is_empty());
        assert!(component_matches("pied", 12).is_empty());
        assert!(component_matches("p", 12).is_empty());
    }
}
