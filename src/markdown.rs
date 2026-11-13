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

/// The most block quotes, lists and list items that can be open at once.
/// The parser makes no element for a deeper one, and its content goes into
/// the deepest element that is open. Without a limit, a remark of 2,000 ">"
/// characters nests 2,000 block quotes, and building or dropping the views
/// for it overflows the stack, which crashes the page.
const MAX_BLOCK_DEPTH: usize = 8;

/// The most emphasis, strong, link and image elements that can be open at
/// once, for the same reason as [`MAX_BLOCK_DEPTH`].
const MAX_INLINE_DEPTH: usize = 8;

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
        skipped_blocks: 0,
        skipped_inlines: 0,
        break_before_text: false,
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
    /// Block quote, list and list item starts that got no frame because of
    /// [`MAX_BLOCK_DEPTH`], and whose ends have not arrived yet. Events nest
    /// properly, so while this is above 0, every block quote, list and list
    /// item start is skipped too, and the next ends of those kinds belong to
    /// skipped starts.
    skipped_blocks: usize,
    /// The same for emphasis, strong, link and image starts, with
    /// [`MAX_INLINE_DEPTH`].
    skipped_inlines: usize,
    /// A list item start was skipped. The next text in a tight list item goes
    /// on a new line, so that it does not run into the previous item's text.
    break_before_text: bool,
}

impl TreeBuilder {
    fn start(&mut self, tag: Tag) {
        match tag {
            Tag::Paragraph | Tag::Heading { .. } => self.stack.push(Frame::Inlines {
                kind: InlineKind::Paragraph,
                items: Vec::new(),
            }),
            Tag::Emphasis => self.open_inline(InlineKind::Emphasis),
            Tag::Strong => self.open_inline(InlineKind::Strong),
            Tag::Link { dest_url, .. } => self.open_inline(InlineKind::Link(web_href(&dest_url))),
            Tag::Image { .. } => self.open_inline(InlineKind::Image),
            // A list needs room for one of its items as well.
            Tag::List(start) => self.open_container(
                Frame::List {
                    start,
                    items: Vec::new(),
                },
                2,
            ),
            Tag::BlockQuote(_) => self.open_container(Frame::Blocks(Vec::new()), 1),
            Tag::Item => {
                self.open_container(Frame::Blocks(Vec::new()), 1);
                if self.skipped_blocks > 0 {
                    self.break_before_text = true;
                }
            }
            Tag::CodeBlock(_) | Tag::HtmlBlock => self.stack.push(Frame::Literal(String::new())),
            // The other tags need parser options that are off. Their content
            // events still arrive and go to the enclosing element.
            _ => {}
        }
    }

    /// Push a block quote, list or list item frame, or skip it when `slots`
    /// more frames would go past [`MAX_BLOCK_DEPTH`].
    fn open_container(&mut self, frame: Frame, slots: usize) {
        let depth = self
            .stack
            .iter()
            .filter(|f| matches!(f, Frame::Blocks(_) | Frame::List { .. }))
            .count()
            // The document frame is not counted.
            .saturating_sub(1);
        if self.skipped_blocks > 0 || depth + slots > MAX_BLOCK_DEPTH {
            self.skipped_blocks += 1;
        } else {
            self.stack.push(frame);
        }
    }

    /// Push an emphasis, strong, link or image frame, or skip it when one
    /// more would go past [`MAX_INLINE_DEPTH`].
    fn open_inline(&mut self, kind: InlineKind) {
        let depth = self
            .stack
            .iter()
            .filter(|f| {
                matches!(f, Frame::Inlines { kind, .. } if !matches!(kind, InlineKind::Paragraph))
            })
            .count();
        if self.skipped_inlines > 0 || depth >= MAX_INLINE_DEPTH {
            self.skipped_inlines += 1;
        } else {
            self.stack.push(Frame::Inlines {
                kind,
                items: Vec::new(),
            });
        }
    }

    fn end(&mut self, end: TagEnd) {
        match end {
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Link | TagEnd::Image
                if self.skipped_inlines > 0 =>
            {
                self.skipped_inlines -= 1;
            }
            TagEnd::BlockQuote(_) | TagEnd::List(_) | TagEnd::Item if self.skipped_blocks > 0 => {
                self.skipped_blocks -= 1;
            }
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
                Some(MdBlock::Plain(items)) => {
                    if std::mem::take(&mut self.break_before_text) {
                        items.push(MdInline::LineBreak);
                    }
                    push_inline(items, inline);
                }
                _ => {
                    self.break_before_text = false;
                    blocks.push(MdBlock::Plain(vec![inline]));
                }
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
        // A block between two pieces of text already puts them on separate lines.
        self.break_before_text = false;
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

    /// How many block quotes, lists and list items are nested in `blocks`,
    /// counted the way `TreeBuilder` counts its frames.
    fn block_nesting(blocks: &[MdBlock]) -> usize {
        blocks
            .iter()
            .map(|block| match block {
                Quote(inner) => 1 + block_nesting(inner),
                List { items, .. } => {
                    1 + items
                        .iter()
                        .map(|i| 1 + block_nesting(i))
                        .max()
                        .unwrap_or(0)
                }
                _ => 0,
            })
            .max()
            .unwrap_or(0)
    }

    /// How many emphasis, strong and link elements are nested in `blocks`.
    fn inline_nesting(blocks: &[MdBlock]) -> usize {
        fn of_inlines(items: &[MdInline]) -> usize {
            items
                .iter()
                .map(|item| match item {
                    Emphasis(c) | Strong(c) | Link { children: c, .. } => 1 + of_inlines(c),
                    _ => 0,
                })
                .max()
                .unwrap_or(0)
        }
        blocks
            .iter()
            .map(|block| match block {
                Paragraph(items) | Plain(items) => of_inlines(items),
                Quote(inner) => inline_nesting(inner),
                List { items, .. } => items.iter().map(|i| inline_nesting(i)).max().unwrap_or(0),
                _ => 0,
            })
            .max()
            .unwrap_or(0)
    }

    /// All the text in `blocks`, with no markup.
    fn plain_text(blocks: &[MdBlock]) -> String {
        fn of_inlines(items: &[MdInline], out: &mut String) {
            for item in items {
                match item {
                    Text(s) | Code(s) => out.push_str(s),
                    Emphasis(c) | Strong(c) | Link { children: c, .. } => of_inlines(c, out),
                    LineBreak => out.push('\n'),
                }
            }
        }
        fn of_blocks(blocks: &[MdBlock], out: &mut String) {
            for block in blocks {
                match block {
                    Paragraph(items) | Plain(items) => of_inlines(items, out),
                    List { items, .. } => items.iter().for_each(|i| of_blocks(i, out)),
                    Literal(s) => out.push_str(s),
                    Quote(inner) => of_blocks(inner, out),
                    Rule => {}
                }
            }
        }
        let mut out = String::new();
        of_blocks(blocks, &mut out);
        out
    }

    /// Run `f` on a thread with a 256 KB stack, so that code which recurses
    /// once per nesting level overflows here and fails the test.
    fn on_small_stack(f: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn(f)
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn deep_block_quotes_are_capped() {
        on_small_stack(|| {
            let blocks = parse(&format!("{}x", ">".repeat(50_000)));
            assert!(block_nesting(&blocks) <= MAX_BLOCK_DEPTH);
            assert_eq!(plain_text(&blocks), "x");
            // Dropping a tree recurses once per level too.
            drop(blocks);
        });
    }

    #[test]
    fn deep_lists_are_capped() {
        on_small_stack(|| {
            let blocks = parse(&format!("{}x", "- ".repeat(50_000)));
            assert!(block_nesting(&blocks) <= MAX_BLOCK_DEPTH);
            assert_eq!(plain_text(&blocks), "x");
            drop(blocks);

            let blocks = parse(&format!("{}x", "> 1. ".repeat(20_000)));
            assert!(block_nesting(&blocks) <= MAX_BLOCK_DEPTH);
            assert_eq!(plain_text(&blocks), "x");
            drop(blocks);
        });
    }

    #[test]
    fn deep_emphasis_is_capped() {
        on_small_stack(|| {
            for (open, close) in [("*a ", " a*"), ("**a ", " a**"), ("_a ", " a_")] {
                let src = format!("{}b{}", open.repeat(5_000), close.repeat(5_000));
                let blocks = parse(&src);
                assert!(inline_nesting(&blocks) <= MAX_INLINE_DEPTH, "{open}");
                // Every "a" and the "b" are still there.
                let text = plain_text(&blocks);
                assert_eq!(text.matches('a').count(), 10_000, "{open}");
                assert!(text.contains('b'), "{open}");
                drop(blocks);
            }
            // Images can hold images, and a link can hold an image.
            let src = format!(
                "{}x{}",
                "[![".repeat(5_000),
                "](https://i.example)](https://l.example)".repeat(5_000)
            );
            let blocks = parse(&src);
            assert!(inline_nesting(&blocks) <= MAX_INLINE_DEPTH);
            drop(blocks);
        });
    }

    #[test]
    fn items_past_the_depth_limit_keep_their_text_apart() {
        // Twelve levels of list: the deepest items go into the deepest list
        // that is kept, one per line.
        let src: String = (0..12)
            .map(|level| format!("{}- item{level}\n", "  ".repeat(level)))
            .collect();
        let blocks = parse(&src);
        assert!(block_nesting(&blocks) <= MAX_BLOCK_DEPTH);
        let text = plain_text(&blocks);
        for level in 0..12 {
            assert!(text.contains(&format!("item{level}")), "{text}");
        }
        assert!(text.contains("item10\nitem11"), "{text}");
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
