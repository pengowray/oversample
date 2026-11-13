//! Licence values from Xeno-canto, the search server and Wikimedia Commons:
//! short labels for badges and search results, Creative Commons full names,
//! dialog titles, and the web address to link a licence to.

/// Parse a licence value into a short label. Takes a Creative Commons address
/// or short name in the forms Xeno-canto, the search server and Wikimedia
/// Commons send:
/// - "//creativecommons.org/licenses/by-nc-sa/4.0/" -> "CC BY-NC-SA 4.0"
/// - ".../licenses/by/3.0/au/" (a licence ported to one country) -> "CC BY 3.0 AU"
/// - ".../publicdomain/zero/1.0/", "CC0", "cc0 1.0" -> "CC0 1.0"
/// - ".../publicdomain/mark/1.0/", "Public Domain Mark 1.0" -> "Public Domain Mark 1.0"
///
/// Other values that start with "CC" come back unchanged. `None` for anything
/// else, such as "GFDL" or "Public domain".
pub(crate) fn parse_cc_license(lic: &str) -> Option<String> {
    let lic = lic.trim();
    let lower = lic.to_ascii_lowercase();
    if let Some(idx) = lower.find("creativecommons.org/") {
        let path = &lower[idx + "creativecommons.org/".len()..];
        let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
        // CC0 and the Public Domain Mark only have version 1.0, so an address
        // without a version still names it.
        let version_or_1 = |part: Option<&&str>| {
            part.filter(|v| v.starts_with(|c: char| c.is_ascii_digit()))
                .map_or("1.0", |v| v)
                .to_string()
        };
        return match parts.as_slice() {
            ["licenses", code, version, rest @ ..] => {
                let code = code.to_uppercase();
                Some(match rest.first().filter(|p| is_cc_port(p)) {
                    Some(port) => format!("CC {code} {version} {}", port.to_uppercase()),
                    None => format!("CC {code} {version}"),
                })
            }
            ["licenses", code] => Some(format!("CC {}", code.to_uppercase())),
            ["publicdomain", "zero", rest @ ..] => {
                Some(format!("CC0 {}", version_or_1(rest.first())))
            }
            ["publicdomain", "mark", rest @ ..] => {
                Some(format!("Public Domain Mark {}", version_or_1(rest.first())))
            }
            _ => None,
        };
    }
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
    if lower.starts_with("cc") {
        return Some(lic.to_string());
    }
    None
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

/// A licence's country or IGO port in a Creative Commons address: "au", "igo".
fn is_cc_port(part: &str) -> bool {
    (part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
        || part.eq_ignore_ascii_case("igo")
}

/// The name of a Creative Commons licence from its short code.
/// e.g. "CC BY-NC 4.0" -> "Creative Commons Attribution-NonCommercial 4.0 International".
/// Versions before 4.0 end in "Unported" (3.0) or "Generic" (1.0 to 2.5), as
/// Creative Commons names them; a ported licence ends in its port code
/// ("CC BY 3.0 AU" -> "... Attribution 3.0 AU"). "CC0 1.0" is "CC0 1.0
/// Universal", and the Public Domain Mark keeps its label. `None` when the code
/// has no version or has an unknown element.
pub(crate) fn cc_license_full_name(code: &str) -> Option<String> {
    if code.eq_ignore_ascii_case(CC0_LABEL) {
        return Some("CC0 1.0 Universal".to_string());
    }
    if code.eq_ignore_ascii_case(PD_MARK_LABEL) {
        return Some(PD_MARK_LABEL.to_string());
    }
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

/// The Creative Commons page for a short licence label with a version, as
/// `parse_cc_license` gives it: "CC BY-NC 4.0" ->
/// "https://creativecommons.org/licenses/by-nc/4.0/", "CC BY 3.0 AU" ->
/// ".../licenses/by/3.0/au/", "CC0 1.0" -> ".../publicdomain/zero/1.0/".
/// `None` for a label without a version or with an unknown element.
fn cc_license_page(label: &str) -> Option<String> {
    const BASE: &str = "https://creativecommons.org/";
    if label.eq_ignore_ascii_case(CC0_LABEL) {
        return Some(format!("{BASE}publicdomain/zero/1.0/"));
    }
    if label.eq_ignore_ascii_case(PD_MARK_LABEL) {
        return Some(format!("{BASE}publicdomain/mark/1.0/"));
    }
    // Only labels with a known name have a page.
    cc_license_full_name(label)?;
    let lower = label.to_ascii_lowercase();
    let mut words = lower.split_whitespace().skip(1);
    let (elements, version) = (words.next()?, words.next()?);
    Some(match words.next() {
        Some(port) => format!("{BASE}licenses/{elements}/{version}/{port}/"),
        None => format!("{BASE}licenses/{elements}/{version}/"),
    })
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
            ("CC0 1.0", Some("CC0 1.0 Universal")),
            ("cc0 1.0", Some("CC0 1.0 Universal")),
            ("CC0", None),
            ("Public Domain Mark 1.0", Some("Public Domain Mark 1.0")),
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
            ("https://creativecommons.org/about/", None),
            // Short names from the search server and Wikimedia Commons.
            ("CC BY-NC 4.0", Some("CC BY-NC 4.0")),
            (" CC BY-SA 3.0 de ", Some("CC BY-SA 3.0 de")),
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
            // No version, or a name Creative Commons doesn't have.
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
