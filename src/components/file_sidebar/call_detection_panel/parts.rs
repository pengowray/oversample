//! Controls shared by every method in the Call detection tab: the settings
//! button and the line under it that lists the settings in force, the pin
//! that shows a row above the spectrogram, the folds, and the section
//! headings.

use crate::components::icons::Icon;
use leptos::prelude::*;

/// The gear button beside the method's main action. It opens the settings
/// inline, under the line that lists them, so they are never cut off at the
/// sidebar's edge and never sit behind another layer.
pub(super) fn settings_button(open: RwSignal<bool>) -> impl IntoView {
    view! {
        <button
            class="cd-gear"
            class:open=move || open.get()
            title="Settings"
            aria-label="Settings"
            aria-expanded=move || open.get().to_string()
            on:click=move |_| open.update(|v| *v = !*v)
        >
            <Icon kind=Icon::Settings />
        </button>
    }
}

/// The line listing the settings in force: facts separated by middle dots,
/// each kept on one line so the line wraps between facts. A press on it
/// opens or closes the settings, like the gear.
pub(super) fn in_force_line(
    open: RwSignal<bool>,
    facts: impl Fn() -> Vec<String> + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <button
            class="cd-in-force"
            aria-expanded=move || open.get().to_string()
            title="Settings"
            on:click=move |_| open.update(|v| *v = !*v)
        >
            {move || {
                facts()
                    .into_iter()
                    .map(|f| view! { <span class="cd-fact">{f}</span> })
                    .collect::<Vec<_>>()
            }}
        </button>
    }
}

/// The pin on a row: shows the row's species (or component, or the sound
/// clusters) in a row above the spectrogram, or removes it. A press on it
/// leaves the row's own click alone.
pub(super) fn pin_button(
    name: String,
    pinned: impl Fn() -> bool + Clone + Send + Sync + 'static,
    toggle: impl Fn() + 'static,
) -> impl IntoView {
    let (p1, p2, p3) = (pinned.clone(), pinned.clone(), pinned);
    let label = {
        let name = name.clone();
        move || {
            if p3() {
                format!("Remove {name} from above the spectrogram")
            } else {
                format!("Show {name} above the spectrogram")
            }
        }
    };
    view! {
        <button
            class="cd-pin"
            class:on=move || p1()
            aria-pressed=move || p2().to_string()
            aria-label=label.clone()
            title=label
            on:click=move |ev: web_sys::MouseEvent| {
                ev.stop_propagation();
                toggle();
            }
        >
            <Icon kind=Icon::Pin />
        </button>
    }
}

/// A section heading with a fact on its right ("In this recording", "2
/// species, 16 windows").
pub(super) fn section_head(
    title: &'static str,
    right: impl IntoView + 'static,
    right_title: impl Fn() -> String + Send + Sync + 'static,
) -> impl IntoView {
    view! {
        <div class="cd-head">
            <span class="cd-head-title">{title}</span>
            <span class="cd-head-fact" title=right_title>{right}</span>
        </div>
    }
}

/// A fold: a heading that opens and closes what is under it. Closed until
/// opened; whether it is open is remembered on this device under `name`.
/// With `keep`, the content stays in the page while closed (hidden), for
/// content drawn by a script into its own element.
#[component]
pub(super) fn Fold(
    /// The name the open state is saved under.
    name: &'static str,
    #[prop(into)] title: Signal<String>,
    /// A grey tag after the title ("Experimental").
    #[prop(optional)]
    tag: Option<&'static str>,
    #[prop(optional)] keep: bool,
    /// Called with the new state when the fold opens or closes.
    #[prop(optional)]
    on_toggle: Option<Callback<bool>>,
    children: ChildrenFn,
) -> impl IntoView {
    let open = RwSignal::new(crate::settings::get_fold_open(name));
    let toggle = move |_| {
        let v = !open.get_untracked();
        open.set(v);
        crate::settings::set_fold_open(name, v);
        if let Some(cb) = on_toggle {
            cb.run(v);
        }
    };
    let body = if keep {
        view! {
            <div class="cd-fold-body" style:display=move || if open.get() { "" } else { "none" }>
                {children()}
            </div>
        }
        .into_any()
    } else {
        let children = StoredValue::new(children);
        view! {
            <Show when=move || open.get()>
                <div class="cd-fold-body">{children.with_value(|c| c())}</div>
            </Show>
        }
        .into_any()
    };
    view! {
        <div class="cd-fold" class:open=move || open.get() data-fold=name>
            <button
                class="fine-out-head cd-fold-head"
                class:open=move || open.get()
                aria-expanded=move || open.get().to_string()
                on:click=toggle
            >
                <Icon kind=Icon::ChevronRight />
                <span>{move || title.get()}</span>
                {tag.map(|t| view! { <span class="live-emb-tag">{t}</span> })}
            </button>
            {body}
        </div>
    }
}
