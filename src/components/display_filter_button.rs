use leptos::prelude::*;

use crate::state::DisplayFilterMode;

/// A single row in the DSP filter grid: label + 4-way segmented control + playback indicator.
#[component]
pub fn DspFilterRow(
    label: &'static str,
    signal: RwSignal<DisplayFilterMode>,
    /// Whether the corresponding playback filter is currently active
    #[prop(into)]
    playback_active: Signal<bool>,
    /// Whether 'custom' is available (greyed out if false)
    custom_available: bool,
    /// Whether 'auto' is available (greyed out if false)
    #[prop(default = true)]
    auto_available: bool,
    /// When true, the sam-dot shows orange instead of green (browser handling resampling)
    #[prop(optional, into)]
    browser_resampling: Option<Signal<bool>>,
    /// Extra tooltip text for the sam-dot
    #[prop(optional, into)]
    sam_tooltip: Option<Signal<String>>,
) -> impl IntoView {
    let modes = DisplayFilterMode::ALL;

    view! {
        <div class="dsp-filter-row">
            <span class="dsp-filter-label">{label}</span>
            <div class="dsp-filter-seg">
                {modes.iter().copied().map(|mode| {
                    let is_custom = mode == DisplayFilterMode::Custom;
                    let is_auto = mode == DisplayFilterMode::Auto;
                    let disabled = (is_custom && !custom_available) || (is_auto && !auto_available);
                    let is_same = mode == DisplayFilterMode::Same;
                    view! {
                        <button
                            class=move || {
                                let sel = signal.get() == mode;
                                match (sel, disabled) {
                                    (true, _) => "sel",
                                    (_, true) => "disabled",
                                    _ => "",
                                }
                            }
                            title=mode.label()
                            disabled=disabled
                            on:click=move |_| {
                                if !disabled {
                                    signal.set(mode);
                                }
                            }
                        >
                            {mode.short_label()}
                            {is_same.then(|| {
                                let br = browser_resampling;
                                let tip = sam_tooltip;
                                view! {
                                    <span
                                        class=move || {
                                            if let Some(br_sig) = br {
                                                if br_sig.get() {
                                                    return "sam-dot browser-resample";
                                                }
                                            }
                                            if playback_active.get() { "sam-dot active" } else { "sam-dot inactive" }
                                        }
                                        title=move || tip.as_ref().map(|t| t.get()).unwrap_or_default()
                                    ></span>
                                }
                            })}
                        </button>
                    }
                }).collect_view()}
            </div>
            <div class=move || {
                if playback_active.get() { "dsp-filter-indicator active" } else { "dsp-filter-indicator inactive" }
            }></div>
        </div>
    }
}
