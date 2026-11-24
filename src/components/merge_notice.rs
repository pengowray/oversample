//! The notice shown under the open recording's "Annotations" heading after
//! its annotations were changed both in this browser and in the Google
//! Drive copy, and the two were merged (`cloud::sync::merge_copies`). It
//! stays until dismissed.
//!
//! Order: what happened; a warning when deleted annotations may be back;
//! what the Drive copy brought in; the changes both copies made, the ones
//! where this browser's version was dropped first (in amber, as they are
//! the only losses); where this browser's copy from before the merge can be
//! found. A clean merge (nothing dropped) shows the heading and the summary
//! only.

use oversample_core::erased_sort::SortErased;
use crate::cloud::sync::{self, ConflictWhat, MergeNotice, NoticeConflict};
use crate::state::store_fields::*;
use crate::state::AppState;
use leptos::prelude::*;

const HEADING: &str = "Merged with the Google Drive copy";
const DISMISS: &str = "Dismiss";
/// Followed by " from <time>" when known.
fn before_in_history() -> &'static str {
    if crate::cloud::store::android_app() {
        "This device's annotations from before the merge are saved in Google Drive as an earlier version"
    } else {
        "This browser's annotations from before the merge are saved in Google Drive as an earlier version"
    }
}
/// How to reach Drive's version history. The link opens the file's page in
/// Drive.
const OPEN_IN_DRIVE: &str = "Open in Google Drive";
const VERSIONS_PATH: &str = "then File information \u{203A} Manage versions.";
const WITHOUT_BASE: &str = "Annotations you deleted may have come back. All annotations from both \
     copies were kept, because the app couldn't tell which ones had been deleted.";
/// Conflict rows shown before "Show all".
const FIRST_ROWS: usize = 3;

/// "From Google Drive: 3 annotations added, 1 removed, 2 changed", leaving
/// out zero parts; the noun goes with the first part. `None` when all are
/// zero.
fn summary(n: &MergeNotice) -> Option<String> {
    let parts: Vec<(usize, &str)> = [
        (n.added, "added"),
        (n.removed, "removed"),
        (n.changed, "changed"),
    ]
    .into_iter()
    .filter(|(c, _)| *c > 0)
    .collect();
    let mut out = String::from("From Google Drive: ");
    for (i, (count, verb)) in parts.iter().enumerate() {
        if i == 0 {
            let noun = if *count == 1 {
                "annotation"
            } else {
                "annotations"
            };
            out.push_str(&format!("{count} {noun} {verb}"));
        } else {
            out.push_str(&format!(", {count} {verb}"));
        }
    }
    (!parts.is_empty()).then_some(out)
}

/// The name shown for a program's annotations (`auto=…`, `xc:set=…`).
fn run_name(tag: &str) -> String {
    match tag.split_once('=') {
        Some(("auto", v)) if oversample_core::dsp::call_detect::is_detector_id(v) => {
            "Call detection".to_string()
        }
        Some(("auto", v)) => format!("Automatic annotations ({v})"),
        Some(("xc:set", _)) => "Xeno-canto annotations".to_string(),
        _ => tag.to_string(),
    }
}

/// A time as the browser shows local times, to find the version in Drive's
/// list ("18 Nov, 14:02").
fn local_time(iso: &str) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(iso));
    let opts = js_sys::Object::new();
    for (k, v) in [
        ("day", "numeric"),
        ("month", "short"),
        ("hour", "2-digit"),
        ("minute", "2-digit"),
    ] {
        let _ = js_sys::Reflect::set(&opts, &k.into(), &v.into());
    }
    d.to_locale_string("default", &opts)
        .as_string()
        .unwrap_or_else(|| iso.to_string())
}

/// One conflict row, and whether it is a loss here (amber).
fn row_text(c: &NoticeConflict) -> (String, bool) {
    let here = crate::cloud::store::here();
    let here_s = crate::cloud::store::heres();
    let whose = if c.kept_here {
        here_s
    } else {
        "Google Drive's"
    };
    let text = match c.what {
        ConflictWhat::BothChanged => format!("{}: changed in both. Kept {whose} change", c.name),
        ConflictWhat::ChangedAndDeleted if c.kept_here => format!(
            "{}: deleted in Google Drive but changed {here}, so it was kept",
            c.name
        ),
        ConflictWhat::ChangedAndDeleted => format!(
            "{}: deleted {here} but changed in Google Drive, so it was kept",
            c.name
        ),
        ConflictWhat::RunTwice if c.kept_here => format!(
            "{}: run in both. Kept {here_s} results; removed Google Drive's {} annotations",
            run_name(&c.name),
            c.count
        ),
        ConflictWhat::RunTwice => format!(
            "{}: run in both. Kept Google Drive's results; removed {here_s} {} annotations",
            run_name(&c.name),
            c.count
        ),
        ConflictWhat::LowestBatHz => {
            format!("Lowest bat frequency: changed in both. Kept {whose} value")
        }
        ConflictWhat::NoiseProfile => {
            format!("Noise profile: changed in both. Kept {whose} profile")
        }
    };
    // A deletion made here that was undone is not a loss of work.
    let lost_here = !c.kept_here && c.what != ConflictWhat::ChangedAndDeleted;
    (text, lost_here)
}

#[component]
pub fn MergeNoticeView() -> impl IntoView {
    let state = expect_context::<AppState>();
    let show_all = RwSignal::new(false);
    let key = Memo::new(move |_| {
        let idx = state.library.current_index().get()?;
        state.library.files().with(|files| {
            let f = files.get(idx)?;
            if f.read_only || crate::file_identity::spot_hash_pending(f.id) {
                return None;
            }
            f.identity.as_ref().map(crate::opfs::opfs_key)
        })
    });
    move || {
        let key = key.get()?;
        let notice = sync::merge_notice(&key)?;
        let mut rows: Vec<(String, bool)> = notice.conflicts.iter().map(row_text).collect();
        // Losses here first, so the short list shows them.
        rows.sort_erased_by_key(|(_, lost)| !*lost);
        let total = rows.len();
        let shown = if show_all.get() {
            total
        } else {
            total.min(FIRST_ROWS)
        };
        let count_line = (total > 0).then(|| format!("Changed in both copies ({total}):"));
        let link = notice.drive_link.clone();
        let before_at = notice
            .before_at
            .as_deref()
            .map(|t| format!(" from {}", local_time(t)));
        let dismiss_key = key.clone();
        Some(view! {
            <div class="merge-notice" role="status">
                <div class="merge-notice-heading">{HEADING}</div>
                {notice.without_base.then(|| view! { <div class="merge-notice-row lost">{WITHOUT_BASE}</div> })}
                {summary(&notice).map(|s| view! { <div>{s}</div> })}
                {count_line.map(|c| view! { <div class="merge-notice-count">{c}</div> })}
                {rows.into_iter().take(shown).map(|(text, lost)| {
                    let class = if lost { "merge-notice-row lost" } else { "merge-notice-row" };
                    view! { <div class=class>{text}</div> }
                }).collect_view()}
                {(total > FIRST_ROWS).then(|| view! {
                    <button class="merge-notice-more" on:click=move |_| show_all.update(|v| *v = !*v)>
                        {move || if show_all.get() {
                            "Show fewer".to_string()
                        } else {
                            format!("Show all {total}")
                        }}
                    </button>
                })}
                {notice.before_in_history.then(|| view! {
                    <div>
                        {before_in_history()}{before_at}". "
                        {match link {
                            Some(href) => view! {
                                <a href=href target="_blank" rel="noopener noreferrer">{OPEN_IN_DRIVE}</a>
                            }.into_any(),
                            None => view! { <span>{OPEN_IN_DRIVE}</span> }.into_any(),
                        }}
                        ", "{VERSIONS_PATH}
                    </div>
                })}
                <button
                    class="sidebar-btn annotation-toolbar-btn merge-notice-dismiss"
                    on:click=move |_| {
                        show_all.set(false);
                        sync::dismiss_merge_notice(&dismiss_key);
                    }
                >{DISMISS}</button>
            </div>
        })
    }
}
