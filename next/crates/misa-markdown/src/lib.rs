//! Markdown, parsed once, in the middle layer.
//!
//! A message body arrives as text and leaves as structure. This is the one place that
//! happens, and it happens here rather than in a client on purpose: a terminal, a
//! browser, a phone, and a pipe must agree about what a quote is, and the only way to
//! guarantee that is to have one parser and to put its output in the view tree.
//!
//! # What is parsed
//!
//! Block level: ATX headings, fenced code, bullet and ordered lists (nested), block
//! quotations, thematic breaks, pipe tables, and paragraphs. Inline: strong,
//! emphasis, strikethrough, inline code, and links. A pipe table is recognised only
//! when its separator row is unambiguous, because a lone `|` in prose is not a
//! table.
//!
//! # What it does not decide
//!
//! Nothing about appearance. A heading is a heading and a level, not a size; a quote
//! is a quote, not an indent; a rule is a rule, not a row of dashes. Each client maps
//! that to its own medium, and a theme names the roles the parser emits under whatever
//! prefix it was given.
//!
//! # Why the roles are prefixed
//!
//! Every role this module emits is `<prefix>.markdown.<shape>`, where the prefix is
//! the role of the thing that contained the text (`message.assistant`, say). Role
//! lookup walks dotted prefixes, so a block inherits the message's own style instead
//! of falling back to the theme's default. A client that wants headings to look
//! different names `message.assistant.markdown.heading`; a client that does not names
//! nothing and still gets readable text.
//!
//! # Line breaks
//!
//! A single newline inside a paragraph is kept. Strict Markdown would fold it into a
//! space, but a person typing into a composer means the line break, and a model that
//! hard-wrapped its output meant it too. Structure comes from blank lines, which is
//! what separates one block from the next.

use misa_proto::view::{Kind, Node, Span, SpanKind};

/// Parse a message body into block nodes, each role under `prefix`.
///
/// The result is empty when the text has nothing in it, and every node it does return
/// passes [`misa_proto::view::validate`].
pub fn blocks(prefix: &str, text: &str) -> Vec<Node> {
    parse_lines(prefix, text, false).0
}

/// Parse `text` into blocks, and each block's byte start in `text`.
fn parse_lines(prefix: &str, text: &str, partial: bool) -> (Vec<Node>, Vec<usize>) {
    let lines: Vec<&str> = text.split('\n').collect();
    Parser { prefix, partial }.blocks_tracked(&lines)
}

/// A parsed Markdown document, kept so a later, longer text can reuse its prefix.
#[derive(Clone, Debug)]
pub struct Document {
    /// The blocks of the parsed text.
    pub blocks: Vec<Node>,
    /// The exact text these blocks were parsed from.
    pub source: String,
    /// Byte start of each block within `source`, parallel to `blocks`.
    starts: Vec<usize>,
}

/// Parse a message body, reusing the unchanged prefix of `previous`.
///
/// When `text` is `previous.source` with something appended (or is unchanged),
/// every block but the last two can be kept: an append can close a partial fence,
/// turn a partial table separator into a table, or add a line to a paragraph, but
/// it cannot reach back past two complete blocks. The suffix is reparsed from the
/// first dropped block's start offset, so the result is identical to a from-scratch
/// parse.
pub fn document(prefix: &str, text: &str, previous: Option<&Document>) -> Document {
    let mut blocks: Vec<Node> = Vec::new();
    let mut starts: Vec<usize> = Vec::new();
    let mut from = 0;
    if let Some(previous) = previous {
        if text == previous.source {
            return previous.clone();
        }
        if text.starts_with(&previous.source) {
            // Keep all but the last two blocks; `retained` is the index of the first
            // block that must be reparsed, and where it starts is where the suffix
            // begins. An empty `previous` reparses from the top.
            let retained = previous.blocks.len().saturating_sub(2);
            blocks.extend(previous.blocks[..retained].iter().cloned());
            starts.extend(previous.starts[..retained].iter().copied());
            from = previous.starts.get(retained).copied().unwrap_or(0);
        }
    }
    let (suffix, suffix_starts) = parse_lines(prefix, &text[from..], true);
    blocks.extend(suffix);
    starts.extend(suffix_starts.into_iter().map(|offset| from + offset));
    Document {
        blocks,
        source: text.to_string(),
        starts,
    }
}

/// Parse one run of text into inline spans, without any block structure.
///
/// Public because a producer that already knows its blocks — a tool result, a panel's
/// field — may want the inline rules without the block ones.
pub fn inline(text: &str) -> Vec<Span> {
    inline_inner(text, false)
}

/// Parse inline runs, treating an unclosed opener as if it closed at the end of
/// the text. Only the streaming path sets `partial`; a settled body requires its
/// closers and keeps an unclosed marker literal.
fn inline_inner(text: &str, partial: bool) -> Vec<Span> {
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut index = 0;
    while index < text.len() {
        let rest = &text[index..];
        let ch = rest
            .chars()
            .next()
            .expect("a non-empty remainder has a first char");

        if ch == '\\'
            && let Some(next) = rest[1..].chars().next()
            && is_escapable(next)
        {
            push(&mut plain, next);
            index += 1 + next.len_utf8();
            continue;
        }
        if ch == '`'
            && let Some((inner, next)) = delimited(text, index, "`")
        {
            flush(&mut out, &mut plain);
            out.push(Span::code(inner));
            index = next;
            continue;
        }
        if (rest.starts_with("***") || rest.starts_with("___"))
            && let Some((inner, next)) = delimited(text, index, &rest[..3])
        {
            flush(&mut out, &mut plain);
            out.push(Span {
                text: inner,
                kind: SpanKind::StrongEmphasis,
            });
            index = next;
            continue;
        }
        if (rest.starts_with("**") || rest.starts_with("__"))
            && let Some((inner, next)) = delimited(text, index, &rest[..2])
        {
            flush(&mut out, &mut plain);
            out.push(Span {
                text: inner,
                kind: SpanKind::Strong,
            });
            index = next;
            continue;
        }
        if rest.starts_with("~~")
            && let Some((inner, next)) = delimited(text, index, "~~")
        {
            flush(&mut out, &mut plain);
            out.push(Span {
                text: inner,
                kind: SpanKind::Strikethrough,
            });
            index = next;
            continue;
        }
        if (ch == '*' || ch == '_')
            && let Some((inner, next)) = delimited(text, index, &rest[..1])
        {
            flush(&mut out, &mut plain);
            out.push(Span {
                text: inner,
                kind: SpanKind::Emphasis,
            });
            index = next;
            continue;
        }
        if ch == '['
            && let Some((label, href, next)) = link(text, index)
        {
            flush(&mut out, &mut plain);
            out.push(Span::link(label, href));
            index = next;
            continue;
        }

        if partial && let Some((inner, kind)) = presumptive(rest) {
            flush(&mut out, &mut plain);
            out.push(Span {
                text: inner.to_string(),
                kind,
            });
            break;
        }

        push(&mut plain, ch);
        index += ch.len_utf8();
    }
    flush(&mut out, &mut plain);
    if out.is_empty() {
        out.push(Span::plain(""));
    }
    out
}

/// An opener at the start of `rest` that has no closer, and the run it styles.
///
/// A single `*`/`_` only counts when it is followed by non-whitespace, so a stray
/// bullet or underscore in prose is not turned italic mid-stream.
fn presumptive(rest: &str) -> Option<(&str, SpanKind)> {
    for (delimiter, kind) in [
        ("***", SpanKind::StrongEmphasis),
        ("___", SpanKind::StrongEmphasis),
        ("**", SpanKind::Strong),
        ("__", SpanKind::Strong),
        ("~~", SpanKind::Strikethrough),
    ] {
        if let Some(inner) = rest.strip_prefix(delimiter)
            && !inner.is_empty()
        {
            return Some((inner, kind));
        }
    }
    let ch = rest.chars().next()?;
    let inner = &rest[ch.len_utf8()..];
    if (ch == '*' || ch == '_') && !inner.starts_with(char::is_whitespace) && !inner.is_empty() {
        return Some((inner, SpanKind::Emphasis));
    }
    if ch == '`' {
        let ticks = rest.len() - rest.trim_start_matches('`').len();
        let inner = &rest[ticks..];
        if !inner.is_empty() {
            return Some((inner, SpanKind::Code));
        }
    }
    None
}

struct Parser<'a> {
    prefix: &'a str,
    /// Streaming parse: an unclosed opener styles the rest of the text.
    partial: bool,
}

impl Parser<'_> {
    fn role(&self, shape: &str) -> String {
        format!("{}.markdown.{shape}", self.prefix)
    }

    /// Parse inline runs for this document's mode (settled or streaming).
    fn inline(&self, text: &str) -> Vec<Span> {
        inline_inner(text, self.partial)
    }

    fn blocks(&self, lines: &[&str]) -> Vec<Node> {
        self.blocks_tracked(lines).0
    }

    /// Parse blocks and record each one's byte start within `lines`.
    ///
    /// The bytes are counted as if `lines` were `'\n'`-joined, so an offset is a
    /// valid position in the text a caller split. The incremental entry point uses
    /// them to find where the retained prefix ends.
    fn blocks_tracked(&self, lines: &[&str]) -> (Vec<Node>, Vec<usize>) {
        let mut line_starts = Vec::with_capacity(lines.len());
        let mut offset = 0;
        for line in lines {
            line_starts.push(offset);
            offset += line.len() + 1;
        }
        let mut out = Vec::new();
        let mut starts = Vec::new();
        let mut index = 0;
        while index < lines.len() {
            let line = lines[index];
            if line.trim().is_empty() {
                index += 1;
                continue;
            }
            let start = line_starts[index];
            let (node, next) = if let Some(lang) = fence(line) {
                self.code(lines, index, lang)
            } else if let Some((level, body)) = heading(line) {
                (
                    Node::new(
                        self.role("heading"),
                        Kind::Heading {
                            level,
                            spans: self.inline(body),
                        },
                    ),
                    index + 1,
                )
            } else if let Some((level, body)) = setext(lines, index) {
                (
                    Node::new(
                        self.role("heading"),
                        Kind::Heading {
                            level,
                            spans: self.inline(body),
                        },
                    ),
                    index + 2,
                )
            } else if is_rule(line) {
                (Node::new(self.role("rule"), Kind::Rule), index + 1)
            } else if is_table(lines, index) {
                self.table(lines, index)
            } else if quote_line(line).is_some() {
                self.quote(lines, index)
            } else if list_marker(line).is_some() {
                self.list(lines, index)
            } else {
                self.paragraph(lines, index)
            };
            out.push(node);
            starts.push(start);
            index = next;
        }
        (out, starts)
    }

    fn code(&self, lines: &[&str], start: usize, lang: Option<String>) -> (Node, usize) {
        let mut body = String::new();
        let mut index = start + 1;
        while index < lines.len() {
            if fence(lines[index]).is_some() {
                index += 1;
                break;
            }
            body.push_str(&normalize(lines[index]));
            body.push('\n');
            index += 1;
        }
        let text = body.trim_end_matches('\n').to_string();
        // A fence that says `diff` is a diff, and the role is how that reaches a frontend:
        // the block is still `Kind::Code`, because a diff *is* code — what it needs is to be
        // laid out line by line, and the role is what says so.
        let diff = lang
            .as_deref()
            .is_some_and(|lang| lang.eq_ignore_ascii_case("diff"));
        let shape = if diff { "diff" } else { "code" };
        (
            Node::new(self.role(shape), Kind::Code { lang, text }),
            index,
        )
    }

    fn quote(&self, lines: &[&str], start: usize) -> (Node, usize) {
        let mut inner = Vec::new();
        let mut index = start;
        while index < lines.len() {
            match quote_line(lines[index]) {
                Some(body) => {
                    inner.push(body);
                    index += 1;
                }
                None => break,
            }
        }
        (
            Node::new(self.role("quote"), Kind::Quote).children(self.blocks(&inner)),
            index,
        )
    }

    fn list(&self, lines: &[&str], start: usize) -> (Node, usize) {
        let ordered = list_marker(lines[start]).expect("called on a marker").0;
        let mut items: Vec<Vec<Node>> = Vec::new();
        let mut markers: Vec<Option<bool>> = Vec::new();
        let mut index = start;
        while index < lines.len() {
            let Some((item_ordered, first, column)) = list_marker(lines[index]) else {
                break;
            };
            // A different marker style is a different list: `1.` after `-` is what a
            // document means by ending one list and starting another.
            if item_ordered != ordered {
                break;
            }
            // A task marker is list-item metadata, not text. It is kept beside the
            // item so a client draws the right box rather than parsing a glyph.
            let (checked, content) = task_marker(first);
            markers.push(checked);
            let mut body = vec![content.to_string()];
            index += 1;
            while index < lines.len() {
                let line = lines[index];
                if line.trim().is_empty() {
                    // A blank line belongs to the item only if more of the item follows
                    // it; otherwise it is the gap between items and the item is over.
                    let mut look = index + 1;
                    while look < lines.len() && lines[look].trim().is_empty() {
                        look += 1;
                    }
                    if look >= lines.len() || indent_of(lines[look]) < column {
                        break;
                    }
                    body.push(String::new());
                    index += 1;
                    continue;
                }
                if indent_of(line) < column {
                    break;
                }
                body.push(dedent(line, column));
                index += 1;
            }
            let borrowed: Vec<&str> = body.iter().map(String::as_str).collect();
            items.push(self.blocks(&borrowed));
        }
        (
            Node::new(
                self.role("list"),
                Kind::List {
                    ordered,
                    items,
                    markers: if markers.iter().any(Option::is_some) {
                        markers
                    } else {
                        Vec::new()
                    },
                },
            ),
            index,
        )
    }

    fn paragraph(&self, lines: &[&str], start: usize) -> (Node, usize) {
        let mut body = String::new();
        let mut index = start;
        while index < lines.len() {
            let line = lines[index];
            if line.trim().is_empty()
                || (index > start && (is_block_start(line) || is_table(lines, index)))
            {
                break;
            }
            if !body.is_empty() {
                body.push('\n');
            }
            body.push_str(line.trim_end());
            index += 1;
        }
        (
            Node::text(self.role("paragraph"), self.inline(&body)),
            index,
        )
    }

    /// A pipe table: a header row, a separator row, then body rows.
    ///
    /// Alignment markers are parsed only to validate them; the semantic tree does
    /// not carry a column alignment, so a client decides how to line a cell up.
    fn table(&self, lines: &[&str], start: usize) -> (Node, usize) {
        let columns = split_table_row(lines[start]).len();
        let head = split_table_row(lines[start])
            .into_iter()
            .map(|cell| self.inline(&cell))
            .collect();
        let mut rows = Vec::new();
        let mut index = start + 2;
        while index < lines.len() {
            let line = lines[index];
            if line.trim().is_empty() || !line.contains('|') {
                break;
            }
            let mut cells = split_table_row(line);
            cells.truncate(columns);
            while cells.len() < columns {
                cells.push(String::new());
            }
            rows.push(cells.iter().map(|cell| self.inline(cell)).collect());
            index += 1;
        }
        (
            Node::new(self.role("table"), Kind::Table { head, rows }),
            index,
        )
    }
}

/// Whether a line begins a block, so a paragraph in front of it ends.
fn is_block_start(line: &str) -> bool {
    fence(line).is_some()
        || heading(line).is_some()
        || is_rule(line)
        || quote_line(line).is_some()
        || list_marker(line).is_some()
}

/// The language of a fenced code block, if this line opens or closes one.
fn fence(line: &str) -> Option<Option<String>> {
    let rest = line.trim_start().strip_prefix("```")?;
    let lang = rest.trim();
    Some((!lang.is_empty()).then(|| lang.to_string()))
}

/// A table separator row: every cell is dashes with optional alignment colons.
fn is_table_separator(line: &str) -> bool {
    let cells = split_table_row(line);
    !cells.is_empty()
        && cells.iter().all(|cell| {
            let dashes = cell.trim().trim_matches(':');
            !dashes.is_empty() && dashes.chars().all(|ch| ch == '-')
        })
}

/// Whether a header row and separator row begin a pipe table.
fn is_table(lines: &[&str], index: usize) -> bool {
    let Some(header) = lines.get(index) else {
        return false;
    };
    let Some(separator) = lines.get(index + 1) else {
        return false;
    };
    header.contains('|') && is_table_separator(separator)
}

/// Split a pipe-table row into trimmed cells, without the outer pipes.
fn split_table_row(line: &str) -> Vec<String> {
    let trimmed = line.trim();
    let trimmed = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let trimmed = trimmed.strip_suffix('|').unwrap_or(trimmed);
    trimmed
        .split('|')
        .map(|cell| cell.trim().to_string())
        .collect()
}

/// A setext heading: a line of text underlined by `===` or `---`.
///
/// Only a single line is taken, which is what a person writes; a multi-line
/// paragraph before an underline stays a paragraph followed by a rule.
fn setext<'a>(lines: &'a [&'a str], index: usize) -> Option<(u8, &'a str)> {
    let text = lines.get(index)?;
    let underline = lines.get(index + 1)?;
    if text.trim().is_empty() || is_block_start(text) {
        return None;
    }
    let marker = underline.trim();
    if marker.is_empty() {
        return None;
    }
    let level = if marker.chars().all(|ch| ch == '=') {
        1
    } else if marker.chars().all(|ch| ch == '-') {
        2
    } else {
        return None;
    };
    Some((level, text.trim()))
}

/// An ATX heading's level and its text.
fn heading(line: &str) -> Option<(u8, &str)> {
    let trimmed = line.trim_start();
    let hashes = trimmed.len() - trimmed.trim_start_matches('#').len();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    // `#1` is not a heading: a heading marker is followed by a space or by nothing.
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some((hashes as u8, rest.trim().trim_end_matches('#').trim_end()))
}

/// A thematic break: three or more of one of `-`, `*`, `_`, and nothing else.
fn is_rule(line: &str) -> bool {
    let trimmed = line.trim();
    let Some(first) = trimmed.chars().next() else {
        return false;
    };
    if !matches!(first, '-' | '*' | '_') {
        return false;
    }
    trimmed.chars().count() >= 3 && trimmed.chars().all(|ch| ch == first)
}

/// A block quote's line, with its marker removed.
fn quote_line(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('>')?;
    Some(rest.strip_prefix(' ').unwrap_or(rest))
}

/// A list item's leading task box: `[ ]`, `[x]`, `[X]`, or any single character
/// inside the brackets. Returns the state and the text after the box.
fn task_marker(text: &str) -> (Option<bool>, &str) {
    let trimmed = text.trim_start();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= 4 && bytes[0] == b'[' && bytes[2] == b']' && bytes[3] == b' ' {
        return (Some(bytes[1] != b' '), &trimmed[4..]);
    }
    (None, text)
}

/// A list item's line: whether it is ordered, its text, and the column its content
/// starts at — which is what continuation lines are measured against.
fn list_marker(line: &str) -> Option<(bool, &str, usize)> {
    let indent = line.len() - line.trim_start().len();
    let rest = &line[indent..];
    for bullet in ["- ", "* ", "+ "] {
        if let Some(content) = rest.strip_prefix(bullet) {
            return Some((false, content, indent + bullet.len()));
        }
    }
    let digits = rest.chars().take_while(char::is_ascii_digit).count();
    if digits > 0 && digits <= 9 {
        for marker in ['.', ')'] {
            let prefix = format!("{marker} ");
            if let Some(content) = rest[digits..].strip_prefix(&prefix) {
                return Some((true, content, indent + digits + prefix.len()));
            }
        }
    }
    None
}

/// How many columns of leading whitespace a line has, counting a tab as four.
fn indent_of(line: &str) -> usize {
    line.chars()
        .take_while(|ch| *ch == ' ' || *ch == '\t')
        .map(|ch| if ch == '\t' { 4 } else { 1 })
        .sum()
}

/// Remove up to `column` columns of leading whitespace.
fn dedent(line: &str, column: usize) -> String {
    let mut removed = 0;
    let mut end = 0;
    for (byte, ch) in line.char_indices() {
        if removed >= column {
            end = byte;
            break;
        }
        match ch {
            ' ' => removed += 1,
            '\t' => removed += 4,
            _ => {
                end = byte;
                break;
            }
        }
        end = byte + ch.len_utf8();
    }
    line[end.min(line.len())..].to_string()
}

/// Text a renderer will be handed: no tab (an indent is the client's decision) and no
/// carriage return (a stray one is a control character the view refuses).
fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\t' => out.push_str("    "),
            '\r' => {}
            _ => out.push(ch),
        }
    }
    out
}

fn push(plain: &mut String, ch: char) {
    match ch {
        '\t' => plain.push_str("    "),
        '\r' => {}
        _ => plain.push(ch),
    }
}

fn flush(out: &mut Vec<Span>, plain: &mut String) {
    if !plain.is_empty() {
        out.push(Span::plain(std::mem::take(plain)));
    }
}

fn is_escapable(ch: char) -> bool {
    matches!(ch, '\\' | '`' | '*' | '_' | '~' | '[' | ']' | '(' | ')')
}

/// The text between a marker at `start` and the next one, and the index after it.
fn delimited(text: &str, start: usize, marker: &str) -> Option<(String, usize)> {
    let after = start + marker.len();
    let rest = text.get(after..)?;
    let end = rest.find(marker)?;
    if end == 0 {
        return None;
    }
    Some((normalize(&rest[..end]), after + end + marker.len()))
}

/// A `[label](href)` link: its text, its target, and the index after it.
fn link(text: &str, start: usize) -> Option<(String, String, usize)> {
    let rest = text.get(start..)?;
    let label_end = rest.find("](")?;
    let label = &rest[1..label_end];
    let tail = &rest[label_end + 2..];
    let href_end = tail.find(')')?;
    let href = tail[..href_end]
        .trim()
        .trim_start_matches('<')
        .trim_end_matches('>');
    // A target a client cannot use is not a link: an empty one, one with whitespace or
    // a control character, or one longer than the view will accept.
    if label.is_empty() || href.is_empty() || href.len() > 4096 {
        return None;
    }
    if href.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
        return None;
    }
    Some((
        normalize(label),
        href.to_string(),
        start + label_end + 2 + href_end + 1,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::view::validate;

    fn parse(text: &str) -> Vec<Node> {
        blocks("message.assistant", text)
    }

    fn roles(nodes: &[Node]) -> Vec<&str> {
        nodes.iter().map(|node| node.role.as_str()).collect()
    }

    fn text_of(spans: &[Span]) -> String {
        spans.iter().map(|span| span.text.as_str()).collect()
    }

    #[test]
    fn a_plain_paragraph_is_one_text_node() {
        let out = parse("hello there");
        assert_eq!(roles(&out), vec!["message.assistant.markdown.paragraph"]);
        match &out[0].kind {
            Kind::Text { spans } => assert_eq!(spans, &vec![Span::plain("hello there")]),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn a_blank_line_separates_paragraphs_and_a_single_one_does_not() {
        assert_eq!(parse("one\n\ntwo").len(), 2);
        match &parse("one\ntwo")[0].kind {
            Kind::Text { spans } => assert_eq!(spans, &vec![Span::plain("one\ntwo")]),
            other => panic!("expected one paragraph, got {other:?}"),
        }
    }

    #[test]
    fn headings_carry_their_level_and_text() {
        let out = parse("# One\n###### Six");
        match &out[0].kind {
            Kind::Heading { level, spans } => {
                assert_eq!(*level, 1);
                assert_eq!(spans, &vec![Span::plain("One")]);
            }
            other => panic!("expected a heading, got {other:?}"),
        }
        assert!(matches!(&out[1].kind, Kind::Heading { level: 6, .. }));
    }

    #[test]
    fn a_hash_without_a_space_is_not_a_heading() {
        // `#1` is a command, not a heading, and a session that thought otherwise would
        // silently eat the first character of somebody's text.
        assert!(matches!(&parse("#1 thing")[0].kind, Kind::Text { .. }));
    }

    #[test]
    fn a_fence_becomes_a_code_node_with_its_language() {
        let out = parse("```rust\nlet x = 1;\n```");
        match &out[0].kind {
            Kind::Code { lang, text, .. } => {
                assert_eq!(lang.as_deref(), Some("rust"));
                assert_eq!(text, "let x = 1;");
            }
            other => panic!("expected code, got {other:?}"),
        }
    }

    #[test]
    fn a_diff_fence_is_marked_as_a_diff() {
        // The block is still code; the role is what tells a frontend to lay it out line by
        // line, which is the whole of what the previous system's `content.diff` component
        // decided.
        let out = parse("```diff\n@@ -1 +1 @@\n-old\n+new\n```");
        assert_eq!(roles(&out), vec!["message.assistant.markdown.diff"]);
        assert!(matches!(&out[0].kind, Kind::Code { lang, .. } if lang.as_deref() == Some("diff")));
    }

    #[test]
    fn an_unterminated_fence_still_produces_a_code_node() {
        // Which is what a streaming answer looks like while the model is still writing.
        let out = parse("```rust\nlet x = 1;");
        assert!(matches!(&out[0].kind, Kind::Code { .. }));
    }

    #[test]
    fn a_tab_in_a_code_block_is_spaces_so_the_view_stays_valid() {
        let out = parse("```\n\tindented\n```");
        match &out[0].kind {
            Kind::Code { text, .. } => assert_eq!(text, "    indented"),
            other => panic!("expected code, got {other:?}"),
        }
        validate(&Node::section("root").children(out)).unwrap();
    }

    #[test]
    fn bullets_and_numbers_become_a_list() {
        let bullets = parse("- one\n- two");
        assert!(
            matches!(&bullets[0].kind, Kind::List { ordered: false, items, .. } if items.len() == 2)
        );
        let numbers = parse("1. one\n2. two");
        assert!(
            matches!(&numbers[0].kind, Kind::List { ordered: true, items, .. } if items.len() == 2)
        );
        assert_eq!(
            parse("1. one\n2) two\n- three").len(),
            2,
            "a style change starts a new list"
        );
    }

    #[test]
    fn a_task_list_carries_its_box_state_beside_the_item() {
        let out = parse("- [ ] todo\n- [x] done\n- [X] also");
        match &out[0].kind {
            Kind::List { markers, items, .. } => {
                assert_eq!(markers, &vec![Some(false), Some(true), Some(true)]);
                match &items[0][0].kind {
                    Kind::Text { spans } => assert_eq!(text_of(spans), "todo"),
                    other => panic!("expected text, got {other:?}"),
                }
            }
            other => panic!("expected a list, got {other:?}"),
        }
        // A list without a box has no marker field at all.
        match &parse("- one\n- two")[0].kind {
            Kind::List { markers, .. } => assert!(markers.is_empty()),
            other => panic!("expected a list, got {other:?}"),
        }
    }

    #[test]
    fn a_nested_list_is_a_list_inside_an_item() {
        let out = parse("- one\n  - nested\n- two");
        let Kind::List { items, .. } = &out[0].kind else {
            panic!("expected a list")
        };
        assert_eq!(items.len(), 2);
        assert!(
            items[0]
                .iter()
                .any(|child| matches!(&child.kind, Kind::List { .. }))
        );
        validate(&Node::section("root").children(out)).unwrap();
    }

    #[test]
    fn a_quote_holds_its_blocks() {
        let out = parse("> quoted\n> more");
        assert!(matches!(&out[0].kind, Kind::Quote));
        match &out[0].children[0].kind {
            Kind::Text { spans } => assert_eq!(spans, &vec![Span::plain("quoted\nmore")]),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn a_setext_underline_makes_a_heading() {
        let out = parse("Title\n=====\n\nSub\n---");
        assert!(
            matches!(&out[0].kind, Kind::Heading { level: 1, .. }),
            "{:?}",
            out[0].kind
        );
        assert!(
            matches!(&out[1].kind, Kind::Heading { level: 2, .. }),
            "{:?}",
            out[1].kind
        );
        // A lone rule is still a rule, and a rule after prose is not a heading
        // when the prose was already consumed as a paragraph.
        assert!(matches!(&parse("---")[0].kind, Kind::Rule));
    }

    #[test]
    fn a_rule_is_a_rule_and_not_a_list() {
        assert!(matches!(&parse("---")[0].kind, Kind::Rule));
        assert!(matches!(&parse("***")[0].kind, Kind::Rule));
        assert!(!matches!(&parse("- item")[0].kind, Kind::Rule));
    }

    #[test]
    fn a_pipe_table_becomes_a_table_node() {
        let out = parse("| a | b |\n| --- | --- |\n| one | two |\n| three | four |");
        assert_eq!(roles(&out), vec!["message.assistant.markdown.table"]);
        match &out[0].kind {
            Kind::Table { head, rows } => {
                assert_eq!(head.len(), 2);
                assert_eq!(rows.len(), 2);
                assert_eq!(text_of(&head[0]), "a");
                assert_eq!(text_of(&rows[1][1]), "four");
            }
            other => panic!("expected a table, got {other:?}"),
        }
        validate(&Node::section("root").children(out)).unwrap();
    }

    #[test]
    fn a_line_that_only_looks_like_a_table_stays_prose() {
        assert!(matches!(&parse("a | b")[0].kind, Kind::Text { .. }));
        assert!(!matches!(&parse("--- | ---")[0].kind, Kind::Table { .. }));
    }

    #[test]
    fn a_table_supports_alignment_markers_and_ragged_rows() {
        let out = parse("| a | b |\n|:--|--:|\n| one |");
        match &out[0].kind {
            Kind::Table { rows, .. } => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].len(), 2, "a short row is padded to the header");
                assert_eq!(text_of(&rows[0][1]), "");
            }
            other => panic!("expected a table, got {other:?}"),
        }
    }

    #[test]
    fn inline_runs_carry_their_meaning() {
        let out = inline("a **b** _c_ `d` ~~e~~ [f](https://example.com)");
        let kinds: Vec<SpanKind> = out.iter().map(|span| span.kind.clone()).collect();
        assert!(kinds.contains(&SpanKind::Strong));
        assert!(kinds.contains(&SpanKind::Emphasis));
        assert!(kinds.contains(&SpanKind::Code));
        assert!(kinds.contains(&SpanKind::Strikethrough));
        assert!(
            kinds.iter().any(
                |kind| matches!(kind, SpanKind::Link { href } if href == "https://example.com")
            )
        );
        // `***x***` is one strong-emphasis run, not strong around a stray `*`.
        let strong = inline("***x***");
        assert_eq!(
            strong,
            vec![Span {
                text: "x".into(),
                kind: SpanKind::StrongEmphasis,
            }]
        );
    }

    #[test]
    fn an_unclosed_marker_stays_literal_in_settled_text() {
        // A settled body requires its closers; only the streaming parse presumes.
        assert_eq!(inline("a **b"), vec![Span::plain("a **b")]);
    }

    #[test]
    fn a_streaming_parse_styles_an_unclosed_span_and_a_settled_one_does_not() {
        let paragraph = |document: &Document| {
            document
                .blocks
                .iter()
                .find_map(|node| match &node.kind {
                    Kind::Text { spans } => Some(spans.clone()),
                    _ => None,
                })
                .expect("a paragraph")
        };
        let streaming = document("message.assistant", "a **bold", None);
        let spans = paragraph(&streaming);
        assert!(
            spans
                .iter()
                .any(|span| span.kind == SpanKind::Strong && span.text == "bold"),
            "{spans:?}"
        );
        assert!(
            !spans.iter().any(|span| span.text.contains("**")),
            "{spans:?}"
        );
        // A code span is presumptive too.
        let code = document("message.assistant", "a `let", None);
        assert!(
            paragraph(&code)
                .iter()
                .any(|span| span.kind == SpanKind::Code && span.text == "let"),
            "{:?}",
            paragraph(&code)
        );
        // The settled parse keeps the marker literal.
        let settled = blocks("message.assistant", "a **bold");
        assert!(
            matches!(&settled[0].kind, Kind::Text { spans } if spans == &vec![Span::plain("a **bold")]),
            "{:?}",
            settled[0].kind
        );
    }

    #[test]
    fn an_escape_removes_the_marker() {
        assert_eq!(
            inline(r"\*not emphasis\*"),
            vec![Span::plain("*not emphasis*")]
        );
    }

    #[test]
    fn a_target_a_client_cannot_use_is_not_a_link() {
        assert_eq!(
            inline("[x](two words)"),
            vec![Span::plain("[x](two words)")]
        );
        assert_eq!(inline("[x]()"), vec![Span::plain("[x]()")]);
    }

    #[test]
    fn structure_survives_a_view_round_trip() {
        let text = "# Title\n\ntext with **bold** and `code`\n\n- one\n- two\n\n> quote\n\n---\n\n```\nfn main() {}\n```";
        let tree = Node::section("message.assistant").children(parse(text));
        validate(&tree).expect("the parser's output is a valid view");
    }

    #[test]
    fn every_role_is_a_legal_dotted_name() {
        let text = "# h\n\ntext\n\n- a\n\n> q\n\n---\n\n```\nc\n```";
        for node in parse(text) {
            for child in std::iter::once(&node).chain(node.children.iter()) {
                assert!(
                    child.role.starts_with("message.assistant.markdown."),
                    "{}",
                    child.role
                );
            }
        }
    }

    /// Parse `text` incrementally and assert it matches a from-scratch parse, that
    /// it kept the whole source, and that each block is still a valid view on its
    /// own.
    fn assert_incremental(text: &str, previous: &Document) -> Document {
        let next = document("message.assistant", text, Some(previous));
        let full = document("message.assistant", text, None);
        assert_eq!(
            next.blocks, full.blocks,
            "incremental diverged for {text:?}"
        );
        assert_eq!(next.source, text);
        for node in &next.blocks {
            validate(&Node::section("message.assistant").child(node.clone()))
                .expect("an incrementally parsed node is a valid view");
        }
        next
    }

    #[test]
    fn a_document_from_scratch_is_a_plain_parse() {
        let text = "# Title\n\ntext\n\n- one\n- two";
        assert_eq!(
            document("message.assistant", text, None).blocks,
            blocks("message.assistant", text)
        );
    }

    #[test]
    fn an_unchanged_document_reuses_its_parse() {
        let text = "hello\n\n> quote";
        let first = document("message.assistant", text, None);
        let second = document("message.assistant", text, Some(&first));
        assert_eq!(second.blocks, first.blocks);
        assert_eq!(second.source, text);
    }

    #[test]
    fn a_previous_that_is_not_a_prefix_falls_back_to_a_full_parse() {
        let previous = document("message.assistant", "one\n\ntwo", None);
        assert_incremental("something else\n\nentirely", &previous);
    }

    #[test]
    fn appending_agrees_with_a_full_parse_at_every_line() {
        // Every prefix of this text is fed through `document`, so the changes a
        // single append can cause all occur: a paragraph gains a line, a lone pipe
        // row becomes a table when its separator arrives, an unterminated fence
        // closes.
        let text = concat!(
            "alpha\n",
            "\n",
            "beta\n",
            "\n",
            "| head | head |\n",
            "| --- | --- |\n",
            "| one | two |\n",
            "\n",
            "```rust\n",
            "let x = 1;\n",
            "```\n",
            "\n",
            "tail line\n",
            "tail continued\n",
        );
        let mut previous: Option<Document> = None;
        let mut built = String::new();
        for line in text.split_inclusive('\n') {
            built.push_str(line);
            let next = match previous.as_ref() {
                Some(previous) => assert_incremental(&built, previous),
                None => document("message.assistant", &built, None),
            };
            previous = Some(next);
        }
        assert_eq!(built, text);
    }

    #[test]
    fn a_changed_tail_is_reparsed_while_the_prefix_is_kept() {
        // A document long enough that the retained-prefix path (all but the last two
        // blocks) runs, not a from-scratch parse, and whose appends each change a
        // different shape.
        let base = concat!(
            "# Title\n",
            "\n",
            "intro\n",
            "\n",
            "- one\n",
            "- two\n",
            "\n",
            "> quoted\n",
            "\n",
            "| head | head |\n",
            "| --- | --- |\n",
            "| one | two |\n",
            "\n",
            "```rust\n",
            "let x = 1;\n",
        );
        let previous = document("message.assistant", base, None);
        // Closing the fence changes the last block.
        let closed = assert_incremental(&format!("{base}```\n"), &previous);
        assert!(matches!(
            closed.blocks.last().map(|node| &node.kind),
            Some(Kind::Code { .. })
        ));
        // A table separator completes a table that was a paragraph a moment ago.
        let partial = document(
            "message.assistant",
            "| head | head |\n| --- | --- |\n| one | two |",
            None,
        );
        let table = assert_incremental(
            "| head | head |\n| --- | --- |\n| one | two |\n| three | four |",
            &partial,
        );
        match &table.blocks[0].kind {
            Kind::Table { rows, .. } => assert_eq!(rows.len(), 2),
            other => panic!("expected a table, got {other:?}"),
        }
        // A paragraph gains a line.
        let paragraph = assert_incremental(
            "p1\np2\np3\np4",
            &document("message.assistant", "p1\np2\np3", None),
        );
        match &paragraph.blocks[0].kind {
            Kind::Text { spans } => assert_eq!(text_of(spans), "p1\np2\np3\np4"),
            other => panic!("expected text, got {other:?}"),
        }
    }
}
