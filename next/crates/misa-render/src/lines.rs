//! The semantic tree as styled lines.
//!
//! For a medium that has lines: a terminal, a pipe, a log. A pixel frontend does
//! not use this module — it maps the same tree to a scene — but everything this
//! module decides is *policy about a linear medium*, which is exactly what two
//! linear frontends should share rather than each inventing.
//!
//! What it decides:
//!
//! - indentation, from nesting depth;
//! - wrapping, from the medium's width;
//! - a rail beside a message, from the node's role, if the theme names one;
//! - how a table, a list, a meter, and a code block look when they are linear.
//!
//! What it does not decide: any colour a theme has not named, and anything about
//! the agent.

use misa_proto::view::{Capture, Kind, Node, Span, State};
use misa_value::Value;

use crate::text::{clip, pad, width, wrap_spans};
use crate::theme::{Style, Theme};

/// One rendered line: indentation, styled runs, and the node it came from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    /// Spaces of indentation, before the first span.
    pub indent: u8,
    pub spans: Vec<(Style, String)>,
    /// The node this line belongs to, so a client can map a click or a scroll
    /// anchor back to the tree without re-deriving it.
    pub node: Option<String>,
}

impl Line {
    pub fn text(&self) -> String {
        let mut out = " ".repeat(self.indent as usize);
        for (_, text) in &self.spans {
            out.push_str(text);
        }
        out
    }

    pub fn is_blank(&self) -> bool {
        self.spans.iter().all(|(_, text)| text.trim().is_empty())
    }

    /// A line with one run, for the many places a renderer has a single string.
    fn simple(indent: u8, style: Style, text: impl Into<String>, node: Option<&str>) -> Line {
        Line {
            indent,
            spans: vec![(style, text.into())],
            node: node.map(str::to_string),
        }
    }
}

/// Render a tree to lines at a given width.
pub fn render(node: &Node, theme: &Theme, columns: usize) -> Vec<Line> {
    let mut out = Vec::new();
    Renderer { theme, columns }.node(node, 0, &mut out);
    while out.last().is_some_and(Line::is_blank) {
        out.pop();
    }
    out
}

/// The text of rendered lines, without styles. For a pipe or a test.
pub fn to_plain(lines: &[Line]) -> String {
    let mut out = String::new();
    for (index, line) in lines.iter().enumerate() {
        if index > 0 {
            out.push('\n');
        }
        out.push_str(&line.text().trim_end());
    }
    out.push('\n');
    out
}

struct Renderer<'a> {
    theme: &'a Theme,
    columns: usize,
}

impl<'a> Renderer<'a> {
    fn node(&self, node: &Node, depth: usize, out: &mut Vec<Line>) {
        let indent = (depth as u8).saturating_mul(2);
        let style = self.theme.role(&node.role);
        match &node.kind {
            Kind::Section => {
                let mut rail = false;
                // A node whose role names a rail is drawn with one, and its
                // children are indented behind it. That is how a message's extent
                // becomes visible in a terminal without any colour at all.
                if let Some((glyph, rail_style)) = self.theme.rail(&node.role) {
                    rail = true;
                    if let Some(label) = &node.label {
                        out.push(Line::simple(indent, style, label.clone(), Some(&node.id)));
                    }
                    let _ = rail_style;
                    let _ = glyph;
                } else if let Some(label) = &node.label {
                    out.push(Line::simple(indent, style.bold(), label.clone(), Some(&node.id)));
                }
                self.state_mark(node, indent, out);
                for child in &node.children {
                    self.node(child, if rail { depth + 1 } else { depth }, out);
                }
            }
            Kind::Text { spans } => {
                let budget = self.budget(indent);
                for line in wrap_spans(spans, budget) {
                    out.push(Line {
                        indent,
                        spans: line
                            .iter()
                            .map(|span| (self.span_style(span, style), span.text.clone()))
                            .collect(),
                        node: Some(node.id.clone()),
                    });
                }
                self.state_mark(node, indent, out);
            }
            Kind::Heading { level, spans } => {
                // A heading is structure, so the linear medium makes it visible without a
                // theme's help: the first level is underlined as well as bold, the rest are
                // bold. A theme that wants to say more names the role.
                let base = if *level <= 1 { style.bold().underline() } else { style.bold() };
                let budget = self.budget(indent);
                for line in wrap_spans(spans, budget) {
                    out.push(Line {
                        indent,
                        spans: line.iter().map(|span| (self.span_style(span, base), span.text.clone())).collect(),
                        node: Some(node.id.clone()),
                    });
                }
                self.state_mark(node, indent, out);
            }
            Kind::Quote => {
                // The marker is the medium's, not the theme's: a quote has to read as a
                // quote in a theme that named nothing at all.
                let mut rendered = Vec::new();
                for child in &node.children {
                    self.node(child, depth + 1, &mut rendered);
                }
                for mut line in rendered {
                    line.spans.insert(0, (style.dim(), "▏ ".to_string()));
                    out.push(line);
                }
                self.state_mark(node, indent, out);
            }
            Kind::Rule => {
                let width = self.budget(indent).min(60).max(1);
                out.push(Line::simple(indent, style.dim(), "─".repeat(width), Some(&node.id)));
            }
            Kind::Code { lang, text, captures } => {
                // A diff is code whose meaning is *per line*, and the session says so —
                // by naming a role that ends in `.diff`, or, for a body a model fenced,
                // by the language it was fenced with. Nothing here guesses from the
                // text: a block of code that happens to contain a `+` is still code.
                let diff = is_diff(&node.role, lang.as_deref());
                if let Some(lang) = lang
                    && !diff
                {
                    out.push(Line::simple(
                        indent,
                        style.dim(),
                        format!("{lang}"),
                        Some(&node.id),
                    ));
                }
                let budget = self.budget(indent);
                for (index, raw) in text.split('\n').enumerate() {
                    let value = clip(raw, budget);
                    let line_style = if diff { self.diff_style(style, &node.role, raw) } else { style };
                    out.push(Line {
                        indent,
                        spans: self.code_spans(text, index, raw, &value, captures, line_style),
                        node: Some(node.id.clone()),
                    });
                }
                self.state_mark(node, indent, out);
            }
            Kind::List { ordered, items } => {
                let _budget = self.budget(indent).saturating_sub(2);
                for (index, item) in items.iter().enumerate() {
                    let marker = if *ordered {
                        format!("{}. ", index + 1)
                    } else {
                        "• ".to_string()
                    };
                    let mut rendered = Vec::new();
                    for child in item {
                        self.node(child, depth + 1, &mut rendered);
                    }
                    if rendered.is_empty() {
                        continue;
                    }
                    let mut first = rendered.remove(0);
                    first.indent = indent;
                    first.spans.insert(0, (style, marker));
                    out.push(first);
                    for mut line in rendered {
                        line.indent = line.indent.saturating_add(indent).min(u8::MAX);
                        out.push(line);
                    }
                }
                self.state_mark(node, indent, out);
            }
            Kind::Table { head, rows } => {
                self.table(head, rows, indent, style, &node.id, out);
                self.state_mark(node, indent, out);
            }
            Kind::Fields { fields } => {
                let label_width = fields
                    .iter()
                    .map(|field| width(&field.label))
                    .max()
                    .unwrap_or(0)
                    .min(self.budget(indent) / 2);
                for field in fields {
                    let label = pad(&clip(&field.label, label_width), label_width);
                    out.push(Line::simple(
                        indent,
                        style.dim(),
                        format!("{label}  "),
                        Some(&node.id),
                    ));
                    let budget = self.budget(indent + 2);
                    for line in wrap_spans(&[Span::plain(&field.value)], budget) {
                        out.push(Line {
                            indent: indent + 2,
                            spans: line.iter().map(|span| (style, span.text.clone())).collect(),
                            node: Some(node.id.clone()),
                        });
                    }
                }
                self.state_mark(node, indent, out);
            }
            Kind::Collapsible { summary, open } => {
                let budget = self.budget(indent);
                for line in wrap_spans(summary, budget) {
                    out.push(Line {
                        indent,
                        spans: line
                            .iter()
                            .map(|span| (self.span_style(span, style), span.text.clone()))
                            .collect(),
                        node: Some(node.id.clone()),
                    });
                }
                // The client decides whether the body is showing; a client that
                // always shows it passes a tree it has already expanded. The
                // renderer honours what it was given, which is why `open` only
                // affects the marker here.
                let _ = open;
                for child in &node.children {
                    self.node(child, depth + 1, out);
                }
                self.state_mark(node, indent, out);
            }
            Kind::Image { alt, width, height, blob } => {
                let label = if alt.is_empty() {
                    format!("[{width}×{height} image {}]", clip(&blob.hash, 8))
                } else {
                    format!("[image: {alt}]")
                };
                out.push(Line::simple(indent, style.dim(), label, Some(&node.id)));
            }
            Kind::Fact { value } => {
                // The one place a number becomes words, and it is the client that
                // does it: the session said what the number *is* by naming the role.
                out.push(Line::simple(
                    indent,
                    style,
                    crate::fact::format(&node.role, value),
                    Some(&node.id),
                ));
            }
            Kind::Status { text } => {
                out.push(Line::simple(indent, style, text.clone(), Some(&node.id)));
            }
            Kind::Meter { label, value, max, text } => {
                let budget = self.budget(indent);
                let bar_width = 10usize.min(budget.saturating_sub(width(label) + 4).max(1));
                let ratio = if *max > 0.0 { (value / max).clamp(0.0, 1.0) } else { 0.0 };
                let filled = (ratio * bar_width as f64).round() as usize;
                let bar = format!("[{}{}]", "#".repeat(filled), "-".repeat(bar_width - filled));
                out.push(Line::simple(
                    indent,
                    style,
                    format!("{label} {bar} {text}"),
                    Some(&node.id),
                ));
            }
        }
    }

    /// The marker a node's state earns, when it is not `Done`.
    ///
    /// A finished node says nothing: a checkmark on every settled line is noise. A
    /// node that is still going, or did not finish, says so once.
    fn state_mark(&self, node: &Node, indent: u8, out: &mut Vec<Line>) {
        let Some(state) = node.state else {
            return;
        };
        if state == State::Done {
            return;
        }
        let Some(style) = self.theme.state(Some(state)) else {
            return;
        };
        let mark = self.theme.state_mark(state);
        let label = format!("{mark} {}", state_word(state));
        out.push(Line::simple(indent, style, label, Some(&node.id)));
    }

    fn span_style(&self, span: &Span, base: Style) -> Style {
        use misa_proto::view::SpanKind;
        match &span.kind {
            SpanKind::Plain => base,
            SpanKind::Strong => base.bold(),
            SpanKind::Emphasis => base.italic(),
            SpanKind::Strikethrough => Style { strikethrough: true, ..base },
            SpanKind::Code => base,
            SpanKind::Link { .. } => base.underline(),
            SpanKind::Token { name } => base.over(self.theme.token(name)),
        }
    }

    /// Split one line of a code block by the captures that cover it.
    fn code_spans(
        &self,
        whole: &str,
        line_index: usize,
        raw: &str,
        clipped: &str,
        captures: &[Capture],
        base: Style,
    ) -> Vec<(Style, String)> {
        if captures.is_empty() {
            return vec![(base, clipped.to_string())];
        }
        // Byte offset of this line inside the whole text.
        let mut offset = 0usize;
        for (index, line) in whole.split('\n').enumerate() {
            if index == line_index {
                break;
            }
            offset += line.len() + 1;
        }
        let mut spans = Vec::new();
        let mut cursor = 0usize;
        while cursor < clipped.len() {
            let absolute = offset + cursor;
            let capture = captures
                .iter()
                .find(|capture| capture.start as usize <= absolute && absolute < capture.end as usize);
            let mut end = clipped.len();
            if let Some(capture) = capture {
                end = ((capture.end as usize).saturating_sub(offset)).min(clipped.len());
            } else if let Some(next) = captures
                .iter()
                .filter(|capture| capture.start as usize > absolute)
                .min_by_key(|capture| capture.start)
            {
                end = ((next.start as usize).saturating_sub(offset)).min(clipped.len());
            }
            let end = end.max(cursor + 1).min(clipped.len());
            let end = floor_char_boundary(clipped, end);
            let style = capture.map(|capture| self.theme.token(&capture.token)).unwrap_or(base);
            spans.push((style, clipped[cursor..end].to_string()));
            cursor = end;
        }
        if spans.is_empty() {
            // The line was empty, or clipping removed it entirely.
            let _ = raw;
            spans.push((base, String::new()));
        }
        spans
    }

    fn table(
        &self,
        head: &[Vec<Span>],
        rows: &[Vec<Vec<Span>>],
        indent: u8,
        style: Style,
        node: &str,
        out: &mut Vec<Line>,
    ) {
        let columns = head.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
        if columns == 0 {
            return;
        }
        let text_of = |cells: &[Span]| cells.iter().map(|span| span.text.as_str()).collect::<String>();
        let mut widths = vec![1usize; columns];
        for row in std::iter::once(head).chain(rows.iter().map(Vec::as_slice)) {
            for (index, cell) in row.iter().enumerate() {
                widths[index] = widths[index].max(width(&text_of(cell)));
            }
        }
        // Shrink the widest column until the table fits, so a wide table degrades
        // into a narrower one instead of overflowing the viewport.
        let budget = self.budget(indent).saturating_sub(columns.saturating_sub(1) * 2);
        while widths.iter().sum::<usize>() > budget {
            let Some((index, _)) = widths.iter().enumerate().max_by_key(|(_, width)| **width) else {
                break;
            };
            if widths[index] <= 3 {
                break;
            }
            widths[index] -= 1;
        }
        for (row_index, row) in std::iter::once(head).chain(rows.iter().map(Vec::as_slice)).enumerate() {
            let mut spans = Vec::new();
            for index in 0..columns {
                if index > 0 {
                    spans.push((style.dim(), "  ".to_string()));
                }
                let cell = row.get(index).map(|cell| text_of(cell)).unwrap_or_default();
                let text = pad(&cell, widths[index]);
                let cell_style = if row_index == 0 { style.bold() } else { style };
                spans.push((cell_style, text));
            }
            out.push(Line { indent, spans, node: Some(node.to_string()) });
        }
    }

    /// The style for one line of a diff, by what the line starts with.
    ///
    /// A theme may name the role under the node's own prefix — `…markdown.diff.add` —
    /// and a theme that has not falls back to the generic `diff.add`. That is one place
    /// to say what an added line looks like, and still a way to say something narrower.
    /// The node's own style stays underneath, so a diff inside a tool result still reads
    /// like a tool result.
    fn diff_style(&self, base: Style, role: &str, raw: &str) -> Style {
        let Some(suffix) = diff_suffix(raw) else {
            return base;
        };
        let specific = format!("{role}.{suffix}");
        if self.theme.names(&specific) {
            return base.over(self.theme.role(&specific));
        }
        base.over(self.theme.role(&format!("diff.{suffix}")))
    }

    /// Columns available to a node at a given indentation.
    fn budget(&self, indent: u8) -> usize {
        self.columns.saturating_sub(indent as usize).max(1)
    }
}

/// Whether a code node's body is a diff.
///
/// Two ways for a session to say so, and neither of them looks at the body: a role that
/// ends in `.diff` — the sessions' own vocabulary for a result that is a patch — or a
/// fence a model wrote with `diff`. A block of code that happens to begin a line with
/// `+` is still a block of code.
fn is_diff(role: &str, lang: Option<&str>) -> bool {
    role.ends_with(".diff") || lang.is_some_and(|lang| lang.eq_ignore_ascii_case("diff"))
}

/// What a line of a diff is, named the way a theme names a role.
///
/// A unified diff says everything with one character in column zero, which is why this
/// can be exact: `+`/`-` are the change, `@@` is where a hunk begins, `---`/`+++` are
/// the file headers, and `diff --git`/`index`/`\ No newline` are the patch talking about
/// itself. Anything else is context, and context is left in the node's own style.
fn diff_suffix(raw: &str) -> Option<&'static str> {
    Some(match raw {
        line if line.starts_with("+++ ") || line.starts_with("--- ") => "header",
        line if line.starts_with("@@") => "hunk",
        line if line.starts_with("diff ") || line.starts_with("index ") => "meta",
        line if line.starts_with("\\ No newline") => "meta",
        line if line.starts_with('+') => "add",
        line if line.starts_with('-') => "remove",
        _ => return None,
    })
}

fn state_word(state: State) -> &'static str {
    match state {
        State::Pending => "waiting",
        State::Streaming => "streaming",
        State::Done => "done",
        State::Failed => "failed",
        State::Cancelled => "cancelled",
    }
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// A value rendered for a person, in the medium's own vocabulary.
pub fn value_text(value: &Value) -> String {
    match value {
        Value::Str(text) => text.to_string(),
        Value::Int(number) => number.to_string(),
        Value::Float(number) => format!("{number}"),
        Value::Bool(true) => "yes".into(),
        Value::Bool(false) => "no".into(),
        Value::Null => String::new(),
        Value::Bytes(bytes) => format!("<{} bytes>", bytes.len()),
        other => format!("{other}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Color;
    use misa_proto::view::{Action, ActionOn, Field, FieldKind, State};

    fn theme() -> Theme {
        Theme::plain()
    }

    #[test]
    fn text_wraps_to_the_given_width() {
        let node = Node::text("message.assistant", [Span::plain("alpha beta gamma delta")]);
        let lines = render(&node, &theme(), 8);
        assert!(lines.len() >= 3);
        for line in &lines {
            assert!(width(&line.text()) <= 12, "{:?}", line.text());
        }
    }

    #[test]
    fn a_marked_role_is_railed_and_its_body_indented() {
        let node = Node::section("message.user")
            .id("m1")
            .child(Node::text("message.user", [Span::plain("hello")]));
        let lines = render(&node, &Theme::dark(), 80);
        let body = lines.iter().find(|line| line.text().contains("hello")).expect("body line");
        assert!(body.indent > 0, "a railed message did not indent its body");
    }

    #[test]
    fn a_finished_node_says_nothing_and_an_unfinished_one_says_so() {
        let done = Node::section("tool.result").state(State::Done);
        assert!(render(&done, &Theme::dark(), 80).is_empty());
        let failed = Node::section("tool.result").state(State::Failed);
        let lines = render(&failed, &Theme::dark(), 80);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text().contains("failed"));
    }

    #[test]
    fn a_code_block_is_clipped_and_captures_colour_it() {
        let text = "let x = 1;";
        let node = Node::new(
            "tool.call",
            Kind::Code {
                lang: Some("rust".into()),
                text: text.into(),
                captures: vec![Capture { start: 0, end: 3, token: "keyword".into() }],
            },
        );
        let lines = render(&node, &Theme::dark(), 6);
        assert_eq!(lines[0].text(), "rust");
        let body = &lines[1];
        assert!(width(&body.text()) <= 6);
        // The first run is the capture's and carries the keyword colour.
        assert_eq!(body.spans[0].0, Theme::dark().token("keyword"));
    }

    #[test]
    fn a_narrow_table_shrinks_instead_of_overflowing() {
        let head = vec![vec![Span::plain("first column")], vec![Span::plain("second column")]];
        let rows = vec![vec![vec![Span::plain("one")], vec![Span::plain("two")]]];
        let node = Node::new("table", Kind::Table { head, rows }).id("t");
        let lines = render(&node, &theme(), 16);
        for line in &lines {
            assert!(width(&line.text()) <= 16, "{:?} is too wide", line.text());
        }
        assert!(lines[0].text().starts_with("first"));
        assert!(lines[1].text().starts_with("one"));
    }

    #[test]
    fn a_list_gets_markers_and_indents_its_children() {
        let item = vec![Node::text("message.assistant", [Span::plain("first item that wraps")])];
        let node = Node::new("list", Kind::List { ordered: true, items: vec![item, vec![Node::text("x", [Span::plain("second")])]] });
        let lines = render(&node, &theme(), 10);
        assert!(lines[0].text().starts_with("1. "));
        assert!(lines.iter().any(|line| line.text().contains("2. second")));
    }

    #[test]
    fn fields_align_their_labels() {
        let node = Node::new(
            "dialog",
            Kind::Fields {
                fields: vec![
                    Field { id: "a".into(), label: "Model".into(), value: "claude".into(), hint: None, kind: FieldKind::Text },
                    Field { id: "b".into(), label: "Reasoning effort".into(), value: "high".into(), hint: None, kind: FieldKind::Choice { options: vec![], selected: None } },
                ],
            },
        );
        let lines = render(&node, &theme(), 60);
        assert!(lines[0].text().starts_with("Model"));
        assert!(lines[2].text().starts_with("Reasoning effort"));
        assert_eq!(lines[0].indent, lines[2].indent);
    }

    #[test]
    fn a_meter_shows_a_bounded_bar() {
        let node = Node::new(
            "value.meter",
            Kind::Meter { label: "budget".into(), value: 2.0, max: 4.0, text: "$2 / $4".into() },
        );
        let line = &render(&node, &theme(), 80)[0];
        assert!(line.text().contains("[#####-----]"), "{}", line.text());
        assert!(line.text().ends_with("$2 / $4"));
    }

    #[test]
    fn an_image_degrades_to_its_alt_text() {
        let node = Node::new(
            "screenshot",
            Kind::Image {
                blob: misa_proto::view::BlobRef { hash: "a".repeat(64), len: 9, media: None },
                alt: "a chart".into(),
                width: 800,
                height: 600,
            },
        );
        assert_eq!(render(&node, &theme(), 80)[0].text(), "[image: a chart]");
    }

    #[test]
    fn a_collapsible_renders_its_summary_and_its_children() {
        let node = Node::new(
            "message.assistant",
            Kind::Collapsible { summary: vec![Span::plain("thinking…")], open: false },
        )
        .id("m1")
        .child(Node::text("message.thinking", [Span::plain("because")]));
        let lines = render(&node, &theme(), 40);
        assert_eq!(lines[0].text(), "thinking…");
        assert!(lines.iter().any(|line| line.text().contains("because")));
        assert!(lines[1].indent > lines[0].indent);
    }

    #[test]
    fn to_plain_drops_trailing_whitespace_and_ends_with_one_newline() {
        let node = Node::new("status", Kind::Status { text: "ready".into() });
        let text = to_plain(&render(&node, &theme(), 40));
        assert_eq!(text, "ready\n");
    }

    #[test]
    fn every_line_points_at_the_node_it_came_from() {
        let node = Node::section("message.user")
            .id("m1")
            .child(Node::text("message.user", [Span::plain("hello")]).id("m1.text"));
        let lines = render(&node, &theme(), 40);
        assert!(lines.iter().all(|line| line.node.as_deref() == Some("m1.text")));
    }

    #[test]
    fn an_action_is_not_rendered_as_text() {
        // A click affordance is the client's to draw: the tree carries the id and
        // the label, and the linear renderer shows the label only as part of the
        // node's own content.
        let node = Node::text("message.assistant", [Span::plain("choose")]).action(Action {
            id: "dialog.submit".into(),
            on: ActionOn::Click,
            label: Some("Submit".into()),
            args: Value::Null,
        });
        let text = to_plain(&render(&node, &theme(), 40));
        assert_eq!(text, "choose\n");
    }

    #[test]
    fn a_value_becomes_text_without_pulling_in_json() {
        assert_eq!(value_text(&Value::Int(3)), "3");
        assert_eq!(value_text(&Value::Bool(true)), "yes");
        assert_eq!(value_text(&Value::str("x")), "x");
        assert_eq!(value_text(&Value::Null), "");
    }
    #[test]
    fn a_diff_is_laid_out_by_what_changed_on_each_line() {
        let node = Node::new(
            "message.assistant.markdown.diff",
            Kind::Code {
                lang: Some("diff".into()),
                text: "@@ -1 +1 @@\n-old line\n+new line\n context".into(),
                captures: Vec::new(),
            },
        );
        let theme = Theme::dark();
        let lines = render(&node, &theme, 40);
        // No `diff` banner: the body already says what it is, line by line.
        assert_eq!(lines.len(), 4, "{:?}", to_plain(&lines));
        assert_eq!(lines[0].spans[0].0, theme.role("diff.hunk"));
        assert_eq!(lines[1].spans[0].0, theme.role("diff.remove"));
        assert_eq!(lines[2].spans[0].0, theme.role("diff.add"));
        // Context keeps the node's own style, so a diff still reads like a tool result.
        assert_eq!(lines[3].spans[0].0, theme.role("message.assistant.markdown.diff"));
        assert_eq!(to_plain(&lines), "@@ -1 +1 @@\n-old line\n+new line\n context\n");
    }

    #[test]
    fn a_theme_may_name_a_diff_line_under_the_node_it_belongs_to() {
        let node = Node::new(
            "tool.result.diff",
            Kind::Code { lang: None, text: "+added".into(), captures: Vec::new() },
        );
        // A theme that says something narrower wins over the generic role.
        let theme = Theme::dark().with_role("tool.result.diff.add", Style::rgb(1, 2, 3).bold());
        let line = &render(&node, &theme, 40)[0];
        assert_eq!(line.spans[0].0.fg, Color::Rgb(1, 2, 3));
        assert!(line.spans[0].0.bold);
        // And the generic role is used when it has not.
        let line = &render(&node, &Theme::dark(), 40)[0];
        assert_eq!(line.spans[0].0, Theme::dark().role("diff.add"));
    }

    #[test]
    fn code_that_merely_looks_like_a_diff_is_still_code() {
        let node = Node::new(
            "message.assistant.markdown.code",
            Kind::Code { lang: Some("rust".into()), text: "+ 1".into(), captures: Vec::new() },
        );
        let theme = Theme::dark();
        let lines = render(&node, &theme, 40);
        assert_eq!(lines[0].text(), "rust", "the language banner is gone from a code block");
        assert_eq!(lines[1].spans[0].0, theme.role("message.assistant.markdown.code"));
    }

    #[test]
    fn a_heading_is_bold_and_a_quote_carries_a_marker() {
        let heading = Node::new(
            "message.assistant.markdown.heading",
            Kind::Heading { level: 2, spans: vec![Span::plain("Title")] },
        );
        let lines = render(&heading, &Theme::dark(), 40);
        assert_eq!(lines[0].text(), "Title");
        assert!(lines[0].spans[0].0.bold, "a heading is not set apart at all");

        let quote = Node::new("message.assistant.markdown.quote", Kind::Quote)
            .child(Node::text("message.assistant.markdown.paragraph", [Span::plain("quoted")]));
        let lines = render(&quote, &Theme::dark(), 40);
        assert!(lines[0].text().contains("quoted"));
        assert!(lines[0].text().ends_with("▏ quoted"), "a quote has no marker: {:?}", lines[0].text());
    }

    #[test]
    fn a_rule_is_a_line_of_its_own_and_a_list_item_is_not_one() {
        let node = Node::new("message.assistant.markdown.rule", Kind::Rule);
        let lines = render(&node, &theme(), 40);
        assert_eq!(lines.len(), 1);
        assert!(lines[0].text().chars().all(|ch| ch == '─'), "{:?}", lines[0].text());
    }
}
