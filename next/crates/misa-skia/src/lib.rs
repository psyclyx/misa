//! The pixel frontend: the semantic tree as a scene, and as a raster.
//!
//! # What is shared and what is not
//!
//! A pixel frontend cannot use the terminal's lines: it has no cells, and it wants
//! real type. What it *can* share is everything that is not a cell —
//! [`misa_render::text`] for measurement, [`misa_render::Theme`] for what a role
//! looks like, and the tree itself.
//!
//! So this frontend maps the same tree to a **scene** of positioned runs and
//! rectangles, and that mapping is the interesting part: it is where a role becomes
//! a colour and a weight, where a rail becomes a bar, and where a code block becomes
//! a raised panel. It is testable with no window and no GPU, which is why the scene
//! and the raster are both built and tested by default.
//!
//! Text flow wraps to a column count and stacks runs, which is a deliberate first
//! slice: a rail is a drawn bar, a code block is highlighted from the client's own
//! grammar, and nothing below a role becomes a terminal cell. A later slice can
//! give the scene proportional type and reflow.

use misa_proto::view::{Kind, Node, Span, SpanKind};
use misa_render::text::{clip, wrap_spans};
use misa_render::{Color, Style, Theme};

/// One thing to draw.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// A retained local scene, positioned without rebuilding its paint operations.
    Group {
        x: f32,
        y: f32,
        ops: std::sync::Arc<Vec<Op>>,
    },
    Image {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        image: std::sync::Arc<image::RgbaImage>,
    },
    /// A run of text at a baseline position.
    Text {
        x: f32,
        y: f32,
        size: f32,
        style: Style,
        text: String,
    },
    /// A filled rectangle, in device pixels.
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        style: Style,
    },
}

/// A drawable frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    pub ops: Vec<Op>,
}

/// The measurements a scene is laid out with.
///
/// Passed in rather than assumed, because a frontend that guesses at its advance
/// width mis-measures every line by a little and the result looks wrong for reasons
/// nobody can name.
#[derive(Clone, Copy, Debug)]
pub struct Layout {
    pub advance: f32,
    pub line_height: f32,
    pub margin: f32,
    pub font_size: f32,
}

impl Default for Layout {
    fn default() -> Self {
        Layout {
            advance: 8.4,
            line_height: 21.0,
            margin: 24.0,
            font_size: 15.0,
        }
    }
}

/// Build a scene from a view tree.
///
/// Two decisions this makes that the terminal renderer does not: a message
/// becomes a *card* whose surface spans the whole block rather than each row, and
/// its rail is one full-height bar rather than a segment per line. Both are things
/// a pixel medium can do and a cell grid cannot.
pub fn scene(view: &Node, theme: &Theme, columns: usize, rows: usize, layout: Layout) -> Scene {
    let mut builder = Builder {
        theme,
        layout,
        columns,
        rows,
        row: 0,
        prefixes: Vec::new(),
        ops: Vec::new(),
    };
    builder.node(view, 0);
    Scene {
        width: layout.margin * 2.0 + columns as f32 * layout.advance,
        height: layout.margin * 2.0 + rows as f32 * layout.line_height,
        ops: builder.ops,
    }
}

/// Map one semantic run onto the resolved style for its kind, over a base style.
pub(crate) fn span_style(theme: &Theme, span: &Span, base: Style) -> Style {
    match &span.kind {
        SpanKind::Plain => base,
        SpanKind::Strong => base.over(theme.role("bold")),
        SpanKind::StrongEmphasis => base.over(theme.role("bold")).over(theme.role("italic")),
        SpanKind::Emphasis => base.over(theme.role("italic")),
        SpanKind::Strikethrough => base.over(theme.role("strikethrough")),
        SpanKind::Underline => base.over(theme.role("underline")),
        SpanKind::Highlight => base.over(theme.role("highlight")),
        SpanKind::Subscript => base.over(theme.role("subscript")),
        SpanKind::Superscript => base.over(theme.role("superscript")),
        SpanKind::Kbd => base.over(theme.role("keybinding")),
        SpanKind::Code => base.over(theme.role("code")),
        SpanKind::Link { .. } => base.over(theme.role("link")).underline(),
    }
}

/// Whether a code node's body is a diff.
///
/// Two ways for a session to say so, and neither of them looks at the body: a role
/// that ends in `.diff`, or a fence a model wrote with `diff`.
pub(crate) fn is_diff(role: &str, lang: Option<&str>) -> bool {
    role.ends_with(".diff") || lang.is_some_and(|lang| lang.eq_ignore_ascii_case("diff"))
}

/// What a line of a unified diff is, named the way a theme names a role.
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

/// The style for one line of a diff, by what the line starts with.
pub(crate) fn diff_style(theme: &Theme, base: Style, role: &str, raw: &str) -> Style {
    let Some(suffix) = diff_suffix(raw) else {
        return base;
    };
    let specific = format!("{role}.{suffix}");
    if theme.names(&specific) {
        base.over(theme.role(&specific))
    } else {
        base.over(theme.role(&format!("diff.{suffix}")))
    }
}

/// Split one clipped code line by the captures that cover it.
pub(crate) fn code_runs(
    whole: &str,
    line_index: usize,
    clipped: &str,
    captures: &[misa_syntax::Capture],
    base: Style,
    theme: &Theme,
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
        let style = capture
            .map(|capture| theme.token(&capture.token))
            .unwrap_or(base);
        spans.push((style, clipped[cursor..end].to_string()));
        cursor = end;
    }
    if spans.is_empty() {
        spans.push((base, String::new()));
    }
    spans
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

/// A native scene walker over the semantic tree.
///
/// It is deliberately linear like the terminal's flow, but every decision is the
/// pixel medium's: a rail is a bar, a rule is a rectangle, and a code block is
/// highlighted here rather than by the session.
struct Builder<'a> {
    theme: &'a Theme,
    layout: Layout,
    columns: usize,
    rows: usize,
    row: usize,
    prefixes: Vec<(Style, String)>,
    ops: Vec<Op>,
}

impl Builder<'_> {
    fn y(&self, row: usize) -> f32 {
        self.layout.margin + row as f32 * self.layout.line_height
    }

    fn budget(&self, indent: usize) -> usize {
        self.columns.saturating_sub(indent).max(1)
    }

    /// Emit one wrapped visual row and advance the cursor. Content past the
    /// caller's row budget advances the cursor but draws nothing.
    fn line(&mut self, indent: usize, runs: Vec<(Style, String)>) {
        let row = self.row;
        self.row += 1;
        if row >= self.rows {
            return;
        }
        let mut all = self.prefixes.clone();
        all.extend(runs);
        let y = self.y(row);
        let mut x = self.layout.margin + indent as f32 * self.layout.advance;
        for (style, text) in all {
            if !text.is_empty() {
                self.ops.push(Op::Text {
                    x,
                    y,
                    size: self.layout.font_size,
                    style,
                    text: text.clone(),
                });
            }
            x += misa_render::width(&text) as f32 * self.layout.advance;
        }
    }

    fn rect(&mut self, indent: usize, height: f32, style: Style) {
        let row = self.row;
        self.row += 1;
        if row >= self.rows {
            return;
        }
        self.ops.push(Op::Rect {
            x: self.layout.margin + indent as f32 * self.layout.advance,
            y: self.y(row),
            width: self.budget(indent) as f32 * self.layout.advance,
            height,
            style,
        });
    }

    fn spans(&mut self, indent: usize, spans: &[Span], base: Style) {
        let budget = self.budget(indent);
        for line in wrap_spans(spans, budget) {
            let runs = line
                .iter()
                .map(|span| (span_style(self.theme, span, base), span.text.clone()))
                .collect();
            self.line(indent, runs);
        }
    }

    fn children(&mut self, node: &Node, indent: usize) {
        for child in &node.children {
            self.node(child, indent);
        }
    }

    fn section(&mut self, node: &Node, indent: usize, base: Style) {
        if let Some((_glyph, rail_style)) = self.theme.rail(&node.role) {
            // Render the block first, then insert its chrome behind the runs at the
            // point they begin. Inserting rather than appending is what turns a run
            // into a card: paint order is op order.
            let start = self.ops.len();
            let first = self.row;
            if let Some(label) = &node.label {
                self.line(indent, vec![(base, label.clone())]);
            }
            self.children(node, indent + 2);
            let last = self.row.min(self.rows);
            if last > first {
                let height = (last - first) as f32 * self.layout.line_height;
                let mut at = start;
                // The card: the whole block shares one surface, not each row.
                if let Some(surface) = self.theme.surface(&node.role)
                    && surface.bg != Color::Default
                {
                    self.ops.insert(
                        at,
                        Op::Rect {
                            x: self.layout.margin * 0.5,
                            y: self.y(first),
                            width: self.columns as f32 * self.layout.advance,
                            height,
                            style: Style::fg(surface.bg),
                        },
                    );
                    at += 1;
                }
                // The rail: one bar beside the whole block.
                self.ops.insert(
                    at,
                    Op::Rect {
                        x: self.layout.margin + indent as f32 * self.layout.advance,
                        y: self.y(first),
                        width: 2.0,
                        height: height - 4.0,
                        style: rail_style,
                    },
                );
            }
        } else if node.role == "session" {
            self.children(node, indent);
        } else if let Some(label) = &node.label {
            self.line(indent, vec![(base.bold(), label.clone())]);
            self.children(node, indent);
        } else {
            self.children(node, indent);
        }
    }

    fn code(&mut self, node: &Node, lang: Option<&str>, text: &str, indent: usize, base: Style) {
        let diff = is_diff(&node.role, lang);
        if let Some(lang) = lang
            && !diff
        {
            self.line(
                indent,
                vec![(self.theme.role("markdown.code.label"), lang.to_string())],
            );
        }
        let captures = if diff {
            Vec::new()
        } else {
            lang.map(|language| misa_syntax::captures(language, text))
                .unwrap_or_default()
        };
        let budget = self.budget(indent);
        for (index, raw) in text.split('\n').enumerate() {
            let clipped = clip(raw, budget);
            let line_style = if diff {
                diff_style(self.theme, base, &node.role, raw)
            } else {
                base
            };
            let runs = code_runs(text, index, &clipped, &captures, line_style, self.theme);
            self.line(indent, runs);
        }
    }

    fn list(
        &mut self,
        ordered: &bool,
        items: &[Vec<Node>],
        markers: &[Option<bool>],
        indent: usize,
        base: Style,
    ) {
        for (index, item) in items.iter().enumerate() {
            let marker = match markers.get(index).copied().flatten() {
                Some(true) => "☑".to_string(),
                Some(false) => "☐".to_string(),
                None if *ordered => format!("{}.", index + 1),
                None => "•".to_string(),
            };
            self.line(indent, vec![(base, marker)]);
            for child in item {
                self.node(child, indent + 2);
            }
        }
    }

    fn table(&mut self, head: &[Vec<Span>], rows: &[Vec<Vec<Span>>], indent: usize, base: Style) {
        for row in std::iter::once(head).chain(rows.iter().map(Vec::as_slice)) {
            let mut spans: Vec<Span> = Vec::new();
            for (index, cell) in row.iter().enumerate() {
                if index > 0 {
                    spans.push(Span::plain("  "));
                }
                spans.extend(cell.iter().cloned());
            }
            self.spans(indent, &spans, base);
        }
    }

    fn fields(&mut self, fields: &[misa_proto::view::Field], indent: usize, base: Style) {
        for field in fields {
            self.line(indent, vec![(base.dim(), field.label.clone())]);
            let value = if field.secret {
                "••••".to_string()
            } else {
                field.value.clone()
            };
            self.spans(indent + 2, &[Span::plain(value)], base);
        }
    }

    fn node(&mut self, node: &Node, indent: usize) {
        let base = self.theme.role(&node.role);
        match &node.kind {
            Kind::Section => self.section(node, indent, base),
            Kind::Text { spans } => self.spans(indent, spans, base),
            Kind::Heading { level, spans } => {
                // A heading is structure, so the medium makes it visible even when a
                // theme says nothing: level one is underlined as well as bold.
                let mut base = base.bold();
                if *level == 1 {
                    base = base.underline();
                }
                self.spans(indent, spans, base);
            }
            Kind::Quote => {
                // An alert is still a quote; its role names the kind, and the theme's
                // global `markdown.alert.<kind>` recolours the marker if it names one.
                let base = match misa_render::alert_role(&node.role) {
                    Some(role) => base.over(self.theme.role(role)),
                    None => base.dim(),
                };
                self.prefixes.push((base, "▏ ".to_string()));
                self.children(node, indent);
                self.prefixes.pop();
            }
            Kind::Rule => self.rect(indent, 2.0, base),
            Kind::Code { lang, text } => self.code(node, lang.as_deref(), text, indent, base),
            Kind::Fact { value } => {
                self.line(
                    indent,
                    vec![(base, misa_render::fact::format(&node.role, value))],
                );
            }
            Kind::Status { text } => self.line(indent, vec![(base, text.clone())]),
            Kind::List {
                ordered,
                items,
                markers,
            } => self.list(ordered, items, markers, indent, base),
            Kind::Table { head, rows, .. } => self.table(head, rows, indent, base),
            Kind::Fields { fields } => self.fields(fields, indent, base),
            Kind::Collapsible { summary } => {
                self.spans(indent, summary, base);
                self.children(node, indent);
            }
            Kind::Image {
                blob,
                alt,
                width,
                height,
            } => {
                let label = if alt.is_empty() {
                    format!("[{width}×{height} image {}]", clip(&blob.hash, 8))
                } else {
                    format!("[image: {alt}]")
                };
                self.line(indent, vec![(base.dim(), label)]);
            }
            Kind::Meter { label, value, max } => {
                self.line(indent, vec![(base, format!("{label} {value}/{max}"))]);
            }
        }
    }
}

/// The scene as a PNG.
pub mod app;
pub mod appearance;
pub mod connection;
mod preferences;
pub mod window;
pub mod workspace;

pub mod paint {
    use super::{Op, Scene};
    use misa_render::Color;
    use skia_safe::{
        Canvas, Font, FontMgr, FontStyle, Paint as SkPaint, PaintStyle, Rect, surfaces,
    };

    fn skia_color(color: Color, fallback: u32) -> u32 {
        match color {
            Color::Default => fallback,
            Color::Indexed(index) => 0xff00_0000 | ((index as u32) * 0x0001_0101),
            Color::Rgb(r, g, b) => 0xff00_0000 | ((r as u32) << 16) | ((g as u32) << 8) | b as u32,
        }
    }

    /// Draw a scene to a raster surface and return the pixels.
    ///
    /// Grayscale antialiasing and no GPU: a view is text, and a text renderer that
    /// needs a device is a text renderer nobody can test.
    pub fn raster(scene: &Scene, background: Color) -> Result<image::RgbaImage, String> {
        let width = scene.width.ceil().max(1.0) as i32;
        let height = scene.height.ceil().max(1.0) as i32;
        let mut surface =
            surfaces::raster_n32_premul((width, height)).ok_or("no raster surface")?;
        let canvas: &Canvas = surface.canvas();
        canvas.clear(skia_safe::Color::from(skia_color(background, 0xff14_161a)));

        let fonts = FontMgr::default();
        let typeface = fonts
            .match_family_style("monospace", FontStyle::default())
            .or_else(|| {
                fonts
                    .family_names()
                    .find_map(|family| fonts.match_family_style(family, FontStyle::default()))
            })
            .ok_or("no typeface available; install a font")?;
        let mut fill = SkPaint::default();
        fill.set_anti_alias(true);

        draw_ops(canvas, &scene.ops, &typeface, &mut fill);

        let pixmap = surface.peek_pixels().ok_or("no pixels")?;
        let bytes = pixmap.bytes().ok_or("no pixel bytes")?;
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for pixel in bytes.chunks_exact(4) {
            // N32 premultiplied is BGRA on a little-endian machine.
            rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], pixel[3]]);
        }
        image::RgbaImage::from_raw(width as u32, height as u32, rgba)
            .ok_or_else(|| "no image".to_string())
    }

    fn draw_ops(canvas: &Canvas, ops: &[Op], typeface: &skia_safe::Typeface, fill: &mut SkPaint) {
        for op in ops {
            match op {
                Op::Group { x, y, ops } => {
                    canvas.save();
                    canvas.translate((*x, *y));
                    draw_ops(canvas, ops, typeface, fill);
                    canvas.restore();
                }
                Op::Image {
                    x,
                    y,
                    width,
                    height,
                    image,
                } => {
                    let info = skia_safe::ImageInfo::new(
                        (image.width() as i32, image.height() as i32),
                        skia_safe::ColorType::RGBA8888,
                        skia_safe::AlphaType::Unpremul,
                        None,
                    );
                    let data = skia_safe::Data::new_copy(image.as_raw());
                    if let Some(bitmap) =
                        skia_safe::images::raster_from_data(&info, data, image.width() as usize * 4)
                    {
                        canvas.draw_image_rect(
                            bitmap,
                            None,
                            Rect::from_xywh(*x, *y, *width, *height),
                            fill,
                        );
                    }
                }
                Op::Rect {
                    x,
                    y,
                    width,
                    height,
                    style,
                } => {
                    fill.set_style(PaintStyle::Fill);
                    fill.set_color(skia_safe::Color::from(skia_color(style.fg, 0xff9a_a2ad)));
                    canvas.draw_rect(Rect::from_xywh(*x, *y, *width, *height), fill);
                }
                Op::Text {
                    x,
                    y,
                    size,
                    style,
                    text,
                } => {
                    let font = Font::from_typeface(typeface.clone(), *size);
                    fill.set_style(PaintStyle::Fill);
                    fill.set_color(skia_safe::Color::from(skia_color(style.fg, 0xffe9_ebee)));
                    // The scene positions a baseline; a line's y is its top.
                    canvas.draw_str(text, (*x, *y + size * 0.85), &font, fill);
                }
            }
        }
    }

    /// A scene as PNG bytes.
    pub fn png(scene: &Scene, background: Color) -> Result<Vec<u8>, String> {
        let image = raster(scene, background)?;
        let mut bytes = std::io::Cursor::new(Vec::new());
        image
            .write_to(&mut bytes, image::ImageFormat::Png)
            .map_err(|err| err.to_string())?;
        Ok(bytes.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::view::{Kind, Span, State};

    fn view() -> Node {
        Node::section("session")
            .id("session")
            .child(
                Node::section("message.user")
                    .id("msg.1")
                    .state(State::Done)
                    .child(Node::text("message.user", [Span::plain("a question")])),
            )
            .child(Node::new(
                "tool.result",
                Kind::Code {
                    lang: Some("rust".into()),
                    text: "let x = 1;".into(),
                },
            ))
    }

    fn scene_of(theme: &Theme) -> Scene {
        scene(&view(), theme, 80, 24, Layout::default())
    }

    #[test]
    fn a_scene_positions_every_run_and_leaves_no_run_unplaced() {
        let scene = scene_of(&Theme::dark());
        assert!(!scene.ops.is_empty());
        assert!(scene.width > 0.0 && scene.height > 0.0);
        for op in &scene.ops {
            match op {
                Op::Text { x, y, text, .. } => {
                    assert!(*x >= 0.0 && *y >= 0.0);
                    assert!(!text.is_empty(), "an empty run was emitted");
                }
                Op::Group { .. } => {}
                Op::Image { width, height, .. } | Op::Rect { width, height, .. } => {
                    assert!(*width > 0.0 && *height > 0.0)
                }
            }
        }
    }

    #[test]
    fn a_captured_run_carries_the_themes_colour_for_that_capture() {
        let theme = Theme::dark();
        let scene = scene_of(&theme);
        let keyword = theme.token("keyword");
        assert!(
            scene
                .ops
                .iter()
                .any(|op| matches!(op, Op::Text { style, .. } if *style == keyword)),
            "no run carried the keyword colour"
        );
    }

    #[test]
    fn a_quote_rule_meter_and_fact_render_natively() {
        let view = Node::section("session")
            .child(
                Node::new("markdown.quote", Kind::Quote)
                    .child(Node::text("quote.body", [Span::plain("quoted")])),
            )
            .child(Node::new("markdown.rule", Kind::Rule))
            .child(Node::new(
                "value.meter",
                Kind::Meter {
                    label: "budget".into(),
                    value: 2.0,
                    max: 4.0,
                },
            ))
            .child(Node::new(
                "value.tokens",
                Kind::Fact {
                    value: misa_value::Value::Int(12_400),
                },
            ));
        let scene = scene(&view, &Theme::dark(), 40, 10, Layout::default());
        assert!(
            scene
                .ops
                .iter()
                .any(|op| matches!(op, Op::Text { text, .. } if text.contains("quoted")))
        );
        assert!(scene.ops.iter().any(|op| matches!(op, Op::Rect { .. })));
        assert!(
            scene
                .ops
                .iter()
                .any(|op| matches!(op, Op::Text { text, .. } if text.contains("12k")))
        );
    }

    #[test]
    fn the_scene_is_a_function_of_the_theme_and_changes_when_it_does() {
        let dark = scene_of(&Theme::dark());
        let plain = scene_of(&Theme::plain());
        let colours = |scene: &Scene| {
            scene
                .ops
                .iter()
                .filter_map(|op| match op {
                    Op::Text { style, .. } => Some(format!("{:?}", style.fg)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_ne!(
            colours(&dark),
            colours(&plain),
            "the theme made no difference"
        );
    }

    #[test]
    fn a_tall_view_is_clipped_to_the_rows_it_was_given() {
        let mut root = Node::section("session");
        for index in 0..200 {
            root = root.child(Node::text(
                "message.assistant",
                [Span::plain(format!("line {index}"))],
            ));
        }
        let scene = scene(&root, &Theme::dark(), 60, 10, Layout::default());
        let lowest = scene
            .ops
            .iter()
            .filter_map(|op| match op {
                Op::Text { y, .. } => Some(*y),
                _ => None,
            })
            .fold(0.0f32, f32::max);
        assert!(lowest < scene.height, "the scene drew past its own height");
    }

    #[test]
    fn the_pixel_frontend_decides_colour_and_the_session_did_not() {
        // The same tree, two themes, two appearances: no colour came from the view.
        let bright = Theme::plain().with_role("message.user", misa_render::Style::rgb(255, 0, 0));
        let scene = scene_of(&bright);
        assert!(scene.ops.iter().any(|op| matches!(
            op,
            Op::Text { style, .. } if style.fg == misa_render::Color::Rgb(255, 0, 0)
        )));
    }

    #[test]
    fn a_message_becomes_a_card_with_one_full_height_rail() {
        let theme = Theme::dark();
        let scene = scene_of(&theme);
        let surface = theme.surface("message.user").expect("a user surface");
        let card = scene
            .ops
            .iter()
            .position(|op| matches!(op, Op::Rect { style, .. } if style.fg == surface.bg))
            .expect("no message card background");
        let rail = scene
            .ops
            .iter()
            .position(|op| matches!(op, Op::Rect { width, .. } if *width == 2.0))
            .expect("no rail");
        let text = scene
            .ops
            .iter()
            .position(|op| matches!(op, Op::Text { text, .. } if text == "a question"))
            .expect("no message text");
        // The chrome is painted under the runs: card, then rail, then text.
        assert!(card < rail && rail < text, "{card} {rail} {text}");
    }

    #[test]
    fn a_scene_paints_to_a_png() {
        let scene = scene_of(&Theme::dark());
        let bytes = paint::png(&scene, misa_render::Color::Rgb(20, 22, 26)).expect("a png");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
        assert!(
            bytes.len() > 1000,
            "the raster is suspiciously small: {} bytes",
            bytes.len()
        );
    }
}
