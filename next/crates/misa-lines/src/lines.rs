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

use misa_proto::view::{Alignment, Kind, Node, Span, State};
use misa_value::Value;

use misa_render::Color;
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

/// A block's leading cells: the rail, quote marker, or list marker that sits
/// before every row of the block's content.
///
/// The first row and the remaining rows can differ — a list writes its marker
/// once and aligns wrapped text under the marker's text — which is why a prefix
/// is a pair rather than a single string. Prefix cells live in `Line::spans`,
/// never in `Line::indent`, so a parent prefix always lands to the left of a
/// child's and nesting composes in ancestor order.
#[derive(Clone, Debug, Default, PartialEq)]
struct Prefix {
    first: Vec<(Style, String)>,
    rest: Vec<(Style, String)>,
}

impl Prefix {
    fn none() -> Prefix {
        Prefix::default()
    }

    fn uniform(cells: Vec<(Style, String)>) -> Prefix {
        Prefix {
            rest: cells.clone(),
            first: cells,
        }
    }

    fn first_rest(first: Vec<(Style, String)>, rest: Vec<(Style, String)>) -> Prefix {
        Prefix { first, rest }
    }

    /// The cells before a row. The first and rest variants have equal width, so
    /// wrapped content stays aligned under the opening row.
    fn cells(&self, first: bool) -> &[(Style, String)] {
        if first { &self.first } else { &self.rest }
    }

    fn width(&self) -> usize {
        cells_width(&self.rest).max(cells_width(&self.first))
    }
}

fn cells_width(cells: &[(Style, String)]) -> usize {
    cells.iter().map(|(_, text)| width(text)).sum()
}

/// Blank cells a block adds inside its own extent.
///
/// Vertical padding is blank rows above and below the content that carry the
/// block's prefix and surface, so a railed block reads as one extent with air
/// around its text. Horizontal padding is the cell between the prefix and the
/// content (left) and the cell reserved before the block's right edge (right).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Padding {
    top: u8,
    right: u8,
    bottom: u8,
    left: u8,
}

impl Padding {
    const NONE: Padding = Padding {
        top: 0,
        right: 0,
        bottom: 0,
        left: 0,
    };
    /// A railed message: a blank row above and below, a cell between the rail
    /// and the text, and a cell reserved before the terminal edge. The rail
    /// glyph's own trailing gap is folded into `left`, so text never touches
    /// the bar and the block never touches the edge.
    const MESSAGE: Padding = Padding {
        top: 1,
        right: 1,
        bottom: 1,
        left: 1,
    };
}

/// Cells a block leaves on each side, outside its prefix and surface.
///
/// A nested block uses a margin to read as subordinate to the block that owns
/// it: its content is indented from the parent's prefix and kept off the edge.
/// A block whose own prefix already provides its left breathing room (a quote's
/// marker, a list's marker) uses [`Margin::MARKED`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Margin {
    left: u8,
    right: u8,
}

impl Margin {
    const NONE: Margin = Margin { left: 0, right: 0 };
    /// A nested code or table: one cell on each side from the parent content.
    const NESTED: Margin = Margin { left: 1, right: 1 };
    /// A block whose own marker (a quote's bar, a list's bullet) already gives
    /// it a left edge still keeps a cell before the right edge.
    const MARKED: Margin = Margin { left: 0, right: 1 };
}

/// A block's prefix plus its padding and margin.
///
/// `horizontal` is the room the chrome takes from a row, so the block can lay
/// its content out against the columns that remain and the caller can reserve
/// exactly those cells before composing the prefix in.
#[derive(Clone, Debug, Default)]
struct Chrome {
    prefix: Prefix,
    padding: Padding,
    margin: Margin,
    /// A railed block does not stack its rail on a nested block that already
    /// draws the same rail; the inner block's own rail wins, as the previous
    /// system's tool calls and thinking blocks require.
    rail: bool,
}

impl Chrome {
    fn nested(prefix: Prefix) -> Chrome {
        Chrome {
            prefix,
            margin: Margin::NESTED,
            ..Chrome::default()
        }
    }

    /// Cells reserved before a row's content: the margins, the prefix, and the
    /// horizontal padding.
    fn horizontal(&self) -> usize {
        self.margin.left as usize
            + self.prefix.width()
            + self.padding.left as usize
            + self.padding.right as usize
            + self.margin.right as usize
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
    Renderer { theme, columns }.node(node, depth, 0)
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

/// The explicit shape of one code block.
///
/// A markdown fence and a tool result are the same medium problem — a quiet
/// label, a gutter with a thin rail, and a body — so they are this component
/// with different options rather than two renderers that drift apart. A diff is
/// the same component again: its body says what each line is, so the component
/// asks the line instead of the caller.
struct CodeBlock<'a> {
    /// The node every row points at, and whose state mark closes the block.
    node: &'a Node,
    /// The authored fence label, drawn quiet on a thin rail. `None` draws no
    /// label; a diff never draws one, because its body already says what it is.
    language: Option<&'a str>,
    /// The body, one source line per row.
    text: &'a str,
    /// Syntax captures over the body, only for a non-diff block.
    captures: &'a [misa_syntax::Capture],
    /// The style every line starts from.
    base: Style,
    /// The role the block was resolved for, so a diff line may name its style
    /// under it.
    role: &'a str,
    /// A diff parses its hunk headers and numbers the old and new sides; a
    /// non-diff block numbers in sequence, or not at all.
    diff: bool,
    /// Whether a non-diff block shows a sequential gutter.
    numbered: bool,
    /// Cells the parent leaves on each side of the block.
    margin: Margin,
    /// The surface behind the block's rows. A diff replaces it per line.
    surface: Option<Style>,
}

/// A styled table cell and the width its runs occupy, plus the style its own
/// padding carries so a cell's surface stays continuous.
struct Cell {
    spans: Vec<(Style, String)>,
    width: usize,
    base: Style,
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

    /// Render a node's children as one run of blocks, with the gap a message
    /// puts between its paragraphs.
    fn children(&self, node: &Node, depth: usize, inset: usize) -> Vec<Line> {
        let spaced = Self::spaces_between_children(&node.role);
        let mut out = Vec::new();
        for child in &node.children {
            let lines = self.node(child, depth, inset);
            if lines.is_empty() {
                continue;
            }
            if spaced && !out.is_empty() {
                // A gap between two blocks of one message belongs to that
                // message: it carries the message's surface so a railed block
                // does not come apart into fragments. Its cells stay an inset,
                // not indentation, so the parent prefix still goes in front.
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

    /// Render one node into a block of rows that already carry its own prefix,
    /// padding, and margin.
    fn node(&self, node: &Node, depth: usize, inset: usize) -> Vec<Line> {
        if let Some(lines) = crate::components::render_default(node, self.theme, self.columns) {
            return lines;
        }
        let indent = (depth as u8).saturating_mul(2);
        let style = self.theme.role(&node.role);
        match &node.kind {
            Kind::Section => {
                // A node whose role names a rail is drawn with one. The rail is
                // a prefix span and its cells are part of the block's chrome,
                // which is what lets a quote, a list marker or a nested rail sit
                // after it and compose in the order the ancestors add them.
                if let Some((glyph, rail_style)) = self.theme.rail(&node.role) {
                    return self.railed(node, depth, inset, indent, style, &glyph, rail_style);
                }
                let mut content = Vec::new();
                if node.role == "session" {
                    // The root is a document boundary. The terminal supplies the
                    // application header; rendering the session id here duplicates
                    // that chrome as transcript content.
                    for child in &node.children {
                        content.extend(self.node(child, depth, inset));
                    }
                } else if let Some(label) = &node.label {
                    content.push(Line::simple(
                        indent,
                        style.bold(),
                        label.clone(),
                        Some(&node.id),
                    ));
                    for child in &node.children {
                        content.extend(self.node(child, depth, inset));
                    }
                } else {
                    content = self.children(node, depth, inset);
                }
                self.state_mark(node, indent, &mut content);
                content
            }
            Kind::Text { spans } => {
                if node.role == "tool.call" {
                    return vec![Line::simple(
                        indent,
                        style,
                        self.tool_title(node),
                        Some(&node.id),
                    )];
                }
                let budget = self.budget(indent, inset);
                let mut content = Vec::new();
                for line in wrap_spans(spans, budget) {
                    content.push(Line {
                        indent,
                        spans: line
                            .iter()
                            .map(|span| (self.span_style(span, style), span.text.clone()))
                            .collect(),
                        surface: self.theme.surface(&node.role),
                        node: Some(node.id.clone()),
                    });
                }
                self.state_mark(node, indent, &mut content);
                content
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
                let mut content = Vec::new();
                for line in wrap_spans(spans, budget) {
                    content.push(Line {
                        indent,
                        spans: line
                            .iter()
                            .map(|span| (self.span_style(span, base), span.text.clone()))
                            .collect(),
                        surface: self.theme.surface(&node.role),
                        node: Some(node.id.clone()),
                    });
                }
                self.state_mark(node, indent, &mut content);
                content
            }
            Kind::Quote => {
                // The marker is the medium's, not the theme's: a quote has to read as a
                // quote in a theme that named nothing at all. A theme can still colour it
                // through the `quote` modifier or the node's own `…quote.marker` role.
                // A GitHub alert is still this shape, so its role names the kind and the
                // theme's global `markdown.alert.<kind>` is folded in the same way a
                // heading level's global is.
                let global = misa_render::alert_role(&node.role).unwrap_or("quote");
                let marker = self.merged(style, &node.role, "marker", global);
                let chrome = Chrome {
                    prefix: Prefix::uniform(vec![(marker, "▏ ".to_string())]),
                    padding: Padding::NONE,
                    margin: Margin::MARKED,
                    rail: false,
                };
                let inner = inset + chrome.horizontal();
                let mut content = Vec::new();
                for child in &node.children {
                    content.extend(self.node(child, depth, inner));
                }
                self.state_mark(node, indent, &mut content);
                self.frame(&chrome, indent, None, &node.id, content)
            }
            Kind::Rule => {
                let width = self.budget(indent, inset).max(1);
                let rule = self.merged(style, &node.role, "rule", "markdown.rule");
                vec![Line::simple(
                    indent,
                    rule,
                    "─".repeat(width),
                    Some(&node.id),
                )]
            }
            Kind::Code { lang, text } => {
                // A diff is code whose meaning is *per line*, and the session says so —
                // by naming a role that ends in `.diff`, or, for a body a model fenced,
                // by the language it was fenced with. Nothing here guesses from the
                // text: a block of code that happens to contain a `+` is still code.
                let diff = is_diff(&node.role, lang.as_deref());
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
                // A prose code block carries a numbered gutter, as the previous
                // markdown renderer did; a tool's own code view keeps its shape.
                // A diff overrides both and numbers its old and new sides.
                self.code_block(
                    CodeBlock {
                        node,
                        language: lang.as_deref(),
                        text,
                        captures: &captures,
                        base: style,
                        role: &node.role,
                        diff,
                        numbered: numbered_code(&node.role),
                        margin: Margin::NESTED,
                        surface: self.theme.surface(&node.role),
                    },
                    indent,
                    inset,
                )
            }
            Kind::List {
                ordered,
                items,
                markers,
            } => {
                let mut out = Vec::new();
                for (index, item) in items.iter().enumerate() {
                    // A task item carries its state, so the client draws the ballot
                    // box rather than re-reading a glyph out of the text.
                    let marker = match markers.get(index).copied().flatten() {
                        Some(true) => "☑ ".to_string(),
                        Some(false) => "☐ ".to_string(),
                        None if *ordered => format!("{}. ", index + 1),
                        None => "• ".to_string(),
                    };
                    let marker_style =
                        self.merged(style, &node.role, "marker", "markdown.list.marker");
                    // The marker is a prefix for the whole item: its cells are the
                    // item's inset, the first row carries the marker, and wrapped
                    // rows carry spaces wide enough to align under its text.
                    let marker_width = width(&marker);
                    let prefix = Prefix::first_rest(
                        vec![(marker_style, marker)],
                        vec![(marker_style, " ".repeat(marker_width))],
                    );
                    let chrome = Chrome {
                        prefix,
                        padding: Padding::NONE,
                        margin: Margin::MARKED,
                        rail: false,
                    };
                    let inner = inset + chrome.horizontal();
                    let mut content = Vec::new();
                    for child in item {
                        content.extend(self.node(child, depth, inner));
                    }
                    if content.is_empty() {
                        continue;
                    }
                    out.extend(self.frame(&chrome, indent, None, &node.id, content));
                }
                self.state_mark(node, indent, &mut out);
                out
            }
            Kind::Definition { entries } => {
                // A definition list is a term on its own row, then its body behind
                // a quiet marker and a nested margin. The term resolves through the
                // global vocabulary so a theme names it once; the marker has its own
                // role so it can stay quiet while the term stays prominent.
                let term_style = self.merged(style, &node.role, "term", "markdown.definition.term");
                let marker_style =
                    self.merged(style, &node.role, "marker", "markdown.definition.marker");
                let chrome = Chrome::nested(Prefix::none());
                let inner = inset + chrome.horizontal();
                let mut content = Vec::new();
                for entry in entries {
                    for line in wrap_spans(&entry.term, self.budget(indent, inner)) {
                        content.push(Line {
                            indent,
                            spans: line
                                .iter()
                                .map(|span| (self.span_style(span, term_style), span.text.clone()))
                                .collect(),
                            surface: self.theme.surface(&node.role),
                            node: Some(node.id.clone()),
                        });
                    }
                    let prefix = Prefix::first_rest(
                        vec![(marker_style, "• ".to_string())],
                        vec![(marker_style, "  ".to_string())],
                    );
                    let definition_chrome = Chrome {
                        prefix,
                        padding: Padding::NONE,
                        margin: Margin::MARKED,
                        rail: false,
                    };
                    let definition_inner = inner + definition_chrome.horizontal();
                    for definition in &entry.definitions {
                        let mut lines = Vec::new();
                        for line in wrap_spans(definition, self.budget(indent, definition_inner)) {
                            lines.push(Line {
                                indent,
                                spans: line
                                    .iter()
                                    .map(|span| (self.span_style(span, style), span.text.clone()))
                                    .collect(),
                                surface: self.theme.surface(&node.role),
                                node: Some(node.id.clone()),
                            });
                        }
                        content.extend(self.frame(
                            &definition_chrome,
                            indent,
                            None,
                            &node.id,
                            lines,
                        ));
                    }
                }
                self.state_mark(node, indent, &mut content);
                self.frame(&chrome, indent, None, &node.id, content)
            }
            Kind::Table { head, rows, align } => {
                let chrome = Chrome::nested(Prefix::none());
                let inner = inset + chrome.horizontal();
                let mut content = self.table(head, rows, align, node, indent, inner, style);
                self.state_mark(node, indent, &mut content);
                self.frame(&chrome, indent, None, &node.id, content)
            }
            Kind::Fields { fields } => {
                let label_width = fields
                    .iter()
                    .map(|field| width(&field.label))
                    .max()
                    .unwrap_or(0)
                    .min(self.budget(indent, inset) / 2);
                let mut content = Vec::new();
                for field in fields {
                    let label = pad(&clip(&field.label, label_width), label_width);
                    content.push(Line::simple(
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
                        content.push(Line {
                            indent: indent + 2,
                            spans: line.iter().map(|span| (style, span.text.clone())).collect(),
                            surface: None,
                            node: Some(node.id.clone()),
                        });
                    }
                }
                self.state_mark(node, indent, &mut content);
                content
            }
            Kind::Collapsible { summary } => {
                let mut content = Vec::new();
                if node.role == "tool.call" {
                    content.push(Line::simple(
                        indent,
                        style,
                        self.tool_title(node),
                        Some(&node.id),
                    ));
                    for child in &node.children {
                        content.extend(self.node(child, depth + 1, inset));
                    }
                    self.state_mark(node, indent, &mut content);
                    return content;
                }
                // A terminal always expands a disclosure: the body follows the
                // summary, with no collapse marker of its own. The summary keeps
                // its own role so a theme can make it read as a heading.
                let summary_style =
                    self.merged(style, &node.role, "summary", "markdown.details.summary");
                let budget = self.budget(indent, inset);
                for line in wrap_spans(summary, budget) {
                    content.push(Line {
                        indent,
                        spans: line
                            .iter()
                            .map(|span| (self.span_style(span, summary_style), span.text.clone()))
                            .collect(),
                        surface: self.theme.surface(&node.role),
                        node: Some(node.id.clone()),
                    });
                }
                // Interactive clients resolve their own expansion before rendering.
                for child in &node.children {
                    content.extend(self.node(child, depth + 1, inset));
                }
                self.state_mark(node, indent, &mut content);
                content
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
                vec![Line::simple(indent, style.dim(), label, Some(&node.id))]
            }
            Kind::Fact { value } => {
                // The one place a number becomes words, and it is the client that
                // does it: the session said what the number *is* by naming the role.
                vec![Line::simple(
                    indent,
                    style,
                    misa_render::fact::format(&node.role, value),
                    Some(&node.id),
                )]
            }
            Kind::Status { text } => {
                vec![Line::simple(indent, style, text.clone(), Some(&node.id))]
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
                vec![Line::simple(
                    indent,
                    style,
                    format!("{label} {bar} {value}/{max}"),
                    Some(&node.id),
                )]
            }
        }
    }

    /// Render a railed section: its label and children behind the rail, with a
    /// blank row above and below that carries the rail and the surface.
    fn railed(
        &self,
        node: &Node,
        depth: usize,
        inset: usize,
        indent: u8,
        style: Style,
        glyph: &str,
        rail_style: Style,
    ) -> Vec<Line> {
        let surface = self.theme.surface(&node.role);
        // The theme's rail glyph carries the gap between the bar and the text as
        // a trailing space. That gap is the block's left padding, not part of the
        // glyph, so the chrome is one data-driven `Padding` with all four sides.
        let (bar, gap) = split_rail(glyph);
        let chrome = Chrome {
            prefix: Prefix::uniform(vec![(rail_style, bar.to_string())]),
            padding: Padding {
                left: gap,
                ..Padding::MESSAGE
            },
            margin: Margin::NONE,
            rail: true,
        };
        let inner = inset + chrome.horizontal();
        let mut content = Vec::new();
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
            return Vec::new();
        }
        let mut lines = self.frame(&chrome, indent, surface, &node.id, content);
        // And the block is separated from what follows by a plain blank, in
        // addition to the padding that closes its rail.
        lines.push(Line::default());
        lines
    }

    /// Render one code block: a quiet language marker on a thin rail, a gutter
    /// with a thin rail between the numbers and the body, and the body.
    ///
    /// This is the single home for a language label, a line-number gutter, the
    /// rail beside it, the left and right margins, and a diff's per-side
    /// numbering. Markdown fences and tool results differ only in the options
    /// they pass, never in the code they reach.
    fn code_block(&self, code: CodeBlock<'_>, indent: u8, inset: usize) -> Vec<Line> {
        let CodeBlock {
            node,
            language,
            text,
            captures,
            base,
            role,
            diff,
            numbered,
            margin,
            surface,
        } = code;
        let chrome = Chrome {
            prefix: Prefix::none(),
            padding: Padding::NONE,
            margin,
            rail: false,
        };
        let border_style = self.merged(base, role, "border", "markdown.code.border");
        // The label keeps the role's colour but loses its bold: it is a marker,
        // not a banner.
        let mut label_style = self.merged(base, role, "label", "markdown.code.label");
        label_style.bold = false;
        label_style.dim = true;
        let source: Vec<&str> = text.split('\n').collect();
        // A diff numbers from its hunk headers; a prose code block numbers in
        // sequence; a tool's own code view keeps its shape and has no gutter.
        let numbers = if diff {
            Some(code_numbers(&source, true))
        } else if numbered {
            Some(code_numbers(&source, false))
        } else {
            None
        };
        let number_width = numbers
            .as_ref()
            .and_then(|numbers| numbers.iter().flatten().map(String::len).max())
            .unwrap_or(0);
        let available = self.budget(indent, inset + chrome.horizontal());
        // A gutter is the number, a space, the rail, and a space: `1 │ `.
        let gutter_width = number_width + 3;
        let gutter = number_width > 0 && available > gutter_width + 1;
        let body_width = if gutter {
            available - gutter_width
        } else {
            available
        };
        // A diff's context keeps the code surface rather than the owning
        // message's, so the whole change reads as one block whatever it sits in.
        let context_surface = if diff {
            self.theme.surface("code").or(surface)
        } else {
            surface
        };
        let mut content = Vec::new();
        if let Some(language) = language
            && !diff
        {
            content.push(Line {
                indent,
                spans: vec![
                    (border_style, "▏ ".to_string()),
                    (label_style, language.to_string()),
                ],
                surface: context_surface,
                node: Some(node.id.clone()),
            });
        }
        for (index, raw) in source.iter().enumerate() {
            let value = clip(raw, body_width);
            let suffix = if diff { diff_suffix(raw) } else { None };
            let line_style = if diff {
                self.diff_style(base, role, raw)
            } else {
                base
            };
            let mut spans = Vec::new();
            if gutter {
                let number = numbers
                    .as_ref()
                    .and_then(|numbers| numbers.get(index))
                    .and_then(Option::as_deref)
                    .unwrap_or("");
                spans.push((border_style, format!("{number:>number_width$} ")));
                spans.push((border_style, "│ ".to_string()));
            }
            spans.extend(self.code_spans(text, index, raw, &value, captures, line_style));
            content.push(Line {
                indent,
                spans,
                surface: self.diff_row_surface(context_surface, suffix),
                node: Some(node.id.clone()),
            });
        }
        self.state_mark(node, indent, &mut content);
        self.frame(&chrome, indent, context_surface, &node.id, content)
    }

    /// The full-row surface for one line of a diff.
    ///
    /// What a line *is* is what makes it read as added or removed, so the change
    /// role's colour is laid behind the whole row — a theme that names a
    /// background for the role wins over its foreground. Context and the
    /// patch's own metadata keep the code surface, so the block stays one extent.
    fn diff_row_surface(&self, context: Option<Style>, suffix: Option<&str>) -> Option<Style> {
        let role = match suffix {
            Some("added") => "diff.added",
            Some("removed") => "diff.removed",
            _ => return context,
        };
        let style = self.theme.role(role);
        let colour = if style.bg != Color::Default {
            style.bg
        } else {
            style.fg
        };
        (colour != Color::Default).then_some(Style::PLAIN.on(colour))
    }

    /// Compose a block: put its chrome in front of rows that were laid out with
    /// `chrome.horizontal()` columns reserved, and add the blank padding rows
    /// that carry the prefix and surface.
    ///
    /// A parent gets a child's rows already prefixed, so a parent only prepends
    /// its own prefix. A child can never land after its parent's rail.
    fn frame(
        &self,
        chrome: &Chrome,
        indent: u8,
        surface: Option<Style>,
        node: &str,
        content: Vec<Line>,
    ) -> Vec<Line> {
        if content.is_empty() {
            return Vec::new();
        }
        let mut out = Vec::with_capacity(
            content.len() + chrome.padding.top as usize + chrome.padding.bottom as usize,
        );
        for _ in 0..chrome.padding.top {
            out.push(self.pad_row(chrome, indent, surface, node));
        }
        for (index, mut line) in content.into_iter().enumerate() {
            self.prefix_line(&mut line, chrome, index == 0);
            line.surface = line.surface.or(surface);
            out.push(line);
        }
        for _ in 0..chrome.padding.bottom {
            out.push(self.pad_row(chrome, indent, surface, node));
        }
        out
    }

    /// A blank row inside a block: its prefix and surface continue with no
    /// content of their own.
    fn pad_row(&self, chrome: &Chrome, indent: u8, surface: Option<Style>, node: &str) -> Line {
        let mut spans = Vec::new();
        if chrome.margin.left > 0 {
            spans.push((Style::PLAIN, " ".repeat(chrome.margin.left as usize)));
        }
        spans.extend_from_slice(&chrome.prefix.rest);
        self.gap_into(&mut spans, chrome);
        Line {
            indent,
            spans,
            surface,
            node: Some(node.to_string()),
        }
    }

    /// Put a block's chrome in front of one content row.
    ///
    /// A railed block leaves a nested block's own rail alone when it is already
    /// at the front, and a bare blank line stays outside the rail; a blank that
    /// already carries the block's surface is an interior gap and stays inside.
    fn prefix_line(&self, line: &mut Line, chrome: &Chrome, first: bool) {
        if chrome.rail {
            // The parent's opening marker is the prefix's first cell with its
            // left padding folded in, which is what a nested rail writes.
            let bar =
                chrome.prefix.first.first().map(|(_, text)| {
                    format!("{}{}", text, " ".repeat(chrome.padding.left as usize))
                });
            let railed = bar.as_deref().is_some_and(|bar| {
                !bar.is_empty() && line.spans.first().is_some_and(|(_, text)| text == bar)
            });
            if line.is_blank() {
                if !line.surface.is_some() || railed {
                    return;
                }
            } else if railed {
                return;
            }
        }
        let mut front = Vec::new();
        if chrome.margin.left > 0 {
            front.push((Style::PLAIN, " ".repeat(chrome.margin.left as usize)));
        }
        front.extend(chrome.prefix.cells(first).iter().cloned());
        self.gap_into(&mut front, chrome);
        front.extend(std::mem::take(&mut line.spans));
        line.spans = front;
    }

    /// Fold the left padding into the last prefix cell so the bar and its gap
    /// are one run, as the theme's glyph read before the split.
    fn gap_into(&self, cells: &mut Vec<(Style, String)>, chrome: &Chrome) {
        if chrome.padding.left == 0 {
            return;
        }
        let gap = " ".repeat(chrome.padding.left as usize);
        match cells.last_mut() {
            Some((_, text)) => text.push_str(&gap),
            None => cells.push((Style::PLAIN, gap)),
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
            SpanKind::Underline => base.over(self.theme.role("underline")),
            SpanKind::Highlight => base.over(self.theme.role("highlight")),
            SpanKind::Subscript => base.over(self.theme.role("subscript")),
            SpanKind::Superscript => base.over(self.theme.role("superscript")),
            SpanKind::Kbd => base.over(self.theme.role("keybinding")),
            SpanKind::Code => base.over(self.theme.role("code")),
            SpanKind::Link { href } => {
                // A footnote reference and its back-link ride the link vocabulary
                // with a `footnote:`/`footnote-back:` target, which is how a marker
                // is told apart from a target a reader can follow. A theme names
                // the marker once; a normal link keeps the global `link` role.
                if href.starts_with("footnote") {
                    base.over(self.theme.role("markdown.footnote.marker"))
                } else {
                    base.over(self.theme.role("link"))
                }
            }
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

    /// One styled table cell: its inline runs resolved against `base`, and the
    /// width they occupy. Inline emphasis, code, and links survive because the
    /// cell is a run of spans, not a flattened string.
    fn cell(&self, spans: &[Span], base: Style) -> Cell {
        let spans: Vec<(Style, String)> = spans
            .iter()
            .map(|span| (self.span_style(span, base), span.text.clone()))
            .collect();
        let width = spans.iter().map(|(_, text)| width(text)).sum();
        Cell { spans, width, base }
    }

    /// One table row: each cell padded to its column's width per the column's
    /// alignment, joined by `gap`, and shifted by `offset` so the table is
    /// centred. `widths` and `align` are parallel to the header's columns.
    fn table_row(
        &self,
        cells: &[Cell],
        widths: &[usize],
        align: &[Alignment],
        gap: usize,
        offset: usize,
        indent: u8,
        node: &Node,
    ) -> Line {
        let mut spans = Vec::new();
        if offset > 0 {
            spans.push((Style::PLAIN, " ".repeat(offset)));
        }
        for (index, column) in widths.iter().enumerate() {
            if index > 0 {
                spans.push((Style::PLAIN, " ".repeat(gap)));
            }
            let cell = cells.get(index);
            let content = cell
                .map(|cell| clip_runs(&cell.spans, *column))
                .unwrap_or_default();
            let used = content.iter().map(|(_, text)| width(text)).sum::<usize>();
            let slack = column.saturating_sub(used);
            let (left, right) = match align.get(index).copied().unwrap_or_default() {
                Alignment::Left => (0, slack),
                Alignment::Center => (slack / 2, slack - slack / 2),
                Alignment::Right => (slack, 0),
            };
            let base = cell.map_or(Style::PLAIN, |cell| cell.base);
            if left > 0 {
                spans.push((base, " ".repeat(left)));
            }
            spans.extend(content);
            if right > 0 {
                spans.push((base, " ".repeat(right)));
            }
        }
        Line {
            indent,
            spans,
            surface: None,
            node: Some(node.id.clone()),
        }
    }

    /// One table separator: a rule the full width of the table, carrying the
    /// rule role and the same centre offset as the rows.
    fn table_rule(
        &self,
        table_width: usize,
        offset: usize,
        indent: u8,
        style: Style,
        node: &Node,
    ) -> Line {
        let mut spans = Vec::new();
        if offset > 0 {
            spans.push((Style::PLAIN, " ".repeat(offset)));
        }
        spans.push((style, "─".repeat(table_width.max(1))));
        Line {
            indent,
            spans,
            surface: None,
            node: Some(node.id.clone()),
        }
    }

    /// A table with too little room for a grid keeps its fields: a header label,
    /// a colon, and the cell, one line each.
    fn stacked(
        &self,
        head: &[Vec<Span>],
        rows: &[Vec<Vec<Span>>],
        columns: usize,
        indent: u8,
        node: &Node,
        style: Style,
        available: usize,
    ) -> Vec<Line> {
        let text_of = |cells: &[Span]| {
            cells
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>()
        };
        let mut out = Vec::new();
        // A header that has arrived before any body row still says something;
        // a streamed table starts life exactly this way.
        if rows.is_empty() {
            for index in 0..columns {
                let label = head
                    .get(index)
                    .map(|cell| text_of(cell))
                    .unwrap_or_default();
                for line in wrap_spans(&[Span::plain(label)], available) {
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
            return out;
        }
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
        out
    }

    fn table(
        &self,
        head: &[Vec<Span>],
        rows: &[Vec<Vec<Span>>],
        align: &[Alignment],
        node: &Node,
        indent: u8,
        inset: usize,
        style: Style,
    ) -> Vec<Line> {
        let role = &node.role;
        let columns = head.len().max(rows.iter().map(Vec::len).max().unwrap_or(0));
        if columns == 0 {
            return Vec::new();
        }
        let available = self.budget(indent, inset);
        // A table that cannot give every column a border, padding and a wide
        // grapheme is shown as stacked fields, as the previous renderer did.
        if available < columns.saturating_mul(5) + 1 {
            return self.stacked(head, rows, columns, indent, node, style, available);
        }
        let header_style = self.merged(style, role, "header", "markdown.table.header");
        let cell_style = self.merged(style, role, "cell", "markdown.table.cell");
        let rule_style = self.merged(style, role, "rule", "markdown.table.rule");
        // A column is as wide as the widest of its header and its cells, so the
        // narrow table stays narrow instead of being stretched to the viewport.
        let mut natural = vec![1usize; columns];
        for (index, cell) in head.iter().enumerate() {
            natural[index] = natural[index].max(self.cell(cell, header_style).width);
        }
        for row in rows {
            for (index, cell) in row.iter().enumerate() {
                natural[index] = natural[index].max(self.cell(cell, cell_style).width);
            }
        }
        const GAP: usize = 2;
        let separators = GAP * columns.saturating_sub(1);
        let widths = fit_columns(&natural, available, separators);
        let table_width: usize = widths.iter().sum::<usize>() + separators;
        // Only the columns' own width is paid for. What is left is split either
        // side, so the table sits in the middle of the budget.
        let offset = available.saturating_sub(table_width) / 2;
        let header: Vec<Cell> = (0..columns)
            .map(|index| match head.get(index) {
                Some(cell) => self.cell(cell, header_style),
                None => self.cell(&[], header_style),
            })
            .collect();
        let mut out = vec![self.table_row(&header, &widths, align, GAP, offset, indent, node)];
        out.push(self.table_rule(table_width, offset, indent, rule_style, node));
        for (row_index, row) in rows.iter().enumerate() {
            // A rule under the header and between body rows, not before the
            // first body row or after the last.
            if row_index > 0 {
                out.push(self.table_rule(table_width, offset, indent, rule_style, node));
            }
            let cells: Vec<Cell> = (0..columns)
                .map(|index| match row.get(index) {
                    Some(cell) => self.cell(cell, cell_style),
                    None => self.cell(&[], cell_style),
                })
                .collect();
            out.push(self.table_row(&cells, &widths, align, GAP, offset, indent, node));
        }
        out
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

/// Split a theme rail glyph into the bar and the gap it draws after it. The gap
/// becomes the block's left padding rather than part of the glyph, so the same
/// space is not counted twice and the chrome keeps all four sides.
fn split_rail(glyph: &str) -> (&str, u8) {
    let bar = glyph.trim_end_matches(' ');
    let gap = width(glyph).saturating_sub(width(bar)) as u8;
    (bar, gap)
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

/// The smallest a column may be squeezed to before the table is stacked.
const MIN_COLUMN: usize = 1;

/// The natural column widths, shrunk from the widest until the table fits
/// `available` cells, including the `separators` between columns.
///
/// A table narrower than the budget keeps its natural width; only one that is
/// wider is narrowed, and never below one cell.
fn fit_columns(natural: &[usize], available: usize, separators: usize) -> Vec<usize> {
    let mut widths = natural.to_vec();
    let total = |widths: &[usize]| widths.iter().sum::<usize>() + separators;
    while total(&widths) > available {
        let Some(index) = widths
            .iter()
            .enumerate()
            .filter(|(_, width)| **width > MIN_COLUMN)
            .max_by_key(|(_, width)| **width)
            .map(|(index, _)| index)
        else {
            break;
        };
        widths[index] -= 1;
    }
    widths
}

/// Clip styled runs to at most `columns`, never inside a character.
fn clip_runs(runs: &[(Style, String)], columns: usize) -> Vec<(Style, String)> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for (style, text) in runs {
        if used >= columns || text.is_empty() {
            continue;
        }
        let clipped = clip(text, columns - used);
        if clipped.is_empty() {
            // The next character is wider than the room that is left, so
            // nothing after this run will fit either.
            break;
        }
        used += width(&clipped);
        out.push((*style, clipped));
    }
    out
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
    use misa_proto::view::{
        Action, ActionOn, Alignment, Definition, Field, FieldKind, SpanKind, State,
    };
    use misa_render::Color;

    /// A table node with the given columns, alignment, and cells.
    fn table_node(head: Vec<Vec<Span>>, rows: Vec<Vec<Vec<Span>>>, align: Vec<Alignment>) -> Node {
        Node::new("table", Kind::Table { head, rows, align }).id("t")
    }

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
        assert_eq!(lines[0].text(), " ▏ rust");
        let body = &lines[1];
        assert!(width(&body.text()) <= 6);
        // The client parsed the block and coloured `let` as a keyword.
        assert!(
            body.spans
                .iter()
                .any(|(style, _)| *style == Theme::dark().token("keyword")),
            "{:?}",
            body.spans
        );
    }

    #[test]
    fn a_wide_table_shrinks_instead_of_overflowing() {
        let head = vec![
            vec![Span::plain("first column")],
            vec![Span::plain("second column")],
        ];
        let rows = vec![vec![vec![Span::plain("one")], vec![Span::plain("two")]]];
        let node = table_node(head, rows, Vec::new());
        let lines = render(&node, &theme(), 16);
        for line in &lines {
            assert!(width(&line.text()) <= 16, "{:?} is too wide", line.text());
        }
        assert!(lines[0].text().trim_start().starts_with("first"));
        assert!(
            lines
                .iter()
                .any(|line| line.text().trim_start().starts_with("one")),
            "{:?}",
            lines.iter().map(Line::text).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_narrow_table_keeps_its_natural_width_and_is_centred() {
        let head = vec![vec![Span::plain("Name")], vec![Span::plain("Age")]];
        let rows = vec![vec![vec![Span::plain("Ada")], vec![Span::plain("36")]]];
        let node = table_node(head, rows, Vec::new());
        let lines = render(&node, &theme(), 60);
        let text: Vec<String> = lines.iter().map(Line::text).collect();
        // Natural width is four plus three plus the two-cell gap: nine cells,
        // not the fifty-eight the row budget has. The slack is split either
        // side, so the table sits in the middle rather than at the left edge.
        let available = 60 - 2;
        let offset = (available - 9) / 2;
        assert_eq!(
            text[0],
            format!("{}{}", " ".repeat(1 + offset), "Name  Age")
        );
        // The table's own content is nine cells, not the fifty-eight the budget
        // leaves; only the centring slack decides where it starts.
        assert_eq!(text[0].trim_start(), "Name  Age");
        assert_eq!(width(text[0].trim_start()), 9);
        assert!(
            width(&text[0]) < available,
            "{:?} filled the budget",
            text[0]
        );
    }

    #[test]
    fn table_columns_align_left_centre_and_right() {
        let head = vec![
            vec![Span::plain("Name")],
            vec![Span::plain("Middle")],
            vec![Span::plain("Score")],
        ];
        let rows = vec![vec![
            vec![Span::plain("a")],
            vec![Span::plain("b")],
            vec![Span::plain("c")],
        ]];
        let node = table_node(
            head,
            rows,
            vec![Alignment::Left, Alignment::Center, Alignment::Right],
        );
        let lines = render(&node, &theme(), 40);
        let text: Vec<String> = lines.iter().map(Line::text).collect();
        // The natural widths are four, six and five; the budget is thirty-eight,
        // so the table is centred with an offset of nine and a leading margin of
        // one. `a` fills its column from the left, `b` is centred, `c` is pushed
        // right, and Rust's own formatting states the padding we expect.
        let lead = " ".repeat(10);
        assert_eq!(
            text[0],
            format!("{lead}{:<4}  {:^6}  {:>5}", "Name", "Middle", "Score")
        );
        // The rule sits under the header, and the body follows it.
        assert!(text[1].trim_start().chars().all(|ch| ch == '─'));
        assert_eq!(text[2], format!("{lead}{:<4}  {:^6}  {:>5}", "a", "b", "c"));
    }

    #[test]
    fn a_table_too_narrow_to_draw_stacks_its_fields() {
        let head = vec![vec![Span::plain("first")], vec![Span::plain("second")]];
        let rows = vec![vec![vec![Span::plain("one")], vec![Span::plain("two")]]];
        let node = table_node(head, rows, Vec::new());
        let lines = render(&node, &theme(), 10);
        assert!(
            lines[0].text().trim_start().starts_with("first"),
            "{:?}",
            lines[0].text()
        );
        assert!(
            lines
                .iter()
                .any(|line| line.text().trim_start().starts_with("second:"))
        );
        assert!(lines.iter().any(|line| line.text().trim() == "one"));
        assert!(lines.iter().any(|line| line.text().trim() == "two"));
        // A header that has no body yet is still a table that is streaming, and
        // it must not vanish just because the budget is narrow.
        let header_only = table_node(
            vec![vec![Span::plain("first")], vec![Span::plain("second")]],
            Vec::new(),
            Vec::new(),
        );
        let lines = render(&header_only, &theme(), 10);
        assert!(lines.iter().any(|line| line.text().contains("first")));
        assert!(lines.iter().any(|line| line.text().contains("second")));
    }

    #[test]
    fn table_rules_and_cell_styles_come_from_their_roles() {
        let head = vec![vec![Span::plain("Name")], vec![Span::plain("Age")]];
        let rows = vec![
            vec![vec![Span::plain("Ada")], vec![Span::plain("36")]],
            vec![vec![Span::plain("Bob")], vec![Span::plain("41")]],
        ];
        let node = table_node(head, rows, Vec::new());
        let theme = Theme::dark();
        let lines = render(&node, &theme, 40);
        // There is a rule under the header and one between the two body rows.
        let rules: Vec<&Line> = lines
            .iter()
            .filter(|line| {
                let text = line.text();
                let trimmed = text.trim();
                !trimmed.is_empty() && trimmed.chars().all(|ch| ch == '─')
            })
            .collect();
        assert_eq!(
            rules.len(),
            2,
            "{:?}",
            lines.iter().map(Line::text).collect::<Vec<_>>()
        );
        for rule in rules {
            assert!(
                rule.spans.iter().any(|(style, text)| text.contains('─')
                    && *style == theme.role("markdown.table.rule")),
                "{:?}",
                rule.spans
            );
        }
        // The header cell carries the header role; a body cell carries the
        // cell role.
        let header = lines[0]
            .spans
            .iter()
            .find(|(_, text)| text.contains("Name"))
            .expect("a header cell");
        assert_eq!(header.0, theme.role("markdown.table.header"));
        let body = lines
            .iter()
            .find(|line| line.text().contains("Ada"))
            .and_then(|line| line.spans.iter().find(|(_, text)| text.contains("Ada")))
            .expect("a body cell");
        assert_eq!(body.0, theme.role("markdown.table.cell"));
    }

    #[test]
    fn a_table_cell_keeps_its_inline_runs() {
        let head = vec![vec![Span::plain("Value")]];
        let rows = vec![vec![vec![
            Span::plain("a "),
            Span::strong("b"),
            Span::code("c"),
        ]]];
        let node = table_node(head, rows, Vec::new());
        let theme = Theme::dark();
        let lines = render(&node, &theme, 30);
        let body = lines
            .iter()
            .find(|line| line.text().contains("a b"))
            .expect("the body row");
        assert!(
            body.spans
                .iter()
                .any(|(style, text)| text == "b" && style.bold),
            "{:?}",
            body.spans
        );
        assert!(
            body.spans.iter().any(|(style, text)| text == "c"
                && *style == theme.role("markdown.table.cell").over(theme.role("code"))),
            "{:?}",
            body.spans
        );
    }

    #[test]
    fn a_partial_table_row_renders_and_reflows_as_columns_grow() {
        let head = vec![vec![Span::plain("Name")], vec![Span::plain("Age")]];
        // The streamed row has only its first cell so far.
        let node = table_node(
            head.clone(),
            vec![vec![vec![Span::plain("Ada")]]],
            Vec::new(),
        );
        let lines = render(&node, &theme(), 40);
        let body = lines
            .iter()
            .find(|line| line.text().contains("Ada"))
            .expect("the arrived cell");
        assert!(!body.text().contains("Age"));
        // When the second cell arrives, the first column's neighbour widens and
        // the row reflows around it.
        let grown = table_node(
            head,
            vec![vec![
                vec![Span::plain("Ada")],
                vec![Span::plain("many years")],
            ]],
            Vec::new(),
        );
        let before: Vec<String> = lines.iter().map(Line::text).collect();
        let after: Vec<String> = render(&grown, &theme(), 40)
            .iter()
            .map(Line::text)
            .collect();
        let row_of = |text: &[String]| {
            text.iter()
                .find(|line| line.contains("Ada"))
                .cloned()
                .expect("the body row")
        };
        assert_ne!(
            row_of(&before),
            row_of(&after),
            "the row did not reflow as the column grew"
        );
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
    fn a_details_block_is_always_expanded_in_the_terminal() {
        // The linear medium has no interaction, so a disclosure is its summary
        // followed by its body: there is no collapse marker and nothing is hidden.
        let blocks = misa_markdown::blocks(
            "message.assistant",
            "<details>\n<summary>More</summary>\n\nhidden **text**\n\n</details>",
        );
        let node = Node::section("message.assistant").children(blocks);
        let theme = Theme::dark();
        let lines = render(&node, &theme, 60);
        let text = to_plain(&lines);
        assert!(text.contains("More"), "{text}");
        assert!(text.contains("hidden text"), "{text}");
        // The summary resolves through the global details-summary role, so a
        // theme names it once.
        let summary = lines
            .iter()
            .find(|line| line.text().contains("More"))
            .and_then(|line| line.spans.iter().find(|(_, text)| text == "More"))
            .expect("the summary run");
        assert_eq!(summary.0, theme.role("markdown.details.summary"));
    }

    #[test]
    fn a_definition_list_puts_terms_above_indented_definitions() {
        let node = Node::new(
            "message.assistant.markdown.definition",
            Kind::Definition {
                entries: vec![
                    Definition {
                        term: vec![Span::plain("Apple")],
                        definitions: vec![
                            vec![Span::plain("A fruit.")],
                            vec![Span::plain("A company.")],
                        ],
                    },
                    Definition {
                        term: vec![Span::plain("Orange")],
                        definitions: vec![vec![Span::plain("A colour.")]],
                    },
                ],
            },
        )
        .id("dl");
        let rows: Vec<String> = render(&node, &theme(), 60).iter().map(Line::text).collect();
        assert_eq!(
            rows,
            vec![
                " Apple",
                " • A fruit.",
                " • A company.",
                " Orange",
                " • A colour.",
            ]
        );
    }

    #[test]
    fn a_dl_renders_the_same_rows_as_the_markdown_form() {
        let markdown = misa_markdown::blocks(
            "message.assistant",
            "Apple\n:   A fruit.\n:   A company.\n\nOrange\n:   A colour.",
        );
        let html = misa_markdown::blocks(
            "message.assistant",
            "<dl>\n<dt>Apple</dt>\n<dd>A fruit.</dd>\n<dd>A company.</dd>\n<dt>Orange</dt>\n<dd>A colour.</dd>\n</dl>",
        );
        let draw = |blocks| {
            to_plain(&render(
                &Node::section("message.assistant").children(blocks),
                &theme(),
                60,
            ))
        };
        assert_eq!(draw(markdown), draw(html));
    }

    #[test]
    fn a_footnote_reference_and_its_section_render() {
        let blocks = misa_markdown::blocks("message.assistant", "Text[^one].\n\n[^one]: The note.");
        let node = Node::section("message.assistant").children(blocks);
        let theme = Theme::dark();
        let lines = render(&node, &theme, 60);
        let text = to_plain(&lines);
        assert!(text.contains("Text1."), "{text}");
        assert!(text.contains("The note."), "{text}");
        assert!(text.contains('↩'), "{text}");
        // The marker is styled by the global footnote-marker role rather than
        // the ordinary link role, so a theme can tell a note from a target.
        let marker = lines
            .iter()
            .flat_map(|line| &line.spans)
            .find(|(_, text)| text == "1")
            .expect("the footnote marker");
        assert_eq!(marker.0, theme.role("markdown.footnote.marker"));
        assert_ne!(marker.0, theme.role("link"));
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
        // The leading run is the block's left margin.
        assert_eq!(lines[0].spans[1].0, theme.role("markdown.code.border"));
        assert_eq!(lines[0].spans[3].0, theme.role("diff.hunk"));
        assert_eq!(lines[1].spans[3].0, theme.role("diff.removed"));
        assert_eq!(lines[2].spans[3].0, theme.role("diff.added"));
        // Context keeps the node's own style, so a diff still reads like a tool result.
        assert_eq!(
            lines[3].spans[3].0,
            theme.role("message.assistant.markdown.diff")
        );
        // The gutter numbers the old file for removals and the new one otherwise,
        // and says nothing for the hunk header.
        assert_eq!(
            to_plain(&lines),
            "   │ @@ -1 +1 @@\n 1 │ -old line\n 1 │ +new line\n 2 │  context\n"
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
        assert_eq!(lines[0].text(), " 1 │ one");
        assert_eq!(lines[1].text(), " 2 │ two");
        let lines = render(&code("tool.result"), &Theme::dark(), 20);
        assert_eq!(lines[0].text(), " one");
        assert_eq!(lines[1].text(), " two");
    }

    #[test]
    fn a_diff_fence_and_a_tool_result_render_the_same_rows() {
        // Two ways a session says "this body is a diff": a fence that named
        // `diff`, and a result the session classified. The same component draws
        // both, so the rows agree apart from the node they point back at.
        let body = "@@ -1 +1 @@\n-old\n+new\n context";
        let fence = Node::new(
            "message.assistant.markdown.diff",
            Kind::Code {
                lang: Some("diff".into()),
                text: body.into(),
            },
        )
        .id("fence");
        let result = Node::new(
            "tool.result.diff",
            Kind::Code {
                lang: None,
                text: body.into(),
            },
        )
        .id("result");
        let theme = Theme::dark();
        let rows = |mut lines: Vec<Line>| {
            for line in &mut lines {
                line.node = None;
            }
            lines
        };
        assert_eq!(
            rows(render(&fence, &theme, 40)),
            rows(render(&result, &theme, 40))
        );
    }

    #[test]
    fn a_diff_numbers_both_sides_across_two_hunks() {
        // Only tracking the `@@ -a,b +c,d @@` headers keeps the second hunk's
        // numbers right: the old side jumps to ten while the new side jumps to
        // eleven, and each change line still numbers its own file.
        let text = "\
diff --git a/x b/x
index 111..222 100644
--- a/x
+++ b/x
@@ -1,2 +1,3 @@
 a
-b
+c
+d
@@ -10,2 +11,1 @@
 e
-f
+g";
        let node = Node::new(
            "tool.result.diff",
            Kind::Code {
                lang: None,
                text: text.into(),
            },
        )
        .id("d");
        let lines = render(&node, &Theme::dark(), 40);
        // Metadata and hunk headers say nothing; a removed line names the old
        // file, an added or context line the new one.
        assert_eq!(
            to_plain(&lines),
            "    │ diff --git a/x b/x\n    │ index 111..222 100644\n    │ --- a/x\n    │ +++ b/x\n    │ @@ -1,2 +1,3 @@\n  1 │  a\n  2 │ -b\n  2 │ +c\n  3 │ +d\n    │ @@ -10,2 +11,1 @@\n 11 │  e\n 11 │ -f\n 12 │ +g\n"
        );
    }

    #[test]
    fn a_change_row_takes_its_role_surface_and_metadata_does_not() {
        let text = "diff --git a/x b/x\n@@ -1 +1 @@\n-gone\n+kept";
        let node = Node::new(
            "tool.result.diff",
            Kind::Code {
                lang: None,
                text: text.into(),
            },
        )
        .id("d");
        let theme = Theme::dark();
        let lines = render(&node, &theme, 40);
        let row = |needle: &str| {
            lines
                .iter()
                .find(|line| line.text().contains(needle))
                .unwrap_or_else(|| panic!("no row for {needle:?}: {}", to_plain(&lines)))
        };
        let added = row("+kept");
        let removed = row("-gone");
        let meta = row("diff --git");
        let hunk = row("@@");
        let code = theme.surface("code").expect("the code surface");
        // The change's own colour is behind the whole row, green-ish for an
        // addition and red-ish for a removal.
        assert_eq!(added.surface.unwrap().bg, theme.role("diff.added").fg);
        assert_eq!(removed.surface.unwrap().bg, theme.role("diff.removed").fg);
        assert_ne!(added.surface.unwrap().bg, removed.surface.unwrap().bg);
        // The patch's own metadata and a hunk header keep the code surface, so
        // the block stays one extent and only the changes stand out.
        assert_eq!(meta.surface, Some(code));
        assert_eq!(hunk.surface, Some(code));
        for row in [meta, hunk] {
            assert_ne!(row.surface, added.surface);
            assert_ne!(row.surface, removed.surface);
        }
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
        // A theme that says something narrower wins over the generic role. The
        // body run follows the gutter a diff shares with a markdown fence.
        let theme = Theme::dark().with_role("tool.result.diff.added", Style::rgb(1, 2, 3).bold());
        let line = &render(&node, &theme, 40)[0];
        assert_eq!(line.spans[3].0.fg, Color::Rgb(1, 2, 3));
        assert!(line.spans[3].0.bold);
        // And the generic role is used when it has not.
        let line = &render(&node, &Theme::dark(), 40)[0];
        assert_eq!(line.spans[3].0, Theme::dark().role("diff.added"));
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
        // The demoted label sits on a thin rail, not a bold banner.
        assert_eq!(lines[0].text(), " ▏ rust");
        assert!(
            !lines[0].spans[2].0.bold,
            "the language label is still bold"
        );
        assert_eq!(
            lines[1].spans[1].0,
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
    fn a_span_kind_resolves_through_its_theme_role() {
        let theme = Theme::dark();
        let text = |kind| {
            Node::text(
                "message.assistant.markdown.paragraph",
                [Span {
                    text: "x".into(),
                    kind,
                }],
            )
        };
        // Highlight is a background, so it sits over whatever the role chose.
        assert_ne!(
            render(&text(SpanKind::Highlight), &theme, 40)[0].spans[0]
                .0
                .bg,
            Color::Default
        );
        assert_eq!(
            render(&text(SpanKind::Kbd), &theme, 40)[0].spans[0].0,
            theme
                .role("message.assistant")
                .over(theme.role("keybinding"))
        );
        assert!(
            render(&text(SpanKind::Underline), &theme, 40)[0].spans[0]
                .0
                .underline
        );
        // A cell grid cannot raise or lower a run, but the two stay distinct.
        let sub = render(&text(SpanKind::Subscript), &theme, 40)[0].spans[0].0;
        let sup = render(&text(SpanKind::Superscript), &theme, 40)[0].spans[0].0;
        assert!(sub.dim);
        assert_ne!(sub, sup);
    }

    #[test]
    fn an_alert_quote_takes_its_kind_from_the_role() {
        let theme = Theme::dark();
        let node =
            Node::new("message.assistant.markdown.alert.warning", Kind::Quote).child(Node::text(
                "message.assistant.markdown.paragraph",
                [Span::plain("careful")],
            ));
        let lines = render(&node, &theme, 40);
        assert_eq!(lines[0].spans[0].0, theme.role("markdown.alert.warning"));
        // An ordinary quote still dims rather than borrowing an alert colour.
        let plain = Node::new("message.assistant.markdown.quote", Kind::Quote).child(Node::text(
            "message.assistant.markdown.paragraph",
            [Span::plain("q")],
        ));
        assert!(render(&plain, &theme, 40)[0].spans[0].0.dim);
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

    #[test]
    fn a_list_in_a_rail_keeps_the_rail_first_and_aligns_the_wrap() {
        // The item wraps, so the marker row and its continuation are both
        // visible; the rail must stay leftmost on each.
        let item = vec![Node::text(
            "message.user.markdown.paragraph",
            [Span::plain("item wrapped")],
        )];
        let list = Node::new(
            "message.user.markdown.list",
            Kind::List {
                ordered: false,
                items: vec![item],
                markers: Vec::new(),
            },
        )
        .id("l");
        let node = Node::section("message.user").id("m1").child(list);
        let rows: Vec<String> = render(&node, &Theme::dark(), 13)
            .iter()
            .map(Line::text)
            .collect();
        assert!(rows.iter().any(|row| row == "┃ • item"), "{rows:?}");
        assert!(rows.iter().any(|row| row == "┃   wrapped"), "{rows:?}");
        // The wrapped row aligns under the marker's text, and the rail is never
        // placed after the list's indent or marker.
        for row in rows.iter().filter(|row| !row.trim().eq("┃")) {
            assert!(row.starts_with("┃ "), "rail is not first: {row:?}");
            assert!(!row.starts_with("  ┃"), "rail after the indent: {row:?}");
        }
    }

    #[test]
    fn nested_lists_compose_their_markers_in_ancestor_order() {
        let inner = Node::new(
            "message.user.markdown.list",
            Kind::List {
                ordered: false,
                items: vec![vec![Node::text("x", [Span::plain("deep")])]],
                markers: Vec::new(),
            },
        )
        .id("inner");
        let outer = Node::new(
            "message.user.markdown.list",
            Kind::List {
                ordered: false,
                items: vec![vec![Node::text("x", [Span::plain("outer")]), inner]],
                markers: Vec::new(),
            },
        )
        .id("outer");
        let node = Node::section("message.user").id("m1").child(outer);
        let rows: Vec<String> = render(&node, &Theme::dark(), 40)
            .iter()
            .map(Line::text)
            .collect();
        assert!(rows.iter().any(|row| row == "┃ • outer"), "{rows:?}");
        assert!(rows.iter().any(|row| row == "┃   • deep"), "{rows:?}");
        // The nested row carries the rail, then the outer item's alignment, then
        // the inner marker: prefixes compose in ancestor order.
        let nested = rows.iter().find(|row| row.contains("deep")).unwrap();
        assert!(nested.starts_with("┃   •"), "{nested:?}");
    }

    #[test]
    fn a_message_block_pads_above_and_below_and_clears_its_rail() {
        let node = Node::section("message.user").id("m1").child(
            Node::text("message.user.markdown.paragraph", [Span::plain("hello")]).id("m1.body"),
        );
        let lines = render_block(&node, &Theme::dark(), 80, 0);
        let surface = Theme::dark().surface("message.user");
        // A padding row above and below carries the rail and the surface but no
        // content of its own.
        for pad in [&lines[0], &lines[2]] {
            assert_eq!(pad.text(), "┃ ");
            assert_eq!(pad.surface, surface);
            assert!(pad.is_blank() || pad.spans.iter().all(|(_, text)| text.trim() == "┃"));
        }
        // The text does not begin against the bar: the rail's gap comes first.
        let body = &lines[1];
        assert_eq!(body.text(), "┃ hello");
        assert!(
            body.spans[0].1.starts_with("┃ "),
            "text touches the rail: {:?}",
            body.spans[0]
        );
    }

    #[test]
    fn a_rule_spans_the_width_left_after_its_indent_and_inset() {
        let node = Node::new("message.assistant.markdown.rule", Kind::Rule);
        let line = &render(&node, &Theme::dark(), 80)[0];
        assert_eq!(line.text().chars().filter(|ch| *ch == '─').count(), 80);

        let nested = Node::section("message.user")
            .id("m1")
            .child(Node::new("message.user.markdown.rule", Kind::Rule));
        let line = render(&nested, &Theme::dark(), 80)
            .into_iter()
            .find(|line| line.text().contains('─'))
            .expect("the nested rule");
        // The rail, its gap, and the reserved right cell are three inset cells.
        assert_eq!(line.text().chars().filter(|ch| *ch == '─').count(), 77);
        assert!(line.text().starts_with("┃ "), "{:?}", line.text());
    }

    #[test]
    fn a_code_block_demotes_its_language_label_and_rails_its_gutter() {
        let node = Node::new(
            "message.assistant.markdown.code",
            Kind::Code {
                lang: Some("rust".into()),
                text: "let x = 1;".into(),
            },
        );
        let theme = Theme::dark();
        let lines = render(&node, &theme, 40);
        // The label is a dim marker on a thin rail, not a bold banner.
        let label = &lines[0];
        assert!(label.text().starts_with(" ▏ rust"), "{:?}", label.text());
        let label_span = label.spans.last().expect("the language label");
        assert_eq!(label_span.1, "rust");
        assert!(
            label_span.0.dim && !label_span.0.bold,
            "the label is still prominent: {:?}",
            label_span.0
        );
        // The numbered gutter is separated from the body by a thin vertical rail.
        let body = &lines[1];
        let rail = body
            .spans
            .iter()
            .position(|(_, text)| text == "│ ")
            .expect("a gutter rail");
        let number = body
            .spans
            .iter()
            .position(|(_, text)| text.trim() == "1")
            .expect("a gutter number");
        assert!(
            number < rail,
            "the rail does not follow the number: {body:?}"
        );
        // A body token is coloured and undimmed, so the label is not the most
        // prominent run on the block.
        let keyword = body
            .spans
            .iter()
            .find(|(_, text)| text == "let")
            .expect("the keyword");
        assert!(!keyword.0.dim, "the keyword is dim: {keyword:?}");
    }
}
