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
//! quotations, thematic breaks, and paragraphs. Inline: strong, emphasis,
//! strikethrough, inline code, and links. A table is left as prose — the vocabulary
//! has [`Kind::Table`], but a pipe table in a chat message is rare enough that
//! guessing wrong is worse than not guessing.
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
    let lines: Vec<&str> = text.split('\n').collect();
    Parser { prefix }.blocks(&lines)
}

/// Parse one run of text into inline spans, without any block structure.
///
/// Public because a producer that already knows its blocks — a tool result, a panel's
/// field — may want the inline rules without the block ones.
pub fn inline(text: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut index = 0;
    while index < text.len() {
        let rest = &text[index..];
        let ch = rest.chars().next().expect("a non-empty remainder has a first char");

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
        if (rest.starts_with("**") || rest.starts_with("__"))
            && let Some((inner, next)) = delimited(text, index, &rest[..2])
        {
            flush(&mut out, &mut plain);
            out.push(Span { text: inner, kind: SpanKind::Strong });
            index = next;
            continue;
        }
        if rest.starts_with("~~")
            && let Some((inner, next)) = delimited(text, index, "~~")
        {
            flush(&mut out, &mut plain);
            out.push(Span { text: inner, kind: SpanKind::Strikethrough });
            index = next;
            continue;
        }
        if (ch == '*' || ch == '_')
            && let Some((inner, next)) = delimited(text, index, &rest[..1])
        {
            flush(&mut out, &mut plain);
            out.push(Span { text: inner, kind: SpanKind::Emphasis });
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

        push(&mut plain, ch);
        index += ch.len_utf8();
    }
    flush(&mut out, &mut plain);
    if out.is_empty() {
        out.push(Span::plain(""));
    }
    out
}

struct Parser<'a> {
    prefix: &'a str,
}

impl Parser<'_> {
    fn role(&self, shape: &str) -> String {
        format!("{}.markdown.{shape}", self.prefix)
    }

    fn blocks(&self, lines: &[&str]) -> Vec<Node> {
        let mut out = Vec::new();
        let mut index = 0;
        while index < lines.len() {
            let line = lines[index];
            if line.trim().is_empty() {
                index += 1;
            } else if let Some(lang) = fence(line) {
                let (node, next) = self.code(lines, index, lang);
                out.push(node);
                index = next;
            } else if let Some((level, body)) = heading(line) {
                out.push(Node::new(self.role("heading"), Kind::Heading { level, spans: inline(body) }));
                index += 1;
            } else if is_rule(line) {
                out.push(Node::new(self.role("rule"), Kind::Rule));
                index += 1;
            } else if quote_line(line).is_some() {
                let (node, next) = self.quote(lines, index);
                out.push(node);
                index = next;
            } else if list_marker(line).is_some() {
                let (node, next) = self.list(lines, index);
                out.push(node);
                index = next;
            } else {
                let (node, next) = self.paragraph(lines, index);
                out.push(node);
                index = next;
            }
        }
        out
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
        (Node::new(self.role("code"), Kind::Code { lang, text, captures: Vec::new() }), index)
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
        (Node::new(self.role("quote"), Kind::Quote).children(self.blocks(&inner)), index)
    }

    fn list(&self, lines: &[&str], start: usize) -> (Node, usize) {
        let ordered = list_marker(lines[start]).expect("called on a marker").0;
        let mut items: Vec<Vec<Node>> = Vec::new();
        let mut index = start;
        while index < lines.len() {
            let Some((item_ordered, first, column)) = list_marker(lines[index]) else { break };
            // A different marker style is a different list: `1.` after `-` is what a
            // document means by ending one list and starting another.
            if item_ordered != ordered {
                break;
            }
            let mut body = vec![first.to_string()];
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
        (Node::new(self.role("list"), Kind::List { ordered, items }), index)
    }

    fn paragraph(&self, lines: &[&str], start: usize) -> (Node, usize) {
        let mut body = String::new();
        let mut index = start;
        while index < lines.len() {
            let line = lines[index];
            if line.trim().is_empty() || (index > start && is_block_start(line)) {
                break;
            }
            if !body.is_empty() {
                body.push('\n');
            }
            body.push_str(line.trim_end());
            index += 1;
        }
        (Node::text(self.role("paragraph"), inline(&body)), index)
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
    let Some(first) = trimmed.chars().next() else { return false };
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
    let href = tail[..href_end].trim().trim_start_matches('<').trim_end_matches('>');
    // A target a client cannot use is not a link: an empty one, one with whitespace or
    // a control character, or one longer than the view will accept.
    if label.is_empty() || href.is_empty() || href.len() > 4096 {
        return None;
    }
    if href.chars().any(|ch| ch.is_whitespace() || ch.is_control()) {
        return None;
    }
    Some((normalize(label), href.to_string(), start + label_end + 2 + href_end + 1))
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
        assert!(matches!(&bullets[0].kind, Kind::List { ordered: false, items } if items.len() == 2));
        let numbers = parse("1. one\n2. two");
        assert!(matches!(&numbers[0].kind, Kind::List { ordered: true, items } if items.len() == 2));
        assert_eq!(parse("1. one\n2) two\n- three").len(), 2, "a style change starts a new list");
    }

    #[test]
    fn a_nested_list_is_a_list_inside_an_item() {
        let out = parse("- one\n  - nested\n- two");
        let Kind::List { items, .. } = &out[0].kind else { panic!("expected a list") };
        assert_eq!(items.len(), 2);
        assert!(items[0].iter().any(|child| matches!(&child.kind, Kind::List { .. })));
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
    fn a_rule_is_a_rule_and_not_a_list() {
        assert!(matches!(&parse("---")[0].kind, Kind::Rule));
        assert!(matches!(&parse("***")[0].kind, Kind::Rule));
        assert!(!matches!(&parse("- item")[0].kind, Kind::Rule));
    }

    #[test]
    fn inline_runs_carry_their_meaning() {
        let out = inline("a **b** _c_ `d` ~~e~~ [f](https://example.com)");
        let kinds: Vec<SpanKind> = out.iter().map(|span| span.kind.clone()).collect();
        assert!(kinds.contains(&SpanKind::Strong));
        assert!(kinds.contains(&SpanKind::Emphasis));
        assert!(kinds.contains(&SpanKind::Code));
        assert!(kinds.contains(&SpanKind::Strikethrough));
        assert!(kinds.iter().any(|kind| matches!(kind, SpanKind::Link { href } if href == "https://example.com")));
    }

    #[test]
    fn an_unclosed_marker_stays_literal() {
        // Which is what a half-typed `**` looks like mid-stream.
        assert_eq!(inline("a **b"), vec![Span::plain("a **b")]);
    }

    #[test]
    fn an_escape_removes_the_marker() {
        assert_eq!(inline(r"\*not emphasis\*"), vec![Span::plain("*not emphasis*")]);
    }

    #[test]
    fn a_target_a_client_cannot_use_is_not_a_link() {
        assert_eq!(inline("[x](two words)"), vec![Span::plain("[x](two words)")]);
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
                assert!(child.role.starts_with("message.assistant.markdown."), "{}", child.role);
            }
        }
    }
}
