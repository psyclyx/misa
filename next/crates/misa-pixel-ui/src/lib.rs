//! Protocol-free pixel drawing and measured interactive primitives.
use misa_style::Style;

mod text;
pub use text::{LaidOutRow, TextFlow};

/// Skia-independent font measurements for pixel text layout.
///
/// `ascent` is negative above the baseline, as in Skia; a line whose top is
/// `y` has its baseline at `y - ascent`. `line_height` is the font's reported
/// spacing, not a guessed multiple of the font size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LineMetrics {
    pub ascent: f32,
    pub descent: f32,
    pub leading: f32,
    pub line_height: f32,
}

/// Measure with the same font and size that the painter uses for a text run.
pub trait TextMetrics: Send + Sync {
    /// Horizontal advance in pixels, not the ink bounds.
    fn measure(&self, text: &str, size: f32) -> f32;
    /// Prefix advances at every Unicode scalar boundary, beginning with zero.
    /// The last advance must equal `measure(text, size)`; boundaries describe
    /// the exact unshaped text run the painter draws.
    fn advances(&self, text: &str, size: f32) -> Vec<f32>;
    fn line_metrics(&self, size: f32) -> LineMetrics;
}

/// One thing to draw.
#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    /// A retained local scene, positioned without rebuilding its paint operations.
    Group {
        x: f32,
        y: f32,
        ops: std::sync::Arc<Vec<Op>>,
    },
    /// Clip child operations to a local pixel viewport (also inside translated groups).
    ClipRect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        ops: std::sync::Arc<Vec<Op>>,
    },
    Image {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        image: std::sync::Arc<image::RgbaImage>,
    },
    /// A run of text with `y` at the line top (the painter uses font ascent for its baseline).
    Text {
        x: f32,
        y: f32,
        size: f32,
        style: Style,
        text: String,
    },
    /// A filled rectangle, in device pixels.
    Rect {
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        style: Style,
    },
}

/// A drawable frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub width: f32,
    pub height: f32,
    pub ops: Vec<Op>,
}

/// Half-open pixel bounds shared by drawing and pointer targeting.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Rect {
    pub fn contains(self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.width && y < self.y + self.height
    }
}

/// A fixed-bound button. The label is measured with the painter's own font metrics;
/// its ink is clipped to the same bounds that receive pointer hits.
pub struct Button<Id> {
    pub id: Id,
    pub bounds: Rect,
    pub label: String,
    pub font_size: f32,
    pub background: Style,
    pub foreground: Style,
}

pub struct PlacedButton<Id> {
    pub id: Id,
    pub bounds: Rect,
    pub label_width: f32,
    pub ops: Vec<Op>,
}

impl<Id> Button<Id> {
    pub fn place(self, metrics: &dyn TextMetrics) -> PlacedButton<Id> {
        let bounds = self.bounds;
        let label_width = metrics.measure(&self.label, self.font_size);
        let line_height = metrics.line_metrics(self.font_size).line_height;
        let padding = 12.0_f32.min(bounds.width.max(0.0));
        // The advance is not an ink bound: clip to the available button area,
        // not precisely to the measured advance of the last glyph.
        let text_width = (bounds.width - padding).max(0.0);
        let text_y = bounds.y + ((bounds.height - line_height) / 2.0).max(0.0);
        PlacedButton {
            id: self.id,
            bounds,
            label_width,
            ops: vec![
                Op::Rect {
                    x: bounds.x,
                    y: bounds.y,
                    width: bounds.width,
                    height: bounds.height,
                    style: self.background,
                },
                Op::ClipRect {
                    x: bounds.x + padding,
                    y: bounds.y,
                    width: text_width,
                    height: bounds.height,
                    ops: std::sync::Arc::new(vec![Op::Text {
                        x: bounds.x + padding,
                        y: text_y,
                        size: self.font_size,
                        style: self.foreground,
                        text: self.label,
                    }]),
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
                .map(|i| i as f32 * size)
                .collect()
        }
        fn line_metrics(&self, size: f32) -> LineMetrics {
            LineMetrics {
                ascent: -size,
                descent: 0.0,
                leading: 0.0,
                line_height: size,
            }
        }
    }

    #[test]
    fn measured_label_resize_paint_and_hit_agree() {
        for width in [30.0, 90.0] {
            let bounds = Rect {
                x: 5.0,
                y: 7.0,
                width,
                height: 20.0,
            };
            let placed = Button {
                id: 42,
                bounds,
                label: "hello".into(),
                font_size: 10.0,
                background: Style::default(),
                foreground: Style::default(),
            }
            .place(&Metrics);
            assert_eq!(placed.id, 42);
            assert_eq!(placed.label_width, 50.0);
            assert!(bounds.contains(5.0 + width - 1.0, 8.0));
            assert!(!bounds.contains(5.0 + width, 8.0));
            assert!(
                matches!(placed.ops[0], Op::Rect { x: 5.0, y: 7.0, width: w, height: 20.0, .. } if w == width)
            );
            assert!(
                matches!(&placed.ops[1], Op::ClipRect { x: 17.0, width: w, ops, .. }
                if *w == width - 12.0 && matches!(&ops[0], Op::Text { text, .. } if text == "hello"))
            );
        }
    }
}
