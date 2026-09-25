use super::{App, FONT_SIZE};
use misa_pixel_ui::{Rect, Scene, TextFlow};
use misa_proto::view::{Node, Span};
use misa_render::Theme;
use misa_style::Style;

impl App {
    pub(super) fn line_height(&self) -> f32 {
        self.metrics.line_metrics(FONT_SIZE).line_height
    }

    pub(super) fn measure(&self, value: &str) -> f32 {
        self.metrics.measure(value, FONT_SIZE)
    }

    /// Clip at a measured character boundary (also used for unwrapped code rows).
    pub(super) fn clip(&self, value: &str, budget: f32) -> String {
        TextFlow::new(self.metrics.as_ref(), FONT_SIZE)
            .clip(value, budget)
            .0
    }

    pub(super) fn prefix_width(&self) -> f32 {
        self.prefixes
            .iter()
            .map(|(_, value)| self.measure(value))
            .sum()
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
        let spans = self.prefixes.iter().cloned().chain(spans).collect();
        let flow = TextFlow::new(self.metrics.as_ref(), FONT_SIZE);
        let bounds = Rect {
            x,
            y,
            width: width.max(0.0),
            height: flow.line_height(),
        };
        let geometry = flow.place(bounds, spans, &mut scene.ops);
        // The entire bounded row is selectable, including blank lines and
        // the space after the last glyph (which maps to the final caret).
        self.interaction
            .add_row(x, y, bounds.width, bounds.height, geometry);
    }

    pub(super) fn wrap_runs(
        &self,
        spans: Vec<(Style, String)>,
        budget: f32,
    ) -> Vec<Vec<(Style, String)>> {
        TextFlow::new(self.metrics.as_ref(), FONT_SIZE).wrap(spans, budget)
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
        let prefix = self.prefix_width();
        let runs = spans
            .iter()
            .map(|span| (crate::span_style(theme, span, base), span.text.clone()))
            .collect();
        for line in self.wrap_runs(runs, (width - prefix).max(0.0)) {
            self.row(scene, x, *y, width, line);
            *y += self.line_height();
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
                    self.clip(lang, (width - self.prefix_width()).max(0.0)),
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
        let budget = (width - self.prefix_width()).max(0.0);
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
