//! Semantic node painting, inside the frame-scoped layout builder.
use super::layout::LayoutBuilder;
use super::retained::indicator_value;
use super::{
    CARD_PADDING_X, CARD_PADDING_Y, CARD_TRAILING, Control, FONT_SIZE, GUTTER, Hit, PARAGRAPH_GAP,
    RAIL,
};
use misa_pixel_ui::{Button, Checkbox, ComboBox, Op, ProgressBar, Rect, Scene, TextFlow};
use misa_proto::view::{FieldKind, Kind, Node};
use misa_render::Theme;
use misa_style::Style;

/// What a gutter band holds beside its block's content.
enum GutterMark {
    /// A quote's rail, spanning the quoted content.
    Rail,
    /// A disclosure mark; the bool is whether the block is open.
    Disclosure(bool),
}

/// A disclosure mark drawn from rectangles: a triangle of bars. No glyph can be
/// missing from a font when it is made of the same rectangles as everything
/// else.
fn chevron(scene: &mut Scene, x: f32, y: f32, open: bool, style: Style) {
    const UNIT: f32 = 2.0;
    for step in 0..3 {
        let extent = (step + 1) as f32 * UNIT;
        let offset = (2 - step) as f32 * UNIT;
        let (x, y, width, height) = if open {
            (x + offset, y + step as f32 * UNIT, extent, UNIT)
        } else {
            (x + step as f32 * UNIT, y + offset, UNIT, extent)
        };
        scene.ops.push(Op::Rect {
            x,
            y,
            width,
            height,
            style,
        });
    }
}

impl LayoutBuilder<'_> {
    /// The status bar: one row of selected indicator facts.
    #[allow(clippy::too_many_arguments)]
    fn indicators(
        &mut self,
        node: &Node,
        x: f32,
        y: &mut f32,
        width: f32,
        theme: &Theme,
        scene: &mut Scene,
    ) {
        let mut spans: Vec<(Style, String)> = Vec::new();
        let listed = match &node.kind {
            Kind::List { items, .. } => items.iter().flatten().collect::<Vec<_>>(),
            _ => Vec::new(),
        };
        for child in node.children.iter().chain(listed) {
            if !child.role.starts_with("indicator.") {
                continue;
            }
            if !spans.is_empty() {
                spans.push((theme.role("status.separator"), "  ".into()));
            }
            // The activity indicator is the client's animation: while a turn is
            // running it shows a moving frame instead of the word for the state.
            if child.role == "indicator.activity" {
                let value = indicator_value(child);
                if value != "ready"
                    && let Some(frame) = misa_render::animations::Registry::stock().frame(
                        "pulse",
                        true,
                        self.retained.phase(),
                    )
                {
                    spans.push((theme.role("indicator.activity"), frame.to_string()));
                    continue;
                }
            }
            if let Some(label) = child.label.as_deref().filter(|label| !label.is_empty()) {
                spans.push((theme.role("label"), label.to_string()));
                spans.push((theme.role("plain"), " ".into()));
            }
            spans.push((theme.role("value"), indicator_value(child)));
        }
        if !spans.is_empty() {
            self.row(scene, x, *y, width, spans);
            *y += self.line_height();
        }
    }

    /// The transcript boundary: a rule and the child facts beside it.
    #[allow(clippy::too_many_arguments)]
    fn group_footer(
        &mut self,
        node: &Node,
        x: f32,
        y: &mut f32,
        width: f32,
        theme: &Theme,
        scene: &mut Scene,
    ) {
        let style = theme.role("message.group.footer");
        scene.ops.push(Op::Rect {
            x,
            y: *y,
            width: width.max(1.0),
            height: 1.0,
            style,
        });
        *y += 4.0;
        let mut text = String::new();
        for child in &node.children {
            let value = match &child.kind {
                Kind::Fact { value } => misa_render::fact::format(&child.role, value),
                Kind::Text { spans } => spans.iter().map(|span| span.text.as_str()).collect(),
                _ => String::new(),
            };
            if !value.is_empty() {
                if !text.is_empty() {
                    text.push_str("  ");
                }
                text.push_str(&value);
            }
        }
        self.row(scene, x, *y, width, vec![(style, text)]);
        *y += self.line_height();
    }

    /// The pending-prompt dock: a heading and one row per queued prompt.
    #[allow(clippy::too_many_arguments)]
    fn queue(
        &mut self,
        node: &Node,
        x: f32,
        y: &mut f32,
        width: f32,
        theme: &Theme,
        scene: &mut Scene,
    ) {
        let count = node
            .children
            .iter()
            .find(|child| child.role == "queue.count")
            .and_then(|child| match &child.kind {
                Kind::Fact { value } => value.as_i64(),
                _ => None,
            })
            .unwrap_or_else(|| {
                node.children
                    .iter()
                    .filter(|child| child.role == "queue.item")
                    .count() as i64
            });
        self.row(
            scene,
            x,
            *y,
            width,
            vec![(theme.role("label"), format!("Queued ({count})"))],
        );
        *y += self.line_height();
        for child in node
            .children
            .iter()
            .filter(|child| child.role == "queue.item")
        {
            if let Kind::Text { spans } = &child.kind {
                let text: String = spans.iter().map(|span| span.text.as_str()).collect();
                self.row(
                    scene,
                    x,
                    *y,
                    width,
                    vec![(
                        theme.role("dim"),
                        self.clip(
                            &text.replace('\n', " ↵ "),
                            (width - self.gutter_width()).max(0.0),
                        ),
                    )],
                );
                *y += self.line_height();
            }
        }
    }

    pub(super) fn node_uncached(
        &mut self,
        node: &Node,
        x: f32,
        y: &mut f32,
        width: f32,
        theme: &Theme,
        scene: &mut Scene,
    ) {
        if matches!(
            node.role.as_str(),
            "status.indicators" | "message.group.footer" | "queue"
        ) {
            self.paint_self(node, x, y, width, theme, scene);
            return;
        }
        #[cfg(test)]
        {
            self.retained.node_rendered();
        }
        // A railed block is a card: remember where its ops begin so the surface and
        // the full-height rail can be painted behind them once its extent is known.
        let card = theme
            .rail(&node.role)
            .map(|(_, rail_style)| (scene.ops.len(), *y, rail_style));
        let trailing = if card.is_some() {
            CARD_TRAILING
        } else {
            PARAGRAPH_GAP
        };
        let mark_start = *y;
        let (children, mark) = self.paint_self(node, x, y, width, theme, scene);
        if children {
            self.paint_children(node, x, y, width, theme, scene);
        }
        if let Some(mark) = mark {
            self.finish_gutter(mark, x, mark_start, y, scene, &node.id);
        }
        // Actions belong to this owner, but historically follow its children.
        self.paint_self_actions(node, x, y, width, scene);
        *y += trailing;
        if let Some((start, first, rail_style)) = card {
            let last = *y - trailing;
            if last > first {
                let height = last - first;
                let mut at = start;
                if let Some(surface) = theme.surface(&node.role)
                    && surface.bg != misa_style::Color::Default
                {
                    scene.ops.insert(
                        at,
                        Op::Rect {
                            x: x - CARD_PADDING_X,
                            y: first - CARD_PADDING_Y,
                            width: (width + 2.0 * CARD_PADDING_X)
                                .min((scene.width - x + CARD_PADDING_X).max(1.0)),
                            height: height + 2.0 * CARD_PADDING_Y,
                            style: Style::fg(surface.bg),
                        },
                    );
                    at += 1;
                }
                scene.ops.insert(
                    at,
                    Op::Rect {
                        x: x - CARD_PADDING_X + 4.0,
                        y: first,
                        width: RAIL,
                        height,
                        style: rail_style,
                    },
                );
            }
        }
    }

    /// Rails and disclosure marks live in the gutter band beside the content:
    /// drawn geometry, so no font can be missing them.
    fn finish_gutter(
        &mut self,
        mark: GutterMark,
        x: f32,
        start: f32,
        y: &mut f32,
        scene: &mut Scene,
        id: &str,
    ) {
        let level = self.gutters.len();
        let style = self.gutters.pop().expect("gutter style");
        let band = x + (level - 1) as f32 * GUTTER;
        match mark {
            GutterMark::Rail => scene.ops.push(Op::Rect {
                x: band + 4.0,
                y: start,
                width: RAIL,
                height: (*y - start).max(0.0),
                style,
            }),
            GutterMark::Disclosure(open) => {
                chevron(
                    scene,
                    band + 3.0,
                    start + (self.line_height() - 6.0) / 2.0,
                    open,
                    style,
                );
                if open {
                    // The mark's band collapses the block; its body stays
                    // selectable text everywhere else.
                    self.interaction.add_hit(Hit {
                        x: band,
                        y: start,
                        width: GUTTER,
                        height: (*y - start).max(0.0),
                        control: Control::Disclosure(id.to_owned()),
                    });
                }
            }
        }
    }

    /// Paint the node's own label and kind. Children are a separate traversal;
    /// a gutter mark stays active through that traversal.
    fn paint_self(
        &mut self,
        node: &Node,
        x: f32,
        y: &mut f32,
        width: f32,
        theme: &Theme,
        scene: &mut Scene,
    ) -> (bool, Option<GutterMark>) {
        if matches!(
            node.role.as_str(),
            "status.indicators" | "message.group.footer" | "queue"
        ) {
            // A registered composite owns its subtree, including the indexed children.
            let model = self
                .document
                .subtree(&node.id)
                .unwrap_or_else(|| node.clone());
            if node.role == "status.indicators" {
                self.retained.observe_status(&model);
            }
            match node.role.as_str() {
                "status.indicators" => self.indicators(&model, x, y, width, theme, scene),
                "message.group.footer" => self.group_footer(&model, x, y, width, theme, scene),
                _ => self.queue(&model, x, y, width, theme, scene),
            }
            return (false, None);
        }
        // A disclosure names itself in its own title row, beside a mark.
        if let Some(label) = &node.label
            && !matches!(node.kind, Kind::Collapsible { .. })
        {
            self.row(
                scene,
                x,
                *y,
                width,
                vec![(theme.role(&node.role), label.clone())],
            );
            *y += 25.0;
        }
        let mut children = true;
        let mut mark = None;
        match &node.kind {
            Kind::Section => {}
            Kind::Collapsible { summary } => {
                let open = self.interaction.is_expanded(&node.id);
                let control = Control::Disclosure(node.id.clone());
                let style = theme.role(&node.role);
                if let Some(label) = &node.label {
                    // A labelled disclosure (a tool call) names itself in a
                    // title row. Its summary is the short form: it never shows
                    // beside the body it previews.
                    chevron(scene, x + 4.0, *y + 9.0, open, style);
                    self.row(
                        scene,
                        x + GUTTER,
                        *y,
                        (width - GUTTER).max(0.0),
                        vec![(style, label.clone())],
                    );
                    self.interaction.add_hit(Hit {
                        x,
                        y: *y,
                        width,
                        height: 25.0,
                        control: control.clone(),
                    });
                    *y += 25.0;
                    if !open {
                        self.wrapped(scene, x, y, width, summary, style, theme);
                    }
                } else {
                    // A label-less short form is its own block: the preview is
                    // its content, the long form replaces that content, and the
                    // gutter holds the mark that opens and closes it.
                    self.gutters.push(style);
                    mark = Some(GutterMark::Disclosure(open));
                    if !open {
                        let start = *y;
                        self.wrapped(scene, x, y, width, summary, style, theme);
                        self.interaction.add_hit(Hit {
                            x,
                            y: start,
                            width,
                            height: (*y - start).max(0.0),
                            control,
                        });
                    }
                }
                children = open;
            }
            Kind::Fields { fields } => {
                for field in fields {
                    self.row(
                        scene,
                        x,
                        *y,
                        width,
                        vec![(theme.role("field.label"), field.label.clone())],
                    );
                    *y += 22.0;
                    let default = match &field.kind {
                        FieldKind::Choice {
                            selected: Some(value),
                            ..
                        } => value,
                        _ => &field.value,
                    };
                    let value = self
                        .drafts
                        .text(&node.id, &field.id)
                        .unwrap_or(default)
                        .to_string();
                    // Keep the form's label row and spacing. A boolean's value is
                    // painted by the control, not by an editor with a fake glyph.
                    // Secret fields still take the masked text path below.
                    if field.kind == FieldKind::Bool && !field.secret {
                        let control = Control::Field {
                            node: node.id.clone(),
                            field: field.id.clone(),
                        };
                        let height = (self.line_height() + 12.0).max(32.0);
                        let focused = !field.read_only && self.interaction.focused(&control);
                        let placed = Checkbox {
                            id: control,
                            bounds: Rect {
                                x,
                                y: *y,
                                width,
                                height,
                            },
                            label: String::new(), // Already painted above as a form label.
                            checked: value == "true",
                            focused,
                            font_size: FONT_SIZE,
                            background: self.colors.field,
                            foreground: self.colors.text,
                            accent: self.colors.accent,
                        }
                        .place(self.metrics)
                        .widget;
                        if !field.read_only {
                            self.interaction.add_hit(Hit {
                                x: placed.bounds.x,
                                y: placed.bounds.y,
                                width: placed.bounds.width,
                                height: placed.bounds.height,
                                control: placed.id,
                            });
                        }
                        scene.ops.extend(placed.ops);
                        *y += height + 9.0;
                        continue;
                    }
                    if let FieldKind::Choice { options, .. } = &field.kind
                        && !field.secret
                        && !field.read_only
                    {
                        let control = Control::Field {
                            node: node.id.clone(),
                            field: field.id.clone(),
                        };
                        let height = (self.line_height() + 12.0).max(32.0);
                        let choices = super::ChoiceRows(options);
                        let placed = ComboBox {
                            options: &choices,
                            selected: Some(&value),
                            bounds: Rect {
                                x,
                                y: *y,
                                width,
                                height,
                            },
                            viewport: Rect {
                                x: 0.0,
                                y: 0.0,
                                width,
                                height: 0.0,
                            },
                            font_size: FONT_SIZE,
                            row_height: 28.0,
                            focused: self.interaction.focused(&control),
                            enabled: true,
                            background: self.colors.field,
                            foreground: self.colors.text,
                            muted: self.colors.muted,
                            border: self.colors.border,
                            highlight: self.colors.accent,
                        }
                        .place(self.metrics);
                        self.interaction.add_hit(Hit {
                            x,
                            y: *y,
                            width,
                            height,
                            control,
                        });
                        scene.ops.extend(placed.ops);
                        *y += height + 9.0;
                        continue;
                    }
                    let display = if field.secret {
                        "•".repeat(value.chars().count())
                    } else {
                        match &field.kind {
                            FieldKind::Choice { options, .. } => format!(
                                "{} ▾",
                                options
                                    .iter()
                                    .find(|choice| choice.value == value)
                                    .map(|choice| choice.label.as_str())
                                    .unwrap_or(&value)
                            ),
                            _ => value.clone(),
                        }
                    };
                    let h = if field.kind == FieldKind::Block {
                        (display.lines().count().max(3) as f32 * self.line_height() + 12.0)
                            .min(180.0)
                    } else {
                        (self.line_height() + 12.0).max(32.0)
                    };
                    if field.read_only {
                        for line in display.lines() {
                            self.row(
                                scene,
                                x,
                                *y,
                                width,
                                vec![(theme.role("value.text"), line.into())],
                            );
                            *y += self.line_height();
                        }
                    } else {
                        self.box_control(
                            scene,
                            x,
                            *y,
                            width,
                            h,
                            &display,
                            Control::Field {
                                node: node.id.clone(),
                                field: field.id.clone(),
                            },
                        );
                        *y += h;
                    }
                    *y += 9.0;
                }
            }
            Kind::Definition { entries } => {
                // A term on its own row, then each definition behind a quiet marker.
                for entry in entries {
                    let term = entry
                        .term
                        .iter()
                        .map(|span| span.text.as_str())
                        .collect::<String>();
                    self.row(
                        scene,
                        x,
                        *y,
                        width,
                        vec![(theme.role("markdown.definition.term"), term)],
                    );
                    *y += 22.0;
                    for definition in &entry.definitions {
                        let body = definition
                            .iter()
                            .map(|span| span.text.as_str())
                            .collect::<String>();
                        self.row(
                            scene,
                            x + 16.0,
                            *y,
                            (width - 16.0).max(0.0),
                            vec![(theme.role(&node.role), format!("• {body}"))],
                        );
                        *y += self.line_height();
                    }
                    *y += 6.0;
                }
            }
            Kind::Meter { label, value, max } => {
                self.row(
                    scene,
                    x,
                    *y,
                    width,
                    vec![(
                        theme.role(&node.role),
                        format!("{label}: {value:.1} / {max:.1}"),
                    )],
                );
                *y += 24.0;
                let fraction = if max.is_finite() && *max > 0.0 && value.is_finite() {
                    (value / max).clamp(0.0, 1.0) as f32
                } else {
                    0.0
                };
                scene.ops.extend(
                    ProgressBar {
                        id: node.id.clone(),
                        bounds: Rect {
                            x,
                            y: *y,
                            width,
                            height: 10.0,
                        },
                        fraction,
                        background: self.colors.meter,
                        foreground: self.colors.accent,
                    }
                    .place()
                    .ops,
                );
                *y += 22.0;
            }
            Kind::Table { head, rows, .. } => {
                let columns = if self.document.contains(&node.id) {
                    self.document.table_columns(&node.id)
                } else {
                    super::document::table_column_count(head, rows)
                };
                for (index, row) in std::iter::once(head).chain(rows.iter()).enumerate() {
                    self.table_row(row, index, columns, x, y, width, theme, scene);
                }
            }
            Kind::List { .. } => {}
            Kind::Image { blob, alt, .. } => {
                if let Some(image) = self.document.image_ref(&blob.hash) {
                    let scale = (width / image.width() as f32)
                        .min(320.0 / image.height() as f32)
                        .min(1.0);
                    let (w, h) = (image.width() as f32 * scale, image.height() as f32 * scale);
                    scene.ops.push(Op::Image {
                        x,
                        y: *y,
                        width: w,
                        height: h,
                        image: image.clone(),
                    });
                    *y += h + 6.0;
                } else {
                    self.box_control(
                        scene,
                        x,
                        *y,
                        width.min(180.0),
                        30.0,
                        "Load image",
                        Control::LoadImage(blob.clone()),
                    );
                    *y += 36.0;
                }
                self.row(
                    scene,
                    x,
                    *y,
                    width,
                    vec![(theme.role(&node.role), alt.clone())],
                );
                *y += 25.0;
            }
            Kind::Text { spans } => {
                let base = theme.role(&node.role);
                self.wrapped(scene, x, y, width, spans, base, theme);
            }
            Kind::Heading { level, spans } => {
                let mut base = theme.role(&node.role).bold();
                if *level == 1 {
                    base = base.underline();
                }
                self.wrapped(scene, x, y, width, spans, base, theme);
            }
            Kind::Quote => {
                // A quote is a quote, not a row of glyphs: its rail is drawn
                // beside the content, and copied text stays content.
                self.gutters.push(theme.role(&node.role).dim());
                mark = Some(GutterMark::Rail);
            }
            Kind::Rule => {
                // A rule is a line, not a row of dashes.
                let inset = self.gutter_width();
                scene.ops.push(Op::Rect {
                    x: x + inset,
                    y: *y + (self.line_height() - 1.0) / 2.0,
                    width: (width - inset).max(0.0),
                    height: 1.0,
                    style: theme.role(&node.role).dim(),
                });
                *y += self.line_height();
            }
            Kind::Code { lang, text } => {
                self.code_block(node, lang.as_deref(), text, x, y, width, theme, scene);
            }
            Kind::Fact { value } => {
                self.row(
                    scene,
                    x,
                    *y,
                    width,
                    vec![(
                        theme.role(&node.role),
                        misa_render::fact::format(&node.role, value),
                    )],
                );
                *y += self.line_height();
            }
            Kind::Status { text } => {
                self.row(
                    scene,
                    x,
                    *y,
                    width,
                    vec![(theme.role(&node.role), text.clone())],
                );
                *y += self.line_height();
            }
        }
        (children, mark)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn table_row(
        &mut self,
        row: &[Vec<misa_proto::view::Span>],
        index: usize,
        columns: usize,
        x: f32,
        y: &mut f32,
        width: f32,
        theme: &Theme,
        scene: &mut Scene,
    ) {
        let cell_width = width / columns as f32;
        let mut source_offset = self.interaction.source_cursor();
        if source_offset > 0 {
            source_offset += 1;
        }
        let base = theme.role("table.cell");
        let cells: Vec<_> = row
            .iter()
            .map(|cell| {
                let start = source_offset;
                let runs: Vec<_> = cell
                    .iter()
                    .map(|span| (crate::span_style(theme, span, base), span.text.clone()))
                    .collect();
                source_offset += runs
                    .iter()
                    .map(|(_, text)| text.chars().count())
                    .sum::<usize>()
                    + 1;
                (
                    start,
                    TextFlow::new(self.metrics, FONT_SIZE)
                        .wrap_with_ranges(runs, (cell_width - 10.0 - self.gutter_width()).max(0.0)),
                )
            })
            .collect();
        let lines = cells
            .iter()
            .map(|(_, rows)| rows.len())
            .max()
            .unwrap_or(1)
            .max(1);
        let height = lines as f32 * self.line_height() + 8.0;
        scene.ops.push(Op::Rect {
            x,
            y: *y,
            width,
            height,
            style: if index == 0 {
                self.colors.selected_button
            } else {
                self.colors.button
            },
        });
        for (column, (start, cell)) in cells.into_iter().enumerate() {
            let mut previous_end = 0;
            for (line, (content, offset, end)) in cell.into_iter().enumerate() {
                // Only a new cell or an explicit source break separates
                // logical rows; a soft wrap continues this cell's text.
                self.interaction
                    .next_source(start + offset, line > 0 && offset == previous_end);
                previous_end = end;
                self.row(
                    scene,
                    x + column as f32 * cell_width + 5.0,
                    *y + 4.0 + line as f32 * self.line_height(),
                    (cell_width - 10.0).max(0.0),
                    content,
                );
            }
        }
        *y += height + 2.0;
        self.interaction
            .finish_source(source_offset.saturating_sub(1));
    }

    /// Emit indexed owner groups and embedded children in their original order.
    fn paint_children(
        &mut self,
        node: &Node,
        x: f32,
        y: &mut f32,
        width: f32,
        theme: &Theme,
        scene: &mut Scene,
    ) {
        if let Kind::List {
            ordered,
            items,
            markers,
        } = &node.kind
        {
            for (index, item) in items.iter().enumerate() {
                self.list_item(
                    node,
                    *ordered,
                    markers.get(index).copied().flatten(),
                    index,
                    item,
                    x,
                    y,
                    width,
                    theme,
                    scene,
                );
            }
        }
        for id in self.document.children(&node.id) {
            self.present(&id, x, y, width, theme, scene);
        }
        for child in &node.children {
            self.node_uncached(child, x, y, width, theme, scene);
        }
        if node.id == self.document.stream_parent() && !self.document.visible_streams().is_empty() {
            self.present("streams", x, y, width, theme, scene);
        }
        if node.id == "streams" {
            let ids = self.document.visible_streams();
            for id in ids {
                self.present(&id, x, y, width, theme, scene);
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn list_item(
        &mut self,
        node: &Node,
        ordered: bool,
        marker: Option<bool>,
        index: usize,
        item: &[Node],
        x: f32,
        y: &mut f32,
        width: f32,
        theme: &Theme,
        scene: &mut Scene,
    ) {
        self.row(
            scene,
            x,
            *y,
            width,
            vec![(
                theme.role(&node.role),
                match marker {
                    Some(true) => "☑".into(),
                    Some(false) => "☐".into(),
                    None if ordered => format!("{}.", index + 1),
                    None => "•".into(),
                },
            )],
        );
        for child in item {
            self.node_uncached(child, x + 25.0, y, (width - 25.0).max(10.0), theme, scene);
        }
    }

    pub(super) fn paint_self_actions(
        &mut self,
        node: &Node,
        x: f32,
        y: &mut f32,
        width: f32,
        scene: &mut Scene,
    ) {
        for action in &node.actions {
            let control = Control::Action {
                node: node.id.clone(),
                action: action.id.clone(),
            };
            let focused = self.interaction.focused(&control);
            let button = Button {
                id: control,
                bounds: Rect {
                    x,
                    y: *y,
                    width,
                    height: 32.0,
                },
                label: action.label.as_deref().unwrap_or(&action.id).into(),
                font_size: FONT_SIZE,
                background: self.colors.field,
                foreground: self.colors.text,
            }
            .place(self.metrics);
            // The outline follows the button, which grows with its label.
            scene.ops.push(Op::Rect {
                x: x - 1.0,
                y: *y - 1.0,
                width: button.bounds.width + 2.0,
                height: 34.0,
                style: if focused {
                    self.colors.accent
                } else {
                    self.colors.border
                },
            });
            scene.ops.extend(button.ops);
            self.interaction.add_hit(Hit {
                x: button.bounds.x,
                y: button.bounds.y,
                width: button.bounds.width,
                height: button.bounds.height,
                control: button.id,
            });
            *y += 39.0;
        }
    }
}
