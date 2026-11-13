//! Markdown written by other people, such as Xeno-canto recordist remarks.
//!
//! [`parse`] turns Markdown into an owned tree of the few elements the app
//! shows: paragraphs, line breaks, emphasis, inline code, lists and links.
//! Bare http and https URLs in the text become links too, as on Xeno-canto.
//! [`markdown_view`] builds Leptos views from that tree. The text never goes
//! through `inner_html`: raw HTML in the source stays literal text, and only
//! http and https links become links.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;

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

/// The most levels of block quotes and lists that can be nested, counted the
/// way a reader sees them: a list and its items are one level, and a block
/// quote is one level. So eight lists can nest, or a quote with seven lists
/// in it. The parser makes no element for a deeper level, and its content
/// goes into the deepest element that is open. Without a limit, a remark of
/// 2,000 ">" characters nests 2,000 block quotes, and building or dropping
/// the views for it overflows the stack, which crashes the page.
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
    let text = escape_emphasis_in_bare_urls(text.trim());
    // No extensions: tables, strikethrough, math and smart punctuation would
    // change how `|`, `~`, `$` and quote marks in plain remarks look.
    let events = TextMergeStream::new(Parser::new_ext(&text, Options::empty()));
    let mut b = TreeBuilder {
        stack: vec![Frame::Blocks(Vec::new())],
        skipped_blocks: 0,
        skipped_inlines: Vec::new(),
        inlines_added: 0,
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
    let mut blocks = b.finish();
    link_bare_urls(&mut blocks);
    blocks
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
    /// The document or a list item.
    Blocks(Vec<MdBlock>),
    Quote(Vec<MdBlock>),
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
    /// Emphasis, strong, link and image starts that got no frame because of
    /// [`MAX_INLINE_DEPTH`], and whose ends have not arrived yet, innermost
    /// last. Events nest properly, so while this is not empty, every start of
    /// those kinds is skipped too, and the next end of those kinds belongs to
    /// the last entry. A link to an http or https address keeps its address
    /// and the value of `inlines_added` at its start. If nothing was added
    /// by its end, it shows its address, as a link that is kept does.
    skipped_inlines: Vec<Option<(String, usize)>>,
    /// How many inline elements have been added so far.
    inlines_added: usize,
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
            Tag::List(start) => self.open_level(Frame::List {
                start,
                items: Vec::new(),
            }),
            Tag::BlockQuote(_) => self.open_level(Frame::Quote(Vec::new())),
            // An item is on its list's level. It has a frame when its list
            // has one.
            Tag::Item => {
                if self.skipped_blocks > 0 {
                    self.skipped_blocks += 1;
                    self.break_before_text = true;
                } else {
                    self.stack.push(Frame::Blocks(Vec::new()));
                }
            }
            Tag::CodeBlock(_) | Tag::HtmlBlock => self.stack.push(Frame::Literal(String::new())),
            // The other tags need parser options that are off. Their content
            // events still arrive and go to the enclosing element.
            _ => {}
        }
    }

    /// Push a block quote or list frame, or skip it when it would go past
    /// [`MAX_BLOCK_DEPTH`] levels.
    fn open_level(&mut self, frame: Frame) {
        let depth = self
            .stack
            .iter()
            .filter(|f| matches!(f, Frame::Quote(_) | Frame::List { .. }))
            .count();
        if self.skipped_blocks > 0 || depth >= MAX_BLOCK_DEPTH {
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
        if !self.skipped_inlines.is_empty() || depth >= MAX_INLINE_DEPTH {
            let link = match kind {
                InlineKind::Link(Some(href)) => Some((href, self.inlines_added)),
                _ => None,
            };
            self.skipped_inlines.push(link);
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
                if !self.skipped_inlines.is_empty() =>
            {
                // A link with no text shows its address, as in `close_inlines`.
                // It is added as a link, not as text: text would join the
                // text around it and be linked by the bare URL rules, which
                // can give a different address.
                if let Some(Some((href, added))) = self.skipped_inlines.pop() {
                    if added == self.inlines_added {
                        self.add_inline(MdInline::Link {
                            children: vec![MdInline::Text(href.clone())],
                            href,
                        });
                    }
                }
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
                if let Some(Frame::Quote(blocks)) = self.pop() {
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
        self.inlines_added += 1;
        match self.stack.last_mut() {
            Some(Frame::Inlines { items, .. }) => push_inline(items, inline),
            // Text straight inside a tight list item.
            Some(Frame::Blocks(blocks) | Frame::Quote(blocks)) => match blocks.last_mut() {
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
            if let Frame::Blocks(blocks) | Frame::Quote(blocks) = frame {
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
                Some(Frame::Quote(blocks)) => self.add_block(MdBlock::Quote(blocks)),
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

/// `src` with a backslash before each `*` and `_` inside its bare URLs, so
/// that the parser reads them as plain characters, not as emphasis. Without
/// it, "https://en.wikipedia.org/wiki/A_(b)_c_(d)" parses as text, an
/// emphasised "c" and more text, and only the text before the emphasis could
/// become a link, which would go to the wrong address.
///
/// It parses `src` once to find where each bare URL starts, then finds where
/// the URL ends in `src` with the rules of [`find_bare_url`]. A URL also ends
/// where a code span, link, image, raw HTML or line break starts. So a URL's
/// range holds only text and emphasis marks. When the URL is inside emphasis,
/// the `*` and `_` at its end that can close that emphasis are left out of
/// the URL and keep no backslash, so "*see https://a.example/x_(y)_z*" stays
/// emphasis.
///
/// The backslashes change no other element, with one exception: a link in
/// brackets, such as "[https://a.example/_x_]", uses a reference definition
/// only when the definition's label is written the same way, backslashes
/// included. With a backslash before each `_`, that text would match a
/// definition "[https://a.example/\_x\_]: ..." and become a link that the
/// source does not have. So `src` is returned unchanged when a reference
/// definition has a backslash in its label.
fn escape_emphasis_in_bare_urls(src: &str) -> Cow<'_, str> {
    if !src.contains(['*', '_']) || !src.contains("://") {
        return Cow::Borrowed(src);
    }
    let parser = Parser::new_ext(src, Options::empty());
    // The parser reads the reference definitions when it is made.
    if parser
        .reference_definitions()
        .iter()
        .any(|(label, _)| label.contains('\\'))
    {
        return Cow::Borrowed(src);
    }
    let events: Vec<(Event, Range<usize>)> = parser.into_offset_iter().collect();
    // Text that reads differently from its source, such as "&" from "&amp;",
    // by the byte where its source starts: where the source ends, and the text.
    let decoded: HashMap<usize, (usize, &str)> = events
        .iter()
        .filter_map(|(event, range)| match event {
            Event::Text(s) if **s != src[range.clone()] => Some((range.start, (range.end, &**s))),
            _ => None,
        })
        .collect();
    let mut escapes = Vec::new();
    // The bare URLs that start before this byte are done.
    let mut done = 0;
    // How many links, images and code blocks are open. Their text is not
    // linked.
    let mut unlinked_depth = 0usize;
    // The opening `*`, `_`, `**` or `__` of each open emphasis and strong
    // element, outermost first.
    let mut open_marks: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < events.len() {
        let (event, range) = &events[i];
        match event {
            Event::Start(Tag::Link { .. } | Tag::Image { .. } | Tag::CodeBlock(_)) => {
                unlinked_depth += 1;
            }
            Event::End(TagEnd::Link | TagEnd::Image | TagEnd::CodeBlock) => {
                unlinked_depth = unlinked_depth.saturating_sub(1);
            }
            Event::Start(Tag::Emphasis) => open_marks.push(&src[range.start..range.start + 1]),
            Event::Start(Tag::Strong) => open_marks.push(&src[range.start..range.start + 2]),
            Event::End(TagEnd::Emphasis | TagEnd::Strong) => {
                open_marks.pop();
            }
            Event::Text(_) if unlinked_depth == 0 => {
                // Text events in a row become one text in the tree.
                let run_end = events[i..]
                    .iter()
                    .position(|(event, _)| !matches!(event, Event::Text(_)))
                    .map_or(events.len(), |n| i + n);
                let mut text = String::new();
                // Where each event's text starts in `text`.
                let mut starts = Vec::new();
                for (event, _) in &events[i..run_end] {
                    if let Event::Text(s) = event {
                        starts.push(text.len());
                        text.push_str(s);
                    }
                }
                // The closing marks of the emphasis around this text, in the
                // order they come in `src`. Only the innermost few are
                // checked, so that emphasis nested thousands deep stays quick.
                let closers: Vec<&str> = open_marks
                    .iter()
                    .rev()
                    .take(MAX_INLINE_DEPTH)
                    .copied()
                    .collect();
                let mut from = 0;
                while let Some((start, end)) = find_bare_url(&text, from) {
                    from = end;
                    let n = starts.partition_point(|&s| s <= start) - 1;
                    let range = &events[i + n].1;
                    // A URL that starts inside an entity has no place in `src`.
                    if decoded.contains_key(&range.start) {
                        continue;
                    }
                    let at = range.start + (start - starts[n]);
                    if at >= done {
                        let url = UrlStart {
                            text_event: i + n,
                            at,
                            closers: &closers,
                        };
                        done = escape_bare_url(src, &events, &url, &decoded, &mut escapes);
                    }
                }
                i = run_end;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    if escapes.is_empty() {
        return Cow::Borrowed(src);
    }
    escapes.sort_unstable();
    escapes.dedup();
    let mut out = String::with_capacity(src.len() + escapes.len());
    let mut copied = 0;
    for at in escapes {
        out.push_str(&src[copied..at]);
        out.push('\\');
        copied = at;
    }
    out.push_str(&src[copied..]);
    Cow::Owned(out)
}

/// Where a bare URL starts, for [`escape_bare_url`].
struct UrlStart<'a> {
    /// The index in the events of the text event that the URL starts in.
    text_event: usize,
    /// The byte of the source where the URL starts.
    at: usize,
    /// The closing marks of the emphasis that is open where the URL starts,
    /// innermost first: `*`, `_`, `**` or `__`.
    closers: &'a [&'a str],
}

/// Find where the bare URL that starts at `url` ends in `src`. Add the byte
/// of each `*` and `_` in the URL that has no backslash before it to
/// `escapes`, and return the byte where the URL ends.
fn escape_bare_url(
    src: &str,
    events: &[(Event, Range<usize>)],
    url: &UrlStart,
    decoded: &HashMap<usize, (usize, &str)>,
    escapes: &mut Vec<usize>,
) -> usize {
    let limit = url_limit(src, events, url.text_event, url.at);
    // The URL's characters as the reader sees them, each with the byte where
    // its source starts and whether it is a `*` or `_` with no backslash.
    let mut chars: Vec<(usize, char, bool)> = Vec::new();
    let mut at = url.at;
    while at < limit {
        if let Some(&(end, text)) = decoded.get(&at) {
            if text.contains(ends_url) {
                break;
            }
            chars.extend(text.chars().map(|c| (at, c, false)));
            at = end;
            continue;
        }
        let Some(c) = src[at..].chars().next() else {
            break;
        };
        // A backslash before ASCII punctuation makes that a plain character.
        if c == '\\' {
            if let Some(next) = src[at + 1..]
                .chars()
                .next()
                .filter(char::is_ascii_punctuation)
            {
                if ends_url(next) {
                    break;
                }
                chars.push((at, next, false));
                at += 2;
                continue;
            }
        }
        if ends_url(c) {
            break;
        }
        chars.push((at, c, matches!(c, '*' | '_')));
        at += c.len_utf8();
    }
    let shown: String = chars.iter().map(|&(_, c, _)| c).collect();
    // The number of characters that `trim_url_end` keeps of the first `n`.
    let trim = |n: usize| {
        let bytes = shown.char_indices().nth(n).map_or(shown.len(), |(b, _)| b);
        trim_url_end(&shown[..bytes]).chars().count()
    };
    let mut kept = trim(chars.len());
    // If the URL ends with the closing marks of the emphasis around it, such
    // as the last "*" of "*see https://a.example/x_(y)_z*", leave them out.
    // Look for the marks of all the open emphasis first, then drop the
    // outermost one at a time, since the innermost emphasis closes first.
    for count in (1..=url.closers.len()).rev() {
        let mut end = kept;
        let mut all_found = true;
        // The outermost of these closes last, at the very end.
        for closer in url.closers[..count].iter().rev() {
            let n = closer.len();
            let found = end >= n
                && chars[end - n..end]
                    .iter()
                    .zip(closer.chars())
                    .all(|(&(_, c, mark), want)| mark && c == want);
            if !found {
                all_found = false;
                break;
            }
            end = trim(end - n);
        }
        if all_found {
            kept = end;
            break;
        }
    }
    escapes.extend(
        chars[..kept]
            .iter()
            .filter(|&&(_, _, mark)| mark)
            .map(|&(at, _, _)| at),
    );
    chars.get(kept).map_or(at, |&(at, _, _)| at)
}

/// The byte of `src` where a bare URL that starts at byte `start`, in the
/// text event `events[text_event]`, ends at the latest: at the next character
/// that [`ends_url`] accepts, where a code span, link, image, raw HTML or line
/// break starts, or where an entity such as "&#32;" that stands for such a
/// character starts.
///
/// It reads `src` and the events only as far as that byte, so a paragraph
/// with thousands of URLs and no spaces takes linear time, not quadratic.
fn url_limit(
    src: &str,
    events: &[(Event, Range<usize>)],
    text_event: usize,
    start: usize,
) -> usize {
    // The byte of the first character in `src[from..to]` that ends a URL.
    let ends_in = |from: usize, to: usize| src[from..to.max(from)].find(ends_url).map(|n| from + n);
    // How far in `src` the events read so far reach.
    let mut reached = events[text_event].1.end.max(start);
    if let Some(stop) = ends_in(start, reached) {
        return stop;
    }
    for (event, range) in &events[text_event + 1..] {
        let next = match event {
            // Text that reads differently from its source, such as " " from
            // "&#32;", and has a character that ends a URL. `escape_bare_url`
            // ends the URL at the start of this text.
            Event::Text(s) if s.contains(ends_url) && **s != src[range.clone()] => {
                return ends_in(reached, range.start).unwrap_or(range.start.max(reached));
            }
            Event::Text(_) => range.end,
            // The range of an emphasis element runs from its opening `*` or
            // `_` to the end of its closing one, and strong has two of each.
            Event::Start(Tag::Emphasis) => range.start + 1,
            Event::Start(Tag::Strong) => range.start + 2,
            Event::End(TagEnd::Emphasis | TagEnd::Strong) => range.end,
            // The end of a paragraph, heading or list item.
            Event::End(_) => return reached,
            // A code span, link, image, raw HTML, line break or any other
            // element.
            _ => return ends_in(reached, range.start).unwrap_or(range.start.max(reached)),
        };
        if let Some(stop) = ends_in(reached, next) {
            return stop;
        }
        reached = next.max(reached);
    }
    ends_in(reached, src.len()).unwrap_or(src.len())
}

/// Turn the bare http and https URLs in the text of `blocks` into links, as
/// Xeno-canto's own pages do. Remarks such as "See here
/// https://observation.org/observation/286354922/" rely on it. Text inside a
/// link, inline code and code blocks stay as they are.
fn link_bare_urls(blocks: &mut [MdBlock]) {
    for block in blocks {
        match block {
            MdBlock::Paragraph(items) | MdBlock::Plain(items) => link_bare_urls_in_paragraph(items),
            MdBlock::List { items, .. } => {
                for item in items {
                    link_bare_urls(item);
                }
            }
            MdBlock::Quote(inner) => link_bare_urls(inner),
            MdBlock::Literal(_) | MdBlock::Rule => {}
        }
    }
}

/// Link the bare URLs in the inline elements of one paragraph.
fn link_bare_urls_in_paragraph(items: &mut Vec<MdInline>) {
    // The paragraph's text as it reads, with a line break for each line
    // break, code span and link, and where each text element ends in it.
    let mut shown = String::new();
    let mut text_ends = Vec::new();
    collect_shown_text(items, &mut shown, &mut text_ends);
    let mut texts_done = 0;
    link_bare_urls_in(items, &shown, &text_ends, &mut texts_done);
}

fn collect_shown_text(items: &[MdInline], shown: &mut String, text_ends: &mut Vec<usize>) {
    for item in items {
        match item {
            MdInline::Text(s) => {
                shown.push_str(s);
                text_ends.push(shown.len());
            }
            MdInline::Emphasis(children) | MdInline::Strong(children) => {
                collect_shown_text(children, shown, text_ends)
            }
            // A URL does not go on into code, a link or the next line.
            MdInline::Code(_) | MdInline::Link { .. } | MdInline::LineBreak => shown.push('\n'),
        }
    }
}

/// Link the bare URLs in `items`. `shown` and `text_ends` come from
/// [`collect_shown_text`] for the whole paragraph, and `texts_done` counts
/// the text elements already linked.
fn link_bare_urls_in(
    items: &mut Vec<MdInline>,
    shown: &str,
    text_ends: &[usize],
    texts_done: &mut usize,
) {
    for item in std::mem::take(items) {
        match item {
            MdInline::Text(text) => {
                let after = &shown[text_ends[*texts_done]..];
                *texts_done += 1;
                push_text_with_links(&text, after, items);
            }
            MdInline::Emphasis(mut children) => {
                link_bare_urls_in(&mut children, shown, text_ends, texts_done);
                items.push(MdInline::Emphasis(children));
            }
            MdInline::Strong(mut children) => {
                link_bare_urls_in(&mut children, shown, text_ends, texts_done);
                items.push(MdInline::Strong(children));
            }
            // A link's text is not linked again.
            other @ (MdInline::Link { .. } | MdInline::Code(_) | MdInline::LineBreak) => {
                items.push(other)
            }
        }
    }
}

/// Append `text` to `items`, with each bare URL in it as a link that shows
/// the URL.
///
/// `after` is the text that the paragraph shows after `text`. A URL that runs
/// to the end of `text` and would go on into `after` stays plain text: the
/// start or end of emphasis cut it, or text from an image or from a link that
/// is not shown as a link joins it. A link to the part before the cut would
/// go to the wrong address. [`escape_emphasis_in_bare_urls`] stops this for
/// almost every URL, so this only catches the ones it cannot see. Only the
/// start of `after` is read, as [`joins_url`] describes.
fn push_text_with_links(text: &str, after: &str, items: &mut Vec<MdInline>) {
    let mut done = 0;
    let mut from = 0;
    while let Some((start, end)) = find_bare_url(text, from) {
        from = end;
        let url = &text[start..end];
        let Some(href) = web_href(url) else {
            continue;
        };
        // Only punctuation that `trim_url_end` left out comes after the URL
        // in `text`, and the URL would be longer with `after` added.
        let cut = !text[end..].contains(ends_url) && joins_url(&text[start..], url.len(), after);
        if cut {
            continue;
        }
        if start > done {
            items.push(MdInline::Text(text[done..start].to_string()));
        }
        items.push(MdInline::Link {
            href,
            children: vec![MdInline::Text(url.to_string())],
        });
        done = end;
    }
    if done < text.len() {
        items.push(MdInline::Text(text[done..].to_string()));
    }
}

/// Whether the URL at the start of `url_and_tail`, which is `url_len` bytes
/// long, would be longer if the text `after` came straight after
/// `url_and_tail`.
///
/// It reads `after` only up to its first character that [`trim_url_end`]
/// can never leave out, or that ends a URL. So it takes no longer for a long
/// `after` than for a short one, and a paragraph with thousands of texts and
/// no spaces takes linear time, not quadratic.
fn joins_url(url_and_tail: &str, url_len: usize, after: &str) -> bool {
    // The end of the punctuation at the start of `after`.
    let mut punctuation_end = 0;
    for (i, c) in after.char_indices() {
        if ends_url(c) {
            break;
        }
        // `trim_url_end` stops at this character, so the URL takes it in.
        if !is_trailing_punctuation(c) && !matches!(c, ')' | ']') {
            return true;
        }
        punctuation_end = i + c.len_utf8();
    }
    punctuation_end > 0
        && trim_url_end(&format!("{url_and_tail}{}", &after[..punctuation_end])).len() > url_len
}

/// The byte range of the first bare http or https URL in `text` that starts
/// at or after byte `from`.
///
/// A URL starts with "http://" or "https://", in any case, at the start of
/// the text or after a character that is not an ASCII letter or digit, and a
/// letter or digit must follow the "//". Chinese and Japanese text has no
/// spaces, so a URL straight after it still counts. It ends before a
/// character that [`ends_url`] accepts, such as a space. Trailing punctuation
/// is left out, as [`trim_url_end`] describes.
fn find_bare_url(text: &str, from: usize) -> Option<(usize, usize)> {
    let mut search = from;
    while let Some(pos) = text[search..].find(|c: char| c == 'h' || c == 'H') {
        let start = search + pos;
        // "h" is one byte, so the next search starts on a character boundary.
        search = start + 1;
        let rest = &text.as_bytes()[start..];
        let scheme_len = if starts_with_ignore_ascii_case(rest, b"https://") {
            8
        } else if starts_with_ignore_ascii_case(rest, b"http://") {
            7
        } else {
            continue;
        };
        if text[..start]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric())
        {
            continue;
        }
        if !text[start + scheme_len..].starts_with(char::is_alphanumeric) {
            continue;
        }
        let len = text[start..].find(ends_url).unwrap_or(text.len() - start);
        // Trimming stops at the letter or digit after the "//" at the latest.
        let url = trim_url_end(&text[start..start + len]);
        return Some((start, start + url.len()));
    }
    None
}

fn starts_with_ignore_ascii_case(bytes: &[u8], prefix: &[u8]) -> bool {
    bytes.len() >= prefix.len() && bytes[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// Whether `c` ends a bare URL: a space, a control character, `<`, `>`, `"`,
/// a backtick, Chinese or Japanese punctuation such as "。" and "、",
/// full-width punctuation such as "（" and "）", or a zero-width character.
/// Chinese and Japanese text has no spaces, so its punctuation is often the
/// only thing after a URL.
fn ends_url(c: char) -> bool {
    c.is_whitespace()
        || c.is_control()
        || matches!(c,
            '<' | '>' | '"' | '`'
            // CJK symbols and punctuation
            | '\u{3000}'..='\u{303F}'
            // Full-width punctuation, but not full-width letters and digits
            | '\u{FF01}'..='\u{FF0F}'
            | '\u{FF1A}'..='\u{FF20}'
            | '\u{FF3B}'..='\u{FF40}'
            | '\u{FF5B}'..='\u{FF65}'
            // Zero-width space, non-joiner and joiner, word joiner, and the
            // byte order mark
            | '\u{200B}'..='\u{200D}'
            | '\u{2060}'
            | '\u{FEFF}'
        )
}

/// `url` without its trailing punctuation, as GitHub leaves it out: any of
/// `. , ; : ! ? ' ~`, and a `)` or `]` that has no opening partner in the URL.
/// So "(see https://a.example/x)." gives "https://a.example/x", and
/// "https://en.wikipedia.org/wiki/Bat_(animal)" is kept whole.
///
/// A trailing `*` or `_` stays. One that marks emphasis is not in the text,
/// and [`escape_emphasis_in_bare_urls`] makes the others plain characters,
/// as in "https://web.archive.org/web/*/example.com/*".
fn trim_url_end(url: &str) -> &str {
    let open_parens = url.matches('(').count();
    let mut close_parens = url.matches(')').count();
    let open_brackets = url.matches('[').count();
    let mut close_brackets = url.matches(']').count();
    let mut end = url.len();
    while let Some(last) = url[..end].chars().next_back() {
        match last {
            c if is_trailing_punctuation(c) => {}
            ')' if close_parens > open_parens => close_parens -= 1,
            ']' if close_brackets > open_brackets => close_brackets -= 1,
            _ => break,
        }
        end -= last.len_utf8();
    }
    &url[..end]
}

/// Whether [`trim_url_end`] always leaves `c` out at the end of a URL.
fn is_trailing_punctuation(c: char) -> bool {
    matches!(c, '.' | ',' | ';' | ':' | '!' | '?' | '\'' | '~')
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
        } => view! {
            <ol start=n.to_string() style=numbered_list_style(*n, items.len())>
                {list_items_view(items)}
            </ol>
        }
        .into_any(),
        MdBlock::Literal(s) => view! { <pre>{s.clone()}</pre> }.into_any(),
        MdBlock::Quote(blocks) => {
            view! { <blockquote>{markdown_view(blocks)}</blockquote> }.into_any()
        }
        MdBlock::Rule => view! { <hr /> }.into_any(),
    }
}

/// The `style` attribute of a numbered list that starts at `start` and has
/// `items` items. It sets `--marker-digits` to the number of digits in the
/// list's largest number, and style.css sizes the list's indent from it, so
/// that a wide number such as "2019." is not cut off at the panel's edge.
fn numbered_list_style(start: u64, items: usize) -> String {
    let last = start.saturating_add((items as u64).saturating_sub(1));
    let digits = last.checked_ilog10().unwrap_or(0) + 1;
    format!("--marker-digits: {digits}")
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

    /// How many levels of block quotes and lists are nested in `blocks`,
    /// counted as [`MAX_BLOCK_DEPTH`] counts them: a list and its items are
    /// one level.
    fn block_nesting(blocks: &[MdBlock]) -> usize {
        blocks
            .iter()
            .map(|block| match block {
                Quote(inner) => 1 + block_nesting(inner),
                List { items, .. } => 1 + items.iter().map(|i| block_nesting(i)).max().unwrap_or(0),
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

    /// Every link in `blocks`, in order.
    fn links(blocks: &[MdBlock]) -> Vec<&MdInline> {
        fn of_inlines<'a>(items: &'a [MdInline], out: &mut Vec<&'a MdInline>) {
            for item in items {
                match item {
                    Link { children, .. } => {
                        out.push(item);
                        of_inlines(children, out);
                    }
                    Emphasis(c) | Strong(c) => of_inlines(c, out),
                    Text(_) | Code(_) | LineBreak => {}
                }
            }
        }
        fn of_blocks<'a>(blocks: &'a [MdBlock], out: &mut Vec<&'a MdInline>) {
            for block in blocks {
                match block {
                    Paragraph(items) | Plain(items) => of_inlines(items, out),
                    List { items, .. } => items.iter().for_each(|i| of_blocks(i, out)),
                    Quote(inner) => of_blocks(inner, out),
                    Literal(_) | Rule => {}
                }
            }
        }
        let mut out = Vec::new();
        of_blocks(blocks, &mut out);
        out
    }

    /// The address of every link in `blocks`, in order.
    fn link_hrefs(blocks: &[MdBlock]) -> Vec<&str> {
        links(blocks)
            .into_iter()
            .filter_map(|link| match link {
                Link { href, .. } => Some(href.as_str()),
                _ => None,
            })
            .collect()
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
            assert_eq!(block_nesting(&blocks), MAX_BLOCK_DEPTH);
            assert_eq!(plain_text(&blocks), "x");
            // Dropping a tree recurses once per level too.
            drop(blocks);
        });
    }

    #[test]
    fn deep_lists_are_capped() {
        on_small_stack(|| {
            let blocks = parse(&format!("{}x", "- ".repeat(50_000)));
            assert_eq!(block_nesting(&blocks), MAX_BLOCK_DEPTH);
            assert_eq!(plain_text(&blocks), "x");
            drop(blocks);

            let blocks = parse(&format!("{}x", "> 1. ".repeat(20_000)));
            assert_eq!(block_nesting(&blocks), MAX_BLOCK_DEPTH);
            assert_eq!(plain_text(&blocks), "x");
            drop(blocks);

            let blocks = parse(&format!("{}x", ">".repeat(2_000)));
            assert_eq!(block_nesting(&blocks), MAX_BLOCK_DEPTH);
            assert_eq!(plain_text(&blocks), "x");
            drop(blocks);
        });
    }

    #[test]
    fn eight_levels_of_lists_are_kept() {
        // Eight lists, each inside an item of the one before, and a quote
        // with seven lists in it: every item keeps its own level.
        let lists: String = (0..8)
            .map(|level| format!("{}- item{level}\n", "  ".repeat(level)))
            .collect();
        let in_quote: String = (0..7)
            .map(|level| format!("> {}- item{level}\n", "  ".repeat(level)))
            .collect();
        for src in [lists, in_quote] {
            let blocks = parse(&src);
            assert_eq!(block_nesting(&blocks), MAX_BLOCK_DEPTH, "{src}");
            // No item text was joined to another item's on a new line.
            let text = plain_text(&blocks);
            assert!(!text.contains('\n'), "{text}");
        }
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
    fn an_empty_link_past_the_depth_limit_shows_its_address() {
        // A link with no text shows its address.
        assert_eq!(
            parse("[](https://a.example)"),
            vec![Paragraph(vec![link("https://a.example")])]
        );
        // Nine levels of emphasis: the ninth and the link in it get no
        // element, but the address is still shown.
        let blocks = parse(
            "*a **b *c **d *e **f *g **h *i [](https://a.example) i* h** g* f** e* d** c* b** a*",
        );
        assert_eq!(
            plain_text(&blocks),
            "a b c d e f g h i https://a.example i h g f e d c b a"
        );
        // The link is one level past the limit, and holds only its address.
        assert_eq!(inline_nesting(&blocks), MAX_INLINE_DEPTH + 1);
        // A skipped link that has text shows only its text.
        let blocks = parse(
            "*a **b *c **d *e **f *g **h *i [x](https://a.example) i* h** g* f** e* d** c* b** a*",
        );
        assert_eq!(plain_text(&blocks), "a b c d e f g h i x i h g f e d c b a");
    }

    #[test]
    fn an_empty_link_past_the_depth_limit_keeps_its_own_address() {
        // Nine levels of emphasis around the link, as in the test above.
        let open = "*a **b *c **d *e **f *g **h *i ";
        let close = " i* h** g* f** e* d** c* b** a*";
        for (src, href) in [
            // The "x" after the link is not added to its address.
            ("[](https://a.example)x", "https://a.example"),
            // A bare URL would end at the space.
            ("[](<https://a.example/a b>)", "https://a.example/a b"),
            // A bare URL would leave out the "." at the end.
            ("[](https://a.example/x.)", "https://a.example/x."),
        ] {
            let blocks = parse(&format!("{open}{src}{close}"));
            assert_eq!(link_hrefs(&blocks), [href], "{src}");
            // The same link as when there is no emphasis around it.
            assert_eq!(links(&blocks), links(&parse(src)), "{src}");
        }
    }

    #[test]
    fn items_past_the_depth_limit_keep_their_text_apart() {
        // Twelve levels of list: eight are kept, and the items of the deeper
        // lists go into the deepest item that is kept, one per line.
        let src: String = (0..12)
            .map(|level| format!("{}- item{level}\n", "  ".repeat(level)))
            .collect();
        let blocks = parse(&src);
        assert_eq!(block_nesting(&blocks), MAX_BLOCK_DEPTH);
        assert_eq!(
            plain_text(&blocks),
            "item0item1item2item3item4item5item6item7\nitem8\nitem9\nitem10\nitem11"
        );
    }

    #[test]
    fn numbered_list_indent_fits_the_largest_number() {
        assert_eq!(numbered_list_style(1, 9), "--marker-digits: 1");
        // 8 to 11
        assert_eq!(numbered_list_style(8, 4), "--marker-digits: 2");
        // A year at the start of a line parses as a numbered list.
        assert_eq!(numbered_list_style(2019, 1), "--marker-digits: 4");
        assert_eq!(numbered_list_style(0, 1), "--marker-digits: 1");
        assert_eq!(numbered_list_style(123_456_789, 1), "--marker-digits: 9");
        assert_eq!(numbered_list_style(u64::MAX, 3), "--marker-digits: 20");
    }

    fn link(url: &str) -> MdInline {
        Link {
            href: url.into(),
            children: vec![text(url)],
        }
    }

    #[test]
    fn bare_urls_become_links() {
        // In the middle of text (XC827322).
        assert_eq!(
            parse("See here https://observation.org/observation/286354922/ for photos"),
            vec![Paragraph(vec![
                text("See here "),
                link("https://observation.org/observation/286354922/"),
                text(" for photos"),
            ])]
        );
        // Several in one text, with any case, and non-ASCII text around them.
        assert_eq!(
            parse("João HTTPS://A.example/é, é http://b.example"),
            vec![Paragraph(vec![
                text("João "),
                link("HTTPS://A.example/é"),
                text(", é "),
                link("http://b.example"),
            ])]
        );
        // Straight after Chinese text, which has no spaces.
        assert_eq!(
            parse("参考https://a.example"),
            vec![Paragraph(vec![text("参考"), link("https://a.example")])]
        );
        // At the very start and end of the text.
        assert_eq!(
            parse("https://a.example"),
            vec![Paragraph(vec![link("https://a.example")])]
        );
    }

    #[test]
    fn trailing_punctuation_is_not_part_of_a_bare_url() {
        // XC827655
        assert_eq!(
            parse("placed here: https://observation.org/observation/285825747/. Sonograms"),
            vec![Paragraph(vec![
                text("placed here: "),
                link("https://observation.org/observation/285825747/"),
                text(". Sonograms"),
            ])]
        );
        for (src, url, after) in [
            ("https://a.example/x?!", "https://a.example/x", "?!"),
            ("https://a.example/x,", "https://a.example/x", ","),
            ("https://a.example/x;", "https://a.example/x", ";"),
            ("https://a.example/x:", "https://a.example/x", ":"),
            ("https://a.example/x'", "https://a.example/x", "'"),
            ("https://a.example/x...", "https://a.example/x", "..."),
            // Inside the URL, the same marks are kept.
            (
                "https://a.example/a.b?c=d,e!",
                "https://a.example/a.b?c=d,e",
                "!",
            ),
        ] {
            assert_eq!(
                parse(src),
                vec![Paragraph(vec![link(url), text(after)])],
                "{src}"
            );
        }
        // Quote marks around a URL.
        assert_eq!(
            parse("\"https://a.example/x\" and 'https://b.example/y'"),
            vec![Paragraph(vec![
                text("\""),
                link("https://a.example/x"),
                text("\" and '"),
                link("https://b.example/y"),
                text("'"),
            ])]
        );
    }

    #[test]
    fn closing_brackets_are_part_of_a_bare_url_only_when_balanced() {
        // XC1037974
        assert_eq!(
            parse("BatDetect2 (https://github.com/macaodha/batdetect2) automatically"),
            vec![Paragraph(vec![
                text("BatDetect2 ("),
                link("https://github.com/macaodha/batdetect2"),
                text(") automatically"),
            ])]
        );
        assert_eq!(
            parse("https://en.wikipedia.org/wiki/Bat_(disambiguation)"),
            vec![Paragraph(vec![link(
                "https://en.wikipedia.org/wiki/Bat_(disambiguation)"
            )])]
        );
        assert_eq!(
            parse("(see https://en.wikipedia.org/wiki/Bat_(animal))."),
            vec![Paragraph(vec![
                text("(see "),
                link("https://en.wikipedia.org/wiki/Bat_(animal)"),
                text(")."),
            ])]
        );
        assert_eq!(
            parse("[https://a.example/x]"),
            vec![Paragraph(vec![
                text("["),
                link("https://a.example/x"),
                text("]")
            ])]
        );
    }

    #[test]
    fn text_that_only_looks_like_a_url_stays_text() {
        for src in [
            "https:// and http://.",
            "xhttps://a.example",
            "ftp://a.example",
            "javascript:alert(1)",
            "https://(a)",
        ] {
            assert_eq!(parse(src), vec![Paragraph(vec![text(src)])], "{src}");
        }
    }

    #[test]
    fn bare_urls_in_lists_quotes_and_emphasis() {
        // A source list, as in XC909905.
        assert_eq!(
            parse("Used sources:\r\n\r\n- https://a.example/x\r\n- see https://b.example/y"),
            vec![
                Paragraph(vec![text("Used sources:")]),
                List {
                    start: None,
                    items: vec![
                        vec![Plain(vec![link("https://a.example/x")])],
                        vec![Plain(vec![text("see "), link("https://b.example/y")])],
                    ],
                },
            ]
        );
        assert_eq!(
            parse("> see https://a.example"),
            vec![Quote(vec![Paragraph(vec![
                text("see "),
                link("https://a.example")
            ])])]
        );
        assert_eq!(
            parse("*https://a.example* and **https://b.example**"),
            vec![Paragraph(vec![
                Emphasis(vec![link("https://a.example")]),
                text(" and "),
                Strong(vec![link("https://b.example")]),
            ])]
        );
    }

    #[test]
    fn link_text_and_code_are_not_linked_again() {
        // XC817761 links a URL to itself.
        assert_eq!(
            parse("[https://a.example/x](https://a.example/x)"),
            vec![Paragraph(vec![link("https://a.example/x")])]
        );
        assert_eq!(
            parse("[see *https://a.example*](https://b.example)"),
            vec![Paragraph(vec![Link {
                href: "https://b.example".into(),
                children: vec![text("see "), Emphasis(vec![text("https://a.example")])],
            }])]
        );
        assert_eq!(
            parse("`https://a.example`"),
            vec![Paragraph(vec![Code("https://a.example".into())])]
        );
        assert_eq!(
            parse("```\nhttps://a.example\n```"),
            vec![Literal("https://a.example".into())]
        );
    }

    #[test]
    fn cjk_punctuation_and_zero_width_characters_end_a_bare_url() {
        assert_eq!(
            parse("参考https://a.example/x。更多"),
            vec![Paragraph(vec![
                text("参考"),
                link("https://a.example/x"),
                text("。更多"),
            ])]
        );
        assert_eq!(
            parse("（https://a.example/x）"),
            vec![Paragraph(vec![
                text("（"),
                link("https://a.example/x"),
                text("）")
            ])]
        );
        assert_eq!(
            parse("see https://a.example/x、https://b.example/y"),
            vec![Paragraph(vec![
                text("see "),
                link("https://a.example/x"),
                text("、"),
                link("https://b.example/y"),
            ])]
        );
        for zero_width in ['\u{200B}', '\u{200C}', '\u{200D}', '\u{2060}', '\u{FEFF}'] {
            let src = format!("https://a.example/x{zero_width}y");
            assert_eq!(
                parse(&src),
                vec![Paragraph(vec![
                    link("https://a.example/x"),
                    text(&format!("{zero_width}y")),
                ])],
                "U+{:04X}",
                zero_width as u32
            );
        }
        // Full-width letters and digits are still part of a URL.
        assert_eq!(
            parse("https://a.example/ｘ１"),
            vec![Paragraph(vec![link("https://a.example/ｘ１")])]
        );
    }

    #[test]
    fn bare_urls_with_emphasis_marks_are_linked_whole() {
        // Each of these has a pair of `_` or `*` that the parser would read
        // as emphasis, which cut the link short.
        for url in [
            "https://en.wikipedia.org/wiki/A_(b)_c_(d)",
            "https://a.example/rec_(1)_x_(2).wav",
            "https://a.example/x/__init__.py",
            "https://www.biodiversitylibrary.org/page/_12345_",
            "https://web.archive.org/web/*/example.com/*",
            "https://a.example/?a=_b_&c=1",
        ] {
            assert_eq!(parse(url), vec![Paragraph(vec![link(url)])], "{url}");
            assert_eq!(
                parse(&format!("See {url}. Next")),
                vec![Paragraph(vec![text("See "), link(url), text(". Next")])],
                "{url}"
            );
            assert_eq!(
                parse(&format!("*see {url}*")),
                vec![Paragraph(vec![Emphasis(vec![text("see "), link(url)])])],
                "{url}"
            );
        }
    }

    #[test]
    fn emphasis_around_a_bare_url_stays_emphasis() {
        assert_eq!(
            parse("_see https://a.example/a_b_ and **https://b.example/c_(d)_e**"),
            vec![Paragraph(vec![
                Emphasis(vec![text("see "), link("https://a.example/a_b")]),
                text(" and "),
                Strong(vec![link("https://b.example/c_(d)_e")]),
            ])]
        );
        // The parser closes the emphasis at "Bat_(" unless the `_` there is
        // escaped.
        assert_eq!(
            parse("_https://en.wikipedia.org/wiki/Bat_(animal)_."),
            vec![Paragraph(vec![
                Emphasis(vec![link("https://en.wikipedia.org/wiki/Bat_(animal)")]),
                text("."),
            ])]
        );
        assert_eq!(
            parse("***https://a.example/x_(y)_z***"),
            vec![Paragraph(vec![Emphasis(vec![Strong(vec![link(
                "https://a.example/x_(y)_z"
            )])])])]
        );
        // Emphasis that starts inside a URL and ends after it is lost.
        assert_eq!(
            parse("https://a.example/?q=*foo bar*"),
            vec![Paragraph(vec![
                link("https://a.example/?q=*foo"),
                text(" bar*")
            ])]
        );
    }

    #[test]
    fn bare_url_escapes_change_nothing_else() {
        // A code span, link or autolink straight after a URL keeps its `_`.
        assert_eq!(
            parse("https://a.example/_x_`a_b_`"),
            vec![Paragraph(vec![
                link("https://a.example/_x_"),
                Code("a_b_".into())
            ])]
        );
        assert_eq!(
            parse("https://a.example/_x_[t_*u*](https://b.example/_y_)"),
            vec![Paragraph(vec![
                link("https://a.example/_x_"),
                Link {
                    href: "https://b.example/_y_".into(),
                    children: vec![text("t_"), Emphasis(vec![text("u")])],
                },
            ])]
        );
        assert_eq!(
            parse("https://a.example/_x_<https://b.example/_y_>"),
            vec![Paragraph(vec![
                link("https://a.example/_x_"),
                link("https://b.example/_y_")
            ])]
        );
        assert_eq!(
            parse("`https://a.example/_x_`\n\n```\nhttps://a.example/*y*\n```"),
            vec![
                Paragraph(vec![Code("https://a.example/_x_".into())]),
                Literal("https://a.example/*y*".into()),
            ]
        );
        // `_` that the source already escapes, an escaped backslash, and an
        // entity.
        assert_eq!(
            parse(r"https://a.example/\_x\_ https://b.example/\\_y_"),
            vec![Paragraph(vec![
                link("https://a.example/_x_"),
                text(" "),
                link(r"https://b.example/\_y_"),
            ])]
        );
        assert_eq!(
            parse("https://a.example/?a=1&amp;b=_c_"),
            vec![Paragraph(vec![link("https://a.example/?a=1&b=_c_")])]
        );
        // Raw HTML around a URL stays as it was typed.
        assert_eq!(
            parse("<b>https://a.example/_x_</b>"),
            vec![Paragraph(vec![
                text("<b>"),
                link("https://a.example/_x_"),
                text("</b>")
            ])]
        );
    }

    #[test]
    fn bare_url_escapes_make_no_reference_links() {
        // "https://a.example/_x_" does not match the label
        // "https://a.example/\_x\_", so the brackets stay text. With a
        // backslash added before each `_`, it would match and link to
        // evil.example.
        let def = "\n\n[https://a.example/\\_x\\_]: https://evil.example";
        assert_eq!(
            parse(&format!("[https://a.example/_x_]{def}")),
            vec![Paragraph(vec![
                text("[https://a.example/"),
                Emphasis(vec![text("x")]),
                text("]"),
            ])]
        );
        assert_eq!(
            link_hrefs(&parse(&format!("[t][https://a.example/_x_]{def}"))),
            Vec::<&str>::new()
        );
        // A definition with no backslash in its label leaves the escapes on.
        assert_eq!(
            parse("see https://a.example/_x_\n\n[a]: https://b.example"),
            vec![Paragraph(vec![text("see "), link("https://a.example/_x_")])]
        );
    }

    #[test]
    fn a_bare_url_cut_by_emphasis_stays_text() {
        // The text of a link that is not shown as a link is parsed with
        // emphasis, and the URL in it is cut. It must not link to the part
        // before the cut.
        assert_eq!(
            parse("[https://a.example/A_(b)_c_(d)](javascript:x)"),
            vec![Paragraph(vec![
                text("https://a.example/A_(b)"),
                Emphasis(vec![text("c")]),
                text("(d)"),
            ])]
        );
        // The cut can be where the emphasis around the URL ends, too: in the
        // text of such a link, in an image's text, and where "\/" hides the
        // URL from the first parse.
        let cut = vec![Paragraph(vec![
            Emphasis(vec![text("https://en.wikipedia.org/wiki/Bat")]),
            text("(animal)_"),
        ])];
        for src in [
            "[_https://en.wikipedia.org/wiki/Bat_(animal)_](javascript:x)",
            "![_https://en.wikipedia.org/wiki/Bat_(animal)_](https://a.example/p.png)",
            r"_https:\/\/en.wikipedia.org/wiki/Bat_(animal)_",
        ] {
            assert_eq!(parse(src), cut, "{src}");
        }
        assert_eq!(
            parse("[**https://a.example/A_(b)_c**x](javascript:x)"),
            vec![Paragraph(vec![
                Strong(vec![text("https://a.example/A_(b)_c")]),
                text("x"),
            ])]
        );
        // The URL goes on through "," and into the next emphasis.
        assert_eq!(
            parse("[**https://a.example/A_(b)_c**,*d*](javascript:x)"),
            vec![Paragraph(vec![
                Strong(vec![text("https://a.example/A_(b)_c")]),
                text(","),
                Emphasis(vec![text("d")]),
            ])]
        );
        // Text after the emphasis that does not go on with the URL leaves
        // the link as it is.
        for (src, after) in [
            ("[*https://a.example/x* y](javascript:z)", " y"),
            ("[*https://a.example/x*.](javascript:z)", "."),
            ("[(*https://a.example/x*)](javascript:z)", ")"),
        ] {
            let blocks = parse(src);
            let Paragraph(items) = &blocks[0] else {
                panic!("{blocks:?}");
            };
            assert_eq!(
                &items[items.len() - 2..],
                [Emphasis(vec![link("https://a.example/x")]), text(after)],
                "{src}"
            );
        }
    }

    // The next two tests take well under a second in a debug build. When
    // each text or URL read on to the end of its paragraph, they took 39 and
    // 27 seconds.

    #[test]
    fn many_texts_with_no_spaces_take_linear_time() {
        let n = 16_000;
        let blocks = parse(&"b*a*".repeat(n));
        assert_eq!(plain_text(&blocks), "ba".repeat(n));
        let blocks = parse(&"*a*".repeat(n));
        assert_eq!(plain_text(&blocks).matches('a').count(), n);
        // Each URL is cut by emphasis, so none of them is linked.
        let n = 4_000;
        let blocks = parse(&format!("[{}](javascript:x)", "https://a.b/c*d*".repeat(n)));
        assert_eq!(plain_text(&blocks), "https://a.b/cd".repeat(n));
        assert!(link_hrefs(&blocks).is_empty());
    }

    #[test]
    fn many_bare_urls_with_no_spaces_take_linear_time() {
        let n = 4_000;
        let blocks = parse(&"https://a.example/_x[l](y)".repeat(n));
        assert_eq!(plain_text(&blocks), "https://a.example/_xl".repeat(n));
        // An entity for a space or "<" ends each URL.
        for (entity, shown) in [("&#32;", " "), ("&lt;", "<")] {
            let blocks = parse(&format!("https://a.example/_x{entity}").repeat(n));
            assert_eq!(
                plain_text(&blocks),
                format!("https://a.example/_x{shown}").repeat(n),
                "{entity}"
            );
            assert_eq!(
                link_hrefs(&blocks),
                vec!["https://a.example/_x"; n],
                "{entity}"
            );
        }
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
