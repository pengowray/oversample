use super::file_groups::{SequenceInfo, TrackInfo};
use crate::components::icons::Icon;
use crate::format_time::format_duration_compact;
use leptos::prelude::*;

/// Parse a CC license URL/string (from XC metadata "lic" field) into a short label.
/// e.g. "//creativecommons.org/licenses/by-nc-sa/4.0/" -> "CC BY-NC-SA 4.0", and a
/// licence ported to one country, ".../by/3.0/au/" -> "CC BY 3.0 AU".
pub(crate) fn parse_cc_license(lic: &str) -> Option<String> {
    let lower = lic.to_lowercase();
    if lower.contains("creativecommons.org/licenses/") {
        if let Some(idx) = lower.find("/licenses/") {
            let rest = &lic[idx + 10..];
            let parts: Vec<&str> = rest.trim_matches('/').split('/').collect();
            if parts.len() >= 2 {
                let license_type = parts[0].to_uppercase();
                let version = parts[1];
                if let Some(port) = parts.get(2).filter(|p| is_cc_port(p)) {
                    return Some(format!(
                        "CC {} {} {}",
                        license_type,
                        version,
                        port.to_uppercase()
                    ));
                }
                return Some(format!("CC {} {}", license_type, version));
            } else if !parts.is_empty() {
                return Some(format!("CC {}", parts[0].to_uppercase()));
            }
        }
    }
    if lower.starts_with("cc") {
        return Some(lic.to_string());
    }
    None
}

/// A licence's country or IGO port in a Creative Commons address: "au", "igo".
fn is_cc_port(part: &str) -> bool {
    (part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
        || part.eq_ignore_ascii_case("igo")
}

/// The name of a Creative Commons licence from its short code.
/// e.g. "CC BY-NC 4.0" -> "Creative Commons Attribution-NonCommercial 4.0 International".
/// Versions before 4.0 end in "Unported" (3.0) or "Generic" (1.0 to 2.5), as
/// Creative Commons names them; a ported licence ends in its port code
/// ("CC BY 3.0 AU" -> "... Attribution 3.0 AU"). `None` when the code has no
/// version or has an unknown element.
pub(crate) fn cc_license_full_name(code: &str) -> Option<String> {
    let mut words = code.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("cc") {
        return None;
    }
    let (elements, version) = (words.next()?, words.next()?);
    let port = words.next();
    if words.next().is_some() || !["1.0", "2.0", "2.5", "3.0", "4.0"].contains(&version) {
        return None;
    }
    // 4.0 licences have no ports.
    if port.is_some_and(|p| !is_cc_port(p) || version == "4.0") {
        return None;
    }
    let names = elements
        .split('-')
        .map(|e| match e.to_ascii_uppercase().as_str() {
            "BY" => Some("Attribution"),
            "NC" => Some("NonCommercial"),
            "SA" => Some("ShareAlike"),
            "ND" if version == "4.0" => Some("NoDerivatives"),
            "ND" => Some("NoDerivs"),
            _ => None,
        })
        .collect::<Option<Vec<_>>>()?;
    let suffix = match (port, version) {
        (Some(p), _) => format!(" {}", p.to_uppercase()),
        (None, "4.0") => " International".to_string(),
        (None, "3.0") => " Unported".to_string(),
        (None, _) => " Generic".to_string(),
    };
    Some(format!(
        "Creative Commons {} {version}{suffix}",
        names.join("-")
    ))
}

/// The web address to link a License value to. An address that starts with
/// "//" (as Xeno-canto sends) gets "https:". `None` for anything else, such as
/// Wikimedia Commons' short names ("CC BY-SA 4.0").
pub(crate) fn license_href(lic: &str) -> Option<String> {
    let v = lic.trim();
    let url = if v.starts_with("//") {
        format!("https:{v}")
    } else {
        v.to_string()
    };
    let lower = url.to_ascii_lowercase();
    ((lower.starts_with("https://") || lower.starts_with("http://"))
        && !url.contains(char::is_whitespace))
    .then_some(url)
}

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
    pub is_unsaved: bool,
    pub is_streaming: bool,
    pub track: Option<TrackInfo>,
    pub sequence: Option<SequenceInfo>,
    pub cc_license: Option<String>,
    pub cc_tooltip: Option<String>,
    pub file_index: usize,
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
/// Renders: [sample_rate] [bit_depth] duration [~] #seq [track] [💾] [CC]
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
    /// Whether to show the download button (file-menu recordings only)
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
    let is_unsaved = data.is_unsaved;
    let is_file_menu = context == "file-menu";
    let cc_license = data.cc_license.clone();
    let cc_tooltip = data.cc_tooltip.clone();

    view! {
        // 1. Sample rate badge (always)
        <span class="file-badge badge-sample-rate">{sr_label}</span>

        // 2. Bit depth badge (only > 16)
        {bit_badge.map(|label| view! {
            <span class="file-badge badge-bit-depth">{label}</span>
        })}

        // 3. Duration (plain text)
        <span class="badge-duration">{dur_label}</span>

        // 4. Streaming badge
        {is_streaming.then(|| view! {
            <span class="file-badge file-badge-streaming" title="Streaming (large file)">"[~]"</span>
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
                        class="file-badge file-badge-seq badge-clickable"
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
                        class="file-badge file-badge-seq"
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
                        class="file-badge file-badge-track badge-clickable"
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
                        class="file-badge file-badge-track"
                        style:display=move || if show_group_badges.get() { "inline" } else { "none" }
                    >{label}</span>
                })
            }
        })}

        // 7. Download button (file-menu only, when show_download is true)
        {(is_file_menu && show_download).then(|| {
            let on_dl = on_download.clone();
            let cls = if is_unsaved {
                "badge-download-btn unsaved"
            } else {
                "badge-download-btn saved"
            };
            let title = if is_unsaved { "Download WAV (unsaved)" } else { "Download WAV" };
            view! {
                <button
                    class=cls
                    title=title
                    on:click=move |e: web_sys::MouseEvent| {
                        e.stop_propagation();
                        if let Some(cb) = &on_dl { cb.run(()); }
                    }
                ><Icon kind=Icon::Download /></button>
            }
        })}

        // 8. CC badge (file-menu only, non-clickable)
        {(is_file_menu && cc_license.is_some()).then(|| {
            let tooltip = cc_tooltip.unwrap_or_else(|| {
                format!("{} license", cc_license.as_deref().unwrap_or(""))
            });
            view! {
                <span class="badge-cc-mini" title=tooltip>
                    <span class="toolbar-cc-icon badge-cc-icon"></span>
                </span>
            }
        })}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_licence_names() {
        for (code, name) in [
            (
                "CC BY-NC 4.0",
                Some("Creative Commons Attribution-NonCommercial 4.0 International"),
            ),
            (
                "CC BY-SA 4.0",
                Some("Creative Commons Attribution-ShareAlike 4.0 International"),
            ),
            (
                "CC BY-NC-ND 3.0",
                Some("Creative Commons Attribution-NonCommercial-NoDerivs 3.0 Unported"),
            ),
            (
                "CC BY-NC-ND 4.0",
                Some("Creative Commons Attribution-NonCommercial-NoDerivatives 4.0 International"),
            ),
            (
                "CC BY-ND 2.5",
                Some("Creative Commons Attribution-NoDerivs 2.5 Generic"),
            ),
            (
                "CC BY 1.0",
                Some("Creative Commons Attribution 1.0 Generic"),
            ),
            (
                "cc by-sa 2.0",
                Some("Creative Commons Attribution-ShareAlike 2.0 Generic"),
            ),
            ("CC BY 3.0 AU", Some("Creative Commons Attribution 3.0 AU")),
            (
                "CC BY-SA 3.0 igo",
                Some("Creative Commons Attribution-ShareAlike 3.0 IGO"),
            ),
            ("CC BY 4.0 AU", None),
            ("CC BY 3.0 legalcode", None),
            ("CC BY-XX 4.0", None),
            ("CC BY", None),
            ("CC BY 5.0", None),
            ("CC0 1.0", None),
            ("CC BY-NC (Attribution-Non-Commercial)", None),
            ("Public domain", None),
            ("", None),
        ] {
            assert_eq!(cc_license_full_name(code).as_deref(), name, "{code}");
        }
    }

    #[test]
    fn full_name_of_parsed_url() {
        let code = parse_cc_license("//creativecommons.org/licenses/by-nc-nd/3.0/").unwrap();
        assert_eq!(
            cc_license_full_name(&code).as_deref(),
            Some("Creative Commons Attribution-NonCommercial-NoDerivs 3.0 Unported")
        );
        let ported = parse_cc_license("http://creativecommons.org/licenses/by/3.0/au/").unwrap();
        assert_eq!(ported, "CC BY 3.0 AU");
        assert_eq!(
            cc_license_full_name(&ported).as_deref(),
            Some("Creative Commons Attribution 3.0 AU")
        );
        assert_eq!(
            parse_cc_license("https://creativecommons.org/licenses/by/4.0/legalcode").as_deref(),
            Some("CC BY 4.0")
        );
    }

    #[test]
    fn license_links() {
        for (lic, href) in [
            (
                "//creativecommons.org/licenses/by-nc-sa/4.0/",
                Some("https://creativecommons.org/licenses/by-nc-sa/4.0/"),
            ),
            (
                "http://creativecommons.org/licenses/by/4.0/",
                Some("http://creativecommons.org/licenses/by/4.0/"),
            ),
            (
                " https://creativecommons.org/licenses/by-nc/4.0/ ",
                Some("https://creativecommons.org/licenses/by-nc/4.0/"),
            ),
            ("CC BY-SA 4.0", None),
            ("Public domain", None),
            ("cc-by-nc", None),
            ("", None),
        ] {
            assert_eq!(license_href(lic).as_deref(), href, "{lic}");
        }
    }
}
