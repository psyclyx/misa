//! Measured, clipped, single-line or soft-wrapped editable field presentation.
//! Editing policy belongs to the caller; the cursor is a byte offset into the
//! *displayed* label, so masked or formatted values can map their own indices.
use crate::{Op, Rect, TextMetrics};
use misa_style::Style;
use std::sync::Arc;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FieldMode {
    /// Explicit newlines start rows; horizontally scroll long rows.
    #[default]
    SingleLine,
    /// Measured word wrapping, with grapheme fallback for unbroken words.
    WordWrap,
}

struct VisualRow {
    start: usize,
    end: usize,
}

/// Keep each display byte exactly once (except explicit newlines). Prefer a
/// whitespace boundary, but split a long word at a grapheme boundary. A single
/// grapheme wider than the budget gets its own row and is clipped by placement.
fn visual_rows(
    label: &str,
    mode: FieldMode,
    budget: f32,
    metrics: &dyn TextMetrics,
    size: f32,
) -> Vec<VisualRow> {
    let mut rows = Vec::new();
    let mut base = 0;
    for paragraph in label.split('\n') {
        if mode == FieldMode::SingleLine {
            rows.push(VisualRow {
                start: base,
                end: base + paragraph.len(),
            });
            base += paragraph.len() + 1;
            continue;
        }
        let boundaries: Vec<usize> = paragraph
            .grapheme_indices(true)
            .map(|(at, _)| at)
            .chain(std::iter::once(paragraph.len()))
            .collect();
        if boundaries.len() == 1 {
            rows.push(VisualRow {
                start: base,
                end: base + paragraph.len(),
            });
        } else {
            let advances = metrics.advances(paragraph, size);
            let mut edges = Vec::with_capacity(boundaries.len());
            let mut scalars = paragraph.char_indices().peekable();
            let mut column = 0;
            for &byte in &boundaries {
                while scalars.peek().is_some_and(|(at, _)| *at < byte) {
                    scalars.next();
                    column += 1;
                }
                edges.push(advances[column]);
            }
            let mut start = 0;
            while start + 1 < boundaries.len() {
                let mut end = start + 1;
                let mut whitespace = None;
                while end < boundaries.len()
                    && (end == start + 1 || edges[end] - edges[start] <= budget)
                {
                    if paragraph[boundaries[end - 1]..boundaries[end]]
                        .chars()
                        .all(char::is_whitespace)
                    {
                        whitespace = Some(end);
                    }
                    end += 1;
                }
                let mut stop = end - 1;
                if stop < boundaries.len() - 1 {
                    stop = whitespace.filter(|&at| at > start).unwrap_or(stop);
                }
                rows.push(VisualRow {
                    start: base + boundaries[start],
                    end: base + boundaries[stop],
                });
                start = stop;
            }
        }
        base += paragraph.len() + 1;
    }
    rows
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FieldViewport {
    /// Horizontal pixel offset within a row (normally zero in WordWrap).
    pub x: f32,
    /// First visible visual row; soft breaks count in WordWrap.
    pub line: usize,
}

#[derive(Clone, Copy, Debug)]
pub struct FieldInsets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

pub struct TextField<Id> {
    pub id: Id,
    pub bounds: Rect,
    pub label: String,
    pub mode: FieldMode,
    pub font_size: f32,
    pub focused: bool,
    /// None suppresses the caret, even when the field is focused.
    pub cursor: Option<usize>,
    pub insets: FieldInsets,
    pub border_width: f32,
    pub caret_width: f32,
    pub background: Style,
    pub border: Style,
    pub focus_border: Style,
    pub foreground: Style,
}

pub struct PlacedField<Id> {
    pub id: Id,
    pub bounds: Rect,
    pub viewport: FieldViewport,
    pub ops: Vec<Op>,
}

impl<Id> TextField<Id> {
    pub fn place(self, metrics: &dyn TextMetrics, viewport: &mut FieldViewport) -> PlacedField<Id> {
        let b = self.bounds;
        let clip = Rect {
            x: b.x + self.insets.left,
            y: b.y + self.insets.top,
            width: (b.width - self.insets.left - self.insets.right).max(0.0),
            height: (b.height - self.insets.top - self.insets.bottom).max(0.0),
        };
        let line_height = metrics.line_metrics(self.font_size).line_height;
        assert!(line_height > 0.0, "field requires positive line spacing");
        let visible_lines = (clip.height / line_height).floor().max(1.0) as usize;
        let lines = visual_rows(&self.label, self.mode, clip.width, metrics, self.font_size);
        if self.mode == FieldMode::WordWrap {
            viewport.x = 0.0;
        }
        // A changed or shortened value may invalidate an old viewport even when
        // this field no longer owns focus. Never leave its surviving text hidden.
        viewport.line = viewport.line.min(lines.len() - 1);
        if viewport.x > 0.0 && (!self.focused || self.cursor.is_none()) {
            let row = &lines[viewport.line];
            let width = metrics.measure(&self.label[row.start..row.end], self.font_size);
            if viewport.x >= width {
                viewport.x = 0.0;
            }
        }
        let caret = if self.focused {
            self.cursor.map(|at| {
                assert!(
                    self.label.is_char_boundary(at),
                    "cursor must be a display byte boundary"
                );
                assert!(
                    at <= self.label.len(),
                    "cursor must be inside display label"
                );
                // A cursor at a soft break starts the next visual row. An
                // explicit newline ends the previous one; its following byte
                // starts the next (including the empty trailing row).
                let line = lines
                    .iter()
                    .position(|row| {
                        at < row.end
                            || (at == row.end
                                && (at == self.label.len()
                                    || self.label.as_bytes().get(at) == Some(&b'\n')))
                    })
                    .unwrap_or(lines.len() - 1);
                let row = &lines[line];
                let column = self.label[row.start..at.min(row.end)].chars().count();
                if line < viewport.line {
                    viewport.line = line;
                } else if line >= viewport.line.saturating_add(visible_lines) {
                    viewport.line = line + 1 - visible_lines;
                }
                let edge =
                    metrics.advances(&self.label[row.start..row.end], self.font_size)[column];
                if self.mode == FieldMode::WordWrap {
                    viewport.x = 0.0;
                }
                viewport.x = viewport.x.min(edge);
                viewport.x = viewport
                    .x
                    .max(edge - (clip.width - self.caret_width).max(0.0));
                (line, edge)
            })
        } else {
            None
        };
        let mut painted: Vec<Op> = lines
            .iter()
            .enumerate()
            .skip(viewport.line)
            .take(visible_lines)
            .map(|(line, value)| Op::Text {
                x: clip.x - viewport.x,
                y: clip.y + (line - viewport.line) as f32 * line_height,
                text: self.label[value.start..value.end].into(),
                size: self.font_size,
                style: self.foreground,
            })
            .collect();
        if let Some((line, edge)) = caret {
            painted.push(Op::Rect {
                x: clip.x + edge - viewport.x,
                y: clip.y + (line - viewport.line) as f32 * line_height,
                width: self.caret_width,
                height: line_height,
                style: self.foreground,
            });
        }
        PlacedField {
            id: self.id,
            bounds: b,
            viewport: *viewport,
            ops: vec![
                Op::Rect {
                    x: b.x - self.border_width,
                    y: b.y - self.border_width,
                    width: b.width + 2.0 * self.border_width,
                    height: b.height + 2.0 * self.border_width,
                    style: if self.focused {
                        self.focus_border
                    } else {
                        self.border
                    },
                },
                Op::Rect {
                    x: b.x,
                    y: b.y,
                    width: b.width,
                    height: b.height,
                    style: self.background,
                },
                Op::ClipRect {
                    x: clip.x,
                    y: clip.y,
                    width: clip.width,
                    height: clip.height,
                    ops: Arc::new(painted),
                },
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Metrics;
    impl TextMetrics for Metrics {
        fn measure(&self, text: &str, size: f32) -> f32 {
            text.chars().count() as f32 * size
        }
        fn advances(&self, text: &str, size: f32) -> Vec<f32> {
            (0..=text.chars().count())
                .map(|n| n as f32 * size)
                .collect()
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
    fn unicode_cursor_scrolls_and_clips_to_hit_bounds() {
        let mut viewport = FieldViewport::default();
        let field = |width, cursor| TextField {
            id: 7,
            bounds: Rect {
                x: 10.0,
                y: 20.0,
                width,
                height: 24.0,
            },
            label: "éééé\nsecond".into(),
            mode: FieldMode::SingleLine,
            font_size: 10.0,
            focused: true,
            cursor: Some(cursor),
            insets: FieldInsets {
                left: 7.0,
                top: 6.0,
                right: 7.0,
                bottom: 3.0,
            },
            border_width: 1.0,
            caret_width: 1.5,
            background: Style::default(),
            border: Style::default(),
            focus_border: Style::default(),
            foreground: Style::default(),
        };
        let narrow = field(30.0, 8).place(&Metrics, &mut viewport);
        assert_eq!(narrow.id, 7);
        assert!(narrow.bounds.contains(39.0, 25.0));
        assert!(!narrow.bounds.contains(40.0, 25.0));
        assert_eq!(narrow.viewport.x, 25.5);
        assert!(
            matches!(&narrow.ops[2], Op::ClipRect { width: 16.0, ops, .. }
            if matches!(&ops[1], Op::Rect { x: 31.5, .. }))
        );
        let wide = field(100.0, 9).place(&Metrics, &mut viewport);
        assert_eq!(wide.viewport.line, 1);
        assert_eq!(wide.viewport.x, 0.0);
    }

    fn wrapped(
        label: &str,
        width: f32,
        height: f32,
        cursor: usize,
        viewport: &mut FieldViewport,
    ) -> PlacedField<()> {
        TextField {
            id: (),
            bounds: Rect {
                x: 10.0,
                y: 20.0,
                width,
                height,
            },
            label: label.into(),
            mode: FieldMode::WordWrap,
            font_size: 10.0,
            focused: true,
            cursor: Some(cursor),
            insets: FieldInsets {
                left: 2.0,
                right: 2.0,
                top: 2.0,
                bottom: 2.0,
            },
            border_width: 1.0,
            caret_width: 1.0,
            background: Style::default(),
            border: Style::default(),
            focus_border: Style::default(),
            foreground: Style::default(),
        }
        .place(&Metrics, viewport)
    }

    fn content(placed: &PlacedField<()>) -> (Rect, &Arc<Vec<Op>>) {
        match &placed.ops[2] {
            Op::ClipRect {
                x,
                y,
                width,
                height,
                ops,
            } => (
                Rect {
                    x: *x,
                    y: *y,
                    width: *width,
                    height: *height,
                },
                ops,
            ),
            _ => panic!("missing field clip"),
        }
    }

    #[test]
    fn wrap_preserves_words_spaces_explicit_breaks_and_unicode_cursor() {
        let label = "abcdef  e\u{301}界\n";
        let rows = visual_rows(label, FieldMode::WordWrap, 30.0, &Metrics, 10.0);
        assert_eq!(
            rows.iter()
                .map(|r| &label[r.start..r.end])
                .collect::<Vec<_>>(),
            ["abc", "def", "  ", "e\u{301}界", ""]
        );
        let mut viewport = FieldViewport::default();
        let placed = wrapped(label, 34.0, 24.0, label.len(), &mut viewport);
        assert_eq!(placed.viewport.line, 3);
        assert_eq!(placed.viewport.x, 0.0);
        let (_, ops) = content(&placed);
        assert!(matches!(
            ops.last(),
            Some(Op::Rect {
                x: 12.0,
                y: 32.0,
                ..
            })
        ));
        let at_break = wrapped("abcdef", 34.0, 24.0, 3, &mut viewport);
        assert_eq!(at_break.viewport.line, 1);
        assert!(matches!(
            content(&at_break).1.last(),
            Some(Op::Rect { x: 12.0, .. })
        ));
    }

    #[test]
    fn wrap_uses_actual_advances_not_scalar_counts() {
        struct Wide;
        impl TextMetrics for Wide {
            fn measure(&self, text: &str, size: f32) -> f32 {
                *self.advances(text, size).last().unwrap()
            }
            fn advances(&self, text: &str, size: f32) -> Vec<f32> {
                let mut edges = vec![0.0];
                for ch in text.chars() {
                    edges.push(
                        edges.last().unwrap() + if ch == 'W' { size * 2.0 } else { size / 2.0 },
                    );
                }
                edges
            }
            fn line_metrics(&self, size: f32) -> crate::LineMetrics {
                Metrics.line_metrics(size)
            }
        }
        let label = "WiWi";
        let rows = visual_rows(label, FieldMode::WordWrap, 25.0, &Wide, 10.0);
        assert_eq!(
            rows.iter()
                .map(|row| &label[row.start..row.end])
                .collect::<Vec<_>>(),
            ["Wi", "Wi"]
        );
    }

    #[test]
    fn narrow_resize_reconciles_visual_scroll_and_clips_caret() {
        let mut viewport = FieldViewport::default();
        let label = "abcdefghij";
        let narrow = wrapped(label, 24.0, 24.0, label.len(), &mut viewport);
        assert_eq!(narrow.viewport.line, 3);
        let (clip, ops) = content(&narrow);
        assert_eq!(clip.width, 20.0);
        assert!(
            matches!(ops.last(), Some(Op::Rect { x, y, .. }) if *x >= clip.x && *x < clip.x + clip.width && *y >= clip.y && *y < clip.y + clip.height)
        );
        let wide = wrapped(label, 104.0, 24.0, label.len(), &mut viewport);
        assert_eq!(wide.viewport.line, 0);
        assert_eq!(wide.viewport.x, 1.0);
    }

    #[test]
    fn shortened_unfocused_value_cannot_remain_scrolled_out_of_view() {
        let mut viewport = FieldViewport { x: 400.0, line: 9 };
        let placed = TextField {
            id: (),
            bounds: Rect {
                x: 0.0,
                y: 0.0,
                width: 80.0,
                height: 24.0,
            },
            label: "new".into(),
            mode: FieldMode::SingleLine,
            font_size: 10.0,
            focused: false,
            cursor: None,
            insets: FieldInsets {
                left: 7.0,
                right: 7.0,
                top: 6.0,
                bottom: 3.0,
            },
            border_width: 1.0,
            caret_width: 1.5,
            background: Style::default(),
            border: Style::default(),
            focus_border: Style::default(),
            foreground: Style::default(),
        }
        .place(&Metrics, &mut viewport);
        assert_eq!(placed.viewport, FieldViewport::default());
        assert!(matches!(&placed.ops[2], Op::ClipRect { ops, .. }
            if matches!(&ops[0], Op::Text { text, .. } if text == "new")));
    }
}
