use leptos::prelude::*;
use wasm_bindgen::JsCast;
use crate::audio::source::ChannelView;
use crate::state::{AppState, FftMode, FlowColorScheme, MainView, SpectrogramDisplay};
use crate::components::slider_row::SliderRow;
use crate::dsp::zero_crossing::zero_crossing_frequency;
use crate::annotations::{Annotation, AnnotationKind, AnnotationSet, SavedSelection, generate_uuid, now_iso8601};

#[component]
pub(crate) fn SpectrogramSettingsPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    view! {
        <div class="sidebar-panel">
            // Gain/Range/Contrast — always shown (applies to all tile modes)
            <div class="setting-group">
                <div class="setting-group-title">"Intensity"</div>
                <SliderRow
                    label="Gain"
                    signal=state.spect_gain_db
                    min=-40.0
                    max=40.0
                    step=1.0
                    default=0.0
                    format_value=Callback::new(move |v: f32| {
                        if state.display_auto_gain.get() {
                            "auto".to_string()
                        } else {
                            format!("{:+.0} dB", v)
                        }
                    })
                    on_change=Callback::new(move |_: f32| {
                        state.display_auto_gain.set(false);
                    })
                />
                <SliderRow
                    label="Range"
                    signal=state.spect_range_db
                    min=20.0
                    max=120.0
                    step=5.0
                    default=120.0
                    format_value=Callback::new(|v: f32| format!("{:.0} dB", v))
                    on_change=Callback::new(move |v: f32| {
                        state.spect_floor_db.set(-v);
                    })
                />
                <SliderRow
                    label="Contrast"
                    signal=state.spect_gamma
                    min=0.2
                    max=3.0
                    step=0.05
                    default=1.0
                    format_value=Callback::new(|g: f32| {
                        if g == 1.0 { "linear".to_string() }
                        else { format!("{:.2}", g) }
                    })
                />
                <div class="setting-row">
                    <label class="setting-label" style="display:flex;align-items:center;gap:4px;cursor:pointer">
                        <input
                            type="checkbox"
                            prop:checked=move || state.display_auto_gain.get()
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let input: web_sys::HtmlInputElement = target.unchecked_into();
                                state.display_auto_gain.set(input.checked());
                            }
                        />
                        "Auto gain"
                    </label>
                </div>
                <div class="setting-row">
                    <label class="setting-label" style="display:flex;align-items:center;gap:4px;cursor:pointer">
                        <input
                            type="checkbox"
                            prop:checked=move || state.display_eq.get()
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let input: web_sys::HtmlInputElement = target.unchecked_into();
                                state.display_eq.set(input.checked());
                            }
                        />
                        "Show EQ"
                    </label>
                </div>
                <div class="setting-row">
                    <label class="setting-label" style="display:flex;align-items:center;gap:4px;cursor:pointer">
                        <input
                            type="checkbox"
                            prop:checked=move || state.display_noise_filter.get()
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let input: web_sys::HtmlInputElement = target.unchecked_into();
                                state.display_noise_filter.set(input.checked());
                            }
                        />
                        "Show noise filter"
                    </label>
                </div>
                <div class="setting-row">
                    <button
                        class="setting-button"
                        on:click=move |_| {
                            state.spect_gain_db.set(0.0);
                            state.spect_floor_db.set(-120.0);
                            state.spect_range_db.set(120.0);
                            state.spect_gamma.set(1.0);
                            state.display_auto_gain.set(false);
                            state.display_eq.set(false);
                            state.display_noise_filter.set(false);
                        }
                    >"Reset"</button>
                </div>
                <div class="setting-row">
                    <span class="setting-label">"FFT size"</span>
                    <select
                        class="setting-select"
                        on:change=move |ev: web_sys::Event| {
                            let target = ev.target().unwrap();
                            let select: web_sys::HtmlSelectElement = target.unchecked_into();
                            let val = select.value();
                            let mode = match val.as_str() {
                                "a512" => FftMode::Adaptive(512),
                                "a1024" => FftMode::Adaptive(1024),
                                "a2048" => FftMode::Adaptive(2048),
                                _ => {
                                    if let Ok(v) = val.parse::<usize>() {
                                        FftMode::Single(v)
                                    } else {
                                        return;
                                    }
                                }
                            };
                            state.spect_fft_mode.set(mode);
                        }
                    >
                        {move || {
                            let current = state.spect_fft_mode.get();
                            let options: [(&str, &str); 10] = [
                                ("128", "128"),
                                ("256", "256"),
                                ("512", "512"),
                                ("1024", "1024"),
                                ("2048", "2048"),
                                ("4096", "4096"),
                                ("8192", "8192"),
                                ("a512", "Adaptive 512"),
                                ("a1024", "Adaptive 1024"),
                                ("a2048", "Adaptive 2048"),
                            ];
                            options.into_iter().map(|(value, label)| {
                                let is_selected = match (value, current) {
                                    ("a512", FftMode::Adaptive(512)) => true,
                                    ("a1024", FftMode::Adaptive(1024)) => true,
                                    ("a2048", FftMode::Adaptive(2048)) => true,
                                    (v, FftMode::Single(sz)) => v.parse::<usize>().ok() == Some(sz),
                                    _ => false,
                                };
                                let v = value.to_string();
                                let l = label.to_string();
                                view! { <option value={v} selected=move || is_selected>{l}</option> }
                            }).collect::<Vec<_>>()
                        }}
                    </select>
                </div>
                <div class="setting-row">
                    <label class="setting-label" style="display:flex;align-items:center;gap:4px;cursor:pointer"
                        title="Sharpen time-frequency localization using the reassignment method (3x FFT cost)">
                        <input
                            type="checkbox"
                            prop:checked=move || state.reassign_enabled.get()
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let input: web_sys::HtmlInputElement = target.unchecked_into();
                                state.reassign_enabled.set(input.checked());
                            }
                        />
                        "Reassignment"
                    </label>
                </div>
                <div class="setting-row">
                    <label class="setting-label" style="display:flex;align-items:center;gap:4px;cursor:pointer">
                        <input
                            type="checkbox"
                            prop:checked=move || state.debug_tiles.get()
                            on:change=move |ev: web_sys::Event| {
                                let target = ev.target().unwrap();
                                let input: web_sys::HtmlInputElement = target.unchecked_into();
                                state.debug_tiles.set(input.checked());
                            }
                        />
                        "Debug tiles"
                    </label>
                </div>
            </div>

            // Flow-specific settings (shown only when Flow view is active)
            {move || {
                if state.main_view.get() == MainView::Flow {
                    let display = state.spectrogram_display.get();
                    let _ = display; // used for reactivity trigger above
                    view! {
                        <div class="setting-group">
                            <div class="setting-group-title">"Color"</div>
                            <div class="setting-row">
                                <span class="setting-label">"Algorithm"</span>
                                <select
                                    class="setting-select"
                                    on:change=move |ev: web_sys::Event| {
                                        let target = ev.target().unwrap();
                                        let select: web_sys::HtmlSelectElement = target.unchecked_into();
                                        let mode = match select.value().as_str() {
                                            "coherence" => SpectrogramDisplay::PhaseCoherence,
                                            "centroid" => SpectrogramDisplay::FlowCentroid,
                                            "gradient" => SpectrogramDisplay::FlowGradient,
                                            "phase" => SpectrogramDisplay::Phase,
                                            _ => SpectrogramDisplay::FlowOptical,
                                        };
                                        state.spectrogram_display.set(mode);
                                    }
                                    prop:value=move || match state.spectrogram_display.get() {
                                        SpectrogramDisplay::FlowOptical => "flow",
                                        SpectrogramDisplay::PhaseCoherence => "coherence",
                                        SpectrogramDisplay::FlowCentroid => "centroid",
                                        SpectrogramDisplay::FlowGradient => "gradient",
                                        SpectrogramDisplay::Phase => "phase",
                                    }
                                >
                                    <option value="flow">"Optical"</option>
                                    <option value="coherence">"Phase Coherence"</option>
                                    <option value="centroid">"Centroid"</option>
                                    <option value="gradient">"Gradient"</option>
                                    <option value="phase">"Phase"</option>
                                </select>
                            </div>
                            // Color scheme selector (only for flow algorithms, not phase)
                            {move || {
                                let display = state.spectrogram_display.get();
                                let is_flow_algo = matches!(display,
                                    SpectrogramDisplay::FlowOptical |
                                    SpectrogramDisplay::FlowCentroid |
                                    SpectrogramDisplay::FlowGradient
                                );
                                if is_flow_algo {
                                    view! {
                                        <div class="setting-row">
                                            <span class="setting-label">"Color scheme"</span>
                                            <select
                                                class="setting-select"
                                                on:change=move |ev: web_sys::Event| {
                                                    let target = ev.target().unwrap();
                                                    let select: web_sys::HtmlSelectElement = target.unchecked_into();
                                                    let scheme = match select.value().as_str() {
                                                        "coolwarm" => FlowColorScheme::CoolWarm,
                                                        "tealorange" => FlowColorScheme::TealOrange,
                                                        "purplegreen" => FlowColorScheme::PurpleGreen,
                                                        "spectral" => FlowColorScheme::Spectral,
                                                        _ => FlowColorScheme::RedBlue,
                                                    };
                                                    state.flow_color_scheme.set(scheme);
                                                }
                                                prop:value=move || match state.flow_color_scheme.get() {
                                                    FlowColorScheme::RedBlue => "redblue",
                                                    FlowColorScheme::CoolWarm => "coolwarm",
                                                    FlowColorScheme::TealOrange => "tealorange",
                                                    FlowColorScheme::PurpleGreen => "purplegreen",
                                                    FlowColorScheme::Spectral => "spectral",
                                                }
                                            >
                                                <option value="redblue">"Red-Blue"</option>
                                                <option value="coolwarm">"Cool-Warm"</option>
                                                <option value="tealorange">"Teal-Orange"</option>
                                                <option value="purplegreen">"Purple-Green"</option>
                                                <option value="spectral">"Spectral"</option>
                                            </select>
                                        </div>
                                    }.into_any()
                                } else {
                                    view! { <span></span> }.into_any()
                                }
                            }}
                            <div class="setting-row">
                                <span class="setting-label">"Intensity gate"</span>
                                <div class="setting-slider-row">
                                    <input
                                        type="range"
                                        class="setting-range"
                                        min="0"
                                        max="100"
                                        step="1"
                                        prop:value=move || (state.flow_intensity_gate.get() * 100.0).round().to_string()
                                        on:input=move |ev: web_sys::Event| {
                                            let target = ev.target().unwrap();
                                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                                            if let Ok(val) = input.value().parse::<f32>() {
                                                state.flow_intensity_gate.set(val / 100.0);
                                            }
                                        }
                                    />
                                    <span class="setting-value">{move || format!("{}%", (state.flow_intensity_gate.get() * 100.0).round() as u32)}</span>
                                </div>
                            </div>
                            <div class="setting-row">
                                <span class="setting-label">"Color gain"</span>
                                <div class="setting-slider-row">
                                    <input
                                        type="range"
                                        class="setting-range"
                                        min="0.5"
                                        max="10.0"
                                        step="0.5"
                                        prop:value=move || state.flow_shift_gain.get().to_string()
                                        on:input=move |ev: web_sys::Event| {
                                            let target = ev.target().unwrap();
                                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                                            if let Ok(val) = input.value().parse::<f32>() {
                                                state.flow_shift_gain.set(val);
                                            }
                                        }
                                    />
                                    <span class="setting-value">{move || format!("{:.1}x", state.flow_shift_gain.get())}</span>
                                </div>
                            </div>
                            <div class="setting-row">
                                <span class="setting-label">{move || {
                                    let g = state.flow_color_gamma.get();
                                    if g == 1.0 { "Color contrast: linear".to_string() }
                                    else { format!("Color contrast: {:.2}", g) }
                                }}</span>
                                <div class="setting-slider-row">
                                    <input
                                        type="range"
                                        class="setting-range"
                                        min="0.2"
                                        max="3.0"
                                        step="0.05"
                                        prop:value=move || state.flow_color_gamma.get().to_string()
                                        on:input=move |ev: web_sys::Event| {
                                            let target = ev.target().unwrap();
                                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                                            if let Ok(val) = input.value().parse::<f32>() {
                                                state.flow_color_gamma.set(val);
                                            }
                                        }
                                    />
                                </div>
                            </div>
                            // Flow gate — threshold for minimum shift/deviation magnitude to show color
                            <div class="setting-row">
                                <span class="setting-label">"Flow gate"</span>
                                <div class="setting-slider-row">
                                    <input
                                        type="range"
                                        class="setting-range"
                                        min="0"
                                        max="100"
                                        step="1"
                                        prop:value=move || (state.flow_gate.get() * 100.0).round().to_string()
                                        on:input=move |ev: web_sys::Event| {
                                            let target = ev.target().unwrap();
                                            let input: web_sys::HtmlInputElement = target.unchecked_into();
                                            if let Ok(val) = input.value().parse::<f32>() {
                                                state.flow_gate.set(val / 100.0);
                                            }
                                        }
                                    />
                                    <span class="setting-value">{move || format!("{}%", (state.flow_gate.get() * 100.0).round() as u32)}</span>
                                </div>
                            </div>
                        </div>
                    }.into_any()
                } else {
                    view! { <span></span> }.into_any()
                }
            }}

            // Chromagram-specific settings (shown only when Chromagram view is active)
            {move || {
                if state.main_view.get() == MainView::Chromagram {
                    view! {
                        <div class="setting-group">
                            <div class="setting-group-title">"Chromagram"</div>
                            <div class="setting-row">
                                <span class="setting-label">{move || {
                                    let g = state.chroma_gain.get();
                                    if g == 1.0 { "Gain: default".to_string() }
                                    else { format!("Gain: {:.2}x", g) }
                                }}</span>
                                <input
                                    type="range"
                                    class="setting-range"
                                    min="0.25"
                                    max="4.0"
                                    step="0.05"
                                    prop:value=move || state.chroma_gain.get().to_string()
                                    on:input=move |ev: web_sys::Event| {
                                        let target = ev.target().unwrap();
                                        let input: web_sys::HtmlInputElement = target.unchecked_into();
                                        if let Ok(v) = input.value().parse::<f32>() {
                                            state.chroma_gain.set(v);
                                        }
                                    }
                                />
                            </div>
                            <div class="setting-row">
                                <span class="setting-label">{move || {
                                    let g = state.chroma_gamma.get();
                                    if g == 1.0 { "Contrast: linear".to_string() }
                                    else { format!("Contrast: {:.2}", g) }
                                }}</span>
                                <input
                                    type="range"
                                    class="setting-range"
                                    min="0.2"
                                    max="3.0"
                                    step="0.05"
                                    prop:value=move || state.chroma_gamma.get().to_string()
                                    on:input=move |ev: web_sys::Event| {
                                        let target = ev.target().unwrap();
                                        let input: web_sys::HtmlInputElement = target.unchecked_into();
                                        if let Ok(v) = input.value().parse::<f32>() {
                                            state.chroma_gamma.set(v);
                                        }
                                    }
                                />
                            </div>
                            <div class="setting-row">
                                <button
                                    class="setting-button"
                                    on:click=move |_| {
                                        state.chroma_gain.set(1.0);
                                        state.chroma_gamma.set(1.0);
                                    }
                                >"Reset"</button>
                            </div>
                        </div>
                    }.into_any()
                } else {
                    view! { <span></span> }.into_any()
                }
            }}
        </div>
    }
}

#[component]
pub(crate) fn SelectionPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    let analysis = move || {
        let selection = state.selection.get()?;
        let dragging = state.is_dragging.get();
        let files = state.files.get();
        let idx = state.current_file_index.get()?;
        let file = files.get(idx)?;

        let sr = file.audio.sample_rate;
        let total = file.audio.source.total_samples() as usize;
        let start = ((selection.time_start * sr as f64) as usize).min(total);
        let end = ((selection.time_end * sr as f64) as usize).min(total);

        if end <= start {
            return None;
        }

        let duration = selection.time_end - selection.time_start;
        let frames = end - start;

        let (crossing_count, estimated_freq) = if dragging {
            (None, None)
        } else {
            let slice = file.audio.source.read_region(ChannelView::MonoMix, start as u64, end - start);
            let zc = zero_crossing_frequency(&slice, sr);
            (Some(zc.crossing_count), Some(zc.estimated_frequency_hz))
        };

        Some((duration, frames, crossing_count, estimated_freq, selection.freq_low, selection.freq_high))
    };

    let save_selection = move |_| {
        let selection = state.selection.get_untracked();
        let file_idx = state.current_file_index.get_untracked();
        if let (Some(sel), Some(idx)) = (selection, file_idx) {
            let annotation = Annotation {
                id: generate_uuid(),
                kind: AnnotationKind::Selection(SavedSelection {
                    time_start: sel.time_start,
                    time_end: sel.time_end,
                    freq_low: sel.freq_low,
                    freq_high: sel.freq_high,
                    label: None,
                    color: None,
                }),
                created_at: now_iso8601(),
                modified_at: now_iso8601(),
                notes: None,
            };
            state.annotation_store.update(|store| {
                store.ensure_len(idx + 1);
                if store.sets[idx].is_none() {
                    // Create a new AnnotationSet for this file
                    let identity = state.files.with_untracked(|files| {
                        files.get(idx).and_then(|f| f.identity.clone())
                    }).unwrap_or_else(|| {
                        let name = state.files.with_untracked(|files| {
                            files.get(idx).map(|f| f.name.clone()).unwrap_or_default()
                        });
                        crate::file_identity::identity_layer1(&name, 0)
                    });
                    store.sets[idx] = Some(AnnotationSet {
                        version: 1,
                        file_identity: identity,
                        annotations: Vec::new(),
                        app_version: env!("CARGO_PKG_VERSION").to_string(),
                    });
                }
                if let Some(ref mut set) = store.sets[idx] {
                    set.annotations.push(annotation);
                }
            });
            state.annotations_dirty.set(true);
            state.show_info_toast("Selection saved");
        }
    };

    view! {
        <div class="sidebar-panel">
            {move || {
                match analysis() {
                    Some((duration, frames, crossing_count, estimated_freq, freq_low, freq_high)) => {
                        view! {
                            <div class="setting-group">
                                <div class="setting-group-title">"Selection"</div>
                                <div class="setting-row">
                                    <span class="setting-label">"Duration"</span>
                                    <span class="setting-value">{format!("{:.3} s", duration)}</span>
                                </div>
                                <div class="setting-row">
                                    <span class="setting-label">"Frames"</span>
                                    <span class="setting-value">{format!("{}", frames)}</span>
                                </div>
                                <div class="setting-row">
                                    <span class="setting-label">"Freq range"</span>
                                    <span class="setting-value">{format!("{:.0} – {:.0} kHz", freq_low / 1000.0, freq_high / 1000.0)}</span>
                                </div>
                                <div class="setting-row">
                                    <span class="setting-label">"ZC count"</span>
                                    <span class="setting-value">{match crossing_count { Some(c) => format!("{c}"), None => "...".into() }}</span>
                                </div>
                                <div class="setting-row">
                                    <span class="setting-label">"ZC est. freq"</span>
                                    <span class="setting-value">{match estimated_freq { Some(f) => format!("~{:.1} kHz", f / 1000.0), None => "...".into() }}</span>
                                </div>
                                <button class="sidebar-btn" on:click=save_selection>"Save Selection"</button>
                            </div>
                        }.into_any()
                    }
                    None => {
                        view! {
                            <div class="sidebar-panel-empty">"No selection"</div>
                        }.into_any()
                    }
                }
            }}
            <SavedSelectionsList />
        </div>
    }
}

#[component]
fn SavedSelectionsList() -> impl IntoView {
    let state = expect_context::<AppState>();

    let saved_selections = move || {
        let idx = state.current_file_index.get()?;
        let store = state.annotation_store.get();
        let set = store.sets.get(idx)?.as_ref()?;
        let selections: Vec<(String, String)> = set.annotations.iter().filter_map(|a| {
            if let AnnotationKind::Selection(ref sel) = a.kind {
                let label = sel.label.clone().unwrap_or_else(|| {
                    format!("{:.3}–{:.3}s, {:.0}–{:.0} kHz",
                        sel.time_start, sel.time_end,
                        sel.freq_low / 1000.0, sel.freq_high / 1000.0)
                });
                Some((a.id.clone(), label))
            } else {
                None
            }
        }).collect();
        if selections.is_empty() { None } else { Some(selections) }
    };

    let on_export = move |_: web_sys::MouseEvent| {
        export_annotations(state);
    };

    let on_import = move |_: web_sys::MouseEvent| {
        import_annotations(state);
    };

    let has_annotations = move || {
        let idx = state.current_file_index.get()?;
        let store = state.annotation_store.get();
        let set = store.sets.get(idx)?.as_ref()?;
        if set.annotations.is_empty() { None } else { Some(true) }
    };

    view! {
        {move || {
            if let Some(selections) = saved_selections() {
                view! {
                    <div class="setting-group">
                        <div class="setting-group-title">"Saved Selections"</div>
                        {selections.into_iter().map(|(id, label)| {
                            let id_click = id.clone();
                            let id_delete = id.clone();
                            view! {
                                <div class="saved-selection-item"
                                    on:click=move |_| {
                                        restore_selection(state, &id_click);
                                    }
                                >
                                    <span class="saved-selection-label">{label}</span>
                                    <button class="saved-selection-delete"
                                        on:click=move |e| {
                                            e.stop_propagation();
                                            delete_annotation(state, &id_delete);
                                        }
                                    >"\u{00d7}"</button>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                }.into_any()
            } else {
                view! { <div></div> }.into_any()
            }
        }}
        <div class="setting-row" style="gap: 4px;">
            <button
                class="sidebar-btn"
                style="flex: 1;"
                on:click=on_export
                disabled=move || has_annotations().is_none()
            >
                "Export .batm"
            </button>
            <button
                class="sidebar-btn"
                style="flex: 1;"
                on:click=on_import
            >
                "Import .batm"
            </button>
        </div>
    }
}

fn restore_selection(state: AppState, annotation_id: &str) {
    let idx = match state.current_file_index.get_untracked() {
        Some(i) => i,
        None => return,
    };
    let store = state.annotation_store.get_untracked();
    let set = match store.sets.get(idx).and_then(|s| s.as_ref()) {
        Some(s) => s,
        None => return,
    };
    for a in &set.annotations {
        if a.id == annotation_id {
            if let AnnotationKind::Selection(ref sel) = a.kind {
                state.selection.set(Some(crate::state::Selection {
                    time_start: sel.time_start,
                    time_end: sel.time_end,
                    freq_low: sel.freq_low,
                    freq_high: sel.freq_high,
                }));
                state.selected_annotation_id.set(Some(annotation_id.to_string()));
            }
            return;
        }
    }
}

fn delete_annotation(state: AppState, annotation_id: &str) {
    let idx = match state.current_file_index.get_untracked() {
        Some(i) => i,
        None => return,
    };
    state.annotation_store.update(|store| {
        if let Some(Some(ref mut set)) = store.sets.get_mut(idx) {
            set.annotations.retain(|a| a.id != annotation_id);
        }
    });
    state.annotations_dirty.set(true);
}

fn export_annotations(state: AppState) {
    let idx = match state.current_file_index.get_untracked() {
        Some(i) => i,
        None => { state.show_error_toast("No file selected"); return; }
    };
    let store = state.annotation_store.get_untracked();
    let set = match store.sets.get(idx).and_then(|s| s.as_ref()) {
        Some(s) => s,
        None => { state.show_error_toast("No annotations to export"); return; }
    };

    let yaml = match yaml_serde::to_string(set) {
        Ok(y) => y,
        Err(e) => { state.show_error_toast(format!("Serialize error: {e}")); return; }
    };

    let arr = js_sys::Array::of1(&wasm_bindgen::JsValue::from_str(&yaml));
    let Ok(blob) = web_sys::Blob::new_with_str_sequence(&arr) else { return };
    let Ok(url) = web_sys::Url::create_object_url_with_blob(&blob) else { return };

    let doc = web_sys::window().unwrap().document().unwrap();
    let a: web_sys::HtmlAnchorElement = doc.create_element("a").unwrap().unchecked_into();
    a.set_href(&url);
    let filename = format!("{}.batm", set.file_identity.filename);
    a.set_download(&filename);
    a.click();
    let _ = web_sys::Url::revoke_object_url(&url);

    state.show_info_toast("Annotations exported");
}

fn import_annotations(state: AppState) {
    let doc = web_sys::window().unwrap().document().unwrap();
    let input: web_sys::HtmlInputElement = doc.create_element("input").unwrap().unchecked_into();
    input.set_type("file");
    input.set_attribute("accept", ".batm,.yaml,.yml").unwrap();

    let on_change = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| {
        let target: web_sys::HtmlInputElement = ev.target().unwrap().unchecked_into();
        let Some(file_list) = target.files() else { return };
        let Some(file) = file_list.get(0) else { return };

        let reader = web_sys::FileReader::new().unwrap();
        let reader_clone = reader.clone();
        let on_load = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
            let result = reader_clone.result().unwrap();
            let text = result.as_string().unwrap_or_default();
            match yaml_serde::from_str::<AnnotationSet>(&text) {
                Ok(imported) => {
                    let idx = state.current_file_index.get_untracked().unwrap_or(0);
                    state.annotation_store.update(|store| {
                        store.ensure_len(idx + 1);
                        if let Some(Some(ref mut existing)) = store.sets.get_mut(idx) {
                            // Merge: append imported annotations
                            existing.annotations.extend(imported.annotations);
                        } else {
                            store.sets[idx] = Some(imported);
                        }
                    });
                    state.annotations_dirty.set(true);
                    state.show_info_toast("Annotations imported");
                }
                Err(e) => {
                    state.show_error_toast(format!("Import error: {e}"));
                }
            }
        });
        reader.set_onload(Some(on_load.as_ref().unchecked_ref()));
        on_load.forget();
        reader.read_as_text(&file).unwrap();
    });
    input.set_onchange(Some(on_change.as_ref().unchecked_ref()));
    on_change.forget();
    input.click();
}
