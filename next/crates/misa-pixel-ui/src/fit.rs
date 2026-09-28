//! The one box model every control is built on. Content measures itself; the
//! box hugs it within the space its caller allows or fills that space; insets
//! keep content off the edges; and only what genuinely does not fit is
//! clipped. Nothing here knows what the content is — a text line, a bar, a
//! mark and an arrow are the same problem, solved once.
use crate::{Op, Rect};
use misa_style::Style;
use std::sync::Arc;

/// The extent of measured content, in the same units as [`Rect`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Size {
    pub width: f32,
    pub height: f32,
}
impl Size {
    /// Content that does not size its box (the box fills its allowance).
    pub const ZERO: Self = Self {
        width: 0.0,
        height: 0.0,
    };
}

/// Room content keeps away from the box edges.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Insets {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}
impl Insets {
    pub const ZERO: Self = Self {
        left: 0.0,
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
    };
    pub const fn horizontal(left: f32, right: f32) -> Self {
        Self {
            left,
            top: 0.0,
            right,
            bottom: 0.0,
        }
    }
}

/// How a box sizes against the allowance its caller gives it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Fit {
    /// The box is the allowance, whatever its content measures.
    Fill,
    /// The box hugs its content across the width and fills the height.
    HugWidth,
    /// The box hugs its content in both axes.
    Hug,
}

/// A box fitted to its content under the shared rule.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Fitted {
    /// The box itself: backgrounds and hit targets use this.
    pub bounds: Rect,
    /// What is left for content after insets. Clipping here is what makes
    /// content "fit"; this is the only line where anything is ever cut.
    pub inner: Rect,
}

impl Fitted {
    /// Size a box to `content` within `allowance`. Hugging grows with the
    /// content up to the allowance — never past it.
    pub fn new(allowance: Rect, fit: Fit, insets: Insets, content: Size) -> Self {
        let horizontal = (insets.left + insets.right).max(0.0);
        let vertical = (insets.top + insets.bottom).max(0.0);
        let needed = natural(content, insets);
        let (width, height) = match fit {
            Fit::Fill => (allowance.width, allowance.height),
            Fit::HugWidth => (needed.width.min(allowance.width), allowance.height),
            Fit::Hug => (
                needed.width.min(allowance.width),
                needed.height.min(allowance.height),
            ),
        };
        let bounds = Rect {
            x: allowance.x,
            y: allowance.y,
            width: width.max(0.0),
            height: height.max(0.0),
        };
        let inner = Rect {
            x: bounds.x + insets.left,
            y: bounds.y + insets.top,
            width: (bounds.width - horizontal).max(0.0),
            height: (bounds.height - vertical).max(0.0),
        };
        Self { bounds, inner }
    }
}

/// The space content needs once its insets are added: the hug target before
/// any allowance caps it. Callers measuring ahead of placement want this.
pub fn natural(content: Size, insets: Insets) -> Size {
    Size {
        width: content.width + insets.left + insets.right,
        height: content.height + insets.top + insets.bottom,
    }
}

/// Clip paint to `bounds`. The one clip implementation.
pub fn clip(bounds: Rect, ops: Vec<Op>) -> Vec<Op> {
    vec![Op::ClipRect {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width.max(0.0),
        height: bounds.height.max(0.0),
        ops: Arc::new(ops),
    }]
}

/// Measure one line of text for the box model: its advance and line box.
pub fn measure_line(text: &str, size: f32, metrics: &dyn crate::TextMetrics) -> Size {
    let height = metrics.line_metrics(size).line_height;
    assert!(height > 0.0, "content requires positive line spacing");
    Size {
        width: metrics.measure(text, size),
        height,
    }
}

/// Place one line vertically centered in `inner`, clipped to it. Text is one
/// kind of content; this is its placement, not a box rule. The line box comes
/// from the metrics — only callers that size a box to their text need
/// [`measure_line`].
pub fn text_line(
    inner: Rect,
    text: String,
    size: f32,
    color: Style,
    metrics: &dyn crate::TextMetrics,
) -> Op {
    let line_height = metrics.line_metrics(size).line_height;
    assert!(line_height > 0.0, "content requires positive line spacing");
    let drawn = Op::Text {
        x: inner.x,
        y: inner.y + ((inner.height - line_height) / 2.0).max(0.0),
        size,
        style: color,
        text,
    };
    clip(inner, vec![drawn]).remove(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LineMetrics, TextMetrics};

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
    fn allowance(width: f32) -> Rect {
        Rect {
            x: 5.0,
            y: 7.0,
            width,
            height: 20.0,
        }
    }

    #[test]
    fn hug_width_grows_with_content_up_to_the_allowance() {
        let content = Size {
            width: 50.0,
            height: 10.0,
        };
        let insets = Insets::horizontal(12.0, 12.0);
        // Room: the box hugs its content and keeps the caller's height.
        let fitted = Fitted::new(allowance(90.0), Fit::HugWidth, insets, content);
        assert_eq!(fitted.bounds.width, 74.0);
        assert_eq!(fitted.bounds.height, 20.0);
        assert_eq!(fitted.inner.width, 50.0);
        // No room: the allowance wins and the content area shrinks to it.
        let fitted = Fitted::new(allowance(30.0), Fit::HugWidth, insets, content);
        assert_eq!(fitted.bounds.width, 30.0);
        assert_eq!(fitted.inner.width, 6.0);
    }

    #[test]
    fn fill_keeps_the_allowance_and_insets_the_content() {
        let fitted = Fitted::new(
            allowance(90.0),
            Fit::Fill,
            Insets::horizontal(7.0, 29.0),
            Size {
                width: 10.0,
                height: 10.0,
            },
        );
        assert_eq!(fitted.bounds.width, 90.0);
        assert_eq!(fitted.inner.x, 12.0);
        assert_eq!(fitted.inner.width, 54.0);
    }

    #[test]
    fn one_line_is_measured_centered_and_clipped() {
        let line = measure_line("hello", 10.0, &Metrics);
        assert_eq!(
            line,
            Size {
                width: 50.0,
                height: 10.0
            }
        );
        let inner = Rect {
            x: 5.0,
            y: 7.0,
            width: 40.0,
            height: 20.0,
        };
        let op = text_line(inner, "hello".into(), 10.0, Style::default(), &Metrics);
        match op {
            Op::ClipRect {
                x,
                y,
                width,
                height,
                ops,
            } => {
                assert_eq!((x, y, width, height), (5.0, 7.0, 40.0, 20.0));
                match &ops[0] {
                    Op::Text { x, y, text, .. } => {
                        assert_eq!((*x, *y), (5.0, 12.0));
                        assert_eq!(text, "hello");
                    }
                    other => panic!("expected text, found {other:?}"),
                }
            }
            other => panic!("expected a clip, found {other:?}"),
        }
    }
}
