//! Misa-specific semantic document presentation for pixel frontends.
//!
//! This adapter interprets protocol nodes, Misa theme roles and local editing
//! policy. Protocol-free scene, font measurement and interactive primitives
//! belong to `misa-pixel-ui`; this crate is not the reusable toolkit. Its scenes
//! are painted by the same Skia renderer in offline tests and native windows.

use misa_proto::view::{Span, SpanKind};
use misa_render::Theme;
use misa_style::Style;

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
