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

use misa_proto::view::{Kind, Node, Span, State};
use misa_value::Value;

use misa_render::text::{clip, pad, width, wrap_spans};
use misa_render::theme::{Style, Theme};

/// One rendered line: indentation, styled runs, and the node it came from.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Line {
    /// Spaces of indentation, before the first span.
    pub indent: u8,
    pub spans: Vec<(Style, String)>,
    /// The surface behind this physical row.  It is separate from span colour:
    /// a message surface continues through its rail and to the terminal edge,
    /// while foreground styling still belongs to each run.
    pub surface: Option<Style>,
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
            surface: None,
            node: node.map(str::to_string),
        }
    }
}

/// Render a tree to lines at a given width.
pub fn render(node: &Node, theme: &Theme, columns: usize) -> Vec<Line> {
    let mut out = render_block(node, theme, columns, 0);
    while out.last().is_some_and(Line::is_blank) {
        out.pop();
    }
    out
}

/// Render one retained layout owner, preserving boundary blank lines and depth.
/// The caller trims only the end of the complete document.
pub fn render_block(node: &Node, theme: &Theme, columns: usize, depth: usize) -> Vec<Line> {
    let mut out = Vec::new();
    Renderer { theme, columns }.node(node, depth, 0, &mut out);
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
    fn spaces_between_children(role: &str) -> bool {
        matches!(
            role,
            "message.group" | "message.user" | "message.assistant" | "error"
        )
    }

    fn children(&self, node: &Node, depth: usize, inset: usize) -> Vec<Line> {
        let spaced = Self::spaces_between_children(&node.role);
        let mut out = Vec::new();
        for child in &node.children {
            let mut lines = Vec::new();
            self.node(child, depth, inset, &mut lines);
            if lines.is_empty() {
                continue;
            }
            if spaced && !out.is_empty() {
                // A gap between two blocks of one message belongs to that message:
                // it carries the message's surface so a railed block does not come
                // apart into fragments. The owning section adds the rail itself.
                // The prefix cells the ancestors will prepend stay an inset, not
                // spaces, so the gap is widened with the rest of the block.
                out.push(Line {
                    indent: (depth as u8).saturating_mul(2),
                    surface: self.theme.surface(&node.role),
                    ..Line::default()
                });
            }
            out.extend(lines);
        }
        out
    }

    fn node(&self, node: &Node, depth: usize, inset: usize, out: &mut Vec<Line>) {
        if let Some(lines) = crate::components::render_default(node, self.theme, self.columns) {
            out.extend(lines);
            return;
        }
        let indent = (depth as u8).saturating_mul(2);
        let style = self.theme.role(&node.role);
        match &node.kind {
            Kind::Section => {
                // A node whose role names a rail is drawn with one. The rail is a
                // prefix span and its two cells are an *inset*, not indentation:
                // that is what lets a quote, a list marker or a nested rail sit at
                // the same column and compose in the order the ancestors add them.
                if let Some((glyph, rail_style)) = self.theme.rail(&node.role) {
                    let surface = self.theme.surface(&node.role);
                    let rail_width = width(&glyph);
                    let inner = inset + rail_width;

                    let mut content: Vec<Line> = Vec::new();
                    if let Some(label) = &node.label {
                        let label = if node.role == "tool.call" {
                            self.tool_title(node)
                        } else {
                            label.clone()
                        };
                        content.push(Line::simple(indent, style, label, Some(&node.id)));
                    }
                    content.extend(self.children(node, depth, inner));
                    self.state_mark(node, indent, &mut content);
                    // An empty railed block says nothing, so it earns no padding.
                    if content.is_empty() {
                        return;
                    }

                    // The rail and surface continue through a row above and below.
                    out.push(rail_pad(indent, &glyph, rail_style, surface, node));
                    for mut line in content {
                        rail_line(&mut line, &glyph, rail_style, surface);
                        out.push(line);
                    }
                    out.push(rail_pad(indent, &glyph, rail_style, surface, node));
                    // And the block is separated from what follows by a plain
                    // blank, in addition to the padding that closes its rail.
                    out.push(Line::default());
                    return;
                }
                if node.role == "session" {
                    // The root is a document boundary. The terminal supplies the
                    // application header; rendering the session id here duplicates
                    // that chrome as transcript content.
                    for child in &node.children {
                        self.node(child, depth, inset, out);
                    }
                } else if let Some(label) = &node.label {
                    out.push(Line::simple(
                        indent,
                        style.bold(),
                        label.clone(),
                        Some(&node.id),
                    ));
                    for child in &node.children {
                        self.node(child, depth, inset, out);
                    }
                } else {
                    out.extend(self.children(node, depth, inset));
                }
                self.state_mark(node, indent, out);
            }
            Kind::Text { spans } => {
                if node.role == "tool.call" {
                    out.push(Line::simple(
                        indent,
                        style,
                        self.tool_title(node),
                        Some(&node.id),
                    ));
                    return;
                }
                let budget = self.budget(indent, inset);
                for line in wrap_spans(spans, budget) {
                    out.push(Line {
                        indent,
                        spans: line
                            .iter()
                            .map(|span| (self.span_style(span, style), span.text.clone()))
                            .collect(),
                        surface: self.theme.surface(&node.role),
                        node: Some(node.id.clone()),
                    });
                }
                self.state_mark(node, indent, out);
            }
            Kind::Heading { level, spans } => {
                // A heading is structure, so the linear medium makes it visible even
                // when a theme says nothing: level one is underlined as well as bold.
                // A theme may name the global `markdown.heading.<level>` or the exact
                // `…markdown.heading.<level>` role, and what it says is merged over the
                // heading's own role rather than replacing it.
                let base = self.merged(
                    style,
                    &node.role,
                    &level.to_string(),
                    &format!("markdown.heading.{level}"),
                );
                let budget = self.budget(indent, inset);
                for line in wrap_spans(spans, budget) {
                    out.push(Line {
                        indent,
                        spans: line
                            .iter()
                            .map(|span| (self.span_style(span, base), span.text.clone()))
                            .collect(),
                        surface: self.theme.surface(&node.role),
                        node: Some(node.id.clone()),
                    });
                }
                self.state_mark(node, indent, out);
            }
            Kind::Quote => {
                // The marker is the medium's, not the theme's: a quote has to read as a
                // quote in a theme that named nothing at all. A theme can still colour it
                // through the `quote` modifier or the node's own `…quote.marker` role.
                let marker = self.merged(style, &node.role, "marker", "quote");
                // The marker's two cells are an inset for the children, and the
                // marker itself is a span, so a quote inside a rail lands at the
                // rail's column instead of being pushed right by an indent.
                let inner = inset + width("▏ ");
                let mut rendered = Vec::new();
                for child in &node.children {
                    self.node(child, depth, inner, &mut rendered);
                }
                for mut line in rendered {
                    line.spans.insert(0, (marker, "▏ ".to_string()));
                    out.push(line);
                }
                self.state_mark(node, indent, out);
            }
            Kind::Rule => {
                let width = self.budget(indent, inset).min(60).max(1);
                let rule = self.merged(style, &node.role, "rule", "markdown.rule");
                out.push(Line::simple(
                    indent,
                    rule,
                    "─".repeat(width),
                    Some(&node.id),
                ));
            }
            Kind::Code { lang, text } => {
                // A diff is code whose meaning is *per line*, and the session says so —
                // by naming a role that ends in `.diff`, or, for a body a model fenced,
                // by the language it was fenced with. Nothing here guesses from the
                // text: a block of code that happens to contain a `+` is still code.
                let diff = is_diff(&node.role, lang.as_deref());
                if let Some(lang) = lang
                    && !diff
                {
                    let label = self.merged(style, &node.role, "label", "markdown.code.label");
                    out.push(Line::simple(
                        indent,
                        label,
                        format!("{lang}"),
                        Some(&node.id),
                    ));
                }
                // Highlighting is the client's. The session sent the code and its
                // authored fence label; the grammar, the parse and the classes are
                // ours. A diff classifies its own lines instead.
                let captures = if diff {
                    Vec::new()
                } else {
                    lang.as_deref()
                        .map(|language| misa_syntax::captures(language, text))
                        .unwrap_or_default()
                };
                let source: Vec<&str> = text.split('\n').collect();
                // A prose code block carries a numbered gutter, as the previous
                // markdown renderer did; a tool's own code view keeps its shape.
                let numbers = numbered_code(&node.role).then(|| code_numbers(&source, diff));
                let number_width = numbers
                    .as_ref()
                    .and_then(|numbers| numbers.iter().flatten().map(String::len).max())
                    .unwrap_or(0);
                let budget = self.budget(indent, inset);
                let gutter = number_width > 0 && budget > number_width + 4;
                let gutter_style = self.merged(style, &node.role, "border", "markdown.code.border");
                let body = if gutter {
                    budget - number_width - 2
                } else {
                    budget
                };
                for (index, raw) in source.iter().enumerate() {
                    let value = clip(raw, body);
                    let line_style = if diff {
                        self.diff_style(style, &node.role, raw)
                    } else {
                        style
                    };
                    let mut spans = Vec::new();
                    if gutter {
                        let number = numbers
                            .as_ref()
                            .and_then(|numbers| numbers.get(index))
                            .and_then(Option::as_deref)
                            .unwrap_or("");
                        spans.push((gutter_style, format!("{number:>number_width$}  ")));
                    }
                    spans.extend(self.code_spans(text, index, raw, &value, &captures, line_style));
                    out.push(Line {
                        indent,
                        spans,
                        surface: self.theme.surface(&node.role),
                        node: Some(node.id.clone()),
                    });
                }
                self.state_mark(node, indent, out);
            }
            Kind::List {
                ordered,
                items,
                markers,
            } => {
                for (index, item) in items.iter().enumerate() {
                    // A task item carries its state, so the client draws the ballot
                    // box rather than re-reading a glyph out of the text.
                    let marker = match markers.get(index).copied().flatten() {
                        Some(true) => "☑ ".to_string(),
                        Some(false) => "☐ ".to_string(),
                        None if *ordered => format!("{}. ", index + 1),
                        None => "• ".to_string(),
                    };
                    // The marker is a prefix span, so its cells are the item's
                    // inset. A wrapped continuation aligns under the text by
                    // taking the marker's width as plain indentation instead.
                    let marker_width = width(&marker);
                    let inner = inset + marker_width;
                    let mut rendered = Vec::new();
                    for child in item {
                        self.node(child, depth, inner, &mut rendered);
                    }
                    if rendered.is_empty() {
                        continue;
                    }
                    let marker_style =
                        self.merged(style, &node.role, "marker", "markdown.list.marker");
                    let mut first = rendered.remove(0);
                    first.spans.insert(0, (marker_style, marker));
                    out.push(first);
                    for mut line in rendered {
                        line.indent = line.indent.saturating_add(marker_width as u8).min(u8::MAX);
                        out.push(line);
                    }
                }
                self.state_mark(node, indent, out);
            }
            Kind::Table { head, rows } => {
                self.table(head, rows, node, indent, inset, style, out);
                self.state_mark(node, indent, out);
            }
            Kind::Fields { fields } => {
                let label_width = fields
                    .iter()
                    .map(|field| width(&field.label))
                    .max()
                    .unwrap_or(0)
                    .min(self.budget(indent, inset) / 2);
                for field in fields {
                    let label = pad(&clip(&field.label, label_width), label_width);
                    out.push(Line::simple(
                        indent,
                        style.dim(),
                        format!("{label}  "),
                        Some(&node.id),
                    ));
                    let budget = self.budget(indent + 2, inset);
                    let value = if field.secret {
                        "••••"
                    } else {
                        &field.value
                    };
                    for line in wrap_spans(&[Span::plain(value)], budget) {
                        out.push(Line {
                            indent: indent + 2,
                            spans: line.iter().map(|span| (style, span.text.clone())).collect(),
                            surface: None,
                            node: Some(node.id.clone()),
                        });
                    }
                }
                self.state_mark(node, indent, out);
            }
            Kind::Collapsible { summary } => {
                if node.role == "tool.call" {
                    out.push(Line::simple(
                        indent,
                        style,
                        self.tool_title(node),
                        Some(&node.id),
                    ));
                    for child in &node.children {
                        self.node(child, depth + 1, inset, out);
                    }
                    self.state_mark(node, indent, out);
                    return;
                }
                let budget = self.budget(indent, inset);
                for line in wrap_spans(summary, budget) {
                    out.push(Line {
                        indent,
                        spans: line
                            .iter()
                            .map(|span| (self.span_style(span, style), span.text.clone()))
                            .collect(),
                        surface: self.theme.surface(&node.role),
                        node: Some(node.id.clone()),
                    });
                }
                // Interactive clients resolve their own expansion before rendering.
                for child in &node.children {
                    self.node(child, depth + 1, inset, out);
                }
                self.state_mark(node, indent, out);
            }
            Kind::Image {
                alt,
                width,
                height,
                blob,
            } => {
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
                    misa_render::fact::format(&node.role, value),
                    Some(&node.id),
                ));
            }
            Kind::Status { text } => {
                out.push(Line::simple(indent, style, text.clone(), Some(&node.id)));
            }
            Kind::Meter { label, value, max } => {
                let budget = self.budget(indent, inset);
                let bar_width = 10usize.min(budget.saturating_sub(width(label) + 4).max(1));
                let ratio = if *max > 0.0 {
                    (value / max).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let filled = (ratio * bar_width as f64).round() as usize;
                let bar = format!("[{}{}]", "#".repeat(filled), "-".repeat(bar_width - filled));
                out.push(Line::simple(
                    indent,
                    style,
                    format!("{label} {bar} {value}/{max}"),
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

    fn tool_title(&self, node: &Node) -> String {
        let mut title = format!("◇ {}", node.label.as_deref().unwrap_or("Tool result"));
        let suffix = match node.state {
            Some(State::Pending) => Some("… pending"),
            Some(State::Streaming) => Some("… running"),
            Some(State::Failed) => Some("⊘ failed"),
            Some(State::Cancelled) => Some("⊘ cancelled"),
            _ => None,
        };
        if let Some(suffix) = suffix {
            title.push_str("  ");
            title.push_str(suffix);
        }
        title
    }

    fn span_style(&self, span: &Span, base: Style) -> Style {
        use misa_proto::view::SpanKind;
        match &span.kind {
            SpanKind::Plain => base,
            SpanKind::Strong => base.over(self.theme.role("bold")),
            SpanKind::StrongEmphasis => base
                .over(self.theme.role("bold"))
                .over(self.theme.role("italic")),
            SpanKind::Emphasis => base.over(self.theme.role("italic")),
            SpanKind::Strikethrough => base.over(self.theme.role("strikethrough")),
            SpanKind::Code => base.over(self.theme.role("code")),
            SpanKind::Link { .. } => base.over(self.theme.role("link")),
        }
    }

    /// Split one line of a code block by the captures that cover it.
    fn code_spans(
        &self,
        whole: &str,
        line_index: usize,
        raw: &str,
        clipped: &str,
        captures: &[misa_syntax::Capture],
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
            let capture = captures.iter().find(|capture| {
                capture.start as usize <= absolute && absolute < capture.end as usize
            });
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
            let style = capture
                .map(|capture| self.theme.token(&capture.token))
                .unwrap_or(base);
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
        node: &Node,
        indent: u8,
        inset: usize,
        style: Style,
        out: &mut Vec<Line>,
    ) {
        let role = &node.role;
        let columns = head.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
        if columns == 0 {
            return;
        }
        let text_of = |cells: &[Span]| {
            cells
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>()
        };
        // A table that cannot give every column a border, padding and a wide
        // grapheme is shown as stacked fields, as the previous renderer did.
        let available = self.budget(indent, inset);
        if available < columns.saturating_mul(5) + 1 {
            for (row_index, row) in rows.iter().enumerate() {
                if row_index > 0 {
                    out.push(Line::default());
                }
                for index in 0..columns {
                    let label = head
                        .get(index)
                        .map(|cell| text_of(cell))
                        .unwrap_or_default();
                    let mut spans = vec![Span::plain(format!("{label}: "))];
                    if let Some(cell) = row.get(index) {
                        spans.extend(cell.iter().cloned());
                    }
                    for line in wrap_spans(&spans, available) {
                        out.push(Line {
                            indent,
                            spans: line
                                .iter()
                                .map(|span| (self.span_style(span, style), span.text.clone()))
                                .collect(),
                            surface: None,
                            node: Some(node.id.clone()),
                        });
                    }
                }
            }
            return;
        }
        let mut widths = vec![1usize; columns];
        for row in std::iter::once(head).chain(rows.iter().map(Vec::as_slice)) {
            for (index, cell) in row.iter().enumerate() {
                widths[index] = widths[index].max(width(&text_of(cell)));
            }
        }
        // Allocate the column widths to the available room, as the previous
        // markdown renderer did: at least three cells and at most forty, widened
        // or narrowed to use what the viewport has.
        let budget = self
            .budget(indent, inset)
            .saturating_sub(columns.saturating_sub(1) * 2);
        allocate_columns(&mut widths, budget);
        let header_style = self.merged(style, role, "header", "markdown.table.header");
        let border_style = self.merged(style, role, "border", "markdown.table.border");
        for (row_index, row) in std::iter::once(head)
            .chain(rows.iter().map(Vec::as_slice))
            .enumerate()
        {
            let mut spans = Vec::new();
            for index in 0..columns {
                if index > 0 {
                    spans.push((border_style, "  ".to_string()));
                }
                let cell = row.get(index).map(|cell| text_of(cell)).unwrap_or_default();
                let text = pad(&cell, widths[index]);
                let cell_style = if row_index == 0 { header_style } else { style };
                spans.push((cell_style, text));
            }
            out.push(Line {
                indent,
                spans,
                surface: None,
                node: Some(node.id.clone()),
            });
        }
    }

    /// The style for one line of a diff, by what the line starts with.
    ///
    /// A theme may name the role under the node's own prefix — `…markdown.diff.added` —
    /// and a theme that has not falls back to the generic `diff.added`. That is one place
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

    /// Merge a role's own style with a narrower or global modifier.
    ///
    /// The exact `…<role>.<suffix>` is used when a theme names it; otherwise the
    /// global `<global>` role applies. Either is merged *over* the base rather than
    /// replacing it, which is what lets a theme add emphasis without restating the
    /// colour the node already resolved to.
    fn merged(&self, base: Style, role: &str, suffix: &str, global: &str) -> Style {
        let specific = format!("{role}.{suffix}");
        // The global vocabulary applies first so a level's shared attributes (an
        // italic heading level, say) survive; a role named for this exact node then
        // patches them rather than replacing the whole style.
        let merged = base.over(self.theme.role(global));
        if self.theme.names(&specific) {
            merged.over(self.theme.role(&specific))
        } else {
            merged
        }
    }

    /// Columns available to a node at a given indentation and prefix inset.
    ///
    /// `indent` is genuine leading spaces; `inset` is the cells owned by the
    /// prefix spans of ancestors (rails, quote markers, list markers, gutters),
    /// which are not yet on the line while it is being wrapped.
    fn budget(&self, indent: u8, inset: usize) -> usize {
        self.columns.saturating_sub(indent as usize + inset).max(1)
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
        line if line.starts_with('+') => "added",
        line if line.starts_with('-') => "removed",
        _ => return None,
    })
}

/// Whether a code block is prose markdown (numbered) rather than a tool's own view.
fn numbered_code(role: &str) -> bool {
    role.contains(".markdown.")
}

/// The line number shown in a code block's gutter, one entry per source line.
///
/// A diff numbers from its hunk headers: a removed line names the old file, an
/// added or context line the new one, and the patch's own metadata names neither.
fn code_numbers(lines: &[&str], diff: bool) -> Vec<Option<String>> {
    if !diff {
        return (1..=lines.len())
            .map(|number| Some(number.to_string()))
            .collect();
    }
    let mut numbers = Vec::with_capacity(lines.len());
    let (mut old, mut new) = (1i64, 1i64);
    for raw in lines {
        match diff_suffix(raw) {
            Some("hunk") => {
                if let Some((old_start, new_start)) = hunk_start(raw) {
                    old = old_start;
                    new = new_start;
                }
                numbers.push(None);
            }
            Some("header") | Some("meta") => numbers.push(None),
            _ => match raw.chars().next() {
                Some('-') => {
                    numbers.push(Some(old.to_string()));
                    old += 1;
                }
                Some('+') => {
                    numbers.push(Some(new.to_string()));
                    new += 1;
                }
                _ => {
                    numbers.push(Some(new.to_string()));
                    old += 1;
                    new += 1;
                }
            },
        }
    }
    numbers
}

/// The starting line numbers of a unified-diff hunk header: `@@ -a,b +c,d @@`.
fn hunk_start(raw: &str) -> Option<(i64, i64)> {
    let rest = raw.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(' ')?;
    let (new, _) = rest.strip_prefix('+')?.split_once(' ')?;
    let old = old.split(',').next()?.parse().ok()?;
    let new = new.split(',').next()?.parse().ok()?;
    Some((old, new))
}

/// Allocate table column widths within `available` cells.
fn allocate_columns(widths: &mut [usize], available: usize) {
    let count = widths.len();
    if count == 0 {
        return;
    }
    for width in widths.iter_mut() {
        *width = (*width).clamp(3, 40);
    }
    let room = available.max(2 * count);
    let mut total: usize = widths.iter().sum();
    while total > room {
        let Some((index, _)) = widths.iter().enumerate().max_by_key(|(_, width)| **width) else {
            break;
        };
        if widths[index] <= 2 {
            break;
        }
        widths[index] -= 1;
        total -= 1;
    }
    loop {
        if total >= room {
            break;
        }
        let mut grew = false;
        for width in widths.iter_mut() {
            if total < room && *width < 40 {
                *width += 1;
                total += 1;
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
}

/// A padding row inside a railed block: the rail and the surface continue with
/// no content of their own, so the block reads as one extent.
fn rail_pad(
    indent: u8,
    glyph: &str,
    rail_style: Style,
    surface: Option<Style>,
    node: &Node,
) -> Line {
    Line {
        indent,
        spans: vec![(rail_style, glyph.to_string())],
        surface,
        node: Some(node.id.clone()),
    }
}

/// Put a block's rail at the front of one of its lines.
///
/// A nested block that already begins with the same glyph keeps its own rail:
/// the parent's must not stack on top of a tool call or an opened thinking
/// block. A blank line that already carries the block's surface is an interior
/// gap and stays inside the rail; a bare blank stays outside it.
fn rail_line(line: &mut Line, glyph: &str, rail_style: Style, surface: Option<Style>) {
    let railed = line.spans.first().is_some_and(|(_, text)| text == glyph);
    if line.is_blank() {
        if line.surface.is_some() && !railed {
            line.spans.insert(0, (rail_style, glyph.to_string()));
        }
    } else if !railed {
        line.spans.insert(0, (rail_style, glyph.to_string()));
    }
    line.surface = line.surface.or(surface);
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
    use misa_proto::view::{Action, ActionOn, Field, FieldKind, State};
    use misa_render::Color;

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
        let body = lines
            .iter()
            .find(|line| line.text().contains("hello"))
            .expect("body line");
        assert!(
            body.text().starts_with("┃ "),
            "a railed message did not draw its rail: {:?}",
            body.text()
        );
    }

    #[test]
    fn a_blank_between_paragraphs_stays_inside_the_messages_rail() {
        let node = Node::section("message.assistant").id("m1").children([
            Node::text("message.assistant.markdown.paragraph", [Span::plain("one")]),
            Node::text("message.assistant.markdown.paragraph", [Span::plain("two")]),
        ]);
        let lines = render(&node, &Theme::dark(), 80);
        let rows: Vec<String> = lines.iter().map(Line::text).collect();
        // The rail opens the block, the paragraphs sit behind it, and the gap
        // between them is a railed row too; the row that closes the rail is the
        // block's bottom padding.
        assert_eq!(rows, vec!["┃ ", "┃ one", "┃ ", "┃ two", "┃ "]);
        // The gap carries the message surface, so the block is visually continuous.
        assert!(lines[2].surface.is_some());
    }

    #[test]
    fn a_finished_node_says_nothing_and_an_unfinished_one_says_so() {
        let done = Node::section("tool.result").state(State::Done);
        assert!(render(&done, &Theme::dark(), 80).is_empty());
        let failed = Node::section("tool.result").state(State::Failed);
        let lines = render(&failed, &Theme::dark(), 80);
        // A state mark is content, so the block earns a rail and its padding.
        assert_eq!(lines.len(), 3);
        assert!(lines[1].text().contains("failed"));
    }

    #[test]
    fn a_code_block_is_clipped_and_the_client_highlights_it() {
        let node = Node::new(
            "tool.call",
            Kind::Code {
                lang: Some("rust".into()),
                text: "let x = 1;".into(),
            },
        );
        let lines = render(&node, &Theme::dark(), 6);
        assert_eq!(lines[0].text(), "rust");
        let body = &lines[1];
        assert!(width(&body.text()) <= 6);
        // The client parsed the block and coloured `let` as a keyword.
        assert_eq!(body.spans[0].0, Theme::dark().token("keyword"));
    }

    #[test]
    fn a_narrow_table_shrinks_instead_of_overflowing() {
        let head = vec![
            vec![Span::plain("first column")],
            vec![Span::plain("second column")],
        ];
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
    fn a_table_too_narrow_to_draw_stacks_its_fields() {
        let head = vec![vec![Span::plain("first")], vec![Span::plain("second")]];
        let rows = vec![vec![vec![Span::plain("one")], vec![Span::plain("two")]]];
        let node = Node::new("table", Kind::Table { head, rows }).id("t");
        let lines = render(&node, &theme(), 10);
        assert!(
            lines[0].text().starts_with("first: one"),
            "{:?}",
            lines[0].text()
        );
        assert!(lines.iter().any(|line| line.text().starts_with("second:")));
        assert!(lines.iter().any(|line| line.text().trim() == "two"));
    }

    #[test]
    fn a_list_gets_markers_and_indents_its_children() {
        let item = vec![Node::text(
            "message.assistant",
            [Span::plain("first item that wraps")],
        )];
        let node = Node::new(
            "list",
            Kind::List {
                ordered: true,
                items: vec![item, vec![Node::text("x", [Span::plain("second")])]],
                markers: Vec::new(),
            },
        );
        let lines = render(&node, &theme(), 10);
        assert!(lines[0].text().starts_with("1. "));
        assert!(lines.iter().any(|line| line.text().contains("2. second")));
    }

    #[test]
    fn a_task_item_draws_the_ballot_box_state() {
        let item = |text: &str| vec![Node::text("message.assistant", [Span::plain(text)])];
        let node = Node::new(
            "message.assistant.markdown.list",
            Kind::List {
                ordered: false,
                items: vec![item("todo"), item("done")],
                markers: vec![Some(false), Some(true)],
            },
        );
        let lines = render(&node, &Theme::dark(), 40);
        assert_eq!(lines[0].text(), "☐ todo");
        assert_eq!(lines[1].text(), "☑ done");
    }

    #[test]
    fn a_strong_emphasis_span_is_both_bold_and_italic() {
        let node = Node::new(
            "message.assistant",
            Kind::Text {
                spans: vec![Span {
                    text: "x".into(),
                    kind: misa_proto::view::SpanKind::StrongEmphasis,
                }],
            },
        );
        let line = &render(&node, &Theme::dark(), 40)[0];
        assert!(line.spans[0].0.bold);
        assert!(line.spans[0].0.italic);
    }

    #[test]
    fn fields_align_their_labels() {
        let node = Node::new(
            "dialog",
            Kind::Fields {
                fields: vec![
                    Field {
                        id: "a".into(),
                        label: "Model".into(),
                        value: "claude".into(),
                        hint: None,
                        read_only: false,
                        secret: false,
                        kind: FieldKind::Inline,
                    },
                    Field {
                        id: "b".into(),
                        label: "Reasoning effort".into(),
                        value: "high".into(),
                        hint: None,
                        read_only: false,
                        secret: false,
                        kind: FieldKind::Choice {
                            options: vec![],
                            selected: None,
                        },
                    },
                ],
            },
        );
        let lines = render(&node, &theme(), 60);
        assert!(lines[0].text().starts_with("Model"));
        assert!(lines[2].text().starts_with("Reasoning effort"));
        assert_eq!(lines[0].indent, lines[2].indent);
    }

    #[test]
    fn a_read_only_block_preserves_lines_and_secret_policy_masks_any_shape() {
        let mut field = Field {
            id: "body".into(),
            label: "Report".into(),
            value: "one\ntwo".into(),
            hint: None,
            kind: FieldKind::Block,
            read_only: true,
            secret: false,
        };
        let draw = |field: Field| {
            render(
                &Node::new(
                    "report",
                    Kind::Fields {
                        fields: vec![field],
                    },
                ),
                &theme(),
                80,
            )
            .iter()
            .map(Line::text)
            .collect::<Vec<_>>()
            .join("\n")
        };
        let text = draw(field.clone());
        assert_eq!(
            text.lines().skip(1).map(str::trim).collect::<Vec<_>>(),
            vec!["one", "two"]
        );
        field.secret = true;
        let text = draw(field);
        assert!(!text.contains("one"), "{text}");
        assert!(text.contains("••••"), "{text}");
    }

    #[test]
    fn a_meter_shows_a_bounded_bar() {
        let node = Node::new(
            "value.meter",
            Kind::Meter {
                label: "budget".into(),
                value: 2.0,
                max: 4.0,
            },
        );
        let line = &render(&node, &theme(), 80)[0];
        assert!(line.text().contains("[#####-----]"), "{}", line.text());
        assert!(line.text().ends_with("2/4"));
    }

    #[test]
    fn an_image_degrades_to_its_alt_text() {
        let node = Node::new(
            "screenshot",
            Kind::Image {
                blob: misa_proto::view::BlobRef {
                    hash: "a".repeat(64),
                    len: 9,
                    media: None,
                },
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
            Kind::Collapsible {
                summary: vec![Span::plain("thinking…")],
            },
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
        let node = Node::new(
            "status",
            Kind::Status {
                text: "ready".into(),
            },
        );
        let text = to_plain(&render(&node, &theme(), 40));
        assert_eq!(text, "ready\n");
    }

    #[test]
    fn every_line_points_at_the_node_it_came_from() {
        let node = Node::section("message.user")
            .id("m1")
            .child(Node::text("message.user", [Span::plain("hello")]).id("m1.text"));
        let lines = render(&node, &theme(), 40);
        assert!(
            lines
                .iter()
                .all(|line| line.node.as_deref() == Some("m1.text"))
        );
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
            },
        );
        let theme = Theme::dark();
        let lines = render(&node, &theme, 40);
        // No `diff` banner: the body already says what it is, line by line.
        assert_eq!(lines.len(), 4, "{:?}", to_plain(&lines));
        // A numbered gutter precedes each line; the diff style is the run after it.
        assert_eq!(lines[0].spans[0].0, theme.role("markdown.code.border"));
        assert_eq!(lines[0].spans[1].0, theme.role("diff.hunk"));
        assert_eq!(lines[1].spans[1].0, theme.role("diff.removed"));
        assert_eq!(lines[2].spans[1].0, theme.role("diff.added"));
        // Context keeps the node's own style, so a diff still reads like a tool result.
        assert_eq!(
            lines[3].spans[1].0,
            theme.role("message.assistant.markdown.diff")
        );
        // The gutter numbers the old file for removals and the new one otherwise,
        // and says nothing for the hunk header.
        assert_eq!(
            to_plain(&lines),
            "   @@ -1 +1 @@\n1  -old line\n1  +new line\n2   context\n"
        );
    }

    #[test]
    fn a_markdown_code_block_numbers_its_lines_and_a_tool_view_does_not() {
        let code = |role: &str| {
            Node::new(
                role,
                Kind::Code {
                    lang: None,
                    text: "one\ntwo".into(),
                },
            )
        };
        let lines = render(&code("message.assistant.markdown.code"), &Theme::dark(), 20);
        assert_eq!(lines[0].text(), "1  one");
        assert_eq!(lines[1].text(), "2  two");
        let lines = render(&code("tool.result"), &Theme::dark(), 20);
        assert_eq!(lines[0].text(), "one");
        assert_eq!(lines[1].text(), "two");
    }

    #[test]
    fn a_theme_may_name_a_diff_line_under_the_node_it_belongs_to() {
        let node = Node::new(
            "tool.result.diff",
            Kind::Code {
                lang: None,
                text: "+added".into(),
            },
        );
        // A theme that says something narrower wins over the generic role.
        let theme = Theme::dark().with_role("tool.result.diff.added", Style::rgb(1, 2, 3).bold());
        let line = &render(&node, &theme, 40)[0];
        assert_eq!(line.spans[0].0.fg, Color::Rgb(1, 2, 3));
        assert!(line.spans[0].0.bold);
        // And the generic role is used when it has not.
        let line = &render(&node, &Theme::dark(), 40)[0];
        assert_eq!(line.spans[0].0, Theme::dark().role("diff.added"));
    }

    #[test]
    fn code_that_merely_looks_like_a_diff_is_still_code() {
        let node = Node::new(
            "message.assistant.markdown.code",
            Kind::Code {
                lang: Some("rust".into()),
                text: "+ 1".into(),
            },
        );
        let theme = Theme::dark();
        let lines = render(&node, &theme, 40);
        assert_eq!(
            lines[0].text(),
            "rust",
            "the language banner is gone from a code block"
        );
        assert_eq!(
            lines[1].spans[0].0,
            theme.role("markdown.code.border"),
            "a markdown code block carries a numbered gutter"
        );
        // A leading `+` is code, not a diff line: no span takes the diff role.
        assert!(
            lines[1]
                .spans
                .iter()
                .all(|(style, _)| *style != theme.role("diff.added")),
            "a leading plus turned code into a diff: {:?}",
            lines[1].spans
        );
    }

    #[test]
    fn a_heading_is_bold_and_a_quote_carries_a_marker() {
        let heading = Node::new(
            "message.assistant.markdown.heading",
            Kind::Heading {
                level: 2,
                spans: vec![Span::plain("Title")],
            },
        );
        let lines = render(&heading, &Theme::dark(), 40);
        assert_eq!(lines[0].text(), "Title");
        assert!(
            lines[0].spans[0].0.bold,
            "a heading is not set apart at all"
        );

        let quote = Node::new("message.assistant.markdown.quote", Kind::Quote).child(Node::text(
            "message.assistant.markdown.paragraph",
            [Span::plain("quoted")],
        ));
        let lines = render(&quote, &Theme::dark(), 40);
        assert!(lines[0].text().contains("quoted"));
        assert!(
            lines[0].text().ends_with("▏ quoted"),
            "a quote has no marker: {:?}",
            lines[0].text()
        );
    }

    #[test]
    fn a_heading_level_uses_the_global_markdown_vocabulary() {
        // The parser emits one prefixed role for every level; the level is expressed
        // through the global `markdown.heading.<n>` roles, so a theme names them once.
        let theme = Theme::dark();
        let heading = |level| {
            Node::new(
                "message.assistant.markdown.heading",
                Kind::Heading {
                    level,
                    spans: vec![Span::plain("Title")],
                },
            )
        };
        assert!(render(&heading(1), &theme, 40)[0].spans[0].0.bold);
        assert!(render(&heading(1), &theme, 40)[0].spans[0].0.underline);
        assert!(render(&heading(3), &theme, 40)[0].spans[0].0.italic);
        assert!(render(&heading(5), &theme, 40)[0].spans[0].0.underline);
        assert!(render(&heading(6), &theme, 40)[0].spans[0].0.dim);
        // A theme that names the fully qualified role overrides just that place, and
        // its attributes merge over the heading's own colour rather than replacing it.
        let specific =
            Theme::dark().with_role("message.assistant.markdown.heading.4", Style::rgb(9, 8, 7));
        let line = &render(&heading(4), &specific, 40)[0];
        assert_eq!(line.spans[0].0.fg, Color::Rgb(9, 8, 7));
        assert!(line.spans[0].0.italic, "the level's own italic is retained");
    }

    #[test]
    fn a_theme_may_name_the_namespaced_syntax_spelling() {
        let theme = Theme::dark();
        assert_eq!(theme.token("keyword"), theme.token("syntax.keyword"));
        assert_ne!(theme.token("syntax.keyword"), Style::PLAIN);
        // Both spellings of an unknown capture remain plain.
        assert_eq!(theme.token("syntax.unknown"), Style::PLAIN);
    }

    #[test]
    fn a_rule_is_a_line_of_its_own_and_a_list_item_is_not_one() {
        let node = Node::new("message.assistant.markdown.rule", Kind::Rule);
        let lines = render(&node, &theme(), 40);
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].text().chars().all(|ch| ch == '─'),
            "{:?}",
            lines[0].text()
        );
    }

    #[test]
    fn a_quote_in_a_message_lands_under_the_rail_not_behind_an_indent() {
        let node = Node::section("message.user").id("m1").child(
            Node::new("message.user.markdown.quote", Kind::Quote).child(Node::text(
                "message.user.markdown.paragraph",
                [Span::plain("text")],
            )),
        );
        let lines = render(&node, &Theme::dark(), 80);
        let body = lines
            .iter()
            .find(|line| line.text().contains("text"))
            .expect("the quoted body");
        assert_eq!(body.text(), "┃ ▏ text");
    }

    #[test]
    fn a_railed_message_pads_above_and_below_and_ends_with_a_blank() {
        let node = Node::section("message.user").id("m1").child(
            Node::text("message.user.markdown.paragraph", [Span::plain("hello")]).id("m1.body"),
        );
        let lines = render_block(&node, &Theme::dark(), 80, 0);
        let rows: Vec<String> = lines.iter().map(Line::text).collect();
        // One padding row above the text, the text, one below, then the blank
        // that separates this block from whatever follows it.
        assert_eq!(rows, vec!["┃ ", "┃ hello", "┃ ", ""]);
        let surface = Theme::dark().surface("message.user");
        for pad in [&lines[0], &lines[2]] {
            assert_eq!(pad.text(), "┃ ");
            assert_eq!(pad.surface, surface);
            assert!(pad.spans.iter().all(|(_, text)| text == "┃ "));
        }
        assert!(lines[3].is_blank());
        assert!(lines[3].surface.is_none());
    }

    #[test]
    fn nested_quotes_compose_their_markers_in_order() {
        let node = Node::section("message.user").id("m1").child(
            Node::new("message.user.markdown.quote", Kind::Quote)
                .id("q1")
                .child(
                    Node::new("message.user.markdown.quote", Kind::Quote)
                        .id("q2")
                        .child(Node::text(
                            "message.user.markdown.paragraph",
                            [Span::plain("deep")],
                        )),
                ),
        );
        let lines = render(&node, &Theme::dark(), 80);
        let body = lines
            .iter()
            .find(|line| line.text().contains("deep"))
            .expect("the doubly quoted body");
        assert_eq!(body.text(), "┃ ▏ ▏ deep");
    }
}
