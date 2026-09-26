//! Measured, protocol-independent text flow and bounded row placement.
use crate::{Op, Rect, TextMetrics};
use misa_style::Style;
use std::sync::Arc;

/// Styled text in the same font and size as the painter.
pub struct TextFlow<'a> {
    metrics: &'a dyn TextMetrics,
    size: f32,
}

/// Semantic text and viewport-bound caret/paint geometry from one placement.
#[derive(Debug)]
pub struct LaidOutRow {
    pub bounds: Rect,
    pub text: String,
    /// Pixel offsets from `bounds.x`, one per visible Unicode scalar boundary.
    pub advances: Vec<f32>,
    /// Style, visible text, and starting scalar column in `advances`.
    pub runs: Vec<(Style, String, usize)>,
}

impl<'a> TextFlow<'a> {
    pub fn new(metrics: &'a dyn TextMetrics, size: f32) -> Self {
        Self { metrics, size }
    }

    pub fn line_height(&self) -> f32 {
        self.metrics.line_metrics(self.size).line_height
    }

    /// Find the visible prefix using fixed-size measurement batches. Include the
    /// first glyph crossing the edge so the clip can paint its visible ink.
    pub fn visible_advances(&self, value: &str, budget: f32) -> (usize, Vec<f32>) {
        let mut advances = vec![0.0];
        let mut end = 0;
        while end < value.len() && (end == 0 || *advances.last().unwrap() < budget) {
            let next = value[end..]
                .char_indices()
                .nth(32)
                .map_or(value.len(), |(at, _)| end + at);
            let chunk = &value[end..next];
            let measured = self.metrics.advances(chunk, self.size);
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

    /// A prefix ending before the crossing glyph, for callers that must discard
    /// offscreen data (e.g. syntax highlighting). Placement itself retains it.
    pub fn clip(&self, value: &str, budget: f32) -> (String, f32) {
        let (end, advances) = self.visible_advances(value, budget);
        let mut boundary = 0;
        let mut used = 0.0;
        for ((index, ch), width) in value[..end].char_indices().zip(advances.iter().skip(1)) {
            if *width > budget {
                break;
            }
            boundary = index + ch.len_utf8();
            used = *width;
        }
        (value[..boundary].to_string(), used)
    }

    /// Wrap by measured glyph advances, preserving whitespace, styles, explicit
    /// breaks and even a glyph wider than the available line.
    pub fn wrap(&self, spans: Vec<(Style, String)>, budget: f32) -> Vec<Vec<(Style, String)>> {
        self.wrap_with_ranges(spans, budget)
            .into_iter()
            .map(|(runs, _, _)| runs)
            .collect()
    }

    /// Each measured row carries its half-open scalar range in the original
    /// source, including positions skipped by explicit line breaks.
    pub fn wrap_with_ranges(
        &self,
        spans: Vec<(Style, String)>,
        budget: f32,
    ) -> Vec<(Vec<(Style, String)>, usize, usize)> {
        let mut chars = Vec::new();
        for (style, text) in spans {
            for segment in text.split_inclusive('\n') {
                let widths = self.metrics.advances(segment, self.size);
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
            let mut width = 0.0;
            while end < chars.len() && chars[end].1 != '\n' {
                let (_, ch, advance) = chars[end];
                if width + advance > budget && end > start {
                    break;
                }
                width += advance;
                end += 1;
                if ch.is_whitespace() {
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
            lines.push((runs, start, end));
            start = end + usize::from(newline);
            if newline && start == chars.len() {
                lines.push((vec![], start, start));
            }
        }
        if lines.is_empty() {
            lines.push((vec![], 0, 0));
        }
        lines
    }

    /// Paint a row clipped to `bounds`; the returned semantic text is complete
    /// while caret and styled paint runs cover only the visible prefix.
    pub fn place(
        &self,
        bounds: Rect,
        spans: Vec<(Style, String)>,
        ops: &mut Vec<Op>,
    ) -> LaidOutRow {
        let bounds = Rect {
            width: bounds.width.max(0.0),
            ..bounds
        };
        let mut text = String::new();
        let mut advances = vec![0.0];
        let mut runs = Vec::new();
        let mut painted = Vec::new();
        let mut offset = 0.0;
        for (style, value) in spans {
            let start = advances.len() - 1;
            if offset < bounds.width && !value.is_empty() {
                let (end, measured) = self.visible_advances(&value, bounds.width - offset);
                advances.extend(measured.iter().skip(1).map(|edge| offset + edge));
                if end > 0 {
                    let visible = &value[..end];
                    painted.push(Op::Text {
                        x: bounds.x + offset,
                        y: bounds.y,
                        size: self.size,
                        style,
                        text: visible.to_string(),
                    });
                    runs.push((style, visible.to_string(), start));
                }
                offset += measured.last().copied().unwrap_or(0.0);
            }
            text.push_str(&value);
        }
        ops.push(Op::ClipRect {
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
            ops: Arc::new(painted),
        });
        LaidOutRow {
            bounds,
            text,
            advances,
            runs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Metrics;
    impl TextMetrics for Metrics {
        fn measure(&self, text: &str, size: f32) -> f32 {
            self.advances(text, size).last().copied().unwrap()
        }
        fn advances(&self, text: &str, size: f32) -> Vec<f32> {
            let mut edges = vec![0.0];
            for ch in text.chars() {
                edges.push(edges.last().unwrap() + if ch == '界' { size * 2.0 } else { size });
            }
            edges
        }
        fn line_metrics(&self, size: f32) -> crate::LineMetrics {
            crate::LineMetrics {
                ascent: -size,
                descent: 0.0,
                leading: 0.0,
                line_height: size,
            }
        }
    }
    #[test]
    fn wraps_styles_whitespace_explicit_breaks_and_wide_glyphs() {
        let flow = TextFlow::new(&Metrics, 10.0);
        let a = Style::rgb(1, 2, 3);
        let b = Style::rgb(4, 5, 6);
        let rows = flow.wrap(vec![(a, "ab  ".into()), (b, "界x\n\ny".into())], 25.0);
        assert_eq!(
            rows,
            vec![
                vec![(a, "ab".into())],
                vec![(a, "  ".into())],
                vec![(b, "界".into())],
                vec![(b, "x".into())],
                vec![],
                vec![(b, "y".into())]
            ]
        );
        assert_eq!(
            flow.wrap(vec![(a, "界\n".into())], 0.0),
            vec![vec![(a, "界".into())], vec![]]
        );
        assert_eq!(
            flow.wrap_with_ranges(vec![(a, "ab\ncd".into())], 10.0),
            vec![
                (vec![(a, "a".into())], 0, 1),
                (vec![(a, "b".into())], 1, 2),
                (vec![(a, "c".into())], 3, 4),
                (vec![(a, "d".into())], 4, 5),
            ]
        );
        let restored = rows
            .iter()
            .flat_map(|line| line.iter())
            .map(|(_, text)| text.as_str())
            .collect::<String>();
        assert_eq!(restored, "ab  界xy");
    }
    #[test]
    fn clipping_measures_only_bounded_batches() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        struct Counting(AtomicUsize);
        impl TextMetrics for Counting {
            fn measure(&self, text: &str, size: f32) -> f32 {
                Metrics.measure(text, size)
            }
            fn advances(&self, text: &str, size: f32) -> Vec<f32> {
                assert!(text.chars().count() <= 32);
                self.0.fetch_add(1, Ordering::Relaxed);
                Metrics.advances(text, size)
            }
            fn line_metrics(&self, size: f32) -> crate::LineMetrics {
                Metrics.line_metrics(size)
            }
        }
        let metrics = Counting(AtomicUsize::new(0));
        let flow = TextFlow::new(&metrics, 10.0);
        let (end, advances) = flow.visible_advances(&"界".repeat(100_000), 15.0);
        assert_eq!(end, "界".len());
        assert_eq!(advances, vec![0.0, 20.0]);
        assert_eq!(metrics.0.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn placement_keeps_copy_text_but_bounds_paint_and_caret() {
        let flow = TextFlow::new(&Metrics, 10.0);
        let a = Style::rgb(1, 2, 3);
        let b = Style::rgb(4, 5, 6);
        let bounds = Rect {
            x: 4.0,
            y: 6.0,
            width: 25.0,
            height: flow.line_height(),
        };
        let mut ops = vec![];
        let row = flow.place(
            bounds,
            vec![(a, "a".into()), (b, format!("界{}", "x".repeat(100_000)))],
            &mut ops,
        );
        assert_eq!(row.text.len(), 100_004);
        assert_eq!(row.advances, vec![0.0, 10.0, 30.0]);
        assert_eq!(row.runs, vec![(a, "a".into(), 0), (b, "界".into(), 1)]);
        assert!(
            matches!(&ops[0], Op::ClipRect { x: 4.0, y: 6.0, width: 25.0, height: 10.0, ops } if matches!(&ops[1], Op::Text { text, .. } if text == "界"))
        );
        assert_eq!(flow.clip("a界x", 25.0), ("a".into(), 10.0));
    }
}
