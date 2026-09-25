use super::{App, Control, FONT_SIZE, Hit, RowGeometry, TextRow, text};
use crate::{Op, Scene};
use misa_proto::view::{Node, Span};
use misa_render::{Style, Theme};
use std::sync::Arc;

impl App {
    pub(super) fn line_height(&self) -> f32 {
        self.metrics.line_metrics(FONT_SIZE).line_height
    }

    pub(super) fn measure(&self, value: &str) -> f32 {
        self.metrics.measure(value, FONT_SIZE)
    }

    /// Clip at a measured character boundary (also used for unwrapped code rows).
    pub(super) fn clip(&self, value: &str, budget: f32) -> String {
        self.clip_advance(value, budget).0
    }

    fn clip_advance(&self, value: &str, budget: f32) -> (String, f32) {
        let (visible_end, advances) = self.visible_advances(value, budget);
        let mut end = 0;
        let mut used = 0.0;
        for ((index, ch), width) in value[..visible_end]
            .char_indices()
            .zip(advances.iter().skip(1))
        {
            if *width > budget {
                break;
            }
            end = index + ch.len_utf8();
            used = *width;
        }
        (value[..end].to_string(), used)
    }

    pub(super) fn prefix_width(&self) -> f32 {
        self.prefixes
            .iter()
            .map(|(_, value)| self.measure(value))
            .sum()
    }

    /// Measure at most one small batch past the viewport. Glyph widths come from
    /// the same metrics as the painter; no character-width estimate is involved.
    /// Include the glyph crossing the right edge so its visible ink is painted.
    fn visible_advances(&self, value: &str, budget: f32) -> (usize, Vec<f32>) {
        let mut advances = vec![0.0];
        let mut end = 0;
        while end < value.len() && *advances.last().unwrap() < budget {
            let next = value[end..]
                .char_indices()
                .nth(32)
                .map_or(value.len(), |(at, _)| end + at);
            let chunk = &value[end..next];
            let measured = self.metrics.advances(chunk, FONT_SIZE);
            let base = *advances.last().unwrap();
            for ((at, ch), edge) in chunk.char_indices().zip(measured.iter().skip(1)) {
                advances.push(base + edge);
                if base + edge >= budget {
                    return (end + at + ch.len_utf8(), advances);
                }
            }
            end = next;
        }
        (end, advances)
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
        let spans = self
            .prefixes
            .iter()
            .cloned()
            .chain(spans)
            .collect::<Vec<_>>();
        let index = self.rows.len();
        let mut plain = String::new();
        let mut advances = vec![0.0];
        let mut runs = Vec::new();
        let mut painted = Vec::new();
        let width = width.max(0.0);
        let mut xx = x;
        for (style, value) in spans {
            let start = advances.len() - 1;
            let base = xx - x;
            let (end, measured) = self.visible_advances(&value, width - base);
            advances.extend(measured.iter().skip(1).map(|advance| base + advance));
            if end > 0 {
                let visible = &value[..end];
                painted.push(text(xx, y, visible, style));
                runs.push((style, visible.to_string(), start));
            }
            xx += measured.last().copied().unwrap_or(0.0);
            plain.push_str(&value);
        }
        scene.ops.push(Op::ClipRect {
            x,
            y,
            width,
            height: self.line_height(),
            ops: Arc::new(painted),
        });
        self.hits.push(Hit {
            x,
            y,
            // The entire bounded row is selectable, including blank lines and
            // the space after the last glyph (which maps to the final caret).
            width,
            height: self.line_height(),
            control: Control::Text(index),
        });
        self.rows.push(TextRow {
            x,
            y,
            width,
            geometry: Arc::new(RowGeometry {
                text: plain,
                advances,
                runs,
            }),
        });
    }

    /// Wrap styled Unicode text by measured pixel advances; preserve styles and
    /// every character (including whitespace) across soft and explicit breaks.
    pub(super) fn wrap_runs(
        &self,
        spans: Vec<(Style, String)>,
        budget: f32,
    ) -> Vec<Vec<(Style, String)>> {
        // One Skia conversion per source run. A newline starts a new painted
        // run; widths are otherwise glyph advances of that exact run.
        let mut chars = Vec::new();
        for (style, text) in spans {
            for segment in text.split_inclusive('\n') {
                let widths = self.metrics.advances(segment, FONT_SIZE);
                for ((_, ch), pair) in segment.char_indices().zip(widths.windows(2)) {
                    chars.push((style, ch, pair[1] - pair[0]));
                }
            }
        }
        let mut lines = Vec::new();
        let mut start = 0;
        while start < chars.len() {
            let mut end = start;
            let mut last_space = None;
            let mut run_width = 0.0;
            let mut run_style = None;
            let mut completed_width = 0.0;
            while end < chars.len() && chars[end].1 != '\n' {
                let (style, _, advance) = chars[end];
                if run_style.is_some_and(|previous| previous != style) {
                    completed_width += run_width;
                    run_width = 0.0;
                }
                run_style = Some(style);
                run_width += advance;
                if completed_width + run_width > budget && end > start {
                    break;
                }
                end += 1;
                if chars[end - 1].1.is_whitespace() {
                    last_space = Some(end);
                }
            }
            let newline = end < chars.len() && chars[end].1 == '\n';
            if !newline && end < chars.len() && end > start {
                end = last_space.filter(|space| *space > start).unwrap_or(end);
            }
            if end == start && !newline {
                end += 1;
            }
            let mut runs: Vec<(Style, String)> = Vec::new();
            for &(style, ch, _) in &chars[start..end] {
                if let Some((last_style, text)) = runs.last_mut()
                    && *last_style == style
                {
                    text.push(ch);
                } else {
                    runs.push((style, ch.to_string()));
                }
            }
            lines.push(runs);
            start = end + usize::from(newline);
            if newline && start == chars.len() {
                lines.push(vec![]);
            }
        }
        if lines.is_empty() {
            lines.push(vec![]);
        }
        lines
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
