//! Form for saving the current recording's annotations as Annota-JSON, the
//! file Xeno-canto takes as an annotation upload. The conversion and the list
//! of problems XC would reject or warn about are in `crate::annota`.

use leptos::prelude::*;

use crate::annota::{export_set, ExportContext, ExportOutcome};
use crate::annotations::AnnotationId;
use crate::state::store_fields::*;
use crate::state::AppState;

/// localStorage keys for the fields people fill in once.
const ANNOTATOR_KEY: &str = "oversample_annota_annotator";
const LICENSE_KEY: &str = "oversample_annota_license";

/// XC number of a loaded file: from its XC page URL in the metadata, or from a
/// file name that starts with `XC<digits>` (as XC downloads do).
pub(crate) fn xc_number_of(name: &str, metadata: Option<&[(String, String)]>) -> Option<String> {
    let from_url = metadata.and_then(|m| {
        m.iter()
            .filter(|(k, _)| k == "URL" || k == "Attribution")
            .find_map(|(_, v)| {
                let idx = v.find("xeno-canto.org/")?;
                let digits: String = v[idx + "xeno-canto.org/".len()..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                (!digits.is_empty()).then_some(digits)
            })
    });
    from_url.or_else(|| {
        let rest = name
            .strip_prefix("XC")
            .or_else(|| name.strip_prefix("xc"))?;
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        (!digits.is_empty()).then_some(digits)
    })
}

fn metadata_value(metadata: Option<&[(String, String)]>, key: &str) -> Option<String> {
    metadata?
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn non_empty(s: String) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

#[component]
pub fn XcAnnotationExport(on_close: Callback<()>) -> impl IntoView {
    let state = expect_context::<AppState>();

    // Fill the fields from the open file once, when the form opens.
    let (file_name, metadata) = state
        .library
        .current_index()
        .get_untracked()
        .and_then(|idx| {
            state.library.files().with_untracked(|files| {
                files
                    .get(idx)
                    .map(|f| (f.name.clone(), f.xc_metadata.clone()))
            })
        })
        .unwrap_or_default();
    let xc_nr = RwSignal::new(xc_number_of(&file_name, metadata.as_deref()).unwrap_or_default());
    let species =
        RwSignal::new(metadata_value(metadata.as_deref(), "Scientific name").unwrap_or_default());
    let annotator = RwSignal::new(crate::settings::get_raw(ANNOTATOR_KEY).unwrap_or_default());
    let license = RwSignal::new(
        crate::settings::get_raw(LICENSE_KEY).unwrap_or_else(|| "CC-BY-4.0".to_string()),
    );
    let set_name = RwSignal::new(if xc_nr.get_untracked().is_empty() {
        format!("Annotations for {file_name}")
    } else {
        format!("Annotations for XC{}", xc_nr.get_untracked())
    });
    let only_selected = RwSignal::new(false);

    let selected_count = move || state.annotations.selected_ids().with(|ids| ids.len());

    let file_name_for_memo = file_name.clone();
    let outcome = Memo::new(move |_| -> Option<ExportOutcome> {
        let file_id = state.current_file_id()?;
        let selected: Vec<AnnotationId> = state.annotations.selected_ids().get();
        let use_selected = only_selected.get() && !selected.is_empty();
        let ctx = ExportContext {
            xc_nr: non_empty(xc_nr.get())
                .map(|s| s.trim_start_matches(['X', 'C', 'x', 'c']).to_string()),
            sound_file: non_empty(file_name_for_memo.clone()),
            default_species: non_empty(species.get()),
            annotator: non_empty(annotator.get()),
            set_name: non_empty(set_name.get()),
            license: non_empty(license.get()),
            software: format!("Oversample {}", env!("CARGO_PKG_VERSION")),
        };
        state.annotations.store().with(|store| {
            let anns = store
                .get(file_id)
                .map(|s| s.annotations.as_slice())
                .unwrap_or(&[]);
            Some(export_set(
                anns,
                use_selected.then_some(selected.as_slice()),
                &ctx,
            ))
        })
    });

    let download = move |_| {
        let Some(out) = outcome.get_untracked() else {
            return;
        };
        if let Some(a) = non_empty(annotator.get_untracked()) {
            crate::settings::set_raw(ANNOTATOR_KEY, &a);
        }
        crate::settings::set_raw(LICENSE_KEY, &license.get_untracked());
        let json = match serde_json::to_string_pretty(&out.set) {
            Ok(j) => j,
            Err(e) => {
                state.show_error_toast(format!("Couldn't write the file: {e}"));
                return;
            }
        };
        let nr = non_empty(xc_nr.get_untracked());
        let filename = match nr {
            Some(n) => format!(
                "XC{}-annotations.json",
                n.trim_start_matches(['X', 'C', 'x', 'c'])
            ),
            None => {
                let stem = file_name
                    .rsplit_once('.')
                    .map(|(s, _)| s)
                    .unwrap_or(&file_name);
                format!("{stem}-annotations.json")
            }
        };
        crate::audio::export::save_exports(
            &state,
            vec![(filename.clone(), json.into_bytes())],
            false,
        );
        if !state.is_tauri {
            state.show_info_toast(format!("Saved {filename}"));
        }
    };

    let field = |label: &'static str, sig: RwSignal<String>, placeholder: &'static str| {
        view! {
            <label class="annota-label">{label}</label>
            <input
                class="annota-input"
                type="text"
                placeholder=placeholder
                prop:value=move || sig.get()
                on:input=move |ev| sig.set(event_target_value(&ev))
            />
        }
    };

    view! {
        <div class="annota-export">
            <div class="annota-heading">"Export for Xeno-canto"</div>
            <div class="annota-grid">
                {field("XC number", xc_nr, "XC928094")}
                {field("Species (scientific name)", species, "Genus species")}
                <div class="annota-help">"Used for every annotation without a #species tag. Filled in from the recording's XC details when available."</div>
                {field("Annotator", annotator, "")}
                <label class="annota-label">"Licence"</label>
                <select
                    class="annota-input"
                    on:change=move |ev| license.set(event_target_value(&ev))
                >
                    {oversample_ipc::annota::LICENSES
                        .iter()
                        .map(|l| {
                            let l = *l;
                            view! {
                                <option value=l selected=move || license.get() == l>{l}</option>
                            }
                        })
                        .collect_view()}
                </select>
                {field("Set name", set_name, "")}
            </div>
            {move || (selected_count() > 0).then(|| {
                let n = selected_count();
                let total = state.current_file_id().map(|id| {
                    state.annotations.store().with(|store| {
                        store.get(id).map(|s| s.annotations.iter()
                            .filter(|a| matches!(a.kind, crate::annotations::AnnotationKind::Region(_)))
                            .count()).unwrap_or(0)
                    })
                }).unwrap_or(0);
                view! {
                    <div class="annota-scope">
                        <label class="annota-check">
                            <input
                                type="radio"
                                name="annota-scope"
                                prop:checked=move || !only_selected.get()
                                on:change=move |_| only_selected.set(false)
                            />
                            {format!("All annotations ({total})")}
                        </label>
                        <label class="annota-check">
                            <input
                                type="radio"
                                name="annota-scope"
                                prop:checked=move || only_selected.get()
                                on:change=move |_| only_selected.set(true)
                            />
                            {format!("Selected only ({n})")}
                        </label>
                    </div>
                }
            })}
            {move || outcome.get().map(|out| {
                let n = out.set.annotations.len();
                let skipped = out.from_xc_skipped;
                let plural = |k: usize| if k == 1 { "annotation" } else { "annotations" };
                let mut summary = format!("{n} {} will be exported.", plural(n));
                if skipped > 0 {
                    summary.push_str(&format!(
                        " {skipped} {} imported from XC {} left out because XC already has them.",
                        plural(skipped),
                        if skipped == 1 { "is" } else { "are" }
                    ));
                }
                let blocking: Vec<String> =
                    out.issues.iter().filter(|i| i.blocking).map(|i| i.message.clone()).collect();
                let warnings: Vec<String> =
                    out.issues.iter().filter(|i| !i.blocking).map(|i| i.message.clone()).collect();
                view! {
                    <div class="annota-summary">{summary}</div>
                    {(!blocking.is_empty()).then(|| view! {
                        <div class="annota-issues blocking">
                            <div class="annota-issues-title">"XC will reject this file:"</div>
                            <ul>{blocking.into_iter().map(|m| view! { <li>{m}</li> }).collect_view()}</ul>
                        </div>
                    })}
                    {(!warnings.is_empty()).then(|| view! {
                        <div class="annota-issues">
                            <div class="annota-issues-title">"XC will accept this file with warnings:"</div>
                            <ul>{warnings.into_iter().map(|m| view! { <li>{m}</li> }).collect_view()}</ul>
                        </div>
                    })}
                }
            })}
            <div class="annota-actions">
                <button class="sidebar-btn annota-btn" on:click=download>"Save file"</button>
                <button class="sidebar-btn annota-btn annota-btn-quiet" on:click=move |_| on_close.run(())>"Close"</button>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xc_number_from_url_or_name() {
        let meta = vec![(
            "URL".to_string(),
            "https://www.xeno-canto.org/928094".to_string(),
        )];
        assert_eq!(
            xc_number_of("whatever.wav", Some(&meta)).as_deref(),
            Some("928094")
        );
        assert_eq!(
            xc_number_of("XC1048144 - Noctule.wav", None).as_deref(),
            Some("1048144")
        );
        assert_eq!(xc_number_of("bat.wav", None), None);
    }
}
