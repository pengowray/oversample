//! Collapsible export section: WAV / MP4 export with format radio buttons,
//! video settings, progress bar, and .batm import/export.

use crate::components::icons::Icon;
use crate::state::store_fields::*;
use leptos::prelude::*;

use crate::audio::export;
use crate::audio::video_export;
use crate::audio::webcodecs_bindings as wc;
use crate::state::{
    AppState, AudioCodecOption, ExportFormat, RightSidebarTab, VideoCodec, VideoResolution,
    VideoViewMode,
};

/// Open the info panel on the Export tab.
pub(crate) fn open_export_panel(state: AppState) {
    state.panels.right_tab().set(RightSidebarTab::Export);
    state.panels.right_collapsed().set(false);
}

/// The Export tab of the info panel: WAV or MP4 export of the selected
/// regions, the selection, or the whole file.
#[component]
pub fn ExportPanel() -> impl IntoView {
    let state = expect_context::<AppState>();

    let webcodecs_available = wc::has_video_encoder() && wc::has_mp4_muxer();
    let audio_encoder_available = wc::has_audio_encoder();

    // Probe specific audio codec support asynchronously (48kHz mono is our standard output rate)
    let aac_supported = RwSignal::new(false);
    let opus_supported = RwSignal::new(false);
    if audio_encoder_available {
        leptos::task::spawn_local(async move {
            let aac = wc::is_audio_config_supported(wc::AAC_WEBCODECS_CODEC, 48000, 1).await;
            let opus = wc::is_audio_config_supported(wc::OPUS_WEBCODECS_CODEC, 48000, 1).await;
            aac_supported.set(aac);
            opus_supported.set(opus);
            log::info!("Audio codec probe: AAC={aac}, Opus={opus}");
        });
    }

    // Export button text (reactive)
    let export_button_text = move || {
        let format = state.export.format().get();
        let ext = match format {
            ExportFormat::Wav => ".wav",
            ExportFormat::Mp4 => ".mp4",
        };
        match export::get_export_info(&state) {
            Some(info) if info.count > 1 => {
                format!("Export {} {} ({ext})", info.count, info.source_label)
            }
            Some(info) => format!("Export {} ({ext})", info.source_label),
            None => format!("Export ({ext})"),
        }
    };

    // Names the export will save, kept current with the settings that change them.
    let file_names = move || {
        let format = state.export.format().get();
        export::get_export_info(&state)?;
        state.playback.mode().track();
        state.transform.te_factor().track();
        state.transform.ps_factor().track();
        state.transform.pv_factor().track();
        state.transform.het_frequency().track();
        state.transform.het_comb_count().track();
        state.transform.zc_factor().track();
        state.viewmode.channel_view().track();
        let names = export::planned_file_names(&state, format);
        (!names.is_empty()).then_some(names)
    };

    let export_disabled = move || {
        export::get_export_info(&state).is_none() || state.export.video_progress().get().is_some()
    };

    let on_export_click = move |_: web_sys::MouseEvent| match state.export.format().get_untracked()
    {
        ExportFormat::Wav => {
            export::export_selected(&state);
        }
        ExportFormat::Mp4 => {
            video_export::start_export(&state);
        }
    };

    let on_format_change = move |format: ExportFormat| {
        state.export.format().set(format);
    };

    view! {
        <div class="sidebar-panel export-panel">
                // Format radio buttons
                <div class="setting-row export-format-row">
                    <span class="export-format-label">"Format:"</span>
                    <label class="export-radio">
                        <input
                            type="radio"
                            name="export-format"
                            checked=move || state.export.format().get() == ExportFormat::Wav
                            on:change=move |_| on_format_change(ExportFormat::Wav)
                        />
                        " WAV"
                    </label>
                    <label class=move || if webcodecs_available {
                        "export-radio"
                    } else {
                        "export-radio disabled"
                    }>
                        <input
                            type="radio"
                            name="export-format"
                            checked=move || state.export.format().get() == ExportFormat::Mp4
                            on:change=move |_| on_format_change(ExportFormat::Mp4)
                            disabled=move || !webcodecs_available
                        />
                        " MP4"
                        {if !webcodecs_available {
                            Some(view! {
                                <span class="export-tooltip" title="WebCodecs not available in this browser">{" (?)"}</span>
                            })
                        } else {
                            None
                        }}
                    </label>
                </div>

                // MP4-specific options (shown when MP4 selected)
                {move || {
                    if state.export.format().get() == ExportFormat::Mp4 && webcodecs_available {
                        Some(view! {
                            <div class="export-mp4-options">
                                <div class="setting-row" style="gap: 4px; align-items: center;">
                                    <span class="export-option-label">"View:"</span>
                                    <select
                                        class="sidebar-select"
                                        on:change=move |ev| {
                                            let val = event_target_value(&ev);
                                            let mode = match val.as_str() {
                                                "scroll" => VideoViewMode::ScrollingView,
                                                _ => VideoViewMode::StaticPlayhead,
                                            };
                                            state.export.video_view_mode().set(mode);
                                        }
                                    >
                                        <option value="static" selected=move || state.export.video_view_mode().get() == VideoViewMode::StaticPlayhead>
                                            "Static + playhead"
                                        </option>
                                        <option value="scroll" selected=move || state.export.video_view_mode().get() == VideoViewMode::ScrollingView>
                                            "Scrolling"
                                        </option>
                                    </select>
                                </div>
                                <div class="setting-row" style="gap: 4px; align-items: center;">
                                    <span class="export-option-label">"Resolution:"</span>
                                    <select
                                        class="sidebar-select"
                                        on:change=move |ev| {
                                            let val = event_target_value(&ev);
                                            let res = match val.as_str() {
                                                "720" => VideoResolution::Hd720,
                                                "1080" => VideoResolution::Hd1080,
                                                "canvas" => VideoResolution::MatchCanvas,
                                                _ => VideoResolution::Hd720,
                                            };
                                            state.export.video_resolution().set(res);
                                        }
                                    >
                                        <option value="720" selected=move || state.export.video_resolution().get() == VideoResolution::Hd720>
                                            "720p"
                                        </option>
                                        <option value="1080" selected=move || state.export.video_resolution().get() == VideoResolution::Hd1080>
                                            "1080p"
                                        </option>
                                        <option value="canvas" selected=move || state.export.video_resolution().get() == VideoResolution::MatchCanvas>
                                            "Match canvas"
                                        </option>
                                    </select>
                                </div>
                                <div class="setting-row" style="gap: 4px; align-items: center;">
                                    <span class="export-option-label">"Video:"</span>
                                    <select
                                        class="sidebar-select"
                                        on:change=move |ev| {
                                            let val = event_target_value(&ev);
                                            let codec = match val.as_str() {
                                                "av1" => VideoCodec::Av1,
                                                _ => VideoCodec::H264,
                                            };
                                            state.export.video_codec().set(codec);
                                        }
                                    >
                                        <option value="h264" selected=move || state.export.video_codec().get() == VideoCodec::H264>
                                            "H.264"
                                        </option>
                                        <option value="av1" selected=move || state.export.video_codec().get() == VideoCodec::Av1>
                                            "AV1"
                                        </option>
                                    </select>
                                </div>
                                <div class="setting-row" style="gap: 4px; align-items: center;">
                                    <span class="export-option-label">"Audio:"</span>
                                    <select
                                        class="sidebar-select"
                                        on:change=move |ev| {
                                            let val = event_target_value(&ev);
                                            let opt = match val.as_str() {
                                                "aac" => AudioCodecOption::Aac,
                                                "opus" => AudioCodecOption::Opus,
                                                "none" => AudioCodecOption::NoAudio,
                                                _ => AudioCodecOption::Auto,
                                            };
                                            state.export.video_audio_codec().set(opt);
                                        }
                                    >
                                        <option
                                            value="auto"
                                            selected=move || state.export.video_audio_codec().get() == AudioCodecOption::Auto
                                        >
                                            {move || {
                                                let aac = aac_supported.get();
                                                let opus = opus_supported.get();
                                                match (aac, opus) {
                                                    (true, true) => "Auto (AAC/Opus)".to_string(),
                                                    (true, false) => "Auto (AAC)".to_string(),
                                                    (false, true) => "Auto (Opus)".to_string(),
                                                    (false, false) => if audio_encoder_available {
                                                        "Auto (checking\u{2026})".to_string()
                                                    } else {
                                                        "Auto (unavailable)".to_string()
                                                    },
                                                }
                                            }}
                                        </option>
                                        <option
                                            value="aac"
                                            selected=move || state.export.video_audio_codec().get() == AudioCodecOption::Aac
                                            disabled=move || !aac_supported.get()
                                        >
                                            {move || if aac_supported.get() { "AAC" } else { "AAC (unavailable)" }}
                                        </option>
                                        <option
                                            value="opus"
                                            selected=move || state.export.video_audio_codec().get() == AudioCodecOption::Opus
                                            disabled=move || !opus_supported.get()
                                        >
                                            {move || if opus_supported.get() { "Opus" } else { "Opus (unavailable)" }}
                                        </option>
                                        <option
                                            value="none"
                                            selected=move || state.export.video_audio_codec().get() == AudioCodecOption::NoAudio
                                        >
                                            "No audio"
                                        </option>
                                    </select>
                                </div>
                            </div>
                        })
                    } else {
                        None
                    }
                }}

                // Main export button
                <div class="setting-row" style="gap: 4px; align-items: center;">
                    <button
                        class="sidebar-btn"
                        style="flex: 1;"
                        on:click=on_export_click
                        disabled=export_disabled
                    >
                        {export_button_text}
                    </button>
                </div>

                // What the export will save
                {move || file_names().map(|names| {
                    let label = if names.len() > 1 { "File names" } else { "File name" };
                    const SHOWN: usize = 4;
                    let more = names.len().saturating_sub(SHOWN);
                    view! {
                        <div class="export-detail">
                            <span class="export-detail-label">{label}</span>
                            {names.into_iter().take(SHOWN).map(|n| view! {
                                <span class="export-file-name">{n}</span>
                            }).collect_view()}
                            {(more > 0).then(|| view! {
                                <span class="export-detail-label">{format!("and {more} more")}</span>
                            })}
                        </div>
                    }
                })}
                {move || {
                    export::get_export_info(&state)
                        .and_then(|info| info.estimated_duration_secs)
                        .map(|dur| view! {
                            <div class="export-detail">
                                <span class="export-detail-label">"Length"</span>
                                <span>{export::format_duration(dur)}</span>
                            </div>
                        })
                }}

                // Progress bar and status text
                {move || {
                    let progress = state.export.video_progress().get();
                    let status = state.export.video_status().get();

                    if progress.is_some() || status.is_some() {
                        let status_text = status.unwrap_or_else(|| "Exporting...".to_string());
                        let is_error = status_text.starts_with("Export failed");
                        Some(view! {
                            <div class="export-progress">
                                {progress.map(|p| view! {
                                    <div class="export-progress-bar">
                                        <div
                                            class="export-progress-fill"
                                            style=move || format!("width: {}%", (p * 100.0) as u32)
                                        ></div>
                                    </div>
                                })}
                                <div
                                    class=if is_error { "export-progress-text export-error" } else { "export-progress-text" }
                                >
                                    {status_text}
                                </div>
                                {if !is_error && progress.is_some() {
                                    Some(view! {
                                        <button
                                            class="sidebar-btn"
                                            style="margin-top: 4px;"
                                            on:click=move |_| {
                                                state.export.video_cancel().set(true);
                                            }
                                        >
                                            "Cancel"
                                        </button>
                                    })
                                } else {
                                    None
                                }}
                            </div>
                        })
                    } else {
                        None
                    }
                }}

        </div>
    }
}

/// Collapsible "Annotation files" section of the Annotations tab: save,
/// import, and export the annotations for Xeno-canto.
#[component]
pub fn AnnotationFilesSection(
    on_export_batm: Callback<()>,
    on_save_sidecar: Callback<()>,
    on_import_batm: Callback<()>,
    has_annotations: Signal<Option<bool>>,
    has_file_path: Signal<Option<bool>>,
) -> impl IntoView {
    let state = expect_context::<AppState>();
    let xc_form_open = RwSignal::new(false);

    view! {
        <div class="export-section">
            <div
                class="export-section-header"
                on:click=move |_| state.export.section_open().update(|v| *v = !*v)
            >
                <span class=move || if state.export.section_open().get() {
                    "export-toggle-arrow open"
                } else {
                    "export-toggle-arrow"
                }>
                    <Icon kind=Icon::ChevronRight />
                </span>
                " Annotation files"
            </div>

            <div class=move || if state.export.section_open().get() {
                "export-section-body open"
            } else {
                "export-section-body"
            }>
                // .batm section
                <div class="setting-row" style="gap: 4px; margin-top: 4px;">
                    {if state.is_tauri {
                        view! {
                            <button
                                class="sidebar-btn"
                                style="flex: 1;"
                                on:click={
                                    let cb = on_save_sidecar;
                                    move |_: web_sys::MouseEvent| cb.run(())
                                }
                                disabled=move || has_annotations.get().is_none() || has_file_path.get().is_none()
                                title="Save .batm sidecar next to the audio file"
                            >
                                "Save .batm"
                            </button>
                            <button
                                class="sidebar-btn"
                                style="flex: 1;"
                                on:click={
                                    let cb = on_export_batm;
                                    move |_: web_sys::MouseEvent| cb.run(())
                                }
                                disabled=move || has_annotations.get().is_none()
                                title="Export .batm to a chosen location"
                            >
                                "Save as\u{2026}"
                            </button>
                        }.into_any()
                    } else {
                        view! {
                            <button
                                class="sidebar-btn"
                                style="flex: 1;"
                                on:click={
                                    let cb = on_export_batm;
                                    move |_: web_sys::MouseEvent| cb.run(())
                                }
                                disabled=move || has_annotations.get().is_none()
                            >
                                "Export .batm"
                            </button>
                        }.into_any()
                    }}
                    <button
                        class="sidebar-btn"
                        style="flex: 1;"
                        on:click={
                            let cb = on_import_batm;
                            move |_: web_sys::MouseEvent| cb.run(())
                        }
                        title="Add annotations from a .batm file, or from a Xeno-canto Annota-JSON file (.json), such as the file from XC's \"Download annotations\" link."
                    >
                        "Import .batm/.json"
                    </button>
                </div>
                <div class="setting-row" style="gap: 4px;">
                    <button
                        class="sidebar-btn"
                        style="flex: 1;"
                        class:active=move || xc_form_open.get()
                        on:click=move |_| xc_form_open.update(|o| *o = !*o)
                        disabled=move || has_annotations.get().is_none()
                        title="Save this recording's annotations as an Annota-JSON file, the format Xeno-canto accepts for uploads."
                    >
                        "Export for XC"
                    </button>
                </div>
                {move || xc_form_open.get().then(|| view! {
                    <crate::components::file_sidebar::xc_annotation_export::XcAnnotationExport
                        on_close=Callback::new(move |()| xc_form_open.set(false))
                    />
                })}
            </div>
        </div>
    }
}
