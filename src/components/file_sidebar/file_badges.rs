use super::file_groups::{SequenceInfo, TrackInfo};
use super::metadata_panel::open_info_panel;
use crate::components::icons::Icon;
use crate::format_time::format_duration_compact;
use crate::license::license_title;
use crate::rate_notes::NoteKind;
use crate::state::store_fields::*;
use crate::state::{AppState, LoadedFile};
use leptos::prelude::*;

/// Get XC metadata field value by key.
pub(crate) fn get_xc_field(metadata: &[(String, String)], key: &str) -> Option<String> {
    metadata
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.clone())
}

/// All data needed to render badges for a single file.
#[derive(Clone, PartialEq)]
pub struct FileBadgeData {
    pub sample_rate: u32,
    pub bits_per_sample: u16,
    pub is_float: bool,
    pub duration_secs: f64,
    pub is_streaming: bool,
    pub track: Option<TrackInfo>,
    pub sequence: Option<SequenceInfo>,
    pub cc_license: Option<String>,
    pub cc_tooltip: Option<String>,
    /// The "Heterodyne recording" badge's tooltip, when the file has one
    /// (see [`heterodyne_tooltip`]).
    pub heterodyne_tooltip: Option<String>,
    /// The "Corrected ×10" badge, when the file's rate is corrected for
    /// time expansion (see [`corrected_rate_badge`]).
    pub corrected_rate: Option<CorrectedRateBadge>,
    pub file_index: usize,
}

/// The badge of a file read at a rate corrected for time expansion, by the
/// app as it opened the file or by the user: "Corrected ×10".
#[derive(Clone, PartialEq)]
pub struct CorrectedRateBadge {
    pub label: String,
    /// The Info panel's time-expansion note: "Corrected for ×10 time
    /// expansion. Sample rate in the file header: 38.4 kHz."
    pub tooltip: String,
}

/// The "Corrected ×10" badge for a file, or `None` when the file is read at
/// the rate its header states.
///
/// It is a box that opens the Info panel at the time-expansion note, so a
/// phone, which shows no tooltips, can reach the full fact. It is grey, not
/// amber: a corrected rate is the right rate, not a warning. Its words are
/// "Corrected ×10", not "TE ×10", which would read as the HEAR bar's TE
/// (time expansion) playback mode, and not "×10" alone, which would read as
/// a multiple of the rate before it.
///
/// The heading shows the badge after the duration. A file row shows the
/// sample rate itself as the box, with the same tooltip and click: at the
/// narrowest Files sidebar (140px) a row has about 20px to spare beside its
/// facts and CC icon, which no words fit, and the user asked that a row wrap
/// no earlier than before. The licence is shown in a similar way: a labelled
/// box in the heading, an icon in a row.
pub(crate) fn corrected_rate_badge(file: &LoadedFile) -> Option<CorrectedRateBadge> {
    let te = file.audio.metadata.time_expansion?;
    Some(CorrectedRateBadge {
        label: format!("Corrected {}", crate::wikimedia::format_factor(te.factor)),
        tooltip: crate::rate_notes::time_expansion_line(te, crate::rate_notes::Kept::of(file)).text,
    })
}

/// The tooltip for a file's "Heterodyne recording" badge, or `None` when the
/// file gets no badge: no hint, or a level C hint, which only the Info panel
/// shows.
pub(crate) fn heterodyne_tooltip(file: &LoadedFile) -> Option<String> {
    let hint = file.heterodyne_hint.as_ref()?;
    crate::heterodyne::shows_badge(hint).then(|| crate::heterodyne::tooltip(hint))
}

/// Format sample rate for display: 44100 → "44.1kHz", 192000 → "192kHz", 48000 → "48kHz"
fn format_sample_rate(sr: u32) -> String {
    let khz = sr as f64 / 1000.0;
    if sr % 1000 == 0 {
        format!("{}kHz", sr / 1000)
    } else {
        // e.g. 44100 → "44.1kHz"
        let formatted = format!("{:.1}kHz", khz);
        // Strip trailing ".0" for clean display
        if formatted.ends_with(".0kHz") {
            format!("{}kHz", sr / 1000)
        } else {
            formatted
        }
    }
}

/// Shared badge row component used by both file menu items and toolbar heading.
///
/// Renders, in order: sample rate, bit depth, duration, "Corrected ×10"
/// (heading only; a file row boxes the sample rate instead),
/// "Heterodyne recording", streaming `[~]`, sequence `#59`, track `[3]`, the
/// download button and the CC icon. Facts get the `badge-fact` class and are
/// plain text; controls get `badge-btn` and are drawn as boxes (style.css,
/// "File badges").
#[component]
pub fn FileBadgeRow(
    data: FileBadgeData,
    /// "toolbar" or "file-menu"
    #[prop(default = "file-menu")]
    context: &'static str,
    /// Controls seq/track badge visibility
    #[prop(into)]
    show_group_badges: Signal<bool>,
    /// Whether seq/track badges are clickable dropdown triggers (toolbar only)
    #[prop(default = false)]
    group_dropdowns: bool,
    /// Whether to show the download button: file-menu rows of browser
    /// recordings that are not downloaded yet. A download clears the file's
    /// recording flag, so the button is only ever shown for an unsaved file.
    #[prop(default = false)]
    show_download: bool,
    /// Download handler (required if show_download is true)
    #[prop(optional)]
    on_download: Option<Callback<()>>,
    /// Sequence badge click handler (toolbar dropdown)
    #[prop(optional)]
    on_seq_click: Option<Callback<()>>,
    /// Track badge click handler (toolbar dropdown)
    #[prop(optional)]
    on_track_click: Option<Callback<()>>,
) -> impl IntoView {
    let sr_label = format_sample_rate(data.sample_rate);

    // Bit depth badge: only for > 16 bit
    let bit_badge = if data.bits_per_sample > 16 {
        if data.is_float && data.bits_per_sample == 32 {
            Some("32 bit float".to_string())
        } else {
            Some(format!("{} bit", data.bits_per_sample))
        }
    } else {
        None
    };

    let dur_label = format_duration_compact(data.duration_secs);
    let is_streaming = data.is_streaming;
    let seq = data.sequence.clone();
    let track = data.track.clone();
    let is_file_menu = context == "file-menu";
    let cc_license = data.cc_license.clone();
    let cc_tooltip = data.cc_tooltip.clone();
    let heterodyne_tooltip = data.heterodyne_tooltip.clone();
    let corrected_rate = data.corrected_rate.clone();
    let file_index = data.file_index;
    let state = expect_context::<AppState>();
    // A badge that opens the Info panel at one of the notes under the Sample
    // rate row. On another file's row the click goes on to the row, which
    // makes that file current, so the Info panel shows it. On the current
    // file's row it stops here: a row click also clears the view history and
    // bookmarks. A Ctrl, Cmd or Shift click is a selection gesture: the row
    // handles it and the current file doesn't change, so Info is not opened.
    let open_note = move |note: NoteKind| {
        move |e: web_sys::MouseEvent| {
            if e.ctrl_key() || e.meta_key() || e.shift_key() {
                return;
            }
            if state.library.current_index().get_untracked() == Some(file_index) {
                e.stop_propagation();
            }
            open_info_panel(state, note);
        }
    };

    // In a file row, the sample rate of a corrected file is the
    // "Corrected ×10" badge's box (see [`corrected_rate_badge`]). The span
    // keeps the dot after the rate outside the box.
    let (row_mark, heading_mark) = if is_file_menu {
        (corrected_rate, None)
    } else {
        (None, corrected_rate)
    };
    let rate = match row_mark {
        Some(badge) => view! {
            <span class="file-badge badge-fact badge-sample-rate">
                <button
                    class="badge-btn file-badge-corrected"
                    title=badge.tooltip
                    on:click=open_note(NoteKind::TimeExpansion)
                >{sr_label}</button>
            </span>
        }
        .into_any(),
        None => view! {
            <span class="file-badge badge-fact badge-sample-rate">{sr_label}</span>
        }
        .into_any(),
    };

    view! {
        // 1. Sample rate (always)
        {rate}

        // 2. Bit depth (only > 16)
        {bit_badge.map(|label| view! {
            <span class="file-badge badge-fact badge-bit-depth">{label}</span>
        })}

        // 3. Duration
        <span class="badge-fact badge-duration">{dur_label}</span>

        // 3a. "Corrected ×10" in the heading, after the rate, bit depth and
        // duration it applies to.
        {heading_mark.map(|badge| view! {
            <button
                class="file-badge badge-btn file-badge-corrected"
                title=badge.tooltip
                on:click=open_note(NoteKind::TimeExpansion)
            >{badge.label}</button>
        })}

        // 3b. Heterodyne badge, after the facts too, so it doesn't split
        // those related values when the row wraps.
        {heterodyne_tooltip.map(|tooltip| view! {
            <button
                class="file-badge badge-btn badge-warn file-badge-heterodyne"
                title=tooltip
                on:click=open_note(NoteKind::Heterodyne)
            >"Heterodyne recording"</button>
        })}

        // 4. Streaming badge
        {is_streaming.then(|| view! {
            <span class="file-badge badge-fact file-badge-streaming" title="Streaming (large file)">"[~]"</span>
        })}

        // 5. Sequence badge (conditional on show_group_badges)
        {seq.map(move |si| {
            let label = format!("#{}", si.sequence_number);
            let tooltip = si.gap_from_prev_secs
                .map(|g| format!("Gap: {}", format_duration_compact(g)))
                .unwrap_or_default();
            if group_dropdowns {
                let on_click = on_seq_click.clone();
                leptos::either::Either::Left(view! {
                    <button
                        class="file-badge badge-btn file-badge-seq"
                        title=tooltip
                        on:click=move |e: web_sys::MouseEvent| {
                            e.stop_propagation();
                            if let Some(cb) = &on_click { cb.run(()); }
                        }
                    >
                        {label}" \u{25BE}"
                    </button>
                })
            } else {
                leptos::either::Either::Right(view! {
                    <span
                        class="file-badge badge-fact file-badge-seq"
                        style:display=move || if show_group_badges.get() { "inline" } else { "none" }
                        title=tooltip
                    >{label}</span>
                })
            }
        })}

        // 6. Track badge (conditional on show_group_badges)
        {track.map(move |ti| {
            let label = format!("[{}]", ti.label);
            if group_dropdowns {
                let on_click = on_track_click.clone();
                leptos::either::Either::Left(view! {
                    <button
                        class="file-badge badge-btn file-badge-track"
                        title="Switch track"
                        on:click=move |e: web_sys::MouseEvent| {
                            e.stop_propagation();
                            if let Some(cb) = &on_click { cb.run(()); }
                        }
                    >
                        {label}" \u{25BE}"
                    </button>
                })
            } else {
                leptos::either::Either::Right(view! {
                    <span
                        class="file-badge badge-fact file-badge-track"
                        style:display=move || if show_group_badges.get() { "inline" } else { "none" }
                    >{label}</span>
                })
            }
        })}

        // 7. Download button (file-menu only, when show_download is true)
        {(is_file_menu && show_download).then(|| {
            let on_dl = on_download.clone();
            view! {
                <button
                    class="badge-btn badge-download-btn unsaved"
                    title="Download WAV (unsaved)"
                    on:click=move |e: web_sys::MouseEvent| {
                        e.stop_propagation();
                        if let Some(cb) = &on_dl { cb.run(()); }
                    }
                ><Icon kind=Icon::Download /></button>
            }
        })}

        // 8. CC badge (file-menu only, non-clickable)
        {(is_file_menu && cc_license.is_some()).then(|| {
            let tooltip = cc_tooltip
                .unwrap_or_else(|| license_title(cc_license.as_deref().unwrap_or("")));
            view! {
                <span class="badge-cc-mini" title=tooltip>
                    <span class="toolbar-cc-icon badge-cc-icon"></span>
                </span>
            }
        })}
    }
}
