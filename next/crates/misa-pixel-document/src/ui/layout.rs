use super::{Control, DocumentUi, FONT_SIZE, FieldViewport, PULSE_PERIOD, text};
use misa_pixel_ui::{FieldMode, Op, Scene};
use misa_proto::view::{FieldKind, Node};
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
    pub(super) viewport: &'a misa_pixel_ui::Viewport,
    pub(super) metrics: &'a dyn misa_pixel_ui::TextMetrics,
    pub(super) colors: crate::appearance::Palette,
    pub(super) theme: Arc<Theme>,
    pub(super) prefixes: Vec<(Style, String)>,
}

impl<'a> LayoutBuilder<'a> {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn new(
        document: &'a super::document::DocumentStore,
        drafts: &'a mut super::drafts::Drafts,
        interaction: &'a mut super::interaction::InteractionMap,
        retained: &'a mut super::retained::RetainedScenes,
        overlays: &'a mut super::overlays::LocalOverlays,
        viewport: &'a misa_pixel_ui::Viewport,
        metrics: &'a dyn misa_pixel_ui::TextMetrics,
        light: bool,
    ) -> Self {
        Self {
            document,
            drafts,
            interaction,
            retained,
            overlays,
            viewport,
            metrics,
            colors: crate::appearance::Palette::new(light),
            theme: Arc::new(if light { Theme::light() } else { Theme::dark() }),
            prefixes: Vec::new(),
        }
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
        let cached = if let Some(cached) = self.retained.get(id, width) {
            cached
        } else {
            let synthetic;
            let node = if id == "streams" {
                synthetic = Node::section("streams").id("streams");
                &synthetic
            } else if let Some(node) = self.document.stream_or_node(id) {
                node
            } else {
                return;
            };
            let outer = self.interaction.take_group();
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
            )
        };
        self.retained.place(&cached, *y);
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
        // Reconciliation mutates DocumentUi's viewport. The builder sees snapshots of that
        // small value while it borrows the other owners for the whole frame.
        let before = self.viewport;
        let mut builder = LayoutBuilder::new(
            &self.document,
            &mut self.drafts,
            &mut self.interaction,
            &mut self.retained,
            &mut self.overlays,
            &before,
            self.metrics.as_ref(),
            self.light,
        );
        let (mut scene, content_height) = builder.layout(width, height);
        // The transcript owns its 40px tail breathing room; the viewport does not.
        let moved = self.viewport.reconcile(content_height, height as f32, 40.0);
        let after = self.viewport;
        builder.viewport = &after;
        if moved {
            scene = builder.layout(width, height).0;
        }
        // Lay out first: the final scroll and collapsed groups determine visibility.
        // An idle frame reuses its display lists and checks only moving owner bounds.
        if builder.retained.invalidate_stale_visible(
            builder.document,
            self.viewport.offset(),
            self.viewport.viewport_height(),
        ) {
            scene = builder.layout(width, height).0;
        }
        builder.paint_report(&mut scene, width, height);
        if let Some(mut menu) = self.menu.take() {
            scene.ops.extend(
                self.menu_widget(&menu.items, menu.anchor)
                    .place(self.metrics.as_ref(), &mut menu.state)
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
    pub(super) fn layout(&mut self, width: u32, height: u32) -> (Scene, f32) {
        self.interaction.begin_frame();
        let mut scene = Scene {
            width: width as f32,
            height: height as f32,
            ops: vec![],
        };
        let root = self.document.root().to_string();
        let theme = Arc::clone(&self.theme);
        let mut y = 20.0 + self.viewport.position(0.0);
        self.present(
            &root,
            20.0,
            &mut y,
            (width as f32 - 40.0).max(40.0),
            &theme,
            &mut scene,
        );
        let content_height = y + self.viewport.offset() + 20.0;
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
        (scene, content_height)
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
        match &control {
            Control::Field { node, field } => self.drafts.set_viewport(node, field, viewport),
            Control::SavePath => self.overlays.set_save_viewport(viewport),
            _ => {}
        }
    }
}
