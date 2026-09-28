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

use std::collections::BTreeMap;

use misa_proto::view::{Alignment, BlobRef, Definition, Kind, Node, Span, SpanKind};

/// Parse a message body into block nodes, each role under `prefix`.
///
/// The result is empty when the text has nothing in it, and every node it does return
/// passes [`misa_proto::view::validate`].
pub fn blocks(prefix: &str, text: &str) -> Vec<Node> {
    let lines: Vec<&str> = text.split('\n').collect();
    let references = collect_references(&lines);
    let footnotes = collect_footnotes(&lines);
    let mut out = Parser {
        prefix,
        partial: false,
        references: &references,
        footnotes: &footnotes,
    }
    .blocks_tracked(&lines)
    .0;
    append_footnotes(&mut out, &references, &footnotes);
    out
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
            // A link reference definition is document state, so a new one can change a
            // block the prefix retained. When the appended text could define one, the
            // prefix is not reusable and the whole document is reparsed.
            let appended = &text[previous.source.len()..];
            // A definition split across the append boundary (`[a` then `]: url`) is
            // still a new definition, so a `]:` that starts the appended text counts,
            // and extending an existing definition line changes its target.
            let tail = previous.source.rsplit('\n').next().unwrap_or("");
            let defines = appended.split('\n').any(is_definition_candidate)
                || appended.starts_with("]:")
                || (tail.trim_start().starts_with('[') && tail.contains("]:"));
            if !defines {
                // Keep all but the last two blocks; `retained` is the index of the first
                // block that must be reparsed, and where it starts is where the suffix
                // begins. An empty `previous` reparses from the top.
                let retained = previous.blocks.len().saturating_sub(2);
                blocks.extend(previous.blocks[..retained].iter().cloned());
                starts.extend(previous.starts[..retained].iter().copied());
                from = previous.starts.get(retained).copied().unwrap_or(0);
            }
        }
    }
    // The suffix is parsed against the *whole* document's definitions, not just its
    // own, so a reference that points at a definition in the retained prefix still
    // resolves; the two parses are then identical.
    let all_lines: Vec<&str> = text.split('\n').collect();
    let references = collect_references(&all_lines);
    let footnotes = collect_footnotes(&all_lines);
    let suffix_lines: Vec<&str> = text[from..].split('\n').collect();
    let (suffix, suffix_starts) = Parser {
        prefix,
        partial: true,
        references: &references,
        footnotes: &footnotes,
    }
    .blocks_tracked(&suffix_lines);
    blocks.extend(suffix);
    starts.extend(suffix_starts.into_iter().map(|offset| from + offset));
    // The footnote section is rebuilt from the whole document every time, so a
    // stale one carried by the retained prefix is dropped before it is re-emitted.
    if blocks.last().is_some_and(is_footnote_section) {
        blocks.pop();
        starts.pop();
    }
    append_footnotes(&mut blocks, &references, &footnotes);
    starts.resize(blocks.len(), text.len());
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
    spans_only(inline_inner(
        text,
        false,
        &BTreeMap::new(),
        &BTreeMap::new(),
    ))
}

/// Text-only view of inline parts: a picture becomes its alt text.
fn spans_only(parts: Vec<InlinePart>) -> Vec<Span> {
    parts
        .into_iter()
        .map(|part| match part {
            InlinePart::Span(span) => span,
            InlinePart::Image { alt, .. } => Span::plain(alt),
        })
        .collect()
}

/// Parse inline runs, treating an unclosed opener as if it closed at the end of
/// the text. Only the streaming path sets `partial`; a settled body requires its
/// closers and keeps an unclosed marker literal. `references` are the document's
/// collected link definitions and `footnotes` its note definitions, so a
/// reference link or a note marker can resolve.
fn inline_inner(
    text: &str,
    partial: bool,
    references: &BTreeMap<String, String>,
    footnotes: &BTreeMap<String, Footnote>,
) -> Vec<InlinePart> {
    let mut out = Parts::default();
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
        if ch == '`' {
            if let Some((inner, next)) = code_span(text, index) {
                out.flush(&mut plain);
                out.push(Span::code(inner));
                index = next;
                continue;
            }
            // A streaming parse styles the rest of an unclosed run; otherwise the
            // whole run is literal and a shorter run inside it is not an opener.
            if partial && let Some((inner, kind)) = presumptive(rest) {
                out.flush(&mut plain);
                out.push(Span {
                    text: inner.to_string(),
                    kind,
                });
                break;
            }
            let ticks = rest.chars().take_while(|ch| *ch == '`').count();
            for _ in 0..ticks {
                plain.push('`');
            }
            index += ticks;
            continue;
        }
        if (rest.starts_with("***") || rest.starts_with("___"))
            && let Some((inner, next)) = delimited(text, index, &rest[..3])
        {
            out.flush(&mut plain);
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
            out.flush(&mut plain);
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
            out.flush(&mut plain);
            out.push(Span {
                text: inner,
                kind: SpanKind::Strikethrough,
            });
            index = next;
            continue;
        }
        if rest.starts_with("==")
            && let Some((inner, next)) = delimited(text, index, "==")
        {
            out.flush(&mut plain);
            out.push(Span {
                text: inner,
                kind: SpanKind::Highlight,
            });
            index = next;
            continue;
        }
        if (ch == '*' || ch == '_')
            && let Some((inner, next)) = delimited(text, index, &rest[..1])
        {
            out.flush(&mut plain);
            out.push(Span {
                text: inner,
                kind: SpanKind::Emphasis,
            });
            index = next;
            continue;
        }
        // A single `~` is subscript, but only after `~~` had its chance; the run of
        // two is strikethrough whether or not it closed.
        if ch == '~'
            && !rest.starts_with("~~")
            && let Some((inner, next)) = delimited(text, index, "~")
        {
            out.flush(&mut plain);
            out.push(Span {
                text: inner,
                kind: SpanKind::Subscript,
            });
            index = next;
            continue;
        }
        if ch == '^'
            && let Some((inner, next)) = delimited(text, index, "^")
        {
            out.flush(&mut plain);
            out.push(Span {
                text: inner,
                kind: SpanKind::Superscript,
            });
            index = next;
            continue;
        }
        if ch == '['
            && let Some((number, next)) = footnote_reference(text, index, footnotes)
        {
            // A footnote reference is a marker whose text is the note's number.
            // It rides the link vocabulary with a `footnote:` target so a client
            // can style it as a marker without a new span kind; the target is
            // never followed, only identified.
            out.flush(&mut plain);
            out.push(Span::link(number.to_string(), format!("footnote:{number}")));
            index = next;
            continue;
        }
        if ch == '!'
            && let Some((alt, target, next)) = image(text, index)
        {
            out.flush(&mut plain);
            out.image(alt, target);
            index = next;
            continue;
        }
        if ch == '['
            && let Some((label, href, next)) = link(text, index)
        {
            out.flush(&mut plain);
            out.push(Span::link(label, href));
            index = next;
            continue;
        }
        if ch == '['
            && let Some((label, href, next)) = reference_link(text, index, references)
        {
            out.flush(&mut plain);
            out.push(Span::link(label, href));
            index = next;
            continue;
        }
        if ch == '<'
            && let Some((piece, next)) = inline_html(text, index)
        {
            out.flush(&mut plain);
            match piece {
                InlineHtml::Break => plain.push('\n'),
                InlineHtml::Span { inner, kind } => out.push(Span { text: inner, kind }),
            }
            index = next;
            continue;
        }
        if ch == '<'
            && let Some((label, href, next)) = autolink(text, index)
        {
            out.flush(&mut plain);
            out.push(Span::link(label, href));
            index = next;
            continue;
        }
        if ch == ':'
            && let Some((emoji, next)) = emoji_shortcode(text, index)
        {
            plain.push_str(emoji);
            index = next;
            continue;
        }
        if let Some((label, href, next)) = bare_url(text, index) {
            out.flush(&mut plain);
            out.push(Span::link(label, href));
            index = next;
            continue;
        }

        if partial && let Some((inner, kind)) = presumptive(rest) {
            out.flush(&mut plain);
            out.push(Span {
                text: inner.to_string(),
                kind,
            });
            break;
        }

        push(&mut plain, ch);
        index += ch.len_utf8();
    }
    out.flush(&mut plain);
    if out.0.is_empty() {
        out.push(Span::plain(""));
    }
    out.0
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
        ("==", SpanKind::Highlight),
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
    if ch == '~' && !rest.starts_with("~~") && !inner.is_empty() {
        return Some((inner, SpanKind::Subscript));
    }
    if ch == '^' && !inner.is_empty() {
        return Some((inner, SpanKind::Superscript));
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

/// An inline code span opened by a run of backticks at `start`.
///
/// The closing run must be exactly as long as the opening one, so a longer run
/// inside the span is content and a shorter one cannot close it.
fn code_span(text: &str, start: usize) -> Option<(String, usize)> {
    let rest = text.get(start..)?;
    let ticks = rest.chars().take_while(|ch| *ch == '`').count();
    if ticks == 0 {
        return None;
    }
    let after = start + ticks;
    let body = &text[after..];
    let mut search = 0;
    while let Some(offset) = body[search..].find('`') {
        let at = search + offset;
        let run = body[at..].chars().take_while(|ch| *ch == '`').count();
        if run == ticks {
            return Some((normalize(&body[..at]), after + at + ticks));
        }
        search = at + run;
    }
    None
}

/// A reference link: `[text][label]`, `[label][]`, or the shortcut `[label]`.
fn reference_link(
    text: &str,
    start: usize,
    references: &BTreeMap<String, String>,
) -> Option<(String, String, usize)> {
    let rest = text.get(start..)?;
    let label_end = rest.find(']')?;
    let first = &rest[1..label_end];
    if first.is_empty() {
        return None;
    }
    let after = &rest[label_end + 1..];
    if let Some(tail) = after.strip_prefix('[') {
        let second_end = tail.find(']')?;
        let second = &tail[..second_end];
        let next = start + label_end + 2 + second_end + 1;
        let key = if second.is_empty() { first } else { second };
        let href = references.get(&key.to_ascii_lowercase())?;
        return Some((normalize(first), href.clone(), next));
    }
    let href = references.get(&first.to_ascii_lowercase())?;
    Some((normalize(first), href.clone(), start + label_end + 1))
}

enum InlineHtml {
    Break,
    Span { inner: String, kind: SpanKind },
}

/// An inline HTML tag the parser understands, and the index after it.
///
/// Only the tags that map onto a run are recognised; anything else is raw text,
/// which is what keeps a stray `<` in prose from eating the rest of the line.
fn inline_html(text: &str, start: usize) -> Option<(InlineHtml, usize)> {
    let rest = text.get(start..)?;
    let end = rest.find('>')?;
    let tag = rest[1..end].trim();
    // `<br/>` and `<br />` are the same void break as `<br>`.
    let tag = tag.strip_suffix('/').map(str::trim_end).unwrap_or(tag);
    if tag.eq_ignore_ascii_case("br") {
        return Some((InlineHtml::Break, start + end + 1));
    }
    let (name, kind) = match tag.to_ascii_lowercase().as_str() {
        "kbd" => ("kbd", SpanKind::Kbd),
        "mark" => ("mark", SpanKind::Highlight),
        "u" => ("u", SpanKind::Underline),
        "sub" => ("sub", SpanKind::Subscript),
        "sup" => ("sup", SpanKind::Superscript),
        _ => return None,
    };
    let after = start + end + 1;
    let close = format!("</{name}>");
    // `to_ascii_lowercase` preserves byte offsets, so the found index is valid in
    // the original text.
    let lower = text[after..].to_ascii_lowercase();
    let close_at = lower.find(&close)?;
    let inner = normalize(&text[after..after + close_at]);
    Some((
        InlineHtml::Span { inner, kind },
        after + close_at + close.len(),
    ))
}

/// An angle-bracket autolink, `<https://…>` or `<mail@example.com>`.
fn autolink(text: &str, start: usize) -> Option<(String, String, usize)> {
    let rest = text.get(start..)?;
    let end = rest.find('>')?;
    let inner = &rest[1..end];
    if inner.is_empty() || inner.chars().any(char::is_whitespace) {
        return None;
    }
    let next = start + end + 1;
    if is_url(inner) {
        return Some((inner.to_string(), inner.to_string(), next));
    }
    if is_email(inner) {
        return Some((inner.to_string(), format!("mailto:{inner}"), next));
    }
    None
}

fn is_url(value: &str) -> bool {
    value.starts_with("http://") || value.starts_with("https://")
}

fn is_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '@' | '.' | '_' | '-' | '+' | '%'))
}

/// An emoji shortcode `:name:` at `start`, and the index after it.
fn emoji_shortcode(text: &str, start: usize) -> Option<(&'static str, usize)> {
    let rest = text.get(start + 1..)?;
    let end = rest.find(':')?;
    if end == 0 || end > 64 {
        return None;
    }
    let name = &rest[..end];
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '+'))
    {
        return None;
    }
    let value = emoji(name)?;
    Some((value, start + 1 + end + 1))
}

/// The small builtin emoji table. An unknown name is not an emoji.
fn emoji(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "smile" => "😄",
        "smiley" => "😃",
        "laughing" | "satisfied" => "😆",
        "joy" => "😂",
        "sob" => "😭",
        "thinking" => "🤔",
        "heart" => "❤️",
        "heart_eyes" => "😍",
        "thumbsup" | "+1" => "👍",
        "thumbsdown" | "-1" => "👎",
        "rocket" => "🚀",
        "warning" => "⚠️",
        "checkered_flag" => "🏁",
        "tada" => "🎉",
        "fire" => "🔥",
        "star" => "⭐",
        "eyes" => "👀",
        "ok_hand" => "👌",
        "clap" => "👏",
        "bulb" => "💡",
        "zap" => "⚡",
        "bug" => "🐛",
        "white_check_mark" | "heavy_check_mark" => "✅",
        "x" => "❌",
        "sparkles" => "✨",
        "wave" => "👋",
        "pray" => "🙏",
        "hundred" => "💯",
        "lock" => "🔒",
        "key" => "🔑",
        "coffee" => "☕",
        "sunglasses" => "😎",
        "robot" => "🤖",
        "skull" => "💀",
        "ghost" => "👻",
        "trophy" => "🏆",
        "dart" => "🎯",
        "book" => "📖",
        "memo" => "📝",
        "pushpin" => "📌",
        "link" => "🔗",
        "mag" => "🔍",
        "bell" => "🔔",
        "calendar" => "📅",
        "clock" => "🕐",
        "hourglass" => "⏳",
        "gear" => "⚙️",
        "wrench" => "🔧",
        "shield" => "🛡️",
        "package" => "📦",
        "mail" => "📧",
        "computer" => "💻",
        _ => return None,
    })
}

/// A bare URL in running text: `https://…`, stopping at whitespace.
///
/// A trailing `.`, `,`, `;`, or `)` is punctuation around the URL, not part of it.
fn bare_url(text: &str, start: usize) -> Option<(String, String, usize)> {
    let rest = text.get(start..)?;
    if !is_url(rest) {
        return None;
    }
    if start > 0 {
        let previous = text[..start].chars().next_back()?;
        if previous.is_alphanumeric() {
            return None;
        }
    }
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    let mut url = &rest[..end];
    while let Some(last) = url.chars().next_back() {
        if matches!(last, '.' | ',' | ';' | ')') {
            url = &url[..url.len() - last.len_utf8()];
        } else {
            break;
        }
    }
    if url.len() <= 7 {
        return None;
    }
    Some((url.to_string(), url.to_string(), start + url.len()))
}

struct Parser<'a> {
    prefix: &'a str,
    /// Streaming parse: an unclosed opener styles the rest of the text.
    partial: bool,
    /// Link reference definitions collected from the whole document.
    references: &'a BTreeMap<String, String>,
    /// Footnote definitions collected from the whole document.
    footnotes: &'a BTreeMap<String, Footnote>,
}

impl Parser<'_> {
    fn role(&self, shape: &str) -> String {
        format!("{}.markdown.{shape}", self.prefix)
    }

    /// Parse inline runs for this document's mode (settled or streaming).
    /// Inline parts: text spans and pictures in document order.
    fn inline_parts(&self, text: &str) -> Vec<InlinePart> {
        inline_inner(text, self.partial, self.references, self.footnotes)
    }

    /// The spans of `text`, with pictures reduced to their alt text. Callers
    /// that cannot host a block (table cells, markers) use this.
    fn inline(&self, text: &str) -> Vec<Span> {
        spans_only(self.inline_parts(text))
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
            // A link reference definition is document state, not a block; it is
            // collected up front and skipped here. A footnote definition is the
            // same, except that its continuation lines are skipped with it.
            if is_footnote_definition(line) {
                index = footnote_definition_end(lines, index);
                continue;
            }
            if is_reference_definition(line) {
                index += 1;
                continue;
            }
            let start = line_starts[index];
            let (node, next) = if let Some(opening) = fence(line) {
                self.code(lines, index, opening)
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
            } else if is_details(line) {
                self.details(lines, index)
            } else if is_definition_list_html(line) {
                self.html_definition_list(lines, index)
            } else if is_html_block(line) {
                self.html_block(lines, index)
            } else if indent_of(line) >= 4 {
                self.indented_code(lines, index)
            } else if quote_line(line).is_some() {
                self.quote(lines, index)
            } else if list_marker(line).is_some() {
                self.list(lines, index)
            } else if starts_definition_list(lines, index) {
                self.definition_list(lines, index)
            } else {
                self.paragraph(lines, index)
            };
            out.push(node);
            starts.push(start);
            index = next;
        }
        (out, starts)
    }

    fn code(&self, lines: &[&str], start: usize, opening: Fence) -> (Node, usize) {
        let mut body = String::new();
        let mut index = start + 1;
        while index < lines.len() {
            if is_closing_fence(lines[index], &opening) {
                index += 1;
                break;
            }
            body.push_str(&normalize(lines[index]));
            body.push('\n');
            index += 1;
        }
        let text = body.trim_end_matches('\n').to_string();
        let lang = opening.lang;
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
        // A GitHub alert is a quote whose first line is a marker. The role names the
        // kind and the marker line is not part of the body; the node stays a `Quote`,
        // so every frontend already knows how to draw one.
        let alert = inner.first().and_then(|first| alert_kind(first));
        if alert.is_some() {
            inner.remove(0);
        }
        let role = match alert {
            Some(kind) => self.role(&format!("alert.{kind}")),
            None => self.role("quote"),
        };
        (
            Node::new(role, Kind::Quote).children(self.blocks(&inner)),
            index,
        )
    }

    /// A run of lines each indented at least four columns, with the indent stripped.
    fn indented_code(&self, lines: &[&str], start: usize) -> (Node, usize) {
        let mut body = String::new();
        let mut index = start;
        while index < lines.len() {
            let line = lines[index];
            if line.trim().is_empty() {
                body.push('\n');
                index += 1;
                continue;
            }
            if indent_of(line) < 4 {
                break;
            }
            body.push_str(&normalize(&dedent(line, 4)));
            body.push('\n');
            index += 1;
        }
        let text = body.trim_end_matches('\n').to_string();
        (
            Node::new(self.role("code"), Kind::Code { lang: None, text }),
            index,
        )
    }

    /// A `<details>` block: a summary and the blocks it hides.
    fn details(&self, lines: &[&str], start: usize) -> (Node, usize) {
        let mut raw = String::new();
        let mut index = start;
        while index < lines.len() {
            raw.push_str(lines[index]);
            raw.push('\n');
            index += 1;
            if raw.to_ascii_lowercase().contains("</details>") {
                break;
            }
        }
        // Lowercasing the ASCII preserves byte offsets, so the found positions index
        // the original text.
        let lower = raw.to_ascii_lowercase();
        let summary_open = lower.find("<summary");
        let summary = summary_open.and_then(|open| {
            let content_start = open + lower[open..].find('>')? + 1;
            let content_end = content_start + lower[content_start..].find("</summary>")?;
            Some(
                normalize(&raw[content_start..content_end])
                    .trim()
                    .to_string(),
            )
        });
        let body_start = if summary.is_some() {
            let open = summary_open.expect("a found summary has an opening tag");
            open + lower[open..].find("</summary>").expect("summary was found") + "</summary>".len()
        } else {
            lower.find('>').map_or(0, |gt| gt + 1)
        };
        let body_end = lower
            .rfind("</details>")
            .unwrap_or(raw.len())
            .max(body_start);
        let body_lines: Vec<&str> = raw[body_start..body_end].split('\n').collect();
        let spans = summary.map_or_else(Vec::new, |summary| self.inline(&summary));
        let children = self.blocks(&body_lines);
        (
            Node::new(self.role("details"), Kind::Collapsible { summary: spans })
                .children(children),
            index,
        )
    }

    /// A block of raw HTML the parser has no shape for: kept as code a client can show.
    fn html_block(&self, lines: &[&str], start: usize) -> (Node, usize) {
        let mut body = String::new();
        let mut index = start;
        while index < lines.len() && !lines[index].trim().is_empty() {
            body.push_str(&normalize(lines[index]));
            body.push('\n');
            index += 1;
        }
        let text = body.trim_end_matches('\n').to_string();
        (
            Node::new(
                self.role("html"),
                Kind::Code {
                    lang: Some("html".into()),
                    text,
                },
            ),
            index,
        )
    }

    fn definition_list(&self, lines: &[&str], start: usize) -> (Node, usize) {
        let mut entries: Vec<Definition> = Vec::new();
        let mut index = start;
        loop {
            // One or more term lines, with a definition marker ending the run.
            let term_start = index;
            while index < lines.len()
                && !lines[index].trim().is_empty()
                && !is_definition_marker(lines[index])
                && !is_block_start(lines[index])
            {
                index += 1;
            }
            if index == term_start || index >= lines.len() || !is_definition_marker(lines[index]) {
                break;
            }
            let term = self.inline(&lines[term_start..index].join("\n"));
            let mut definitions = Vec::new();
            while index < lines.len() && is_definition_marker(lines[index]) {
                let mut text = definition_marker_body(lines[index]).to_string();
                index += 1;
                // A continuation line is part of the definition whether it is
                // indented (the usual case) or lazy: only a new block ends it.
                while index < lines.len() {
                    let line = lines[index];
                    if line.trim().is_empty() || is_definition_marker(line) {
                        break;
                    }
                    if is_block_start(line) {
                        break;
                    }
                    text.push('\n');
                    text.push_str(&dedent(line, 4));
                    index += 1;
                }
                definitions.push(self.inline(&text));
            }
            entries.push(Definition { term, definitions });
            // A blank line between entries is ordinary spacing; the next entry
            // exists only if another term run is followed by a marker.
            let mut next = index;
            while next < lines.len() && lines[next].trim().is_empty() {
                next += 1;
            }
            if next >= lines.len() || !starts_definition_list(lines, next) {
                break;
            }
            index = next;
        }
        (
            Node::new(self.role("definition"), Kind::Definition { entries }),
            index,
        )
    }

    /// A `<dl>` block: each `<dt>` term and the `<dd>` definitions under it.
    fn html_definition_list(&self, lines: &[&str], start: usize) -> (Node, usize) {
        let mut raw = String::new();
        let mut index = start;
        while index < lines.len() {
            raw.push_str(lines[index]);
            raw.push('\n');
            index += 1;
            if raw.to_ascii_lowercase().contains("</dl>") {
                break;
            }
        }
        let entries = self.html_definition_entries(&raw);
        if entries.is_empty() {
            // A `<dl>` the parser could not read as terms and definitions is
            // still raw HTML, which is what the fallback already says.
            return self.html_block(lines, start);
        }
        (
            Node::new(self.role("definition"), Kind::Definition { entries }),
            index,
        )
    }

    fn html_definition_entries(&self, raw: &str) -> Vec<Definition> {
        // Lowercasing ASCII preserves byte offsets, so the found positions index
        // the original text.
        let lower = raw.to_ascii_lowercase();
        let mut entries = Vec::new();
        let mut terms: Vec<String> = Vec::new();
        let mut definitions: Vec<String> = Vec::new();
        let mut index = 0;
        while index < lower.len() {
            let next_dt = lower[index..].find("<dt");
            let next_dd = lower[index..].find("<dd");
            let (at, closing) = match (next_dt, next_dd) {
                (Some(dt), Some(dd)) if dt <= dd => (index + dt, "</dt>"),
                (_, Some(dd)) => (index + dd, "</dd>"),
                (Some(dt), None) => (index + dt, "</dt>"),
                (None, None) => break,
            };
            let Some(open_end) = lower[at..].find('>') else {
                break;
            };
            let content_start = at + open_end + 1;
            let Some(close_at) = lower[content_start..].find(closing) else {
                break;
            };
            let inner = normalize(&raw[content_start..content_start + close_at]);
            if closing == "</dt>" {
                if !definitions.is_empty() && !terms.is_empty() {
                    entries.push(Definition {
                        term: self.inline(&terms.join("\n")),
                        definitions: definitions
                            .iter()
                            .map(|definition| self.inline(definition))
                            .collect(),
                    });
                    terms.clear();
                    definitions.clear();
                }
                terms.push(inner);
            } else if !terms.is_empty() {
                definitions.push(inner);
            }
            index = content_start + close_at + closing.len();
        }
        if !definitions.is_empty() && !terms.is_empty() {
            entries.push(Definition {
                term: self.inline(&terms.join("\n")),
                definitions: definitions
                    .iter()
                    .map(|definition| self.inline(definition))
                    .collect(),
            });
        }
        entries
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
        // A picture is a block: a paragraph that contains one splits around it.
        let mut blocks: Vec<Node> = Vec::new();
        let mut spans: Vec<Span> = Vec::new();
        for part in self.inline_parts(&body) {
            match part {
                InlinePart::Span(span) => spans.push(span),
                InlinePart::Image { alt, target } => {
                    if !spans.is_empty() {
                        blocks.push(Node::text(
                            self.role("paragraph"),
                            std::mem::take(&mut spans),
                        ));
                    }
                    blocks.push(Node::new(
                        self.role("image"),
                        Kind::Image {
                            blob: BlobRef {
                                hash: target,
                                len: 0,
                                media: None,
                            },
                            alt,
                            width: 0,
                            height: 0,
                        },
                    ));
                }
            }
        }
        if !spans.is_empty() {
            blocks.push(Node::text(self.role("paragraph"), spans));
        }
        let node = match blocks.len() {
            0 => Node::text(self.role("paragraph"), vec![Span::plain("")]),
            1 => blocks.pop().expect("one block"),
            _ => Node::new(self.role("paragraph"), Kind::Section).children(blocks),
        };
        (node, index)
    }

    /// A pipe table: a header row, a separator row, then body rows.
    ///
    /// The separator row's colons are the author's alignment for each column, so
    /// they travel on the tree rather than being reduced to a validation. A cell's
    /// inline runs are parsed with the same rules as any other text, so emphasis,
    /// code, and links inside a cell mean what they mean everywhere else. A short
    /// row while streaming is padded with empty cells.
    fn table(&self, lines: &[&str], start: usize) -> (Node, usize) {
        let header_cells = split_table_row(lines[start]);
        let columns = header_cells.len();
        let delimiters = split_table_row(lines[start + 1]);
        // A delimiter row of a different shape still yields one alignment per
        // header column; a missing one is the left default.
        let align: Vec<Alignment> = (0..columns)
            .map(|index| {
                delimiters
                    .get(index)
                    .map(|cell| table_alignment(cell))
                    .unwrap_or_default()
            })
            .collect();
        let head = header_cells
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
            Node::new(self.role("table"), Kind::Table { head, rows, align }),
            index,
        )
    }
}

/// Whether a line begins a block, so a paragraph in front of it ends.
fn is_block_start(line: &str) -> bool {
    fence(line).is_some()
        || heading(line).is_some()
        || is_rule(line)
        || is_reference_definition(line)
        || is_details(line)
        || is_html_block(line)
        || quote_line(line).is_some()
        || list_marker(line).is_some()
}

/// An opening code fence: the run that opened it, its length, and its info string.
struct Fence {
    marker: char,
    len: usize,
    lang: Option<String>,
}

/// The opening fence of a line, if it is one.
///
/// Both backtick and tilde fences are recognised. A backtick fence's info string
/// may not contain a backtick, which is what keeps inline code out of the way.
fn fence(line: &str) -> Option<Fence> {
    let rest = line.trim_start();
    let marker = rest.chars().next()?;
    if marker != '`' && marker != '~' {
        return None;
    }
    let len = rest.chars().take_while(|ch| *ch == marker).count();
    if len < 3 {
        return None;
    }
    let info = rest[len..].trim();
    if marker == '`' && info.contains('`') {
        return None;
    }
    Some(Fence {
        marker,
        len,
        lang: (!info.is_empty()).then(|| info.to_string()),
    })
}

/// Whether a line closes `opening`: the same marker, at least as long, nothing else.
fn is_closing_fence(line: &str, opening: &Fence) -> bool {
    let trimmed = line.trim_start();
    let run = trimmed
        .chars()
        .take_while(|ch| *ch == opening.marker)
        .count();
    run >= opening.len && run > 0 && trimmed[run..].trim().is_empty()
}

/// Whether a line opens a `<details>` element.
fn is_details(line: &str) -> bool {
    starts_with_tag(line, "details")
}

/// Whether a line opens a tag the parser has no shape for, so it is kept raw.
///
/// The inline tags map onto runs inside a paragraph and are deliberately excluded,
/// and a URL or an email (`<https://…>`, `<mail@example.com>`) has a `:` or `@`
/// where a tag would have `>` or whitespace.
fn is_html_block(line: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix('<') else {
        return false;
    };
    if rest.starts_with('/') || rest.starts_with('!') || rest.starts_with('?') {
        return false;
    }
    let name: String = rest
        .chars()
        .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '-')
        .collect();
    if !name
        .chars()
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic())
    {
        return false;
    }
    match rest[name.len()..].chars().next() {
        None | Some('>') | Some('/') => {}
        Some(ch) if ch.is_whitespace() => {}
        _ => return false,
    }
    !matches!(
        name.to_ascii_lowercase().as_str(),
        "kbd" | "mark" | "u" | "sub" | "sup" | "br"
    )
}

/// Whether a line opens a tag with the given name.
fn starts_with_tag(line: &str, name: &str) -> bool {
    let trimmed = line.trim_start();
    let Some(rest) = trimmed.strip_prefix('<') else {
        return false;
    };
    let lower = rest.to_ascii_lowercase();
    if !lower.starts_with(name) {
        return false;
    }
    match lower[name.len()..].chars().next() {
        None | Some('>') | Some('/') => true,
        Some(ch) => ch.is_whitespace(),
    }
}

/// Whether a line opens a definition list element, `<dl>`.
fn is_definition_list_html(line: &str) -> bool {
    starts_with_tag(line, "dl")
}

/// A definition-list marker: a `:` or `~` followed by whitespace or nothing,
/// indented at most three columns. Returns the text after the marker.
fn definition_marker_body(line: &str) -> &str {
    let indent = indent_of(line);
    if indent > 3 {
        return "";
    }
    let rest = &line[indent..];
    let Some(marker) = rest.chars().next() else {
        return "";
    };
    if marker != ':' && marker != '~' {
        return "";
    }
    let after = &rest[marker.len_utf8()..];
    after.trim_start()
}

/// Whether a line is a definition-list marker.
fn is_definition_marker(line: &str) -> bool {
    let indent = indent_of(line);
    if indent > 3 {
        return false;
    }
    let rest = &line[indent..];
    matches!(rest.chars().next(), Some(':') | Some('~'))
        && rest.chars().nth(1).is_none_or(char::is_whitespace)
}

/// Whether the run of lines at `index` is a term followed by a definition
/// marker, which is what makes it a definition list rather than a paragraph.
fn starts_definition_list(lines: &[&str], index: usize) -> bool {
    let Some(first) = lines.get(index) else {
        return false;
    };
    if first.trim().is_empty() || is_definition_marker(first) {
        return false;
    }
    let mut look = index;
    while look < lines.len() {
        let line = lines[look];
        if line.trim().is_empty() {
            return false;
        }
        if is_definition_marker(line) {
            return look > index;
        }
        if is_block_start(line) {
            return false;
        }
        look += 1;
    }
    false
}

/// A footnote definition line: `[^label]: body`.
fn footnote_definition(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("[^")?;
    let close = rest.find("]:")?;
    let label = &rest[..close];
    if label.is_empty() || label.contains('[') || label.contains(']') {
        return None;
    }
    Some((label, rest[close + 2..].trim()))
}

/// Whether a line is a footnote definition.
fn is_footnote_definition(line: &str) -> bool {
    footnote_definition(line).is_some()
}

/// The index after a footnote definition's body, so its continuation lines are
/// skipped with it rather than parsed as an indented code block.
fn footnote_definition_end(lines: &[&str], start: usize) -> usize {
    let mut index = start + 1;
    while index < lines.len() {
        let line = lines[index];
        if line.trim().is_empty() || is_block_start(line) {
            break;
        }
        index += 1;
    }
    index
}

/// A footnote definition as the document knows it: the number its marker shows
/// and the raw body text, before inline parsing.
struct Footnote {
    number: usize,
    body: String,
}

/// Every footnote definition in the document, keyed by its lowercased label.
///
/// The number comes from the definition's order in the document, so it is stable
/// as an answer is appended to: a new definition can only be later, never
/// renumber an earlier one.
fn collect_footnotes(lines: &[&str]) -> BTreeMap<String, Footnote> {
    let mut footnotes = BTreeMap::new();
    let mut number = 0;
    let mut index = 0;
    while index < lines.len() {
        if let Some((label, body)) = footnote_definition(lines[index]) {
            number += 1;
            let mut text = body.to_string();
            let end = footnote_definition_end(lines, index);
            for line in &lines[index + 1..end] {
                text.push('\n');
                text.push_str(&dedent(line, 4));
            }
            footnotes.insert(label.to_ascii_lowercase(), Footnote { number, body: text });
            index = end;
            continue;
        }
        index += 1;
    }
    footnotes
}

/// A footnote reference `[^label]`, when a definition with that label exists.
/// Returns the number the marker shows and the index after the reference.
fn footnote_reference(
    text: &str,
    start: usize,
    footnotes: &BTreeMap<String, Footnote>,
) -> Option<(usize, usize)> {
    let rest = text.get(start..)?;
    let inner = rest.strip_prefix("[^")?;
    let close = inner.find(']')?;
    let label = &inner[..close];
    if label.is_empty() || label.contains('[') {
        return None;
    }
    let footnote = footnotes.get(&label.to_ascii_lowercase())?;
    Some((footnote.number, start + 2 + close + 1))
}

/// Whether a node is the footnotes section the parser appends last.
fn is_footnote_section(node: &Node) -> bool {
    node.role == "markdown.footnote" && matches!(node.kind, Kind::Section)
}

/// Append the document's footnotes under a quiet heading, one entry per note.
///
/// The marker is a link with a `footnote:` target, which is the parser's whole
/// contract for it: a client styles it as a marker and never follows it. The
/// back-link carries `footnote-back:` and points the reader home.
fn append_footnotes(
    out: &mut Vec<Node>,
    references: &BTreeMap<String, String>,
    footnotes: &BTreeMap<String, Footnote>,
) {
    if footnotes.is_empty() {
        return;
    }
    let mut ordered: Vec<&Footnote> = footnotes.values().collect();
    ordered.sort_by_key(|footnote| footnote.number);
    let mut section = Node::new("markdown.footnote", Kind::Section).child(Node::new(
        "markdown.footnote.heading",
        Kind::Text {
            spans: vec![Span::plain("Footnotes")],
        },
    ));
    for footnote in ordered {
        let mut spans = vec![
            Span::link(
                footnote.number.to_string(),
                format!("footnote:{}", footnote.number),
            ),
            Span::plain(" "),
        ];
        spans.extend(spans_only(inline_inner(
            &footnote.body,
            false,
            references,
            footnotes,
        )));
        spans.push(Span::plain(" "));
        spans.push(Span::link(
            "↩",
            format!("footnote-back:{}", footnote.number),
        ));
        section = section.child(Node::new("markdown.footnote.entry", Kind::Text { spans }));
    }
    out.push(section);
}

/// The GitHub-alert kind of a blockquote's first line, if it names one.
fn alert_kind(line: &str) -> Option<&'static str> {
    Some(match line.trim().to_ascii_lowercase().as_str() {
        "[!note]" => "note",
        "[!tip]" => "tip",
        "[!important]" => "important",
        "[!warning]" => "warning",
        "[!caution]" => "caution",
        _ => return None,
    })
}

/// A link reference definition line: `[label]: url`.
fn reference_definition(line: &str) -> Option<(&str, &str)> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix('[')?;
    let close = rest.find("]:")?;
    let label = &rest[..close];
    if label.is_empty() || label.contains('[') || label.contains(']') || label.starts_with('^') {
        return None;
    }
    let raw = rest[close + 2..].trim();
    if raw.is_empty() {
        return None;
    }
    let url = if let Some(inner) = raw.strip_prefix('<') {
        inner.split_once('>')?.0
    } else {
        raw.split_whitespace().next()?
    };
    if url.is_empty() {
        return None;
    }
    Some((label, url))
}

/// Whether a line is a link reference definition, so a block parser skips it.
fn is_reference_definition(line: &str) -> bool {
    reference_definition(line).is_some()
}

/// Whether a line could be a definition, for the incremental guard. Deliberately
/// loose: a false positive costs one extra parse, a false negative is a wrong tree.
fn is_definition_candidate(line: &str) -> bool {
    line.trim_start().starts_with('[') && line.contains("]:")
}

/// Every link reference definition in the document, keyed by its lowercased label.
fn collect_references(lines: &[&str]) -> BTreeMap<String, String> {
    let mut references = BTreeMap::new();
    for line in lines {
        if let Some((label, url)) = reference_definition(line) {
            references.insert(label.to_ascii_lowercase(), url.to_string());
        }
    }
    references
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

/// The alignment a delimiter cell asks for: `:--` left, `:-:` centre, `--:` right.
fn table_alignment(cell: &str) -> Alignment {
    let cell = cell.trim();
    match (cell.starts_with(':'), cell.ends_with(':')) {
        (true, true) => Alignment::Center,
        (false, true) => Alignment::Right,
        _ => Alignment::Left,
    }
}

/// Split a pipe-table row into trimmed cells, without the outer pipes.
///
/// A backslash escapes the character after it, so `\|` is content, not a
/// separator. The pair is kept in the cell text and resolved by the inline
/// parser, which is the one place an escape becomes the character it protected.
fn split_table_row(line: &str) -> Vec<String> {
    let mut cells = Vec::new();
    let mut current = String::new();
    let mut chars = line.trim().chars().peekable();
    // An unescaped leading pipe is the row's frame, not its first cell.
    if chars.peek() == Some(&'|') {
        chars.next();
    }
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            current.push(ch);
            if let Some(next) = chars.next() {
                current.push(next);
            }
            continue;
        }
        if ch == '|' {
            if chars.peek().is_none() {
                // A trailing pipe closes the row rather than opening an empty cell.
                cells.push(current.trim().to_string());
                current.clear();
                break;
            }
            cells.push(current.trim().to_string());
            current.clear();
            continue;
        }
        current.push(ch);
    }
    if !current.is_empty() || cells.is_empty() {
        cells.push(current.trim().to_string());
    }
    cells
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

/// One inline part: a run of text, or a picture. A picture is a block where it
/// lands, so the scanner reports it as itself instead of pretending it is
/// text with a strange prefix.
#[derive(Clone, Debug, PartialEq)]
pub enum InlinePart {
    Span(Span),
    Image { alt: String, target: String },
}

/// The scanner output: spans and pictures in document order.
#[derive(Default)]
struct Parts(Vec<InlinePart>);
impl Parts {
    fn push(&mut self, span: Span) {
        self.0.push(InlinePart::Span(span));
    }
    fn image(&mut self, alt: String, target: String) {
        self.0.push(InlinePart::Image { alt, target });
    }
    fn flush(&mut self, plain: &mut String) {
        if !plain.is_empty() {
            self.push(Span::plain(std::mem::take(plain)));
        }
    }
}

/// `![alt](hash)`: a picture, which is a block wherever it lands. The target
/// is a content hash — anything else is an ordinary link with a bang in front.
fn image(text: &str, start: usize) -> Option<(String, String, usize)> {
    let rest = text.get(start..)?;
    rest.strip_prefix("![")?;
    let (label, href, next) = link(text, start + 1)?;
    is_blob_hash(&href).then_some((label, href, next))
}

/// A lowercase hex content hash: what a blob reference names.
fn is_blob_hash(target: &str) -> bool {
    target.len() == 64
        && target
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_escapable(ch: char) -> bool {
    matches!(
        ch,
        '\\' | '`' | '*' | '_' | '~' | '[' | ']' | '(' | ')' | '|'
    )
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

    #[test]
    fn a_picture_is_a_block_and_the_paragraph_splits_around_it() {
        let blocks = parse(
            "before ![alt text](0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef) after",
        );
        assert_eq!(blocks.len(), 1, "{blocks:?}");
        let children = &blocks[0].children;
        assert_eq!(children.len(), 3, "{children:?}");
        assert!(matches!(&children[0].kind, Kind::Text { spans } if text_of(spans) == "before "));
        assert!(matches!(&children[1].kind, Kind::Image { blob, alt, .. }
                if blob.hash == "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef" && alt == "alt text"));
        assert!(matches!(&children[2].kind, Kind::Text { spans } if text_of(spans) == " after"));
    }

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
            Kind::Table { head, rows, align } => {
                assert_eq!(head.len(), 2);
                assert_eq!(rows.len(), 2);
                assert_eq!(text_of(&head[0]), "a");
                assert_eq!(text_of(&rows[1][1]), "four");
                assert_eq!(align, &[Alignment::Left, Alignment::Left]);
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
        let out = parse("| a | b | c |\n|:--|:-:|--:|\n| one |");
        match &out[0].kind {
            Kind::Table { rows, align, .. } => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].len(), 3, "a short row is padded to the header");
                assert_eq!(text_of(&rows[0][1]), "");
                assert_eq!(
                    align,
                    &[Alignment::Left, Alignment::Center, Alignment::Right]
                );
            }
            other => panic!("expected a table, got {other:?}"),
        }
        // A bare separator is left-aligned, and a table with no markers still
        // carries one alignment per column rather than nothing at all.
        match &parse("| a | b |\n| --- | --- |\n| one | two |")[0].kind {
            Kind::Table { align, .. } => {
                assert_eq!(align, &[Alignment::Left, Alignment::Left])
            }
            other => panic!("expected a table, got {other:?}"),
        }
    }

    #[test]
    fn an_escaped_pipe_is_content_and_a_cell_keeps_its_inline_runs() {
        // `\|` does not split the cell, and the inline parser resolves the escape
        // to the pipe it protected while keeping the surrounding emphasis.
        let out =
            parse("| a | b |\n| --- | --- |\n| x \\| y | **bold** `code` [link](https://x) |");
        match &out[0].kind {
            Kind::Table { head, rows, .. } => {
                assert_eq!(head.len(), 2, "the escaped pipe must not add a column");
                assert_eq!(text_of(&rows[0][0]), "x | y");
                let spans = &rows[0][1];
                assert!(
                    spans.iter().any(|span| span.kind == SpanKind::Strong),
                    "{spans:?}"
                );
                assert!(
                    spans.iter().any(|span| span.kind == SpanKind::Code),
                    "{spans:?}"
                );
                assert!(
                    spans.iter().any(|span| matches!(
                        &span.kind,
                        SpanKind::Link { href } if href == "https://x"
                    )),
                    "{spans:?}"
                );
                // A doubled backslash is a literal backslash followed by a real
                // separator, so the second cell still starts a new column.
                let doubled = split_table_row(r"| a \\| b |");
                // The pair is kept for the inline parser; `\\` is two cells once
                // the escape is resolved.
                assert_eq!(doubled, vec!["a \\\\", "b"]);
            }
            other => panic!("expected a table, got {other:?}"),
        }
    }

    #[test]
    fn a_streaming_table_cell_styles_an_unclosed_span() {
        // The partial parse is what a table sees while it is still arriving: the
        // last cell's opener has no closer yet, and the row is padded to the
        // header's width so the renderer already has a column for it.
        let document = document(
            "message.assistant",
            "| a | b |\n| --- | --- |\n| one | **two",
            None,
        );
        match &document.blocks[0].kind {
            Kind::Table { rows, align, .. } => {
                assert_eq!(rows[0].len(), 2);
                assert_eq!(align.len(), 2);
                assert!(
                    rows[0][1]
                        .iter()
                        .any(|span| span.kind == SpanKind::Strong && span.text == "two"),
                    "{:?}",
                    rows[0][1]
                );
            }
            other => panic!("expected a table, got {other:?}"),
        }
        // A settled parse keeps the same row's marker literal instead.
        match &parse("| a | b |\n| --- | --- |\n| one | **two")[0].kind {
            Kind::Table { rows, .. } => {
                assert_eq!(text_of(&rows[0][1]), "**two");
            }
            other => panic!("expected a table, got {other:?}"),
        }
    }

    #[test]
    fn a_streamed_table_keeps_its_alignment_as_rows_arrive() {
        let first = document(
            "message.assistant",
            "| a | b |\n|:--|--:|\n| one | two |",
            None,
        );
        let next = assert_incremental(
            "| a | b |\n|:--|--:|\n| one | two |\n| three | four |",
            &first,
        );
        match &next.blocks[0].kind {
            Kind::Table { rows, align, .. } => {
                assert_eq!(rows.len(), 2, "the appended row arrived");
                assert_eq!(
                    align,
                    &[Alignment::Left, Alignment::Right],
                    "the delimiter row's alignment survived the append"
                );
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
    fn inline_builds_highlight_subscript_and_superscript() {
        assert_eq!(
            inline("a ==b== c"),
            vec![
                Span::plain("a "),
                Span {
                    text: "b".into(),
                    kind: SpanKind::Highlight,
                },
                Span::plain(" c"),
            ]
        );
        assert_eq!(
            inline("H~2~O"),
            vec![
                Span::plain("H"),
                Span {
                    text: "2".into(),
                    kind: SpanKind::Subscript,
                },
                Span::plain("O"),
            ]
        );
        assert_eq!(
            inline("x^2^"),
            vec![
                Span::plain("x"),
                Span {
                    text: "2".into(),
                    kind: SpanKind::Superscript,
                },
            ]
        );
        // `~~` is strikethrough; a single `~` is subscript. The doubled run wins.
        let strike = inline("a ~~b~~ c");
        assert!(
            strike
                .iter()
                .any(|span| span.kind == SpanKind::Strikethrough && span.text == "b"),
            "{strike:?}"
        );
    }

    #[test]
    fn inline_html_maps_known_tags_and_keeps_the_rest_literal() {
        let find = |text: &str, kind: SpanKind| {
            inline(text)
                .into_iter()
                .find(|span| span.kind == kind)
                .map(|span| span.text)
        };
        assert_eq!(
            find("<kbd>Ctrl</kbd>", SpanKind::Kbd).as_deref(),
            Some("Ctrl")
        );
        assert_eq!(
            find("<mark>hi</mark>", SpanKind::Highlight).as_deref(),
            Some("hi")
        );
        assert_eq!(
            find("<u>hi</u>", SpanKind::Underline).as_deref(),
            Some("hi")
        );
        assert_eq!(
            find("<sub>2</sub>", SpanKind::Subscript).as_deref(),
            Some("2")
        );
        assert_eq!(
            find("<sup>2</sup>", SpanKind::Superscript).as_deref(),
            Some("2")
        );
        // `<br>` is a line break, not a run.
        assert_eq!(text_of(&inline("a<br>b")), "a\nb");
        assert_eq!(text_of(&inline("a<BR/>b")), "a\nb");
        // A tag the parser has no shape for is raw text.
        assert_eq!(
            inline("<span>x</span>"),
            vec![Span::plain("<span>x</span>")]
        );
    }

    #[test]
    fn autolinks_become_links() {
        assert_eq!(
            inline("<https://example.com>"),
            vec![Span::link("https://example.com", "https://example.com")]
        );
        assert_eq!(
            inline("<mail@example.com>"),
            vec![Span::link("mail@example.com", "mailto:mail@example.com")]
        );
        // A bare URL links, and trailing sentence punctuation is not part of it.
        assert_eq!(
            inline("see https://example.com/x, then"),
            vec![
                Span::plain("see "),
                Span::link("https://example.com/x", "https://example.com/x"),
                Span::plain(", then"),
            ]
        );
        assert_eq!(
            inline("(https://example.com)"),
            vec![
                Span::plain("("),
                Span::link("https://example.com", "https://example.com"),
                Span::plain(")"),
            ]
        );
        // A URL glued to a word is not a link.
        assert_eq!(
            inline("xhttps://example.com"),
            vec![Span::plain("xhttps://example.com")]
        );
    }

    #[test]
    fn emoji_shortcodes_replace_known_names_and_leave_unknown_ones() {
        assert_eq!(inline(":rocket:"), vec![Span::plain("🚀")]);
        assert_eq!(text_of(&inline(":HEART:")), "❤️");
        assert!(text_of(&inline("hi :smile:")).contains('😄'));
        // An unknown shortcode stays literal, and a bare colon is not one.
        assert_eq!(text_of(&inline(":nope:")), ":nope:");
        assert_eq!(text_of(&inline("10:30")), "10:30");
    }

    #[test]
    fn multi_backtick_code_spans() {
        assert_eq!(
            inline("``code with ` inside``"),
            vec![Span::code("code with ` inside")]
        );
        // A shorter run inside a longer one is content.
        assert_eq!(inline("```a `` b```"), vec![Span::code("a `` b")]);
        // An unmatched run is literal, and a suffix of it is not a new opener.
        assert_eq!(inline("``a`"), vec![Span::plain("``a`")]);
    }

    #[test]
    fn an_indented_run_becomes_a_code_block() {
        let out = parse("intro\n\n    let x = 1;\n    let y = 2;\n\nafter");
        assert_eq!(
            roles(&out),
            vec![
                "message.assistant.markdown.paragraph",
                "message.assistant.markdown.code",
                "message.assistant.markdown.paragraph",
            ]
        );
        match &out[1].kind {
            Kind::Code { lang, text } => {
                assert!(lang.is_none());
                assert_eq!(text, "let x = 1;\nlet y = 2;");
            }
            other => panic!("expected code, got {other:?}"),
        }
    }

    #[test]
    fn an_unknown_html_block_is_kept_as_raw_code() {
        let out = parse("<div class=\"x\">\nraw\n</div>\n\nafter");
        assert_eq!(
            roles(&out),
            vec![
                "message.assistant.markdown.html",
                "message.assistant.markdown.paragraph",
            ]
        );
        match &out[0].kind {
            Kind::Code { lang, text } => {
                assert_eq!(lang.as_deref(), Some("html"));
                assert_eq!(text, "<div class=\"x\">\nraw\n</div>");
            }
            other => panic!("expected code, got {other:?}"),
        }
        // A known inline tag at the start of a line is still parsed inline.
        let inline_tag = parse("<kbd>Ctrl</kbd>");
        assert_eq!(
            roles(&inline_tag),
            vec!["message.assistant.markdown.paragraph"]
        );
    }

    #[test]
    fn a_github_alert_is_a_quote_with_a_role() {
        let out = parse("> [!WARNING]\n> be careful\n> indeed");
        assert_eq!(
            roles(&out),
            vec!["message.assistant.markdown.alert.warning"]
        );
        assert!(matches!(&out[0].kind, Kind::Quote));
        match &out[0].children[0].kind {
            Kind::Text { spans } => assert_eq!(text_of(spans), "be careful\nindeed"),
            other => panic!("expected text, got {other:?}"),
        }
        assert_eq!(
            roles(&parse("> [!note]\n> x")),
            vec!["message.assistant.markdown.alert.note"]
        );
        // An ordinary quote keeps the plain role and its body.
        assert_eq!(
            roles(&parse("> hello")),
            vec!["message.assistant.markdown.quote"]
        );
    }

    #[test]
    fn a_details_block_becomes_a_collapsible() {
        let out = parse("<details>\n<summary>More</summary>\n\nhidden **text**\n\n</details>");
        assert_eq!(roles(&out), vec!["message.assistant.markdown.details"]);
        match &out[0].kind {
            Kind::Collapsible { summary } => assert_eq!(text_of(summary), "More"),
            other => panic!("expected a collapsible, got {other:?}"),
        }
        assert!(
            out[0]
                .children
                .iter()
                .any(|child| matches!(child.kind, Kind::Text { .. })),
            "{:?}",
            out[0].children
        );
    }

    #[test]
    fn a_definition_list_parses_terms_and_definitions() {
        let out = parse("Apple\n:   A fruit.\n:   A company.\n\nOrange\n:   A colour.");
        assert_eq!(roles(&out), vec!["message.assistant.markdown.definition"]);
        match &out[0].kind {
            Kind::Definition { entries } => {
                assert_eq!(entries.len(), 2, "two entries, one per term block");
                assert_eq!(text_of(&entries[0].term), "Apple");
                assert_eq!(entries[0].definitions.len(), 2);
                assert_eq!(text_of(&entries[0].definitions[0]), "A fruit.");
                assert_eq!(text_of(&entries[0].definitions[1]), "A company.");
                assert_eq!(text_of(&entries[1].term), "Orange");
                assert_eq!(text_of(&entries[1].definitions[0]), "A colour.");
            }
            other => panic!("expected a definition list, got {other:?}"),
        }
        validate(&Node::section("root").children(out)).unwrap();
    }

    #[test]
    fn a_tilde_marker_and_multiple_terms_share_a_definition() {
        let out = parse("Term A\nTerm B\n~ shared body\n    continued lazily\nand more");
        match &out[0].kind {
            Kind::Definition { entries } => {
                assert_eq!(entries.len(), 1);
                // Two term lines are one term whose newline is content.
                assert_eq!(text_of(&entries[0].term), "Term A\nTerm B");
                assert_eq!(
                    text_of(&entries[0].definitions[0]),
                    "shared body\ncontinued lazily\nand more"
                );
            }
            other => panic!("expected a definition list, got {other:?}"),
        }
    }

    #[test]
    fn a_dl_element_maps_to_the_same_kind() {
        let out = parse("<dl>\n<dt>Apple</dt>\n<dd>A fruit.</dd>\n</dl>");
        assert_eq!(roles(&out), vec!["message.assistant.markdown.definition"]);
        match &out[0].kind {
            Kind::Definition { entries } => {
                assert_eq!(entries.len(), 1);
                assert_eq!(text_of(&entries[0].term), "Apple");
                assert_eq!(text_of(&entries[0].definitions[0]), "A fruit.");
            }
            other => panic!("expected a definition list, got {other:?}"),
        }
    }

    #[test]
    fn footnote_references_resolve_and_emit_a_section() {
        let out = parse("Text[^one].\n\n[^one]: The note.");
        // The marker is a link with a `footnote:` target, whose text is the number.
        match &out[0].kind {
            Kind::Text { spans } => assert!(
                spans.iter().any(|span| matches!(
                    &span.kind,
                    SpanKind::Link { href } if href == "footnote:1"
                ) && span.text == "1"),
                "{spans:?}"
            ),
            other => panic!("expected a paragraph, got {other:?}"),
        }
        // The section is emitted last and carries the definition and a back-link.
        let section = out.last().expect("a footnotes section");
        assert_eq!(section.role, "markdown.footnote");
        assert!(matches!(section.kind, Kind::Section));
        let text = section
            .children
            .iter()
            .filter_map(|child| match &child.kind {
                Kind::Text { spans } => Some(text_of(spans)),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" ");
        assert!(text.contains("The note."), "{text}");
        assert!(text.contains('↩'), "{text}");
        validate(&Node::section("root").children(out)).unwrap();
    }

    #[test]
    fn a_footnote_definition_is_not_a_link_reference() {
        // A bare note definition is document state, not a paragraph, and it does
        // not register as a link reference for an ordinary shortcut.
        let out = parse("[^one]: The note.");
        assert_eq!(out.len(), 1, "{:?}", roles(&out));
        assert_eq!(out[0].role, "markdown.footnote");
        assert!(
            !out.iter().any(|node| node.role.ends_with(".paragraph")),
            "a definition became a paragraph"
        );
    }

    #[test]
    fn a_footnote_reference_to_a_later_definition_resolves_incrementally() {
        // A streaming parse of a marker with no definition yet does not resolve
        // it, and emits no footnotes section.
        let previous = document("message.assistant", "Text[^one].", None);
        let resolves = |blocks: &[Node]| {
            blocks.iter().any(|node| match &node.kind {
                Kind::Text { spans } => spans.iter().any(
                    |span| matches!(&span.kind, SpanKind::Link { href } if href == "footnote:1"),
                ),
                _ => false,
            })
        };
        assert!(!resolves(&previous.blocks));
        assert!(
            previous
                .blocks
                .iter()
                .all(|node| node.role != "markdown.footnote"),
            "{:?}",
            roles(&previous.blocks)
        );
        // ...and once it is appended, a full parse and the incremental one agree,
        // and the marker resolves to the definition's number.
        let next = assert_incremental("Text[^one].\n\n[^one]: The note.", &previous);
        assert!(resolves(&next.blocks), "{:?}", next.blocks);
        assert_eq!(next.blocks.last().unwrap().role, "markdown.footnote");
    }

    #[test]
    fn reference_links_resolve_from_definitions_anywhere() {
        let out = parse(
            "See [the docs][docs] and [docs][] and [docs].\n\n[docs]: https://example.com/docs",
        );
        match &out[0].kind {
            Kind::Text { spans } => {
                let hrefs: Vec<&str> = spans
                    .iter()
                    .filter_map(|span| match &span.kind {
                        SpanKind::Link { href } => Some(href.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(hrefs, vec!["https://example.com/docs"; 3]);
            }
            other => panic!("expected text, got {other:?}"),
        }
        // The definition line is not a block of its own.
        assert_eq!(out.len(), 1, "{:?}", roles(&out));
    }

    #[test]
    fn tilde_fences_and_longer_fences() {
        let out = parse("~~~rust\nlet x = 1;\n~~~");
        match &out[0].kind {
            Kind::Code { lang, text } => {
                assert_eq!(lang.as_deref(), Some("rust"));
                assert_eq!(text, "let x = 1;");
            }
            other => panic!("expected code, got {other:?}"),
        }
        // A longer opening fence is not closed by a shorter run.
        let out = parse("````\n```\nstill code\n````");
        match &out[0].kind {
            Kind::Code { text, .. } => assert_eq!(text, "```\nstill code"),
            other => panic!("expected code, got {other:?}"),
        }
        // A backtick fence whose info string contains a backtick is not a fence.
        assert!(matches!(
            &parse("```a`b\nx\n```")[0].kind,
            Kind::Text { .. }
        ));
    }

    #[test]
    fn an_appended_definition_reparses_the_prefix_it_changes() {
        let previous = document("message.assistant", "[docs]\n\nbody", None);
        // Before the definition arrives the shortcut is literal...
        match &previous.blocks[0].kind {
            Kind::Text { spans } => assert_eq!(spans, &vec![Span::plain("[docs]")]),
            other => panic!("expected text, got {other:?}"),
        }
        // ...and once it is appended, a full parse and the incremental one agree.
        let next = assert_incremental(
            "[docs]\n\nbody\n\n[docs]: https://example.com/docs",
            &previous,
        );
        match &next.blocks[0].kind {
            Kind::Text { spans } => assert!(
                spans.iter().any(|span| matches!(
                    &span.kind,
                    SpanKind::Link { href } if href == "https://example.com/docs"
                )),
                "{spans:?}"
            ),
            other => panic!("expected text, got {other:?}"),
        }
    }

    #[test]
    fn extending_a_definition_line_reparses_the_retained_prefix() {
        // Four blocks plus a trailing definition, so the shortcut in the first block
        // sits in the retained prefix, not the reparsed suffix.
        let previous = document(
            "message.assistant",
            "[docs]\n\nb1\n\nb2\n\nb3\n\n[docs]: http",
            None,
        );
        // Appending to the definition changes its target without introducing a new
        // `[label]:` line, so the guard has to notice the tail's own definition.
        let next = assert_incremental("[docs]\n\nb1\n\nb2\n\nb3\n\n[docs]: https://x", &previous);
        match &next.blocks[0].kind {
            Kind::Text { spans } => assert!(
                spans.iter().any(|span| matches!(
                    &span.kind,
                    SpanKind::Link { href } if href == "https://x"
                )),
                "{spans:?}"
            ),
            other => panic!("expected text, got {other:?}"),
        }
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
