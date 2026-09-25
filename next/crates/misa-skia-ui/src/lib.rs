//! Backend-neutral pixel scene and interaction model.
//!
//! # What is shared and what is not
//!
//! A pixel frontend cannot use the terminal's lines: it has no cells, and it wants
//! real type. What it *can* share is everything that is not a cell —
//! [`misa_render::text`] for measurement, [`misa_render::Theme`] for what a role
//! looks like, and the tree itself.
//!
//! So this frontend maps the same tree to a **scene** of positioned runs and
//! rectangles, and that mapping is the interesting part: it is where a role becomes
//! a colour and a weight, where a rail becomes a bar, and where a code block becomes
//! a raised panel. It is testable with no window and no GPU, which is why the scene
//! is built and tested without a painter or a window.
//!
//! Text flow wraps to measured pixel width and stacks runs; a rail is a drawn
//! bar, a code block is highlighted from the client's own grammar, and nothing
//! below a role becomes a terminal cell.

use misa_proto::view::{Span, SpanKind};
use misa_render::Theme;
use misa_style::Style;

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

/// Map one semantic run onto the resolved style for its kind, over a base style.
pub(crate) fn span_style(theme: &Theme, span: &Span, base: Style) -> Style {
    match &span.kind {
        SpanKind::Plain => base,
        SpanKind::Strong => base.over(theme.role("bold")),
        SpanKind::StrongEmphasis => base.over(theme.role("bold")).over(theme.role("italic")),
        SpanKind::Emphasis => base.over(theme.role("italic")),
        SpanKind::Strikethrough => base.over(theme.role("strikethrough")),
        SpanKind::Underline => base.over(theme.role("underline")),
        SpanKind::Highlight => base.over(theme.role("highlight")),
        SpanKind::Subscript => base.over(theme.role("subscript")),
        SpanKind::Superscript => base.over(theme.role("superscript")),
        SpanKind::Kbd => base.over(theme.role("keybinding")),
        SpanKind::Code => base.over(theme.role("code")),
        SpanKind::Link { .. } => base.over(theme.role("link")).underline(),
    }
}

/// Whether a code node's body is a diff.
///
/// Two ways for a session to say so, and neither of them looks at the body: a role
/// that ends in `.diff`, or a fence a model wrote with `diff`.
pub(crate) fn is_diff(role: &str, lang: Option<&str>) -> bool {
    role.ends_with(".diff") || lang.is_some_and(|lang| lang.eq_ignore_ascii_case("diff"))
}

/// What a line of a unified diff is, named the way a theme names a role.
fn diff_suffix(raw: &str) -> Option<&'static str> {
    Some(match raw {
        line if line.starts_with("+++ ") || line.starts_with("--- ") => "header",
        line if line.starts_with("@@") => "hunk",
        line if line.starts_with("diff ") || line.starts_with("index ") => "meta",
        line if line.starts_with("\\ No newline") => "meta",
        line if line.starts_with('+') => "added",
        line if line.starts_with('-') => "removed",
        _ => return None,
    })
}

/// The style for one line of a diff, by what the line starts with.
pub(crate) fn diff_style(theme: &Theme, base: Style, role: &str, raw: &str) -> Style {
    let Some(suffix) = diff_suffix(raw) else {
        return base;
    };
    let specific = format!("{role}.{suffix}");
    if theme.names(&specific) {
        base.over(theme.role(&specific))
    } else {
        base.over(theme.role(&format!("diff.{suffix}")))
    }
}

/// Split one clipped code line by the captures that cover it.
pub(crate) fn code_runs(
    offset: usize,
    clipped: &str,
    captures: &[misa_syntax::Capture],
    base: Style,
    theme: &Theme,
) -> Vec<(Style, String)> {
    if captures.is_empty() {
        return vec![(base, clipped.to_string())];
    }
    // Captures are sorted and non-overlapping. Skip earlier lines once, then
    // advance the capture cursor alongside the visible bytes of this line.
    let mut next = captures.partition_point(|capture| capture.end as usize <= offset);
    let mut spans = Vec::new();
    let mut cursor = 0usize;
    while cursor < clipped.len() {
        let absolute = offset + cursor;
        while next < captures.len() && captures[next].end as usize <= absolute {
            next += 1;
        }
        let capture = captures
            .get(next)
            .filter(|capture| capture.start as usize <= absolute);
        let end = match capture {
            Some(capture) => (capture.end as usize).saturating_sub(offset),
            None => captures.get(next).map_or(clipped.len(), |capture| {
                (capture.start as usize).saturating_sub(offset)
            }),
        }
        .min(clipped.len());
        let end = floor_char_boundary(clipped, end);
        // Valid grammar captures have character-aligned boundaries. Still make
        // progress if a caller supplies a range in the middle of a UTF-8 scalar.
        let end = if end <= cursor {
            cursor + clipped[cursor..].chars().next().unwrap().len_utf8()
        } else {
            end
        };
        let style = capture
            .map(|capture| theme.token(&capture.token))
            .unwrap_or(base);
        spans.push((style, clipped[cursor..end].to_string()));
        cursor = end;
    }
    if spans.is_empty() {
        spans.push((base, String::new()));
    }
    spans
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    if index >= text.len() {
        return text.len();
    }
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

pub mod app;
pub mod appearance;

#[cfg(test)]
mod tests {
    use super::*;
    use misa_syntax::Capture;

    #[test]
    fn dense_long_code_line_keeps_the_same_coloured_visible_prefix() {
        let theme = Theme::dark();
        let base = theme.role("markdown.code");
        let unit = "let x = 1; ";
        let long = unit.repeat(2000);
        let captures = (0..2000)
            .flat_map(|index| {
                let start = (index * unit.len()) as u32;
                [
                    Capture {
                        start,
                        end: start + 3,
                        token: "keyword".into(),
                    },
                    Capture {
                        start: start + 8,
                        end: start + 9,
                        token: "number".into(),
                    },
                ]
            })
            .collect::<Vec<_>>();
        let visible = &long[..unit.len() * 2 + 3];
        let clipped = code_runs(0, visible, &captures, base, &theme);
        let mut full = code_runs(0, &long, &captures, base, &theme);
        let mut bytes = visible.len();
        full.retain_mut(|(_, text)| {
            if bytes == 0 {
                return false;
            }
            text.truncate(text.len().min(bytes));
            bytes -= text.len();
            true
        });
        assert_eq!(clipped, full);
        assert_eq!(clipped[0], (theme.token("keyword"), "let".into()));
        assert_eq!(clipped[1], (base, " x = ".into()));
        assert_eq!(clipped[2], (theme.token("number"), "1".into()));
        assert_eq!(
            clipped.last(),
            Some(&(theme.token("keyword"), "let".into()))
        );
    }

    #[test]
    fn code_runs_respect_original_byte_offsets_and_unicode_boundaries() {
        let theme = Theme::dark();
        let base = theme.role("markdown.code");
        // The earlier line's multibyte character and newline count towards the
        // second line's capture coordinates; the capture spans both lines.
        let offset = "界\n".len();
        let captures = vec![Capture {
            start: 0,
            end: (offset + 3) as u32,
            token: "keyword".into(),
        }];
        assert_eq!(
            code_runs(offset, "let x", &captures, base, &theme),
            vec![(theme.token("keyword"), "let".into()), (base, " x".into())]
        );
        assert_eq!(
            code_runs(offset, "", &captures, base, &theme),
            vec![(base, String::new())]
        );
        let unicode = vec![Capture {
            start: 0,
            end: "界".len() as u32,
            token: "string".into(),
        }];
        assert_eq!(
            code_runs(0, "界let", &unicode, base, &theme),
            vec![(theme.token("string"), "界".into()), (base, "let".into())]
        );
    }
}
