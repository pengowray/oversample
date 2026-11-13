//! Licence values from Xeno-canto, the search server and Wikimedia Commons:
//! short labels for badges and search results, Creative Commons full names,
//! dialog titles, and the web address to link a licence to.

/// Parse a licence value into a short label. Takes a Creative Commons address
/// or short name in the forms Xeno-canto, the search server, iNaturalist and
/// Wikimedia Commons send:
/// - "//creativecommons.org/licenses/by-nc-sa/4.0/" -> "CC BY-NC-SA 4.0"
/// - ".../licenses/by/3.0/au/" (a licence ported to one country) -> "CC BY 3.0 AU"
/// - ".../publicdomain/zero/1.0/", "CC0", "cc0 1.0" -> "CC0 1.0"
/// - ".../publicdomain/mark/1.0/", "Public Domain Mark 1.0" -> "Public Domain Mark 1.0"
/// - "CC BY-NC (Attribution-Non-Commercial)" (GBIF) -> "CC BY-NC"
/// - "cc-by-nc" (iNaturalist) -> "CC BY-NC"
/// - "CC BY-SA 3.0 de" (Commons), "cc by-sa 3.0 de", "CC-BY-SA-3.0-DE" -> "CC BY-SA 3.0 DE"
///
/// Other values that start with "CC" and a space or hyphen come back with
/// "CC" in capitals and the rest unchanged. `None` for anything else, such as
/// "GFDL" or "Public domain".
pub(crate) fn parse_cc_license(lic: &str) -> Option<String> {
    let lic = lic.trim();
    let lower = lic.to_ascii_lowercase();
    if let Some(idx) = lower.find("creativecommons.org/") {
        let path = &lower[idx + "creativecommons.org/".len()..];
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        // CC0 and the Public Domain Mark only have version 1.0, so an address
        // without a version still names it.
        let version_or_1 = |part: Option<&&str>| {
            part.filter(|v| is_version(v))
                .map_or("1.0", |v| v)
                .to_string()
        };
        return match parts.as_slice() {
            ["licenses", code, version, rest @ ..] if is_version(version) => {
                let code = code.to_uppercase();
                Some(match rest.first().filter(|p| is_cc_port(p)) {
                    Some(port) => format!("CC {code} {version} {}", port.to_uppercase()),
                    None => format!("CC {code} {version}"),
                })
            }
            // ".../licenses/by-nc/" or ".../licenses/by-nc/legalcode".
            ["licenses", code, ..] => Some(format!("CC {}", code.to_uppercase())),
            ["publicdomain", "zero", rest @ ..] => {
                Some(format!("CC0 {}", version_or_1(rest.first())))
            }
            ["publicdomain", "mark", rest @ ..] => {
                Some(format!("Public Domain Mark {}", version_or_1(rest.first())))
            }
            _ => None,
        };
    }
    let name = without_description(lic);
    let lower = name.to_ascii_lowercase();
    // "CC0", "CC0 1.0", "CC0-1.0", "CC0 1.0 Universal".
    if let Some(rest) = lower.strip_prefix("cc0") {
        let rest = rest.trim_start_matches([' ', '-', '_']);
        if rest.is_empty() || rest.starts_with("1.0") {
            return Some(CC0_LABEL.to_string());
        }
    }
    if let Some(rest) = lower.strip_prefix("public domain mark") {
        if matches!(rest.trim(), "" | "1.0") {
            return Some(PD_MARK_LABEL.to_string());
        }
    }
    let rest = lower.strip_prefix("cc")?;
    if !(rest.is_empty() || rest.starts_with([' ', '-', '_'])) {
        return None;
    }
    // "cc" is ASCII, so the rest of `name` starts at byte 2.
    Some(cc_short_name(rest).unwrap_or_else(|| format!("CC{}", &name[2..])))
}

/// A short CC name with its parts in capitals, from what follows "CC" in a
/// short name: " by-sa 3.0 de" -> "CC BY-SA 3.0 DE", "-by-nc" -> "CC BY-NC".
/// The parts are licence elements (BY, NC, SA, ND), then an optional version,
/// then an optional port. `None` when any part is something else.
fn cc_short_name(rest: &str) -> Option<String> {
    let mut parts = rest
        .split([' ', '-', '_'])
        .filter(|p| !p.is_empty())
        .peekable();
    let mut elements = Vec::new();
    while let Some(e) = parts.next_if(|p| matches!(*p, "by" | "nc" | "sa" | "nd")) {
        elements.push(e.to_ascii_uppercase());
    }
    if elements.is_empty() {
        return None;
    }
    let mut label = format!("CC {}", elements.join("-"));
    if let Some(version) = parts.next_if(|p| is_version(p)) {
        label = format!("{label} {version}");
        if let Some(port) = parts.next_if(|p| is_cc_port(p)) {
            label = format!("{label} {}", port.to_ascii_uppercase());
        }
    }
    parts.next().is_none().then_some(label)
}

/// A licence version: "4.0", "2.5".
fn is_version(part: &str) -> bool {
    part.starts_with(|c: char| c.is_ascii_digit())
        && part
            .split('.')
            .all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

/// The name without a description in brackets after it:
/// "CC BY-NC (Attribution-Non-Commercial)" -> "CC BY-NC".
fn without_description(name: &str) -> &str {
    match name.find(" (") {
        Some(idx) if name.ends_with(')') => name[..idx].trim_end(),
        _ => name,
    }
}

/// A file's licence as the toolbar badge and the search results show it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LicenseLabel {
    /// The short label for a Creative Commons licence ("CC BY-NC 4.0",
    /// "CC0 1.0"), or the licence value as sent ("GFDL", "Fair use").
    pub text: String,
    /// A Creative Commons licence or the Public Domain Mark, shown with the
    /// CC icon.
    pub is_cc: bool,
}

impl LicenseLabel {
    /// The toolbar badge's text. The CC icon stands for "CC ", so a CC
    /// label drops it: "BY-NC 4.0".
    pub fn badge_text(&self) -> &str {
        if self.is_cc {
            self.text.strip_prefix("CC ").unwrap_or(&self.text)
        } else {
            &self.text
        }
    }
}

/// The label for a licence value: its short CC label, or the value as sent
/// for other licences. `None` for an empty value.
pub(crate) fn license_label(lic: &str) -> Option<LicenseLabel> {
    match parse_cc_license(lic) {
        Some(text) => Some(LicenseLabel { text, is_cc: true }),
        None => {
            let lic = lic.trim();
            (!lic.is_empty()).then(|| LicenseLabel {
                text: lic.to_string(),
                is_cc: false,
            })
        }
    }
}

const CC0_LABEL: &str = "CC0 1.0";
const PD_MARK_LABEL: &str = "Public Domain Mark 1.0";

/// The heading for a short licence label: "CC BY 4.0 license", "CC0 1.0 license".
/// The Public Domain Mark labels a work as free of copyright and is not a
/// licence, so its label is used alone.
pub(crate) fn license_title(label: &str) -> String {
    if label.eq_ignore_ascii_case(PD_MARK_LABEL) {
        label.to_string()
    } else {
        format!("{label} license")
    }
}

/// A licence's country or IGO port in a Creative Commons address: "au",
/// "igo", or "scotland" (the 2.5 port for Scotland).
fn is_cc_port(part: &str) -> bool {
    (part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
        || part.eq_ignore_ascii_case("igo")
        || part.eq_ignore_ascii_case("scotland")
}

/// The name of a Creative Commons licence from its short code.
/// e.g. "CC BY-NC 4.0" -> "Creative Commons Attribution-NonCommercial 4.0 International".
/// Versions before 4.0 end in "Unported" (3.0) or "Generic" (1.0 to 2.5), as
/// Creative Commons names them; a ported licence ends in its port code
/// ("CC BY 3.0 AU" -> "... Attribution 3.0 AU"), or "UK: Scotland" for
/// Scotland's. Version 2.1 has only ported licences. Version 1.0 names
/// NoDerivs before NonCommercial, as its pages do: "CC BY-NC-ND 1.0" ->
/// "... Attribution-NoDerivs-NonCommercial 1.0 Generic". "CC0 1.0" is "CC0 1.0
/// Universal" and the Public Domain Mark is "Public Domain Mark 1.0
/// Universal", the titles of their Creative Commons pages. `None` when the
/// code has no version, or names a licence Creative Commons never published
/// ("CC NC-SA 4.0", "CC BY-XX 4.0", "CC BY 2.1 DE", "CC NC-ND 1.0 IL").
pub(crate) fn cc_license_full_name(code: &str) -> Option<String> {
    if code.eq_ignore_ascii_case(CC0_LABEL) {
        return Some("CC0 1.0 Universal".to_string());
    }
    if code.eq_ignore_ascii_case(PD_MARK_LABEL) {
        return Some("Public Domain Mark 1.0 Universal".to_string());
    }
    CcLicense::from_label(code).map(|l| l.full_name())
}

/// A licence Creative Commons published, read from a short label such as
/// "CC BY-NC 4.0" or "CC BY 3.0 AU". Each part is in lower case, as the
/// licence's address spells it.
struct CcLicense {
    /// "by-nc", "nd-nc".
    elements: String,
    /// "4.0".
    version: String,
    /// "au", "igo", "scotland", or `None` for a licence without a port.
    port: Option<String>,
}

impl CcLicense {
    /// `None` when the label has no version, or names a licence Creative
    /// Commons never published ("CC NC-SA 4.0", "CC BY-XX 4.0"), including
    /// one whose port has no licences of that version:
    /// "CC BY 2.1 DE", "CC BY 3.0 SCOTLAND".
    fn from_label(label: &str) -> Option<Self> {
        let lower = label.to_ascii_lowercase();
        let mut words = lower.split_whitespace();
        if words.next()? != "cc" {
            return None;
        }
        let (elements, version) = (words.next()?, words.next()?);
        let port = words.next();
        if words.next().is_some() || !["1.0", "2.0", "2.1", "2.5", "3.0", "4.0"].contains(&version)
        {
            return None;
        }
        // 4.0 licences have no ports. Every 2.1 licence has a port, and only
        // the au, ca, es and jp ports have version 2.1. The Scotland port
        // has only version 2.5.
        let port_has_version = match port {
            None => version != "2.1",
            Some(p) if !is_cc_port(p) => false,
            Some("scotland") => version == "2.5",
            Some(p) => match version {
                "4.0" => false,
                "2.1" => ["au", "ca", "es", "jp"].contains(&p),
                _ => true,
            },
        };
        if !port_has_version {
            return None;
        }
        // Version 1.0's addresses put ND before NC ("by-nd-nc", "nd-nc"), and
        // creativecommons.org redirects "by-nc-nd" and "nc-nd" to them.
        let elements = match (version, elements) {
            ("1.0", "by-nc-nd") => "by-nd-nc",
            ("1.0", "nc-nd") => "nd-nc",
            _ => elements,
        };
        is_published(elements, version, port).then(|| Self {
            elements: elements.to_string(),
            version: version.to_string(),
            port: port.map(str::to_string),
        })
    }

    /// "Creative Commons Attribution-NonCommercial 4.0 International".
    fn full_name(&self) -> String {
        let version = self.version.as_str();
        // The published licences have only these four elements.
        let names: Vec<&str> = self
            .elements
            .split('-')
            .map(|e| match e {
                "by" => "Attribution",
                "nc" => "NonCommercial",
                "sa" => "ShareAlike",
                _ if version == "4.0" => "NoDerivatives",
                _ => "NoDerivs",
            })
            .collect();
        let suffix = match (self.port.as_deref(), version) {
            (Some("scotland"), _) => "UK: Scotland".to_string(),
            (Some(p), _) => p.to_uppercase(),
            (None, "4.0") => "International".to_string(),
            (None, "3.0") => "Unported".to_string(),
            (None, _) => "Generic".to_string(),
        };
        format!("Creative Commons {} {version} {suffix}", names.join("-"))
    }

    /// "https://creativecommons.org/licenses/by/3.0/au/".
    fn page(&self) -> String {
        let (elements, version) = (&self.elements, &self.version);
        match &self.port {
            Some(port) => format!("{CC_BASE}licenses/{elements}/{version}/{port}/"),
            None => format!("{CC_BASE}licenses/{elements}/{version}/"),
        }
    }
}

/// Whether Creative Commons published the licence with these elements, in
/// lower case as its addresses spell them ("by-nc"), in this version and port.
fn is_published(elements: &str, version: &str, port: Option<&str>) -> bool {
    match version {
        // The 1.0 Israel port has only the licences with BY:
        // ".../licenses/by-sa/1.0/il/", but no ".../licenses/sa/1.0/il/".
        "1.0" if port.is_some_and(|p| p.eq_ignore_ascii_case("il")) => {
            CC_ELEMENTS_1_0.contains(&elements) && (elements == "by" || elements.starts_with("by-"))
        }
        "1.0" => CC_ELEMENTS_1_0.contains(&elements),
        "2.0" if port.is_some_and(|p| p.eq_ignore_ascii_case("jp")) => {
            CC_ELEMENTS.contains(&elements) || CC_ELEMENTS_2_0_JP.contains(&elements)
        }
        _ => CC_ELEMENTS.contains(&elements),
    }
}

/// The licences Creative Commons published from version 2.0, by their
/// elements as its addresses spell them. Each one starts with BY, except the
/// 2.0 Japan licences in `CC_ELEMENTS_2_0_JP`.
const CC_ELEMENTS: &[&str] = &["by", "by-sa", "by-nd", "by-nc", "by-nc-sa", "by-nc-nd"];

/// The 2.0 Japan licences without BY, which no other 2.0 port or later
/// version has: ".../licenses/nc/2.0/jp/". Its NoDerivs-NonCommercial
/// licence is "nd-nc"; there is no "nc-nd" page for it.
const CC_ELEMENTS_2_0_JP: &[&str] = &["nc", "nd", "sa", "nc-sa", "nd-nc"];

/// The version 1.0 licences, which include some without BY. Their
/// NoDerivs-NonCommercial licences are "nd-nc" and "by-nd-nc";
/// `CcLicense::from_label` turns "nc-nd" and "by-nc-nd" into these, as
/// creativecommons.org redirects them.
const CC_ELEMENTS_1_0: &[&str] = &[
    "by", "by-sa", "by-nd", "by-nc", "by-nc-sa", "by-nd-nc", "sa", "nd", "nc", "nc-sa", "nd-nc",
];

const CC_BASE: &str = "https://creativecommons.org/";

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

/// The web address to link a file's licence to, from its License value and its
/// "License URL" field. In order of preference:
/// 1. the License value, when it is an address (Xeno-canto sends these);
/// 2. the License URL field (Wikimedia Commons sends one, also for licences
///    that are not Creative Commons, such as GFDL);
/// 3. the Creative Commons page for a short CC name with a version, such as
///    "CC BY-NC 4.0" from the search server or "CC BY-SA 4.0" from Commons.
pub(crate) fn license_link(license: &str, license_url: Option<&str>) -> Option<String> {
    license_href(license)
        .or_else(|| license_url.and_then(license_href))
        .or_else(|| cc_license_page(&parse_cc_license(license)?))
}

/// The licence dialog's title: "CC BY 4.0 license" for a Creative Commons
/// licence, "License" for others.
pub(crate) fn license_dialog_title(license: &str) -> String {
    parse_cc_license(license).map_or_else(|| "License".to_string(), |l| license_title(&l))
}

/// The licence dialog's row under the attribution: the licence's full name
/// ("Creative Commons Attribution-NonCommercial 4.0 International"), or its
/// name as sent for a licence that isn't Creative Commons ("GFDL"), with the
/// address `license_link` finds for it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LicenseNameRow {
    pub text: String,
    pub href: Option<String>,
}

/// The licence dialog's name row for a License value and its "License URL"
/// field. `None` when the row would only repeat the dialog title, with no
/// full name and no link: GBIF's "CC BY-NC (Attribution-Non-Commercial)"
/// has no version, so its title "CC BY-NC license" is all there is to say.
pub(crate) fn license_name_row(license: &str, license_url: Option<&str>) -> Option<LicenseNameRow> {
    let lic = license.trim();
    if lic.is_empty() {
        return None;
    }
    let label = parse_cc_license(lic);
    let full_name = label.as_deref().and_then(cc_license_full_name);
    let href = license_link(lic, license_url);
    let has_more = full_name.is_some() || href.is_some();
    let text = full_name.unwrap_or_else(|| label.as_deref().map_or(lic.to_string(), license_title));
    (has_more || text != license_dialog_title(lic)).then_some(LicenseNameRow { text, href })
}

/// The Creative Commons page for a short licence label with a version, as
/// `parse_cc_license` gives it: "CC BY-NC 4.0" ->
/// "https://creativecommons.org/licenses/by-nc/4.0/", "CC BY 3.0 AU" ->
/// ".../licenses/by/3.0/au/", "CC BY-NC-ND 1.0" -> ".../licenses/by-nd-nc/1.0/",
/// "CC0 1.0" -> ".../publicdomain/zero/1.0/". `None` for a label without a
/// version, or for a licence Creative Commons never published (see
/// `cc_license_full_name`).
fn cc_license_page(label: &str) -> Option<String> {
    if label.eq_ignore_ascii_case(CC0_LABEL) {
        return Some(format!("{CC_BASE}publicdomain/zero/1.0/"));
    }
    if label.eq_ignore_ascii_case(PD_MARK_LABEL) {
        return Some(format!("{CC_BASE}publicdomain/mark/1.0/"));
    }
    CcLicense::from_label(label).map(|l| l.page())
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
            // Scotland's port is named as on its Creative Commons page.
            (
                "CC BY-SA 2.5 SCOTLAND",
                Some("Creative Commons Attribution-ShareAlike 2.5 UK: Scotland"),
            ),
            // Version 2.1 licences all have a port.
            ("CC BY 2.1 JP", Some("Creative Commons Attribution 2.1 JP")),
            (
                "CC BY-NC-SA 2.1 es",
                Some("Creative Commons Attribution-NonCommercial-ShareAlike 2.1 ES"),
            ),
            ("CC BY 2.1 CA", Some("Creative Commons Attribution 2.1 CA")),
            ("CC BY 2.1", None),
            // Only the au, ca, es and jp ports have version 2.1, and the
            // Scotland port has only version 2.5.
            ("CC BY 2.1 DE", None),
            ("CC BY 3.0 SCOTLAND", None),
            ("CC BY-SA 2.0 scotland", None),
            ("CC BY 4.0 AU", None),
            ("CC BY 3.0 legalcode", None),
            ("CC BY-XX 4.0", None),
            ("CC BY", None),
            ("CC BY 5.0", None),
            // Combinations Creative Commons never published.
            ("CC NC-SA 4.0", None),
            ("CC SA 3.0", None),
            ("CC NC 2.0", None),
            ("CC NC 2.0 DE", None),
            ("CC NC 2.5 JP", None),
            ("CC NC-ND 2.0 JP", None),
            ("CC NC-BY 4.0", None),
            ("CC BY-SA-NC 4.0", None),
            ("CC BY-ND-SA 3.0", None),
            ("CC BY-SA-NC 1.0", None),
            ("CC BY-ND-NC 4.0", None),
            // Version 1.0 has licences without BY.
            (
                "CC NC-SA 1.0",
                Some("Creative Commons NonCommercial-ShareAlike 1.0 Generic"),
            ),
            ("CC SA 1.0", Some("Creative Commons ShareAlike 1.0 Generic")),
            (
                "CC BY-ND-NC 1.0",
                Some("Creative Commons Attribution-NoDerivs-NonCommercial 1.0 Generic"),
            ),
            // creativecommons.org redirects 1.0's "by-nc-nd" and "nc-nd" to
            // "by-nd-nc" and "nd-nc", so they get the same names.
            (
                "CC BY-NC-ND 1.0",
                Some("Creative Commons Attribution-NoDerivs-NonCommercial 1.0 Generic"),
            ),
            (
                "CC NC-ND 1.0",
                Some("Creative Commons NoDerivs-NonCommercial 1.0 Generic"),
            ),
            (
                "CC ND-NC 1.0",
                Some("Creative Commons NoDerivs-NonCommercial 1.0 Generic"),
            ),
            (
                "CC BY-NC-ND 1.0 FI",
                Some("Creative Commons Attribution-NoDerivs-NonCommercial 1.0 FI"),
            ),
            (
                "CC NC 1.0 FI",
                Some("Creative Commons NonCommercial 1.0 FI"),
            ),
            // The 1.0 Israel port has only the licences with BY.
            (
                "CC BY-SA 1.0 IL",
                Some("Creative Commons Attribution-ShareAlike 1.0 IL"),
            ),
            (
                "CC BY-NC-ND 1.0 IL",
                Some("Creative Commons Attribution-NoDerivs-NonCommercial 1.0 IL"),
            ),
            ("CC NC-ND 1.0 IL", None),
            ("CC SA 1.0 IL", None),
            // The 2.0 Japan port also has licences without BY.
            (
                "CC NC 2.0 JP",
                Some("Creative Commons NonCommercial 2.0 JP"),
            ),
            (
                "CC NC-SA 2.0 JP",
                Some("Creative Commons NonCommercial-ShareAlike 2.0 JP"),
            ),
            (
                "cc nd-nc 2.0 jp",
                Some("Creative Commons NoDerivs-NonCommercial 2.0 JP"),
            ),
            ("CC0 1.0", Some("CC0 1.0 Universal")),
            ("cc0 1.0", Some("CC0 1.0 Universal")),
            ("CC0", None),
            (
                "Public Domain Mark 1.0",
                Some("Public Domain Mark 1.0 Universal"),
            ),
            ("CC BY-NC (Attribution-Non-Commercial)", None),
            ("Public domain", None),
            ("", None),
        ] {
            assert_eq!(cc_license_full_name(code).as_deref(), name, "{code}");
        }
    }

    #[test]
    fn short_labels() {
        for (lic, label) in [
            // Xeno-canto and other Creative Commons addresses.
            (
                "//creativecommons.org/licenses/by-nc-sa/4.0/",
                Some("CC BY-NC-SA 4.0"),
            ),
            (
                "http://creativecommons.org/licenses/by/3.0/au/",
                Some("CC BY 3.0 AU"),
            ),
            (
                "https://creativecommons.org/licenses/by-sa/3.0",
                Some("CC BY-SA 3.0"),
            ),
            (
                "https://creativecommons.org/licenses/by-nc/",
                Some("CC BY-NC"),
            ),
            (
                "https://creativecommons.org/publicdomain/zero/1.0/",
                Some("CC0 1.0"),
            ),
            // Wikimedia Commons' License URL for CC0.
            (
                "http://creativecommons.org/publicdomain/zero/1.0/deed.en",
                Some("CC0 1.0"),
            ),
            ("//creativecommons.org/publicdomain/zero/", Some("CC0 1.0")),
            (
                "https://creativecommons.org/publicdomain/mark/1.0/",
                Some("Public Domain Mark 1.0"),
            ),
            (
                "https://creativecommons.org/licenses/nc/2.0/jp/",
                Some("CC NC 2.0 JP"),
            ),
            (
                "http://creativecommons.org/licenses/by-sa/2.5/scotland/",
                Some("CC BY-SA 2.5 SCOTLAND"),
            ),
            (
                "https://creativecommons.org/licenses/by/2.1/jp/",
                Some("CC BY 2.1 JP"),
            ),
            ("https://creativecommons.org/about/", None),
            // An address with a page name where the version would be.
            (
                "https://creativecommons.org/licenses/by-nc/legalcode",
                Some("CC BY-NC"),
            ),
            // Short names from the search server and Wikimedia Commons.
            ("CC BY-NC 4.0", Some("CC BY-NC 4.0")),
            (" CC BY-SA 3.0 de ", Some("CC BY-SA 3.0 DE")),
            ("cc by-sa 4.0", Some("CC BY-SA 4.0")),
            ("CC BY-SA 3.0 igo", Some("CC BY-SA 3.0 IGO")),
            ("cc by-sa 2.5 scotland", Some("CC BY-SA 2.5 SCOTLAND")),
            // A 1.0 licence keeps its spelling here; only its full name and
            // page address use the order creativecommons.org uses.
            ("CC BY-NC-ND 1.0", Some("CC BY-NC-ND 1.0")),
            ("cc nc-nd 1.0", Some("CC NC-ND 1.0")),
            // GBIF's short name with a description.
            ("CC BY-NC (Attribution-Non-Commercial)", Some("CC BY-NC")),
            ("CC0 (Public Domain Dedication)", Some("CC0 1.0")),
            // iNaturalist's codes, and SPDX-style names.
            ("cc-by-nc", Some("CC BY-NC")),
            ("cc-by-nc-sa", Some("CC BY-NC-SA")),
            ("CC-BY 4.0", Some("CC BY 4.0")),
            ("CC-BY-NC-4.0", Some("CC BY-NC 4.0")),
            ("CC-BY-SA-3.0-DE", Some("CC BY-SA 3.0 DE")),
            // A CC name the parser can't read keeps its text, with "CC" in capitals.
            ("CC", Some("CC")),
            ("cc Attribution 4.0", Some("CC Attribution 4.0")),
            ("CC BY 4.0 International", Some("CC BY 4.0 International")),
            ("CC BY-NC (", Some("CC BY-NC (")),
            ("ccMixter", None),
            ("CC0", Some("CC0 1.0")),
            ("cc0", Some("CC0 1.0")),
            ("CC0 1.0", Some("CC0 1.0")),
            ("CC0-1.0", Some("CC0 1.0")),
            ("CC0 1.0 Universal", Some("CC0 1.0")),
            ("Public Domain Mark 1.0", Some("Public Domain Mark 1.0")),
            ("public domain mark", Some("Public Domain Mark 1.0")),
            ("Public domain", None),
            ("GFDL", None),
            ("Fair use", None),
            ("", None),
        ] {
            assert_eq!(parse_cc_license(lic).as_deref(), label, "{lic}");
        }
    }

    /// Every licence value in the search server's index on 2026-11-14, with
    /// its source and number of recordings, and the forms the task names.
    /// The search results list shows `text`; the toolbar badge shows
    /// `badge_text`.
    #[test]
    fn labels_for_search_results_and_badges() {
        for (lic, text, badge) in [
            // GBIF
            (
                "http://creativecommons.org/licenses/by-sa/4.0/",
                "CC BY-SA 4.0",
                "BY-SA 4.0",
            ), // 3,096
            ("CC BY-NC (Attribution-Non-Commercial)", "CC BY-NC", "BY-NC"), // 1,634
            (
                "http://creativecommons.org/licenses/by-nc/4.0/",
                "CC BY-NC 4.0",
                "BY-NC 4.0",
            ), // 731
            (
                "https://creativecommons.org/licenses/by-sa/4.0/deed.es",
                "CC BY-SA 4.0",
                "BY-SA 4.0",
            ), // 662
            (
                "http://creativecommons.org/licenses/by/4.0/",
                "CC BY 4.0",
                "BY 4.0",
            ), // 541
            (
                "http://creativecommons.org/licenses/by-nc-nd/4.0/",
                "CC BY-NC-ND 4.0",
                "BY-NC-ND 4.0",
            ), // 33
            (
                "http://creativecommons.org/licenses/by/4.0/legalcode",
                "CC BY 4.0",
                "BY 4.0",
            ), // 12
            (
                "http://creativecommons.org/licenses/by-nc-sa/4.0/",
                "CC BY-NC-SA 4.0",
                "BY-NC-SA 4.0",
            ), // 12
            (
                "http://creativecommons.org/licenses/by-sa/3.0/",
                "CC BY-SA 3.0",
                "BY-SA 3.0",
            ), // 1
            (
                "http://creativecommons.org/licenses/by-nd/4.0/",
                "CC BY-ND 4.0",
                "BY-ND 4.0",
            ), // 1
            (
                "http://creativecommons.org/licenses/by-nc/4.0/legalcode",
                "CC BY-NC 4.0",
                "BY-NC 4.0",
            ), // 1
            (
                "http://creativecommons.org/publicdomain/zero/1.0/legalcode",
                "CC0 1.0",
                "CC0 1.0",
            ),
            // Xeno-canto
            (
                "https://creativecommons.org/licenses/by-nc-sa/4.0/",
                "CC BY-NC-SA 4.0",
                "BY-NC-SA 4.0",
            ), // 12,740
            (
                "https://creativecommons.org/licenses/by-nc-nd/4.0/",
                "CC BY-NC-ND 4.0",
                "BY-NC-ND 4.0",
            ), // 3,037
            (
                "https://creativecommons.org/licenses/by-sa/4.0/",
                "CC BY-SA 4.0",
                "BY-SA 4.0",
            ), // 890
            (
                "https://creativecommons.org/licenses/by-nc/4.0/",
                "CC BY-NC 4.0",
                "BY-NC 4.0",
            ), // 883
            (
                "https://creativecommons.org/licenses/by/4.0/",
                "CC BY 4.0",
                "BY 4.0",
            ), // 392
            (
                "https://creativecommons.org/publicdomain/zero/1.0/",
                "CC0 1.0",
                "CC0 1.0",
            ), // 78
            (
                "https://creativecommons.org/licenses/by-sa/3.0/",
                "CC BY-SA 3.0",
                "BY-SA 3.0",
            ), // 5
            (
                "https://creativecommons.org/licenses/by-nc-sa/3.0/",
                "CC BY-NC-SA 3.0",
                "BY-NC-SA 3.0",
            ), // 4
            (
                "https://creativecommons.org/licenses/by-nc-nd/2.5/",
                "CC BY-NC-ND 2.5",
                "BY-NC-ND 2.5",
            ), // 1
            // Xeno-canto's API sends addresses without "https:".
            (
                "//creativecommons.org/licenses/by-nc-sa/4.0/",
                "CC BY-NC-SA 4.0",
                "BY-NC-SA 4.0",
            ),
            // Short names
            ("CC0", "CC0 1.0", "CC0 1.0"),
            ("CC0 1.0", "CC0 1.0", "CC0 1.0"),
            (
                "https://creativecommons.org/publicdomain/mark/1.0/",
                "Public Domain Mark 1.0",
                "Public Domain Mark 1.0",
            ),
            ("cc-by-nc", "CC BY-NC", "BY-NC"),
            (
                "http://creativecommons.org/licenses/by/3.0/au/",
                "CC BY 3.0 AU",
                "BY 3.0 AU",
            ),
            ("CC BY-SA 3.0 de", "CC BY-SA 3.0 DE", "BY-SA 3.0 DE"),
        ] {
            let label = license_label(lic).unwrap();
            assert_eq!(label.text, text, "{lic}");
            assert_eq!(label.badge_text(), badge, "{lic}");
            assert!(label.is_cc, "{lic}");
        }
        // Licences that are not Creative Commons keep their name, in the
        // results and on the badge.
        for (lic, text) in [
            ("GFDL", "GFDL"),
            (" Fair use ", "Fair use"),
            ("Public domain", "Public domain"),
        ] {
            let label = license_label(lic).unwrap();
            assert_eq!(label.text, text, "{lic}");
            assert_eq!(label.badge_text(), text, "{lic}");
            assert!(!label.is_cc, "{lic}");
        }
        assert_eq!(license_label("  "), None);
    }

    #[test]
    fn dialog_name_rows() {
        let row = |text: &str, href: Option<&str>| {
            Some(LicenseNameRow {
                text: text.to_string(),
                href: href.map(str::to_string),
            })
        };
        for (lic, url, title, name_row) in [
            // Xeno-canto: the full name, linked.
            (
                "//creativecommons.org/licenses/by-nc-sa/4.0/",
                None,
                "CC BY-NC-SA 4.0 license",
                row(
                    "Creative Commons Attribution-NonCommercial-ShareAlike 4.0 International",
                    Some("https://creativecommons.org/licenses/by-nc-sa/4.0/"),
                ),
            ),
            // A 2.0 Japan licence without BY.
            (
                "https://creativecommons.org/licenses/nc/2.0/jp/",
                None,
                "CC NC 2.0 JP license",
                row(
                    "Creative Commons NonCommercial 2.0 JP",
                    Some("https://creativecommons.org/licenses/nc/2.0/jp/"),
                ),
            ),
            // A 1.0 licence: the title keeps the spelling sent, and the full
            // name and link use the order creativecommons.org uses.
            (
                "CC BY-NC-ND 1.0",
                None,
                "CC BY-NC-ND 1.0 license",
                row(
                    "Creative Commons Attribution-NoDerivs-NonCommercial 1.0 Generic",
                    Some("https://creativecommons.org/licenses/by-nd-nc/1.0/"),
                ),
            ),
            // The Scotland port, and a 2.1 licence.
            (
                "http://creativecommons.org/licenses/by-sa/2.5/scotland/",
                None,
                "CC BY-SA 2.5 SCOTLAND license",
                row(
                    "Creative Commons Attribution-ShareAlike 2.5 UK: Scotland",
                    Some("http://creativecommons.org/licenses/by-sa/2.5/scotland/"),
                ),
            ),
            (
                "https://creativecommons.org/licenses/by/2.1/jp/",
                None,
                "CC BY 2.1 JP license",
                row(
                    "Creative Commons Attribution 2.1 JP",
                    Some("https://creativecommons.org/licenses/by/2.1/jp/"),
                ),
            ),
            // GBIF's short name has no version, so no full name or link:
            // the row would repeat the title.
            (
                "CC BY-NC (Attribution-Non-Commercial)",
                None,
                "CC BY-NC license",
                None,
            ),
            ("cc-by-nc", None, "CC BY-NC license", None),
            // An address without a version still gives a link.
            (
                "https://creativecommons.org/licenses/by-nc/",
                None,
                "CC BY-NC license",
                row(
                    "CC BY-NC license",
                    Some("https://creativecommons.org/licenses/by-nc/"),
                ),
            ),
            // Commons: CC0 with its License URL, the Public Domain Mark, GFDL.
            (
                "CC0",
                Some("http://creativecommons.org/publicdomain/zero/1.0/deed.en"),
                "CC0 1.0 license",
                row(
                    "CC0 1.0 Universal",
                    Some("http://creativecommons.org/publicdomain/zero/1.0/deed.en"),
                ),
            ),
            (
                "Public Domain Mark 1.0",
                None,
                "Public Domain Mark 1.0",
                row(
                    "Public Domain Mark 1.0 Universal",
                    Some("https://creativecommons.org/publicdomain/mark/1.0/"),
                ),
            ),
            (
                "GFDL",
                Some("http://www.gnu.org/copyleft/fdl.html"),
                "License",
                row("GFDL", Some("http://www.gnu.org/copyleft/fdl.html")),
            ),
            // A Wikipedia's non-free file: the name differs from the title.
            ("Fair use", None, "License", row("Fair use", None)),
            ("", None, "License", None),
        ] {
            assert_eq!(license_dialog_title(lic), title, "{lic}");
            assert_eq!(license_name_row(lic, url), name_row, "{lic}");
        }
    }

    #[test]
    fn titles() {
        assert_eq!(license_title("CC BY-NC 4.0"), "CC BY-NC 4.0 license");
        assert_eq!(license_title("CC0 1.0"), "CC0 1.0 license");
        assert_eq!(
            license_title("Public Domain Mark 1.0"),
            "Public Domain Mark 1.0"
        );
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

    #[test]
    fn license_links_for_short_names() {
        for (lic, href) in [
            (
                "CC BY-NC 4.0",
                Some("https://creativecommons.org/licenses/by-nc/4.0/"),
            ),
            (
                "CC BY-NC-SA 4.0",
                Some("https://creativecommons.org/licenses/by-nc-sa/4.0/"),
            ),
            (
                "CC BY 3.0 AU",
                Some("https://creativecommons.org/licenses/by/3.0/au/"),
            ),
            (
                "CC BY-SA 3.0 igo",
                Some("https://creativecommons.org/licenses/by-sa/3.0/igo/"),
            ),
            (
                "cc by-sa 2.5",
                Some("https://creativecommons.org/licenses/by-sa/2.5/"),
            ),
            (
                "CC0 1.0",
                Some("https://creativecommons.org/publicdomain/zero/1.0/"),
            ),
            // There is only one CC0, so a name without its version still links.
            (
                "CC0",
                Some("https://creativecommons.org/publicdomain/zero/1.0/"),
            ),
            (
                "Public Domain Mark 1.0",
                Some("https://creativecommons.org/publicdomain/mark/1.0/"),
            ),
            (
                "CC NC-SA 1.0",
                Some("https://creativecommons.org/licenses/nc-sa/1.0/"),
            ),
            (
                "CC BY-ND-NC 1.0",
                Some("https://creativecommons.org/licenses/by-nd-nc/1.0/"),
            ),
            (
                "CC BY-NC-ND 1.0",
                Some("https://creativecommons.org/licenses/by-nd-nc/1.0/"),
            ),
            (
                "CC NC-ND 1.0",
                Some("https://creativecommons.org/licenses/nd-nc/1.0/"),
            ),
            (
                "CC BY-NC-ND 1.0 FI",
                Some("https://creativecommons.org/licenses/by-nd-nc/1.0/fi/"),
            ),
            (
                "CC NC 2.0 JP",
                Some("https://creativecommons.org/licenses/nc/2.0/jp/"),
            ),
            (
                "CC NC-SA 2.0 JP",
                Some("https://creativecommons.org/licenses/nc-sa/2.0/jp/"),
            ),
            (
                "CC BY-SA 2.5 SCOTLAND",
                Some("https://creativecommons.org/licenses/by-sa/2.5/scotland/"),
            ),
            (
                "CC BY 2.1 JP",
                Some("https://creativecommons.org/licenses/by/2.1/jp/"),
            ),
            (
                "CC BY-SA 2.1 CA",
                Some("https://creativecommons.org/licenses/by-sa/2.1/ca/"),
            ),
            (
                "CC BY-NC-ND 1.0 IL",
                Some("https://creativecommons.org/licenses/by-nd-nc/1.0/il/"),
            ),
            // No version, or a name Creative Commons doesn't have.
            ("CC BY 2.1", None),
            ("CC BY 2.1 DE", None),
            ("CC BY 3.0 SCOTLAND", None),
            ("CC NC-ND 1.0 IL", None),
            ("CC NC-SA 4.0", None),
            ("CC NC 2.0", None),
            ("CC BY-SA-NC 2.5", None),
            ("CC BY", None),
            ("CC BY-NC (Attribution-Non-Commercial)", None),
            ("CC BY 4.0 AU", None),
            ("CC BY-XX 4.0", None),
            ("cc-by-nc", None),
            ("Public domain", None),
            ("GFDL", None),
            ("", None),
        ] {
            assert_eq!(license_link(lic, None).as_deref(), href, "{lic}");
        }
    }

    #[test]
    fn license_link_preference() {
        // A License value that is an address wins.
        assert_eq!(
            license_link(
                "//creativecommons.org/licenses/by-nc-sa/4.0/",
                Some("https://example.org/other")
            )
            .as_deref(),
            Some("https://creativecommons.org/licenses/by-nc-sa/4.0/")
        );
        // Then the License URL field, as Wikimedia Commons sends it.
        assert_eq!(
            license_link(
                "CC BY-SA 3.0",
                Some("https://creativecommons.org/licenses/by-sa/3.0")
            )
            .as_deref(),
            Some("https://creativecommons.org/licenses/by-sa/3.0")
        );
        assert_eq!(
            license_link("GFDL", Some("http://www.gnu.org/copyleft/fdl.html")).as_deref(),
            Some("http://www.gnu.org/copyleft/fdl.html")
        );
        // A License URL that isn't a web address is skipped.
        assert_eq!(
            license_link("CC BY 4.0", Some("javascript:alert(1)")).as_deref(),
            Some("https://creativecommons.org/licenses/by/4.0/")
        );
        assert_eq!(license_link("GFDL", Some("not a link")), None);
        assert_eq!(license_link("Public domain", None), None);
    }
}
