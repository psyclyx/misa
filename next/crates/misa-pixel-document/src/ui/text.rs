use super::{FONT_SIZE, GUTTER, layout::LayoutBuilder};
use misa_pixel_ui::{Rect, Scene, TextFlow};
use misa_proto::view::{Node, Span, SpanKind};
use misa_render::Theme;
use misa_style::Style;
use std::sync::Arc;

impl LayoutBuilder<'_> {
    pub(super) fn line_height(&self) -> f32 {
        self.metrics.line_metrics(FONT_SIZE).line_height
    }

    /// Clip at a measured character boundary (also used for unwrapped code rows).
    pub(super) fn clip(&self, value: &str, budget: f32) -> String {
        TextFlow::new(self.metrics, FONT_SIZE).clip(value, budget).0
    }

    /// Indent for the rails and disclosure marks drawn beside the content.
    pub(super) fn gutter_width(&self) -> f32 {
        self.gutters.len() as f32 * GUTTER
    }

    /// Retain the whole row for copying; only its ink and interaction are viewport-bound.
    pub(super) fn row(
        &mut self,
        scene: &mut Scene,
        x: f32,
        y: f32,
        width: f32,
        spans: Vec<(Style, String)>,
    ) {
        self.row_sized(scene, x, y, width, spans, FONT_SIZE, Arc::new([]));
    }

    /// One measured line of text at an explicit size. A heading is the same
    /// content as prose at its own size; the layout measures whatever it gets.
    /// `links` are source ranges of what the text promises when clicked.
    pub(super) fn row_sized(
        &mut self,
        scene: &mut Scene,
        x: f32,
        y: f32,
        width: f32,
        spans: Vec<(Style, String)>,
        size: f32,
        links: Arc<[(usize, usize, String)]>,
    ) {
        // Gutter bands hold rails and disclosure marks, drawn beside the text:
        // a copied row carries content only, never chrome characters.
        let gutter = self.gutter_width();
        let flow = TextFlow::new(self.metrics, size);
        let bounds = Rect {
            x: x + gutter,
            y,
            width: (width - gutter).max(0.0),
            height: flow.line_height(),
        };
        let geometry = flow.place(bounds, spans, &mut scene.ops);
        // The entire bounded row is selectable, including blank lines and
        // the space after the last glyph (which maps to the final caret).
        self.interaction
            .add_row(bounds.x, y, bounds.width, bounds.height, geometry, 0, links);
    }

    pub(super) fn wrapped(
        &mut self,
        scene: &mut Scene,
        x: f32,
        y: &mut f32,
        width: f32,
        spans: &[Span],
        base: Style,
        theme: &Theme,
    ) {
        self.wrapped_sized(scene, x, y, width, spans, base, theme, FONT_SIZE);
    }

    /// Wrapped text at an explicit size: the same layout, measured at `size`.
    pub(super) fn wrapped_sized(
        &mut self,
        scene: &mut Scene,
        x: f32,
        y: &mut f32,
        width: f32,
        spans: &[Span],
        base: Style,
        theme: &Theme,
        size: f32,
    ) {
        let gutter = self.gutter_width();
        let runs = spans
            .iter()
            .map(|span| (crate::span_style(theme, span, base), span.text.clone()))
            .collect();
        // Links are recorded in the owner's source coordinates: what this text
        // promises when clicked survives wrapping and framing.
        let mut links = Vec::new();
        let mut at = 0usize;
        for span in spans {
            let end = at + span.text.chars().count();
            if let SpanKind::Link { href } = &span.kind {
                links.push((at, end, href.clone()));
            }
            at = end;
        }
        let links = Arc::from(links);
        let mut previous_end = 0;
        for (index, (line, start, end)) in TextFlow::new(self.metrics, size)
            .wrap_with_ranges(runs, (width - gutter).max(0.0))
            .into_iter()
            .enumerate()
        {
            if index > 0 {
                self.interaction.continue_row(start - previous_end);
            }
            self.row_sized(scene, x, *y, width, line, size, Arc::clone(&links));
            previous_end = end;
            *y += self.metrics.line_metrics(size).line_height;
        }
    }

    /// Render a code block, highlighting it from the client's own grammar.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn code_block(
        &mut self,
        node: &Node,
        lang: Option<&str>,
        text: &str,
        x: f32,
        y: &mut f32,
        width: f32,
        theme: &Theme,
        scene: &mut Scene,
    ) {
        let base = theme.role(&node.role);
        let diff = crate::is_diff(&node.role, lang);
        if let Some(lang) = lang
            && !diff
        {
            self.row(
                scene,
                x,
                *y,
                width,
                vec![(
                    theme.role("markdown.code.label"),
                    self.clip(lang, (width - self.gutter_width()).max(0.0)),
                )],
            );
            *y += self.line_height();
        }
        let captures = if diff {
            Vec::new()
        } else {
            lang.map(|language| misa_syntax::captures(language, text))
                .unwrap_or_default()
        };
        let budget = (width - self.gutter_width()).max(0.0);
        let mut offset = 0;
        for raw in text.split('\n') {
            let line_style = if diff {
                crate::diff_style(theme, base, &node.role, raw)
            } else {
                base
            };
            // Highlight only the visible prefix, never the offscreen captures
            // of an unwrapped code row. Keep byte offsets in the original block.
            let clipped = self.clip(raw, budget);
            let runs = crate::code_runs(offset, &clipped, &captures, line_style, theme);
            self.row(scene, x, *y, width, runs);
            *y += self.line_height();
            offset += raw.len() + 1;
        }
    }
}
