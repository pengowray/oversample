//! Markdown written by other people, such as Xeno-canto recordist remarks.
//!
//! [`parse`] turns Markdown into an owned tree of the few elements the app
//! shows: paragraphs, line breaks, emphasis, inline code, lists and links.
//! [`markdown_view`] builds Leptos views from that tree. The text never goes
//! through `inner_html`: raw HTML in the source stays literal text, and only
//! http and https links become links.

use leptos::prelude::*;
use leptos::tachys::view::any_view::AnyView;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd, TextMergeStream};

/// A block-level element.
#[derive(Clone, Debug, PartialEq)]
pub enum MdBlock {
    /// A paragraph. Headings are shown as paragraphs too.
    Paragraph(Vec<MdInline>),
    /// The text of a tight list item, which has no paragraph around it.
    Plain(Vec<MdInline>),
    /// A bullet list (`start` is `None`) or a numbered list whose first item
    /// has the number `start`.
    List {
        start: Option<u64>,
        items: Vec<Vec<MdBlock>>,
    },
    /// A code block or a raw HTML block, kept exactly as written, with its
    /// spaces, tabs and line breaks.
    Literal(String),
    /// A block quote (lines starting with `>`). It is shown indented, as on
    /// Xeno-canto, so a line such as ">50 kHz" does not read as plain "50 kHz".
    Quote(Vec<MdBlock>),
    /// A horizontal rule (`---` on a line of its own).
    Rule,
}

/// An element inside a paragraph.
#[derive(Clone, Debug, PartialEq)]
pub enum MdInline {
    Text(String),
    Code(String),
    Emphasis(Vec<MdInline>),
    Strong(Vec<MdInline>),
    /// A link whose address starts with `http://` or `https://`.
    Link {
        href: String,
        children: Vec<MdInline>,
    },
    LineBreak,
}

/// Parse Markdown into a list of blocks.
///
/// Leading and trailing whitespace is removed first: some Xeno-canto remarks
/// start with a tab and a line break, and an indented first line would
/// otherwise become a code block. A single line break inside a paragraph is
/// kept as a line break, since Xeno-canto users break lines that way.
pub fn parse(text: &str) -> Vec<MdBlock> {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    let text = text.trim();
    // No extensions: tables, strikethrough, math and smart punctuation would
    // change how `|`, `~`, `$` and quote marks in plain remarks look.
    let events = TextMergeStream::new(Parser::new_ext(text, Options::empty()));
    let mut b = TreeBuilder {
        stack: vec![Frame::Blocks(Vec::new())],
    };
    for event in events {
        match event {
            Event::Start(tag) => b.start(tag),
            Event::End(end) => b.end(end),
            Event::Text(s) => b.text(&s),
            // Raw HTML stays as the text the user typed.
            Event::Html(s) | Event::InlineHtml(s) => b.text(&s),
            Event::Code(s) => b.add_inline(MdInline::Code(s.into_string())),
            Event::SoftBreak | Event::HardBreak => b.add_inline(MdInline::LineBreak),
            Event::Rule => b.add_block(MdBlock::Rule),
            // These events need parser options that `Options::empty()` leaves
            // off, so they never arrive.
            Event::FootnoteReference(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_)
            | Event::TaskListMarker(_) => {}
        }
    }
    b.finish()
}

/// The link address to use for a Markdown link destination, or `None` when
/// it is not an http or https URL (`javascript:`, `data:`, `mailto:`, a
/// relative path).
fn web_href(dest: &str) -> Option<String> {
    let dest = dest.trim();
    let lower = dest.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://")).then(|| dest.to_string())
}

enum InlineKind {
    Paragraph,
    Emphasis,
    Strong,
    /// A link, with `None` when its address is not allowed.
    Link(Option<String>),
    /// An image: its alt text is shown as plain text.
    Image,
}

enum Frame {
    /// The document, a list item or a block quote.
    Blocks(Vec<MdBlock>),
    List {
        start: Option<u64>,
        items: Vec<Vec<MdBlock>>,
    },
    Inlines {
        kind: InlineKind,
        items: Vec<MdInline>,
    },
    Literal(String),
}

struct TreeBuilder {
    stack: Vec<Frame>,
}

impl TreeBuilder {
    fn start(&mut self, tag: Tag) {
        let frame = match tag {
            Tag::Paragraph | Tag::Heading { .. } => Frame::Inlines {
                kind: InlineKind::Paragraph,
                items: Vec::new(),
            },
            Tag::Emphasis => Frame::Inlines {
                kind: InlineKind::Emphasis,
                items: Vec::new(),
            },
            Tag::Strong => Frame::Inlines {
                kind: InlineKind::Strong,
                items: Vec::new(),
            },
            Tag::Link { dest_url, .. } => Frame::Inlines {
                kind: InlineKind::Link(web_href(&dest_url)),
                items: Vec::new(),
            },
            Tag::Image { .. } => Frame::Inlines {
                kind: InlineKind::Image,
                items: Vec::new(),
            },
            Tag::List(start) => Frame::List {
                start,
                items: Vec::new(),
            },
            Tag::Item | Tag::BlockQuote(_) => Frame::Blocks(Vec::new()),
            Tag::CodeBlock(_) | Tag::HtmlBlock => Frame::Literal(String::new()),
            // The other tags need parser options that are off. Their content
            // events still arrive and go to the enclosing element.
            _ => return,
        };
        self.stack.push(frame);
    }

    fn end(&mut self, end: TagEnd) {
        match end {
            TagEnd::Paragraph
            | TagEnd::Heading(_)
            | TagEnd::Emphasis
            | TagEnd::Strong
            | TagEnd::Link
            | TagEnd::Image => {
                if let Some(Frame::Inlines { kind, items }) = self.pop() {
                    self.close_inlines(kind, items);
                }
            }
            TagEnd::Item => {
                if let Some(Frame::Blocks(blocks)) = self.pop() {
                    if let Some(Frame::List { items, .. }) = self.stack.last_mut() {
                        items.push(blocks);
                    }
                }
            }
            TagEnd::BlockQuote(_) => {
                if let Some(Frame::Blocks(blocks)) = self.pop() {
                    self.add_block(MdBlock::Quote(blocks));
                }
            }
            TagEnd::List(_) => {
                if let Some(Frame::List { start, items }) = self.pop() {
                    self.add_block(MdBlock::List { start, items });
                }
            }
            TagEnd::CodeBlock | TagEnd::HtmlBlock => {
                if let Some(Frame::Literal(s)) = self.pop() {
                    let s = s.trim_end_matches('\n');
                    if !s.is_empty() {
                        self.add_block(MdBlock::Literal(s.to_string()));
                    }
                }
            }
            _ => {}
        }
    }

    /// Pop the top frame, but never the document itself.
    fn pop(&mut self) -> Option<Frame> {
        if self.stack.len() > 1 {
            self.stack.pop()
        } else {
            None
        }
    }

    fn close_inlines(&mut self, kind: InlineKind, items: Vec<MdInline>) {
        match kind {
            InlineKind::Paragraph => {
                if !items.is_empty() {
                    self.add_block(MdBlock::Paragraph(items));
                }
            }
            InlineKind::Emphasis => self.add_inline(MdInline::Emphasis(items)),
            InlineKind::Strong => self.add_inline(MdInline::Strong(items)),
            InlineKind::Link(Some(href)) => {
                // A link with no text shows its address.
                let children = if items.is_empty() {
                    vec![MdInline::Text(href.clone())]
                } else {
                    items
                };
                self.add_inline(MdInline::Link { href, children });
            }
            InlineKind::Link(None) | InlineKind::Image => {
                for item in items {
                    self.add_inline(item);
                }
            }
        }
    }

    fn text(&mut self, s: &str) {
        if let Some(Frame::Literal(buf)) = self.stack.last_mut() {
            buf.push_str(s);
        } else {
            self.add_inline(MdInline::Text(s.to_string()));
        }
    }

    fn add_inline(&mut self, inline: MdInline) {
        match self.stack.last_mut() {
            Some(Frame::Inlines { items, .. }) => push_inline(items, inline),
            // Text straight inside a tight list item.
            Some(Frame::Blocks(blocks)) => match blocks.last_mut() {
                Some(MdBlock::Plain(items)) => push_inline(items, inline),
                _ => blocks.push(MdBlock::Plain(vec![inline])),
            },
            Some(Frame::Literal(buf)) => {
                if let MdInline::Text(s) = inline {
                    buf.push_str(&s);
                }
            }
            Some(Frame::List { .. }) | None => {}
        }
    }

    fn add_block(&mut self, block: MdBlock) {
        // Blocks only arrive where a block container is on top, but look down
        // the stack anyway so no text is lost.
        for frame in self.stack.iter_mut().rev() {
            if let Frame::Blocks(blocks) = frame {
                blocks.push(block);
                return;
            }
        }
    }

    fn finish(mut self) -> Vec<MdBlock> {
        // Close anything left open, which a well-formed event stream never does.
        while self.stack.len() > 1 {
            match self.stack.pop() {
                Some(Frame::Inlines { kind, items }) => self.close_inlines(kind, items),
                Some(Frame::Blocks(blocks)) => {
                    for block in blocks {
                        self.add_block(block);
                    }
                }
                Some(Frame::List { start, items }) => {
                    self.add_block(MdBlock::List { start, items })
                }
                Some(Frame::Literal(s)) => self.add_block(MdBlock::Literal(s)),
                None => {}
            }
        }
        match self.stack.pop() {
            Some(Frame::Blocks(blocks)) => blocks,
            _ => Vec::new(),
        }
    }
}

/// Append an inline element, joining it to the text before it when both are
/// text.
fn push_inline(items: &mut Vec<MdInline>, inline: MdInline) {
    if let (Some(MdInline::Text(prev)), MdInline::Text(next)) = (items.last_mut(), &inline) {
        prev.push_str(next);
    } else {
        items.push(inline);
    }
}

/// Views for a parsed Markdown tree. Links open in a new tab.
pub fn markdown_view(blocks: &[MdBlock]) -> Vec<AnyView> {
    blocks.iter().map(block_view).collect()
}

fn block_view(block: &MdBlock) -> AnyView {
    match block {
        MdBlock::Paragraph(inlines) => view! { <p>{inlines_view(inlines)}</p> }.into_any(),
        MdBlock::Plain(inlines) => inlines_view(inlines).into_any(),
        MdBlock::List { start: None, items } => {
            view! { <ul>{list_items_view(items)}</ul> }.into_any()
        }
        MdBlock::List {
            start: Some(n),
            items,
        } => view! { <ol start=n.to_string()>{list_items_view(items)}</ol> }.into_any(),
        MdBlock::Literal(s) => view! { <pre>{s.clone()}</pre> }.into_any(),
        MdBlock::Quote(blocks) => {
            view! { <blockquote>{markdown_view(blocks)}</blockquote> }.into_any()
        }
        MdBlock::Rule => view! { <hr /> }.into_any(),
    }
}

fn list_items_view(items: &[Vec<MdBlock>]) -> Vec<AnyView> {
    items
        .iter()
        .map(|blocks| view! { <li>{markdown_view(blocks)}</li> }.into_any())
        .collect()
}

fn inlines_view(inlines: &[MdInline]) -> Vec<AnyView> {
    inlines.iter().map(inline_view).collect()
}

fn inline_view(inline: &MdInline) -> AnyView {
    match inline {
        MdInline::Text(s) => s.clone().into_any(),
        MdInline::Code(s) => view! { <code>{s.clone()}</code> }.into_any(),
        MdInline::Emphasis(c) => view! { <em>{inlines_view(c)}</em> }.into_any(),
        MdInline::Strong(c) => view! { <strong>{inlines_view(c)}</strong> }.into_any(),
        MdInline::Link { href, children } => view! {
            <a href=href.clone() target="_blank" rel="noopener noreferrer">
                {inlines_view(children)}
            </a>
        }
        .into_any(),
        MdInline::LineBreak => view! { <br /> }.into_any(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use MdBlock::*;
    use MdInline::*;

    fn text(s: &str) -> MdInline {
        Text(s.to_string())
    }

    #[test]
    fn emphasis_and_strong() {
        assert_eq!(
            parse("most likely *Molossus molossus*, **not** `M. rufus`"),
            vec![Paragraph(vec![
                text("most likely "),
                Emphasis(vec![text("Molossus molossus")]),
                text(", "),
                Strong(vec![text("not")]),
                text(" "),
                Code("M. rufus".into()),
            ])]
        );
    }

    #[test]
    fn web_links_become_links() {
        assert_eq!(
            parse("[SNR 26dB](https://xeno-canto.org/article/275) and <https://example.org/a>"),
            vec![Paragraph(vec![
                Link {
                    href: "https://xeno-canto.org/article/275".into(),
                    children: vec![text("SNR 26dB")],
                },
                text(" and "),
                Link {
                    href: "https://example.org/a".into(),
                    children: vec![text("https://example.org/a")],
                },
            ])]
        );
        assert_eq!(
            parse("[upper case](HTTP://EXAMPLE.ORG)"),
            vec![Paragraph(vec![Link {
                href: "HTTP://EXAMPLE.ORG".into(),
                children: vec![text("upper case")],
            }])]
        );
    }

    #[test]
    fn other_link_schemes_show_only_their_text() {
        for src in [
            "[click](javascript:alert(1))",
            "[click](JavaScript:alert(1))",
            "[click](data:text/html,hi)",
            "[click](/relative/page)",
            "[click](mailto:someone@example.org)",
        ] {
            assert_eq!(parse(src), vec![Paragraph(vec![text("click")])], "{src}");
        }
        // An email autolink has no http address either.
        assert_eq!(
            parse("<someone@example.org>"),
            vec![Paragraph(vec![text("someone@example.org")])]
        );
    }

    #[test]
    fn raw_html_stays_as_text() {
        assert_eq!(
            parse("Recorded with <b onclick=\"x()\">bold</b> text"),
            vec![Paragraph(vec![text(
                "Recorded with <b onclick=\"x()\">bold</b> text"
            )])]
        );
        // A recorder's XML header is an HTML block: it keeps its lines and tabs.
        let xml = "<BatRecord Model=\"M\">\r\n\t<Firmware>2.4.6</Firmware>\r\n</BatRecord>";
        assert_eq!(
            parse(xml),
            vec![Literal(
                "<BatRecord Model=\"M\">\n\t<Firmware>2.4.6</Firmware>\n</BatRecord>".into()
            )]
        );
        assert_eq!(
            parse("<script>alert(1)</script>"),
            vec![Literal("<script>alert(1)</script>".into())]
        );
    }

    #[test]
    fn bullet_and_numbered_lists() {
        assert_eq!(
            parse("Sources:\r\n\r\n- [A](https://a.example)\r\n- B"),
            vec![
                Paragraph(vec![text("Sources:")]),
                List {
                    start: None,
                    items: vec![
                        vec![Plain(vec![Link {
                            href: "https://a.example".into(),
                            children: vec![text("A")],
                        }])],
                        vec![Plain(vec![text("B")])],
                    ],
                },
            ]
        );
        assert_eq!(
            parse("3. three\n4. four"),
            vec![List {
                start: Some(3),
                items: vec![
                    vec![Plain(vec![text("three")])],
                    vec![Plain(vec![text("four")])]
                ],
            }]
        );
    }

    #[test]
    fn single_line_breaks_are_kept() {
        assert_eq!(
            parse("Cultivos de maíz\r\nTemperatura: 30 °\r\n\r\nSecond paragraph"),
            vec![
                Paragraph(vec![
                    text("Cultivos de maíz"),
                    LineBreak,
                    text("Temperatura: 30 °"),
                ]),
                Paragraph(vec![text("Second paragraph")]),
            ]
        );
    }

    #[test]
    fn leading_whitespace_is_trimmed() {
        assert_eq!(
            parse("\t\r\nHigh-pass filter applied (Time domain filter:15 kHz)\r\n"),
            vec![Paragraph(vec![text(
                "High-pass filter applied (Time domain filter:15 kHz)"
            )])]
        );
        // Four spaces at the start would otherwise make a code block.
        assert_eq!(
            parse("    Recording with Magenta Bat5"),
            vec![Paragraph(vec![text("Recording with Magenta Bat5")])]
        );
        assert_eq!(parse(" \r\n "), Vec::<MdBlock>::new());
    }

    #[test]
    fn block_quotes_keep_their_content() {
        assert_eq!(
            parse("Calls\n>50 kHz"),
            vec![
                Paragraph(vec![text("Calls")]),
                Quote(vec![Paragraph(vec![text("50 kHz")])]),
            ]
        );
    }

    #[test]
    fn a_link_definition_alone_shows_nothing() {
        // The Info panel shows the raw text in this case.
        assert_eq!(parse("[ref]: https://example.org"), Vec::<MdBlock>::new());
    }

    #[test]
    fn headings_and_images_show_their_text() {
        assert_eq!(
            parse("# Notes\n\n![a sonogram](https://example.org/s.png)"),
            vec![
                Paragraph(vec![text("Notes")]),
                Paragraph(vec![text("a sonogram")]),
            ]
        );
    }
}
