//! One line of a note under the Info panel's Sample rate row
//! ([`crate::rate_notes`]): its text and its role, which sets its colour.
//!
//! This module uses no other module of the app. [`crate::heterodyne`] and
//! [`crate::species_check`] build lines with it, and [`crate::rate_notes`]
//! builds notes from those lines. With the line type here and not in
//! `rate_notes`, heterodyne and species_check do not import `rate_notes`,
//! which imports them.
//!
//! Each "kHz" in a line stays on its number's line: the app writes its own
//! frequencies with [`crate::format_units`], and a [`LineRole::Quote`] line's
//! frequencies get a no-break space from
//! [`crate::format_units::keep_units_with_numbers`].

/// How a line of a note is styled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineRole {
    /// A finding or its consequence, in the warning colour (amber).
    Warning,
    /// How to read the file, in the panel's normal text colour. It is a
    /// reading rule, not another warning.
    Plain,
    /// Supporting detail, or a possibility that is not a finding, in the
    /// muted colour.
    Muted,
    /// The words of the recording's metadata that a finding is based on, with
    /// the name of their field: `Remarks: "Recording with Magenta Bat5"`.
    /// Muted. The words keep the recordist's spelling and spacing, with three
    /// changes: a quote longer than
    /// [`EXCERPT_MAX_CHARS`](oversample_core::heterodyne_hint::EXCERPT_MAX_CHARS)
    /// characters is cut, with "…" at each cut; text that is not ASCII is in
    /// Unicode normalization form C; and the space between a number and its
    /// "kHz" or "Hz" is a no-break space.
    Quote,
}

impl LineRole {
    /// The role's name in the line's CSS class (`rate-note-warning`).
    pub fn key(self) -> &'static str {
        match self {
            LineRole::Warning => "warning",
            LineRole::Plain => "plain",
            LineRole::Muted => "muted",
            LineRole::Quote => "quote",
        }
    }
}

/// One line of a note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NoteLine {
    pub text: String,
    pub role: LineRole,
}

impl NoteLine {
    pub fn warning(text: impl Into<String>) -> Self {
        Self::new(text, LineRole::Warning)
    }

    pub fn plain(text: impl Into<String>) -> Self {
        Self::new(text, LineRole::Plain)
    }

    pub fn muted(text: impl Into<String>) -> Self {
        Self::new(text, LineRole::Muted)
    }

    pub fn quote(text: impl Into<String>) -> Self {
        Self::new(text, LineRole::Quote)
    }

    fn new(text: impl Into<String>, role: LineRole) -> Self {
        Self {
            text: text.into(),
            role,
        }
    }
}

/// The lines' text, one line of the note per line of text. The
/// "Heterodyne recording" badge's tooltip is the heterodyne note written
/// this way.
pub fn lines_text(lines: &[NoteLine]) -> String {
    lines
        .iter()
        .map(|line| line.text.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_text_puts_each_line_on_its_own_line() {
        assert_eq!(
            lines_text(&[NoteLine::warning("One."), NoteLine::muted("Two.")]),
            "One.\nTwo."
        );
        assert_eq!(lines_text(&[]), "");
    }
}
