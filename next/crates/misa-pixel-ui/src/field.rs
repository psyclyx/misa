//! Measured, clipped, single or multi-line editable field presentation.
//! Editing policy belongs to the caller; the cursor is a byte offset into the
//! *displayed* label, so masked or formatted values can map their own indices.
use crate::{Op, Rect, TextMetrics};
use misa_style::Style;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FieldViewport {
    pub x: f32,
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
        let lines: Vec<_> = self.label.split('\n').collect();
        // A changed or shortened value may invalidate an old viewport even when
        // this field no longer owns focus. Never leave its surviving text hidden.
        viewport.line = viewport.line.min(lines.len() - 1);
        if !self.focused || self.cursor.is_none() {
            let width = metrics.measure(lines[viewport.line], self.font_size);
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
                let before = &self.label[..at];
                let line = before.bytes().filter(|byte| *byte == b'\n').count();
                let column = before.rsplit('\n').next().unwrap_or("").chars().count();
                if line < viewport.line {
                    viewport.line = line;
                } else if line >= viewport.line.saturating_add(visible_lines) {
                    viewport.line = line + 1 - visible_lines;
                }
                let edge = metrics.advances(lines[line], self.font_size)[column];
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
                text: (*value).into(),
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
