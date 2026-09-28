use super::flow::FlowId;
use super::{
    CARD_PADDING_X, CARD_PADDING_Y, CARD_TRAILING, Control, DocumentUi, FONT_SIZE, FieldViewport,
    PARAGRAPH_GAP, PULSE_PERIOD, RAIL, text,
};
use misa_pixel_ui::{FieldMode, FlowConstraints, FlowViewport, Op, Scene};
use misa_proto::view::{FieldKind, Kind, Node};
use misa_render::Theme;
use misa_style::Style;
use std::sync::Arc;
use std::time::Duration;

/// One paint pass over distinct owners; no layout scratch survives the pass.
pub(super) struct LayoutBuilder<'a> {
    pub(super) document: &'a super::document::DocumentStore,
    pub(super) drafts: &'a mut super::drafts::Drafts,
    pub(super) interaction: &'a mut super::interaction::InteractionMap,
    pub(super) retained: &'a mut super::retained::RetainedScenes,
    pub(super) overlays: &'a mut super::overlays::LocalOverlays,
    pub(super) metrics: &'a dyn misa_pixel_ui::TextMetrics,
    pub(super) colors: crate::appearance::Palette,
    light: bool,
    pub(super) theme: Arc<Theme>,
    pub(super) gutters: Vec<Style>,
    retain_only: bool,
    /// A private measurement store shrinks indexed containers to one row; this
    /// maps that row back to its original marker and content index.
    pub(super) row_content_index: Option<usize>,
    viewport_effects: Vec<(Control, FieldViewport)>,
}

impl<'a> LayoutBuilder<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        document: &'a super::document::DocumentStore,
        drafts: &'a mut super::drafts::Drafts,
        interaction: &'a mut super::interaction::InteractionMap,
        retained: &'a mut super::retained::RetainedScenes,
        overlays: &'a mut super::overlays::LocalOverlays,
        metrics: &'a dyn misa_pixel_ui::TextMetrics,
        light: bool,
    ) -> Self {
        Self {
            document,
            drafts,
            interaction,
            retained,
            overlays,
            metrics,
            colors: crate::appearance::Palette::new(light),
            light,
            theme: Arc::new(if light { Theme::light() } else { Theme::dark() }),
            gutters: Vec::new(),
            retain_only: false,
            row_content_index: None,
            viewport_effects: Vec::new(),
        }
    }

    /// Retain one owner's exact laid-out display list without placing it in the
    /// frame. This path leaves the frame's hits and persistent editor scroll alone.
    pub(super) fn measure_owner(&mut self, id: &str, width: f32, theme: &Theme) -> Option<f32> {
        #[cfg(test)]
        self.retained.owner_measured();
        self.retain_only = true;
        let cached = self.retain_owner(id, width, theme);
        self.retain_only = false;
        self.viewport_effects.clear();
        cached.map(|group| group.height)
    }

    pub(super) fn measure_flow(&mut self, flow: &FlowId, width: f32, theme: &Theme) -> Option<f32> {
        #[cfg(test)]
        self.retained.owner_measured();
        self.retain_only = true;
        let cached = self.retain_flow(flow, width, theme);
        self.retain_only = false;
        self.viewport_effects.clear();
        cached.map(|group| group.height)
    }

    fn retain_flow(
        &mut self,
        flow: &FlowId,
        width: f32,
        theme: &Theme,
    ) -> Option<Arc<super::retained::Cached>> {
        let owner = match flow {
            FlowId::Node(id) | FlowId::Row(id, _) | FlowId::End(id) | FlowId::Stream(id) => id,
            _ => return None,
        };
        if self.document.row_count(owner).is_none() || matches!(flow, FlowId::Stream(_)) {
            return self.retain_owner(owner, width, theme);
        }
        let key = flow.cache_key();
        if let Some(cached) = self.retained.get(&key, width) {
            return Some(cached);
        }
        let node = self.document.node(owner)?;
        // A carded owner leaves its paddings plus the gap between cards; an
        // undecorated one keeps the paragraph rhythm.
        let trailing = if theme.rail(&node.role).is_some() {
            CARD_TRAILING
        } else {
            PARAGRAPH_GAP
        };
        let outer = self.interaction.take_group();
        self.interaction.begin_owner(&key);
        self.retained.begin_group();
        let effect_start = self.viewport_effects.len();
        let mut scene = Scene::default();
        let mut height = 0.0;
        match flow {
            FlowId::Node(_) => {
                if let Some(label) = &node.label {
                    self.row(
                        &mut scene,
                        0.0,
                        height,
                        width,
                        vec![(theme.role(&node.role), label.clone())],
                    );
                    height += 25.0;
                }
            }
            FlowId::Row(_, index) => match &node.kind {
                Kind::List {
                    ordered,
                    items,
                    markers,
                } => self.list_item(
                    node,
                    *ordered,
                    markers
                        .get(self.row_content_index.unwrap_or(*index))
                        .copied()
                        .flatten(),
                    *index,
                    items.get(self.row_content_index.unwrap_or(*index))?,
                    0.0,
                    &mut height,
                    width,
                    theme,
                    &mut scene,
                ),
                Kind::Table { head, rows, .. } => {
                    let actual = self.row_content_index.unwrap_or(*index);
                    let row = if actual == 0 {
                        head
                    } else {
                        rows.get(actual - 1)?
                    };
                    self.table_row(
                        row,
                        *index,
                        self.document.table_columns(owner),
                        0.0,
                        &mut height,
                        width,
                        theme,
                        &mut scene,
                    );
                }
                _ => return None,
            },
            FlowId::End(_) => {
                // Non-indexed children and actions still follow the rows.
                for child in self.document.children(owner) {
                    if !self.document.pinned(&child) {
                        self.present(&child, 0.0, &mut height, width, theme, &mut scene);
                    }
                }
                for child in &node.children {
                    self.node_uncached(child, 0.0, &mut height, width, theme, &mut scene);
                }
                if owner == self.document.stream_parent()
                    && !self.document.visible_streams().is_empty()
                {
                    self.present("streams", 0.0, &mut height, width, theme, &mut scene);
                }
                self.paint_self_actions(node, 0.0, &mut height, width, &mut scene);
                height += trailing;
            }
            _ => unreachable!(),
        }
        // Each visible slice carries the outer card; adjacent surfaces meet
        // without painting over content in the preceding fragment.
        let painted_height = if matches!(flow, FlowId::End(_)) {
            height - trailing
        } else {
            height
        };
        if painted_height > 0.0 {
            if let Some((_, rail)) = theme.rail(&node.role) {
                let follows = !node.actions.is_empty()
                    || self.document.tree.first_child(owner).is_some()
                    || !node.children.is_empty()
                    || (owner == self.document.stream_parent()
                        && !self.document.visible_streams().is_empty());
                let first = matches!(flow, FlowId::Node(_))
                    || matches!(flow, FlowId::Row(_, 0) if node.label.is_none())
                    || matches!(flow, FlowId::End(_) if node.label.is_none() && self.document.row_count(owner) == Some(0));
                let last = matches!(flow, FlowId::End(_))
                    || matches!(flow, FlowId::Node(_) if self.document.row_count(owner) == Some(0) && !follows)
                    || matches!(flow, FlowId::Row(_, index) if index + 1 == self.document.row_count(owner).unwrap_or(0) && !follows);
                let top = if first { CARD_PADDING_Y } else { 0.0 };
                let bottom = if last { CARD_PADDING_Y } else { 0.0 };
                let mut at = 0;
                if let Some(surface) = theme.surface(&node.role)
                    && surface.bg != misa_style::Color::Default
                {
                    scene.ops.insert(
                        at,
                        Op::Rect {
                            x: -CARD_PADDING_X,
                            y: -top,
                            width: width + 2.0 * CARD_PADDING_X,
                            height: painted_height + top + bottom,
                            style: Style::fg(surface.bg),
                        },
                    );
                    at += 1;
                }
                scene.ops.insert(
                    at,
                    Op::Rect {
                        x: -CARD_PADDING_X + 4.0,
                        y: 0.0,
                        width: RAIL,
                        height: painted_height,
                        style: rail,
                    },
                );
            }
        }
        let geometry = self.interaction.restore_group(outer);
        Some(self.retained.finish_group(
            &key,
            width,
            height,
            scene.ops,
            geometry,
            false,
            self.viewport_effects[effect_start..].to_vec(),
        ))
    }

    fn retain_owner(
        &mut self,
        id: &str,
        width: f32,
        theme: &Theme,
    ) -> Option<Arc<super::retained::Cached>> {
        let cached = if let Some(cached) = self.retained.get(id, width) {
            if self.retain_only {
                self.viewport_effects
                    .extend(cached.viewport_effects.iter().cloned());
            }
            cached
        } else {
            let synthetic;
            let node = if id == "streams" {
                synthetic = Node::section("streams").id("streams");
                &synthetic
            } else if let Some(node) = self.document.stream_or_node(id) {
                node
            } else {
                return None;
            };
            let effect_start = self.viewport_effects.len();
            let outer = self.interaction.take_group();
            self.interaction.begin_owner(id);
            let mut local = Scene::default();
            let mut height = 0.0;
            self.retained.begin_group();
            self.node_uncached(node, 0.0, &mut height, width, theme, &mut local);
            let geometry = self.interaction.restore_group(outer);
            self.retained.finish_group(
                id,
                width,
                height,
                local.ops,
                geometry,
                node.role == "status.indicators",
                self.viewport_effects[effect_start..].to_vec(),
            )
        };
        Some(cached)
    }

    pub(super) fn present_flow(
        &mut self,
        flow: &FlowId,
        x: f32,
        y: &mut f32,
        width: f32,
        theme: &Theme,
        scene: &mut Scene,
    ) {
        let Some(cached) = self.retain_flow(flow, width, theme) else {
            return;
        };
        let key = flow.cache_key();
        #[cfg(test)]
        self.retained.owner_placed();
        self.retained.place(&key, &cached, *y);
        for (control, viewport) in &cached.viewport_effects {
            if let Control::Field { node, field } = control {
                self.drafts.set_viewport(node, field, *viewport);
            }
        }
        scene.ops.push(Op::Group {
            x,
            y: *y,
            ops: cached.ops.clone(),
        });
        self.interaction.place_group(&cached.geometry, x, *y);
        *y += cached.height;
    }

    pub(super) fn present(
        &mut self,
        id: &str,
        x: f32,
        y: &mut f32,
        width: f32,
        theme: &Theme,
        scene: &mut Scene,
    ) {
        let Some(cached) = self.retain_owner(id, width, theme) else {
            return;
        };
        #[cfg(test)]
        self.retained.owner_placed();
        self.retained.place(id, &cached, *y);
        if !self.retain_only {
            for (control, viewport) in &cached.viewport_effects {
                if let Control::Field { node, field } = control {
                    self.drafts.set_viewport(node, field, *viewport);
                }
            }
        }
        scene.ops.push(Op::Group {
            x,
            y: *y,
            ops: cached.ops.clone(),
        });
        self.interaction.place_group(&cached.geometry, x, *y);
        *y += cached.height;
    }
}

impl DocumentUi {
    /// Offline snapshots advance a synthetic clock one pulse per call, without wall time.
    pub fn frame(&mut self, width: u32, height: u32) -> Scene {
        self.offline_elapsed = self.offline_elapsed.saturating_add(PULSE_PERIOD);
        self.frame_at(width, height, self.offline_elapsed)
    }

    /// Render at a caller-provided elapsed time. Repaints within a phase retain the
    /// same display lists; only the animated indicator's owner and ancestors change.
    pub fn frame_at(&mut self, width: u32, height: u32, elapsed: Duration) -> Scene {
        self.size = misa_window_core::Size { width, height };
        self.retained.begin_frame(width, elapsed);
        let mut builder = LayoutBuilder::new(
            &self.document,
            &mut self.drafts,
            &mut self.interaction,
            &mut self.retained,
            &mut self.overlays,
            self.metrics.as_ref(),
            self.light,
        );
        let mut scene = builder.layout(&mut self.viewport, width, height);
        // A pulse invalidates only moving owners that were actually placed.
        if builder.retained.invalidate_stale_visible(builder.document) {
            let ids: Vec<_> = self
                .viewport
                .visible()
                .iter()
                .map(|p| p.id.clone())
                .collect();
            for id in ids {
                self.viewport.invalidate(&id);
            }
            scene = builder.layout(&mut self.viewport, width, height);
        }
        builder.paint_report(&mut scene, width, height);
        if let Some((control, mut state)) = self.choice.take() {
            if let Some(widget) = self.choice_widget(&control) {
                scene
                    .ops
                    .extend(widget.popup(self.metrics.as_ref(), &mut state));
                self.choice = Some((control, state));
            }
        }
        if let Some(mut menu) = self.menu.take() {
            scene.ops.extend(
                self.menu_widget(&menu.items, menu.anchor)
                    .place_with_width(self.metrics.as_ref(), &mut menu.state, menu.width)
                    .ops,
            );
            self.menu = Some(menu);
        }
        scene
    }
}

impl LayoutBuilder<'_> {
    pub(super) fn paint_report(&mut self, scene: &mut Scene, width: u32, height: u32) {
        if self.overlays.report_mut().is_some() {
            self.interaction.clear_hits();
            let colors = self.colors;
            let report = self.overlays.report_mut().unwrap();
            let budget = (width as f32 - 96.0).max(1.0);
            let visible = report.reflow(width as f32, height as f32, self.metrics, colors.text);
            let spacing = self.metrics.line_metrics(FONT_SIZE).line_height + 3.0;
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
                &misa_pixel_ui::TextFlow::new(self.metrics, FONT_SIZE)
                    .clip(&report.title, budget)
                    .0,
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
        }
    }
    pub(super) fn layout(
        &mut self,
        viewport: &mut FlowViewport<FlowId>,
        width: u32,
        height: u32,
    ) -> Scene {
        self.interaction.begin_frame();
        self.retained.begin_placement(height as f32);
        let mut scene = Scene {
            width: width as f32,
            height: height as f32,
            ops: vec![],
        };
        let content_width = (width as f32 - 40.0).max(40.0);
        let theme = Arc::clone(&self.theme);
        // A composer pins to the bottom of the screen: it is what the reader
        // types into, not content that scrolls. Its height is reserved out of
        // the flow, and it paints below the transcript at every scroll
        // position — at the tail and while reading history alike.
        let pinned: Vec<String> = self.document.pinned_owners().map(str::to_owned).collect();
        let mut pinned_height = 0.0;
        for id in &pinned {
            pinned_height += self.measure_owner(id, content_width, &theme).unwrap_or(0.0);
        }
        viewport.layout(
            self,
            FlowConstraints {
                width: content_width,
                height: (height as f32 - pinned_height).max(0.0),
                style_generation: u64::from(self.light),
            },
        );
        for placement in viewport.visible() {
            match &placement.id {
                FlowId::Node(_) | FlowId::Row(_, _) | FlowId::End(_) | FlowId::Stream(_) => {
                    let mut y = placement.y;
                    self.present_flow(
                        &placement.id,
                        20.0,
                        &mut y,
                        content_width,
                        &theme,
                        &mut scene,
                    );
                    debug_assert_eq!(y, placement.y + placement.height);
                }
                FlowId::Top | FlowId::Close(_) | FlowId::StreamClose | FlowId::Bottom => {}
            }
        }
        // Pinned input paints along the bottom edge, below the transcript.
        let mut pin = height as f32 - pinned_height;
        for id in &pinned {
            self.present(id, 20.0, &mut pin, content_width, &theme, &mut scene);
        }
        self.interaction.finish_frame();
        self.interaction
            .paint_selection(&mut scene, self.line_height(), self.colors.selection);
        if !self.overlays.notice_text().is_empty() {
            scene.ops.push(Op::Rect {
                x: 0.0,
                y: height as f32 - 26.0,
                width: width as f32,
                height: 26.0,
                style: self.colors.surface,
            });
            scene.ops.push(text(
                12.0,
                height as f32 - 23.0,
                &self.clip(self.overlays.notice_text(), (width as f32 - 24.0).max(1.0)),
                self.colors.text,
            ));
        }
        if let Some(path) = self.overlays.save_text() {
            let path = path.to_string();
            let x = 30.0;
            let y = (height as f32 / 2.0 - 70.0).max(20.0);
            let w = (width as f32 - 60.0).max(80.0);
            self.interaction.clear_hits();
            scene.ops.push(Op::Rect {
                x,
                y,
                width: w,
                height: 150.0,
                style: self.colors.surface,
            });
            scene.ops.push(text(
                x + 12.0,
                y + 10.0,
                &self.clip("Save attachment · local destination", (w - 24.0).max(1.0)),
                self.colors.text,
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
        if let Some(picker) = self.overlays.picker() {
            self.interaction.clear_hits();
            let y = 35.0;
            scene.ops.push(Op::Rect {
                x: 30.0,
                y,
                width: (width as f32 - 60.0).max(80.0),
                height: 300.0,
                style: self.colors.surface,
            });
            scene.ops.push(text(
                42.0,
                y + 12.0,
                &self.clip(
                    &format!("Commands · {}", picker.query),
                    (width as f32 - 84.0).max(1.0),
                ),
                self.colors.text,
            ));
            let matches = picker.matches();
            let start = picker.selected_index().saturating_sub(7);
            if matches.is_empty() {
                scene.ops.push(text(
                    42.0,
                    y + 48.0,
                    "No matching commands",
                    self.colors.muted,
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
                    self.colors.text,
                ));
            }
            scene.ops.push(text(
                42.0,
                y + 270.0,
                "↑↓ select · Enter insert · Escape close",
                self.colors.muted,
            ));
        }
        scene
    }
    pub(super) fn box_control(
        &mut self,
        scene: &mut Scene,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        label: &str,
        control: Control,
    ) {
        let focused = self.interaction.focused(&control);
        // Semantic policy stops here: discrete values have no editor caret, and a
        // secret editor's byte cursor maps to a scalar index in the masked run.
        let (cursor, mut viewport) = match &control {
            Control::Field { node, field } => {
                let model = self.document.field(node, field);
                let cursor = self
                    .drafts
                    .cursor(node, field)
                    .filter(|_| {
                        !model.is_some_and(|value| {
                            matches!(value.kind, FieldKind::Bool | FieldKind::Choice { .. })
                        })
                    })
                    .map(|at| {
                        if model.is_some_and(|value| value.secret) {
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
            Control::SavePath => {
                let state = self.overlays.save_cursor_viewport();
                (
                    state.map(|(cursor, _)| cursor),
                    state.map_or(FieldViewport::default(), |(_, viewport)| viewport),
                )
            }
            _ => (None, FieldViewport::default()),
        };
        let mode = match &control {
            Control::Field { node, field }
                if self
                    .document
                    .field(node, field)
                    .is_some_and(|value| value.kind == FieldKind::Block) =>
            {
                FieldMode::WordWrap
            }
            _ => FieldMode::SingleLine,
        };
        let placed = misa_pixel_ui::TextField {
            id: control,
            bounds: misa_pixel_ui::Rect {
                x,
                y,
                width,
                height,
            },
            label: label.into(),
            mode,
            font_size: FONT_SIZE,
            focused,
            cursor,
            insets: misa_pixel_ui::FieldInsets {
                left: 7.0,
                top: 6.0,
                right: 7.0,
                bottom: 3.0,
            },
            border_width: 1.0,
            caret_width: 1.5,
            background: self.colors.field,
            border: self.colors.border,
            focus_border: self.colors.accent,
            foreground: self.colors.text,
        }
        .place(self.metrics, &mut viewport);
        let (control, viewport) = self.interaction.place_field(scene, placed);
        if self.retain_only {
            if matches!(control, Control::Field { .. }) {
                self.viewport_effects.push((control, viewport));
            }
        } else {
            match &control {
                Control::Field { node, field } => self.drafts.set_viewport(node, field, viewport),
                Control::SavePath => self.overlays.set_save_viewport(viewport),
                _ => {}
            }
        }
    }
}
