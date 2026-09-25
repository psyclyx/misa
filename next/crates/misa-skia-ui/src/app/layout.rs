use super::{
    App, Cached, Control, FONT_SIZE, FieldViewport, Hit, IndicatorBounds, PULSE_PERIOD, TextRow,
    pulse_phase, text,
};
use misa_pixel_ui::{Button, Op, Rect, Scene};
use misa_proto::view::{FieldKind, Kind, Node};
use misa_render::Theme;
use misa_style::Style;
use std::sync::Arc;
use std::time::Duration;

impl App {
    fn present(
        &mut self,
        id: &str,
        x: f32,
        y: &mut f32,
        width: f32,
        theme: &Theme,
        scene: &mut Scene,
    ) {
        let cached = if let Some(cached) = self.cache.get(id).filter(|cached| cached.width == width)
        {
            cached.clone()
        } else {
            let node = if id == "streams" {
                Node::section("streams").id("streams")
            } else if let Some(node) = self.document.stream_or_node(id) {
                node.clone()
            } else {
                return;
            };
            let outer_hits = std::mem::take(&mut self.hits);
            let outer_rows = std::mem::take(&mut self.rows);
            let mut local = Scene::default();
            let mut height = 0.0;
            self.indicator_stack.push(Vec::new());
            self.node_uncached(&node, 0.0, &mut height, width, theme, &mut local);
            let mut indicators = self.indicator_stack.pop().unwrap();
            if node.role == "status.indicators" && self.moving_indicators.contains(id) {
                indicators.push(IndicatorBounds {
                    id: id.to_string(),
                    top: 0.0,
                    bottom: height,
                });
            }
            let cached = Arc::new(Cached {
                width,
                height,
                ops: Arc::new(local.ops),
                hits: std::mem::replace(&mut self.hits, outer_hits),
                rows: std::mem::replace(&mut self.rows, outer_rows),
                indicators,
                phase: (node.role == "status.indicators" && self.moving_indicators.contains(id))
                    .then_some(self.tick),
            });
            self.cache.insert(id.to_string(), cached.clone());
            cached
        };
        if let Some(parent) = self.indicator_stack.last_mut() {
            parent.extend(cached.indicators.iter().map(|bounds| IndicatorBounds {
                id: bounds.id.clone(),
                top: bounds.top + *y,
                bottom: bounds.bottom + *y,
            }));
        }
        scene.ops.push(Op::Group {
            x,
            y: *y,
            ops: cached.ops.clone(),
        });
        let base = self.rows.len();
        self.rows.extend(cached.rows.iter().map(|row| TextRow {
            x: row.x + x,
            y: row.y + *y,
            width: row.width,
            geometry: Arc::clone(&row.geometry),
        }));
        self.hits.extend(cached.hits.iter().map(|hit| {
            let mut hit = hit.clone();
            hit.x += x;
            hit.y += *y;
            if let Control::Text(row) = &mut hit.control {
                *row += base;
            }
            hit
        }));
        *y += cached.height;
    }
    /// Offline snapshots advance a synthetic clock one pulse per call, without wall time.
    pub fn frame(&mut self, width: u32, height: u32) -> Scene {
        self.offline_elapsed = self.offline_elapsed.saturating_add(PULSE_PERIOD);
        self.frame_at(width, height, self.offline_elapsed)
    }

    /// Render at a caller-provided elapsed time. Repaints within a phase retain the
    /// same display lists; only the animated indicator's owner and ancestors change.
    pub fn frame_at(&mut self, width: u32, height: u32, elapsed: Duration) -> Scene {
        self.tick = pulse_phase(elapsed);
        if self.cache_width != width {
            self.cache.clear();
            self.moving_indicators.clear();
            self.cache_width = width;
        }
        #[cfg(test)]
        {
            self.rendered_nodes = 0;
        }
        self.viewport_height = height as f32;
        let mut scene = self.layout(width, height);
        let max = (self.content_height - height as f32 + 40.0).max(0.0);
        let wanted = if self.follow {
            max
        } else {
            self.scroll.min(max)
        };
        if (wanted - self.scroll).abs() > 0.5 {
            self.scroll = wanted;
            scene = self.layout(width, height);
        }
        // Lay out first: the final scroll and collapsed groups determine visibility.
        // An idle frame reuses its display lists and checks only moving owner bounds.
        let stale: Vec<_> = self
            .visible_indicators()
            .filter(|id| self.cache[*id].phase != Some(self.tick))
            .map(str::to_owned)
            .collect();
        if !stale.is_empty() {
            for id in stale {
                self.invalidate(&id);
            }
            scene = self.layout(width, height);
        }
        let colors = self.colors();
        if let Some(mut report) = self.report.take() {
            self.hits.clear();
            let budget = (width as f32 - 96.0).max(1.0);
            if report.width != budget {
                report.lines.clear();
                for entry in &report.entries {
                    let runs = self.wrap_runs(vec![(colors.text, entry.clone())], budget);
                    report.lines.extend(
                        runs.into_iter()
                            .map(|line| line.into_iter().map(|(_, text)| text).collect()),
                    );
                }
                report.width = budget;
            }
            let spacing = self.line_height() + 3.0;
            let visible = ((height as f32 - 140.0).max(spacing) / spacing).floor() as usize;
            report.offset = report
                .offset
                .min(report.lines.len().saturating_sub(visible));
            scene.ops.push(Op::Rect {
                x: 24.0,
                y: 24.0,
                width: (width as f32 - 48.0).max(1.0),
                height: (height as f32 - 48.0).max(1.0),
                style: colors.surface,
            });
            scene.ops.push(text(
                42.0,
                40.0,
                &self.clip(&report.title, budget),
                colors.text,
            ));
            for (index, line) in report
                .lines
                .iter()
                .skip(report.offset)
                .take(visible)
                .enumerate()
            {
                scene
                    .ops
                    .push(text(42.0, 78.0 + index as f32 * spacing, line, colors.text));
            }
            scene.ops.push(text(
                42.0,
                (height as f32 - 52.0).max(0.0),
                "↑↓ scroll · Escape close",
                colors.muted,
            ));
            self.report = Some(report);
        }
        scene
    }
    fn layout(&mut self, width: u32, height: u32) -> Scene {
        self.hits.clear();
        self.rows.clear();
        let mut scene = Scene {
            width: width as f32,
            height: height as f32,
            ops: vec![],
        };
        let root = self.document.root().to_string();
        let theme = if self.light {
            Theme::light()
        } else {
            Theme::dark()
        };
        let mut y = 20.0 - self.scroll;
        self.present(
            &root,
            20.0,
            &mut y,
            (width as f32 - 40.0).max(40.0),
            &theme,
            &mut scene,
        );
        self.content_height = y + self.scroll + 20.0;
        if let Some((a, b)) = self.selection {
            let (start, end) = if a <= b { (a, b) } else { (b, a) };
            for (index, row) in self
                .rows
                .iter()
                .enumerate()
                .skip(start.0)
                .take(end.0.saturating_sub(start.0) + 1)
            {
                let from = if index == start.0 { start.1 } else { 0 };
                let to = if index == end.0 { end.1 } else { usize::MAX };
                let from = from.min(row.geometry.text.chars().count());
                let to = to.min(row.geometry.text.chars().count());
                if to <= from {
                    continue;
                }
                let mut selected = vec![Op::Rect {
                    x: row.x + row.edge(from),
                    y: row.y,
                    width: row.edge(to) - row.edge(from),
                    height: self.line_height(),
                    style: self.colors().selection,
                }];
                for (style, run, start) in &row.geometry.runs {
                    let end = start + run.chars().count();
                    let left = from.max(*start);
                    let right = to.min(end);
                    if left < right {
                        let value: String =
                            run.chars().skip(left - start).take(right - left).collect();
                        selected.push(text(row.x + row.edge(left), row.y, &value, *style));
                    }
                }
                scene.ops.push(Op::ClipRect {
                    x: row.x,
                    y: row.y,
                    width: row.width,
                    height: self.line_height(),
                    ops: Arc::new(selected),
                });
            }
        }
        if !self.notice.is_empty() {
            scene.ops.push(Op::Rect {
                x: 0.0,
                y: height as f32 - 26.0,
                width: width as f32,
                height: 26.0,
                style: self.colors().surface,
            });
            scene.ops.push(text(
                12.0,
                height as f32 - 23.0,
                &self.clip(&self.notice, (width as f32 - 24.0).max(1.0)),
                self.colors().text,
            ));
        }
        if let Some((_, edit)) = &self.save {
            let path = edit.text().to_string();
            let x = 30.0;
            let y = (height as f32 / 2.0 - 70.0).max(20.0);
            let w = (width as f32 - 60.0).max(80.0);
            self.hits.clear();
            scene.ops.push(Op::Rect {
                x,
                y,
                width: w,
                height: 150.0,
                style: self.colors().surface,
            });
            scene.ops.push(text(
                x + 12.0,
                y + 10.0,
                &self.clip("Save attachment · local destination", (w - 24.0).max(1.0)),
                self.colors().text,
            ));
            self.box_control(
                &mut scene,
                x + 12.0,
                y + 40.0,
                w - 24.0,
                32.0,
                &path,
                Control::SavePath,
            );
            self.box_control(
                &mut scene,
                x + 12.0,
                y + 92.0,
                95.0,
                32.0,
                "Save",
                Control::SaveConfirm,
            );
            self.box_control(
                &mut scene,
                x + 120.0,
                y + 92.0,
                95.0,
                32.0,
                "Cancel",
                Control::SaveCancel,
            );
        }
        if let Some(picker) = &self.picker {
            self.hits.clear();
            let y = 35.0;
            scene.ops.push(Op::Rect {
                x: 30.0,
                y,
                width: (width as f32 - 60.0).max(80.0),
                height: 300.0,
                style: self.colors().surface,
            });
            scene.ops.push(text(
                42.0,
                y + 12.0,
                &self.clip(
                    &format!("Commands · {}", picker.query),
                    (width as f32 - 84.0).max(1.0),
                ),
                self.colors().text,
            ));
            let matches = picker.matches();
            let start = picker.selected_index().saturating_sub(7);
            if matches.is_empty() {
                scene.ops.push(text(
                    42.0,
                    y + 48.0,
                    "No matching commands",
                    self.colors().muted,
                ));
            }
            for (index, candidate) in matches.iter().enumerate().skip(start).take(8) {
                let label = format!(
                    "{} /{} · {}",
                    if index == picker.selected_index() {
                        ">"
                    } else {
                        " "
                    },
                    candidate.value,
                    candidate.label
                );
                scene.ops.push(text(
                    42.0,
                    y + 48.0 + (index - start) as f32 * 26.0,
                    &self.clip(&label, (width as f32 - 84.0).max(1.0)),
                    self.colors().text,
                ));
            }
            scene.ops.push(text(
                42.0,
                y + 270.0,
                "↑↓ select · Enter insert · Escape close",
                self.colors().muted,
            ));
        }
        scene
    }
    fn box_control(
        &mut self,
        scene: &mut Scene,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        label: &str,
        control: Control,
    ) {
        let focused = self.focus.as_ref() == Some(&control);
        scene.ops.push(Op::Rect {
            x: x - 1.0,
            y: y - 1.0,
            width: width + 2.0,
            height: height + 2.0,
            style: if focused {
                self.colors().accent
            } else {
                self.colors().border
            },
        });
        scene.ops.push(Op::Rect {
            x,
            y,
            width,
            height,
            style: self.colors().field,
        });
        let line_height = self.line_height();
        let paint_width = (width - 14.0).max(0.0);
        let paint_height = (height - 9.0).max(0.0);
        let visible_lines = (paint_height / line_height).floor().max(1.0) as usize;
        let (cursor, mut viewport) = match &control {
            Control::Field { node, field } => {
                let field_model = self.document.field(node, field);
                let cursor = self
                    .drafts
                    .cursor(node, field)
                    .filter(|_| {
                        !field_model.is_some_and(|value| {
                            matches!(value.kind, FieldKind::Bool | FieldKind::Choice { .. })
                        })
                    })
                    .map(|at| {
                        if field_model.is_some_and(|value| value.secret) {
                            // The displayed run has one bullet per scalar, including any newline.
                            let column = self.drafts.text(node, field).unwrap_or("")[..at]
                                .chars()
                                .count();
                            label
                                .char_indices()
                                .nth(column)
                                .map_or(label.len(), |(at, _)| at)
                        } else {
                            at
                        }
                    });
                (cursor, self.drafts.viewport(node, field))
            }
            Control::SavePath => (
                self.save.as_ref().map(|(_, edit)| edit.cursor()),
                self.save_viewport,
            ),
            _ => (None, FieldViewport::default()),
        };
        let cursor_line = if focused {
            cursor.map(|at| {
                let before = &label[..at];
                let line = before.bytes().filter(|byte| *byte == b'\n').count();
                let column = before.rsplit('\n').next().unwrap_or("").chars().count();
                (line, column)
            })
        } else {
            None
        };
        let lines: Vec<_> = label.split('\n').collect();
        let mut cursor_edge = None;
        if let Some((line, column)) = cursor_line {
            if line < viewport.line {
                viewport.line = line;
            } else if line >= viewport.line.saturating_add(visible_lines) {
                viewport.line = line + 1 - visible_lines;
            }
            // Measure the same complete run that will be painted, not the unmasked draft
            // or repeatedly measured character prefixes. Keep the cursor in the viewport.
            let advances = self.metrics.advances(lines[line], FONT_SIZE);
            let edge = advances[column];
            cursor_edge = Some(edge);
            viewport.x = viewport.x.min(edge);
            viewport.x = viewport.x.max(edge - (paint_width - 1.5).max(0.0));
            match &control {
                Control::Field { node, field } => {
                    self.drafts.set_viewport(node, field, viewport);
                }
                Control::SavePath => self.save_viewport = viewport,
                _ => {}
            }
        }
        let mut painted = Vec::new();
        for (line, value) in lines
            .iter()
            .enumerate()
            .skip(viewport.line)
            .take(visible_lines)
        {
            painted.push(text(
                x + 7.0 - viewport.x,
                y + 6.0 + (line - viewport.line) as f32 * line_height,
                value,
                self.colors().text,
            ));
        }
        if let Some((line, _)) = cursor_line {
            painted.push(Op::Rect {
                x: x + 7.0 + cursor_edge.unwrap() - viewport.x,
                y: y + 6.0 + (line - viewport.line) as f32 * line_height,
                width: 1.5,
                height: line_height,
                style: self.colors().text,
            });
        }
        scene.ops.push(Op::ClipRect {
            x: x + 7.0,
            y: y + 6.0,
            width: paint_width,
            height: paint_height,
            ops: Arc::new(painted),
        });
        self.hits.push(Hit {
            x,
            y,
            width,
            height,
            control,
        });
    }
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
        for child in &node.children {
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
                    && let Some(frame) =
                        misa_render::animations::Registry::stock().frame("pulse", true, self.tick)
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
                            (width - self.prefix_width()).max(0.0),
                        ),
                    )],
                );
                *y += self.line_height();
            }
        }
    }

    fn node_uncached(
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
            // Indexed nodes contain no children. A registered composite owns its
            // subtree, so materialize that subtree only when its cache is dirty.
            let model = self
                .document
                .subtree(&node.id)
                .unwrap_or_else(|| node.clone());
            if node.role == "status.indicators" {
                if model.children.iter().any(|child| {
                    child.role == "indicator.activity" && indicator_value(child) != "ready"
                }) {
                    self.moving_indicators.insert(node.id.clone());
                } else {
                    self.moving_indicators.remove(&node.id);
                }
            }
            match node.role.as_str() {
                "status.indicators" => self.indicators(&model, x, y, width, theme, scene),
                "message.group.footer" => self.group_footer(&model, x, y, width, theme, scene),
                _ => self.queue(&model, x, y, width, theme, scene),
            }
            return;
        }
        #[cfg(test)]
        {
            self.rendered_nodes += 1;
        }
        // A railed block is a card: remember where its ops begin so the surface and
        // the full-height rail can be painted behind them once its extent is known.
        let card = theme
            .rail(&node.role)
            .map(|(_, rail_style)| (scene.ops.len(), *y, rail_style));
        if let Some(label) = &node.label {
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
        let mut quote_prefix = false;
        match &node.kind {
            Kind::Section => {}
            Kind::Collapsible { summary } => {
                let open = self.expanded.contains(&node.id);
                let label = format!(
                    "{} {}",
                    if open { "▾" } else { "▸" },
                    summary
                        .iter()
                        .map(|span| span.text.as_str())
                        .collect::<String>()
                );
                self.box_control(
                    scene,
                    x,
                    *y,
                    width,
                    28.0,
                    &label,
                    Control::Disclosure(node.id.clone()),
                );
                *y += 34.0;
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
                    let value = self
                        .field_text(&node.id, &field.id)
                        .unwrap_or(&field.value)
                        .to_string();
                    let display = if field.secret {
                        "•".repeat(value.chars().count())
                    } else {
                        match &field.kind {
                            FieldKind::Bool => {
                                format!("[{}]", if value == "true" { "✓" } else { " " })
                            }
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
                scene.ops.push(Op::Rect {
                    x,
                    y: *y,
                    width,
                    height: 10.0,
                    style: self.colors().meter,
                });
                scene.ops.push(Op::Rect {
                    x,
                    y: *y,
                    width: width
                        * if *max > 0.0 {
                            (value / max).clamp(0.0, 1.0) as f32
                        } else {
                            0.0
                        },
                    height: 10.0,
                    style: self.colors().accent,
                });
                *y += 22.0;
            }
            Kind::Table { head, rows, .. } => {
                let columns = head
                    .len()
                    .max(rows.iter().map(Vec::len).max().unwrap_or(1))
                    .max(1);
                let cell_width = width / columns as f32;
                for (index, row) in std::iter::once(head).chain(rows.iter()).enumerate() {
                    let base = theme.role("table.cell");
                    let cells: Vec<Vec<Vec<(Style, String)>>> = row
                        .iter()
                        .map(|cell| {
                            self.wrap_runs(
                                cell.iter()
                                    .map(|span| {
                                        (crate::span_style(theme, span, base), span.text.clone())
                                    })
                                    .collect(),
                                (cell_width - 10.0 - self.prefix_width()).max(0.0),
                            )
                        })
                        .collect();
                    let lines = cells.iter().map(Vec::len).max().unwrap_or(1).max(1);
                    let height = lines as f32 * self.line_height() + 8.0;
                    scene.ops.push(Op::Rect {
                        x,
                        y: *y,
                        width,
                        height,
                        style: if index == 0 {
                            self.colors().selected_button
                        } else {
                            self.colors().button
                        },
                    });
                    for (column, cell) in cells.into_iter().enumerate() {
                        for (line, content) in cell.into_iter().enumerate() {
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
                }
            }
            Kind::List {
                ordered,
                items,
                markers,
            } => {
                for (index, item) in items.iter().enumerate() {
                    self.row(
                        scene,
                        x,
                        *y,
                        width,
                        vec![(
                            theme.role(&node.role),
                            match markers.get(index).copied().flatten() {
                                Some(true) => "☑".into(),
                                Some(false) => "☐".into(),
                                None if *ordered => format!("{}.", index + 1),
                                None => "•".into(),
                            },
                        )],
                    );
                    for child in item {
                        self.node_uncached(
                            child,
                            x + 25.0,
                            y,
                            (width - 25.0).max(10.0),
                            theme,
                            scene,
                        );
                    }
                }
            }
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
                self.prefixes
                    .push((theme.role(&node.role).dim(), "▏ ".to_string()));
                quote_prefix = true;
            }
            Kind::Rule => {
                let dash = self.measure("─");
                let count = if dash > 0.0 {
                    ((width - self.prefix_width()).max(0.0) / dash).floor() as usize
                } else {
                    0
                };
                self.row(
                    scene,
                    x,
                    *y,
                    width,
                    vec![(theme.role(&node.role), "─".repeat(count))],
                );
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
        if children {
            for id in self.document.children(&node.id) {
                self.present(&id, x, y, width, theme, scene);
            }
            for child in &node.children {
                self.node_uncached(child, x, y, width, theme, scene);
            }
            if node.id == self.document.stream_parent()
                && !self.document.visible_streams().is_empty()
            {
                self.present("streams", x, y, width, theme, scene);
            }
            if node.id == "streams" {
                let ids = self.document.visible_streams();
                for id in ids {
                    self.present(&id, x, y, width, theme, scene);
                }
            }
        }
        if quote_prefix {
            self.prefixes.pop();
        }
        for action in &node.actions {
            let control = Control::Action {
                node: node.id.clone(),
                action: action.id.clone(),
            };
            let bounds = Rect {
                x,
                y: *y,
                width: width.min(260.0),
                height: 32.0,
            };
            scene.ops.push(Op::Rect {
                x: x - 1.0,
                y: *y - 1.0,
                width: bounds.width + 2.0,
                height: 34.0,
                style: if self.focus.as_ref() == Some(&control) {
                    self.colors().accent
                } else {
                    self.colors().border
                },
            });
            let button = Button {
                id: control,
                bounds,
                label: action.label.as_deref().unwrap_or(&action.id).into(),
                font_size: FONT_SIZE,
                background: self.colors().field,
                foreground: self.colors().text,
            }
            .place(self.metrics.as_ref());
            scene.ops.extend(button.ops);
            self.hits.push(Hit {
                x: button.bounds.x,
                y: button.bounds.y,
                width: button.bounds.width,
                height: button.bounds.height,
                control: button.id,
            });
            *y += 39.0;
        }
        *y += 5.0;
        if let Some((start, first, rail_style)) = card {
            let last = *y - 5.0;
            if last > first {
                let height = last - first;
                let mut at = start;
                if let Some(surface) = theme.surface(&node.role)
                    && surface.bg != misa_style::Color::Default
                {
                    scene.ops.insert(
                        at,
                        Op::Rect {
                            x: x - 6.0,
                            y: first - 4.0,
                            width: (width + 12.0).min((scene.width - x + 6.0).max(1.0)),
                            height: height + 8.0,
                            style: Style::fg(surface.bg),
                        },
                    );
                    at += 1;
                }
                scene.ops.insert(
                    at,
                    Op::Rect {
                        x: x - 4.0,
                        y: first,
                        width: 2.0,
                        height,
                        style: rail_style,
                    },
                );
            }
        }
    }
}
/// The text of an indicator fact, recursing through wrapper sections.
fn indicator_value(node: &Node) -> String {
    match &node.kind {
        Kind::Fact { value } => misa_render::fact::format(&node.role, value),
        Kind::Meter { value, max, .. } => format!(
            "{}/{}",
            misa_render::fact::count(Some(*value as i64), ""),
            misa_render::fact::count(Some(*max as i64), "")
        ),
        Kind::Status { text } => text.clone(),
        Kind::Text { spans } => spans.iter().map(|span| span.text.as_str()).collect(),
        _ => node
            .children
            .iter()
            .map(indicator_value)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
    }
}
