//! Hashtags on annotations.
//!
//! An annotation's tags are stored as plain strings (`Annotation.tags` in the
//! app, a list of strings in `.batm` sidecars). This module gives those strings
//! a structure:
//!
//! | Typed by the user                   | Stored as                         |
//! |-------------------------------------|-----------------------------------|
//! | `#call`                             | `call`                            |
//! | `#size=10`                          | `size=10`                         |
//! | `#xc:sound_type="feeding buzz"`     | `xc:sound_type=feeding buzz`      |
//! | `#species=Pipistrellus_pipistrellus`| `species=Pipistrellus pipistrellus` |
//!
//! Namespaces and names are lowercase letters, digits, `_` and `-`, starting
//! with a letter. Values keep their case. A value with spaces is typed in
//! double quotes, or with `_` in place of each space for `species` values.
//!
//! Tags from before hashtags (free text such as `good example`) still load:
//! [`Tag::from_stored`] keeps them as a plain tag and [`Tag::display`] shows
//! them without a `#`.

use std::fmt;

/// One call: a single pulse or chirp.
pub const CALL: &str = "call";
/// A sequence of calls from one bat (a pass). Calls tagged [`CALL`] can sit
/// inside a train annotation as its children, so a file can be marked both ways.
pub const TRAIN: &str = "train";
/// A feeding buzz: the fast run of calls at the end of an attack on prey.
pub const BUZZ: &str = "buzz";
/// A social call.
pub const SOCIAL: &str = "social";
/// Scientific name of the animal making the sound, such as
/// `species=Nyctalus noctula`.
pub const SPECIES: &str = "species";
/// Made by a detector rather than by hand. The value names the detector, such
/// as `auto=calls-1`.
pub const AUTO: &str = "auto";

/// Namespace for fields that came from, or go to, Xeno-canto's Annota-JSON.
pub const XC_NS: &str = "xc";
/// The annotation's id on Xeno-canto (`annotation_xc_id`).
pub const XC_ANNOTATION_ID: &str = "annotation_id";
/// The Xeno-canto annotation set it came from (`set_uri`).
pub const XC_SET: &str = "set";
/// Xeno-canto's sound type (`sound_type`), such as `echolocation`.
pub const XC_SOUND_TYPE: &str = "sound_type";
/// Xeno-canto's annotator name (`annotator`).
pub const XC_ANNOTATOR: &str = "annotator";
/// Xeno-canto's sex (`sex`).
pub const XC_SEX: &str = "sex";
/// Xeno-canto's life stage (`life_stage`).
pub const XC_LIFE_STAGE: &str = "life_stage";

/// Longest name, namespace or value accepted, in bytes.
pub const MAX_PART_LEN: usize = 120;

/// A parsed tag.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Tag {
    /// `xc` in `#xc:sound_type=echolocation`.
    pub namespace: Option<String>,
    /// `sound_type` in `#xc:sound_type=echolocation`. For a plain tag from
    /// before hashtags, the whole text.
    pub name: String,
    /// `echolocation` in `#xc:sound_type=echolocation`.
    pub value: Option<String>,
}

/// Why typed tag text could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TagError {
    /// `#` with no name after it, or a name that does not start with a letter.
    MissingName { at: usize },
    /// A character that names and namespaces cannot contain.
    BadNameChar { at: usize, ch: char },
    /// `=` with nothing after it.
    MissingValue { at: usize },
    /// A `"` with no closing `"`.
    UnclosedQuote { at: usize },
    /// A control character (such as a tab or line break) in a value.
    ControlChar { at: usize },
    /// A part longer than [`MAX_PART_LEN`].
    TooLong { at: usize },
}

impl fmt::Display for TagError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TagError::MissingName { .. } => {
                write!(f, "Tag name must start with a letter, right after #.")
            }
            TagError::BadNameChar { ch, .. } => write!(
                f,
                "Tag names can't contain \"{ch}\". Use letters, digits, _ or -."
            ),
            TagError::MissingValue { .. } => {
                write!(f, "Missing value after =. Add a value or remove the =.")
            }
            TagError::UnclosedQuote { .. } => {
                write!(f, "Missing closing quote. Add \" at the end of the value.")
            }
            TagError::ControlChar { .. } => write!(f, "Values can't contain tabs or line breaks."),
            TagError::TooLong { .. } => write!(
                f,
                "Too long: tag names and values can be up to {MAX_PART_LEN} characters."
            ),
        }
    }
}

impl std::error::Error for TagError {}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic()
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// True for a valid tag name or namespace: ASCII letters, digits, `_` and `-`,
/// starting with a letter.
pub fn is_valid_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if is_name_start(c) => {}
        _ => return false,
    }
    s.len() <= MAX_PART_LEN && chars.all(is_name_char)
}

impl Tag {
    /// A tag with no namespace and no value, such as `#call`.
    pub fn flag(name: &str) -> Self {
        Tag {
            namespace: None,
            name: name.to_ascii_lowercase(),
            value: None,
        }
    }

    /// A tag with a value, such as `#species=Nyctalus noctula`.
    pub fn with_value(name: &str, value: impl Into<String>) -> Self {
        Tag {
            namespace: None,
            name: name.to_ascii_lowercase(),
            value: Some(value.into()),
        }
    }

    /// A namespaced tag with a value, such as `#xc:annotation_id=70231`.
    pub fn namespaced(namespace: &str, name: &str, value: impl Into<String>) -> Self {
        Tag {
            namespace: Some(namespace.to_ascii_lowercase()),
            name: name.to_ascii_lowercase(),
            value: Some(value.into()),
        }
    }

    /// Read a stored tag string. Never fails: text that is not a valid
    /// `[ns:]name[=value]` becomes a plain tag holding the whole text.
    pub fn from_stored(s: &str) -> Self {
        let s = s.trim();
        let s = s.strip_prefix('#').unwrap_or(s);
        let (key, value) = match s.split_once('=') {
            Some((k, v)) => (k, Some(v.to_string())),
            None => (s, None),
        };
        let (namespace, name) = match key.split_once(':') {
            Some((ns, n)) => (Some(ns), n),
            None => (None, key),
        };
        let ns_ok = namespace.map_or(true, is_valid_name);
        if ns_ok && is_valid_name(name) {
            Tag {
                namespace: namespace.map(|n| n.to_ascii_lowercase()),
                name: name.to_ascii_lowercase(),
                value,
            }
        } else {
            Tag {
                namespace: None,
                name: s.to_string(),
                value: None,
            }
        }
    }

    /// The form kept in `Annotation.tags`: `ns:name=value`, no `#`, no quotes.
    pub fn to_stored(&self) -> String {
        let mut out = self.key();
        if let Some(v) = &self.value {
            out.push('=');
            out.push_str(v);
        }
        out
    }

    /// `ns:name`, or `name` with no namespace.
    pub fn key(&self) -> String {
        match &self.namespace {
            Some(ns) => format!("{ns}:{}", self.name),
            None => self.name.clone(),
        }
    }

    /// True for a free-text tag from before hashtags.
    pub fn is_plain(&self) -> bool {
        !is_valid_name(&self.name)
    }

    /// True when this tag has the given namespace (None for none) and name.
    pub fn is(&self, namespace: Option<&str>, name: &str) -> bool {
        self.namespace.as_deref() == namespace && self.name == name
    }

    /// How the tag is shown and edited: `#xc:sound_type="feeding buzz"`.
    /// Plain tags are shown as they are, with no `#`.
    pub fn display(&self) -> String {
        if self.is_plain() {
            return self.name.clone();
        }
        let mut out = format!("#{}", self.key());
        if let Some(v) = &self.value {
            out.push('=');
            if v.is_empty() || v.chars().any(|c| c.is_whitespace() || c == ',' || c == '#') {
                out.push('"');
                out.push_str(v);
                out.push('"');
            } else {
                out.push_str(v);
            }
        }
        out
    }
}

impl fmt::Display for Tag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}

/// Read tags typed into a tag field.
///
/// Hashtags (`#call #species="Myotis daubentonii"`) can be separated by spaces
/// or commas. Text between commas that has no `#` is kept as a plain tag, so
/// the comma-separated tags typed before hashtags still work.
pub fn parse_tag_input(text: &str) -> Result<Vec<Tag>, TagError> {
    let mut tags = Vec::new();
    let bytes: Vec<(usize, char)> = text.char_indices().collect();
    let mut i = 0;
    let mut plain = String::new();

    let flush_plain = |plain: &mut String, tags: &mut Vec<Tag>| {
        let t = plain.trim();
        if !t.is_empty() {
            let cleaned: String = t.chars().filter(|c| !c.is_control()).collect();
            tags.push(Tag {
                namespace: None,
                name: cleaned,
                value: None,
            });
        }
        plain.clear();
    };

    while i < bytes.len() {
        let (pos, c) = bytes[i];
        if c == '#' {
            flush_plain(&mut plain, &mut tags);
            let (tag, next) = parse_hashtag(&bytes, i)?;
            if tags.iter().all(|t: &Tag| t != &tag) {
                tags.push(tag);
            }
            i = next;
            continue;
        }
        if c == ',' {
            flush_plain(&mut plain, &mut tags);
            i += 1;
            continue;
        }
        if plain.is_empty() && c.is_whitespace() {
            i += 1;
            continue;
        }
        if plain.len() >= MAX_PART_LEN {
            return Err(TagError::TooLong { at: pos });
        }
        plain.push(c);
        i += 1;
    }
    flush_plain(&mut plain, &mut tags);
    tags.dedup();
    Ok(tags)
}

/// Parse one hashtag starting at `bytes[start]` (the `#`). Returns the tag and
/// the index just past it.
fn parse_hashtag(bytes: &[(usize, char)], start: usize) -> Result<(Tag, usize), TagError> {
    let at = |i: usize| bytes.get(i).map(|b| b.0).unwrap_or(usize::MAX);
    let mut i = start + 1;

    let read_name = |i: &mut usize| -> Result<String, TagError> {
        match bytes.get(*i) {
            Some((_, c)) if is_name_start(*c) => {}
            _ => return Err(TagError::MissingName { at: at(*i) }),
        }
        let mut name = String::new();
        while let Some(&(pos, c)) = bytes.get(*i) {
            if is_name_char(c) {
                name.push(c.to_ascii_lowercase());
                *i += 1;
            } else if c == ':' || c == '=' || c == ',' || c == '#' || c.is_whitespace() {
                break;
            } else {
                return Err(TagError::BadNameChar { at: pos, ch: c });
            }
        }
        if name.len() > MAX_PART_LEN {
            return Err(TagError::TooLong { at: at(*i) });
        }
        Ok(name)
    };

    let first = read_name(&mut i)?;
    let (namespace, name) = if matches!(bytes.get(i), Some((_, ':'))) {
        i += 1;
        let n = read_name(&mut i)?;
        (Some(first), n)
    } else {
        (None, first)
    };

    let mut value = None;
    if matches!(bytes.get(i), Some((_, '='))) {
        i += 1;
        match bytes.get(i) {
            Some((open, '"')) => {
                let open = *open;
                i += 1;
                let mut v = String::new();
                loop {
                    match bytes.get(i) {
                        None => return Err(TagError::UnclosedQuote { at: open }),
                        Some((_, '"')) => {
                            i += 1;
                            break;
                        }
                        Some((pos, c)) if c.is_control() => {
                            return Err(TagError::ControlChar { at: *pos })
                        }
                        Some((_, c)) => {
                            v.push(*c);
                            i += 1;
                        }
                    }
                }
                value = Some(v);
            }
            Some((_, c)) if !c.is_whitespace() && *c != ',' && *c != '#' => {
                let mut v = String::new();
                while let Some(&(pos, c)) = bytes.get(i) {
                    if c.is_whitespace() || c == ',' || c == '#' {
                        break;
                    }
                    if c.is_control() {
                        return Err(TagError::ControlChar { at: pos });
                    }
                    v.push(c);
                    i += 1;
                }
                if name == SPECIES && namespace.is_none() {
                    v = v.replace('_', " ");
                }
                value = Some(v);
            }
            _ => return Err(TagError::MissingValue { at: at(i) }),
        }
        if value.as_ref().is_some_and(|v| v.len() > MAX_PART_LEN) {
            return Err(TagError::TooLong { at: at(i) });
        }
    }

    Ok((
        Tag {
            namespace,
            name,
            value,
        },
        i,
    ))
}

/// Read stored tag strings.
pub fn parse_stored(tags: &[String]) -> Vec<Tag> {
    tags.iter().map(|s| Tag::from_stored(s)).collect()
}

/// Turn tags back into stored strings, dropping duplicates.
pub fn to_stored(tags: &[Tag]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(tags.len());
    for t in tags {
        let s = t.to_stored();
        if !out.contains(&s) {
            out.push(s);
        }
    }
    out
}

/// The tags as they are typed in a tag field, separated by spaces.
pub fn display_all(tags: &[String]) -> String {
    let parts: Vec<String> = tags.iter().map(|s| Tag::from_stored(s).display()).collect();
    // Plain tags may contain spaces, so they need commas around them to read
    // back the same way.
    if parts.iter().any(|p| !p.starts_with('#')) {
        parts.join(", ")
    } else {
        parts.join(" ")
    }
}

/// True when the stored tags include `name` with no namespace, such as `call`.
pub fn has_flag(tags: &[String], name: &str) -> bool {
    tags.iter().any(|s| Tag::from_stored(s).is(None, name))
}

/// The value of the first tag with this namespace and name.
pub fn value_of(tags: &[String], namespace: Option<&str>, name: &str) -> Option<String> {
    tags.iter()
        .map(|s| Tag::from_stored(s))
        .find(|t| t.is(namespace, name))
        .and_then(|t| t.value)
}

/// Set (or replace) a tag's value, keeping the order of the other tags.
pub fn set_value(tags: &mut Vec<String>, tag: Tag) {
    let key = tag.key();
    if let Some(slot) = tags.iter_mut().find(|s| Tag::from_stored(s).key() == key) {
        *slot = tag.to_stored();
    } else {
        tags.push(tag.to_stored());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(text: &str) -> Vec<String> {
        to_stored(&parse_tag_input(text).unwrap())
    }

    #[test]
    fn flags_values_and_namespaces() {
        assert_eq!(
            stored("#call #size=10 #xc:species=Boa"),
            vec!["call", "size=10", "xc:species=Boa"]
        );
    }

    #[test]
    fn quoted_values_keep_spaces() {
        assert_eq!(
            stored("#xc:sound_type=\"feeding buzz\", #train"),
            vec!["xc:sound_type=feeding buzz", "train"]
        );
    }

    #[test]
    fn species_underscores_become_spaces() {
        assert_eq!(
            stored("#species=Pipistrellus_pipistrellus"),
            vec!["species=Pipistrellus pipistrellus"]
        );
        // Only for species: other values keep their underscores.
        assert_eq!(stored("#note=a_b"), vec!["note=a_b"]);
    }

    #[test]
    fn names_are_lowercased_values_keep_case() {
        assert_eq!(stored("#XC:Species=Boa"), vec!["xc:species=Boa"]);
    }

    #[test]
    fn plain_comma_tags_still_work() {
        assert_eq!(
            stored("good example, Pipistrelle"),
            vec!["good example", "Pipistrelle"]
        );
        assert_eq!(stored("nice one, #call"), vec!["nice one", "call"]);
    }

    #[test]
    fn duplicates_are_dropped() {
        assert_eq!(stored("#call #call, #call"), vec!["call"]);
    }

    #[test]
    fn errors() {
        assert!(matches!(
            parse_tag_input("#"),
            Err(TagError::MissingName { .. })
        ));
        assert!(matches!(
            parse_tag_input("#1st"),
            Err(TagError::MissingName { .. })
        ));
        assert!(matches!(
            parse_tag_input("#a.b"),
            Err(TagError::BadNameChar { ch: '.', .. })
        ));
        assert!(matches!(
            parse_tag_input("#size="),
            Err(TagError::MissingValue { .. })
        ));
        assert!(matches!(
            parse_tag_input("#note=\"open"),
            Err(TagError::UnclosedQuote { .. })
        ));
    }

    #[test]
    fn stored_round_trip_through_display() {
        let tags = vec![
            "call".to_string(),
            "xc:sound_type=feeding buzz".to_string(),
            "species=Nyctalus noctula".to_string(),
        ];
        let shown = display_all(&tags);
        assert_eq!(
            shown,
            "#call #xc:sound_type=\"feeding buzz\" #species=\"Nyctalus noctula\""
        );
        assert_eq!(stored(&shown), tags);
    }

    #[test]
    fn plain_tags_round_trip_through_display() {
        let tags = vec!["good example".to_string(), "call".to_string()];
        let shown = display_all(&tags);
        assert_eq!(shown, "good example, #call");
        assert_eq!(stored(&shown), tags);
    }

    #[test]
    fn from_stored_is_lenient() {
        let t = Tag::from_stored("odd: thing=1");
        assert!(t.is_plain());
        assert_eq!(t.display(), "odd: thing=1");
        let t = Tag::from_stored("xc:annotation_id=70231");
        assert_eq!(t.namespace.as_deref(), Some("xc"));
        assert_eq!(t.value.as_deref(), Some("70231"));
    }

    #[test]
    fn helpers() {
        let mut tags = vec!["call".to_string(), "species=Myotis".to_string()];
        assert!(has_flag(&tags, CALL));
        assert!(!has_flag(&tags, TRAIN));
        assert_eq!(value_of(&tags, None, SPECIES).as_deref(), Some("Myotis"));
        set_value(&mut tags, Tag::with_value(SPECIES, "Myotis daubentonii"));
        assert_eq!(tags, vec!["call", "species=Myotis daubentonii"]);
    }
}
