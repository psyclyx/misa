//! Text measurement and wrapping.
//!
//! The previous system learned that cell measurement belongs in native code and
//! decided nothing else did. This keeps that division and makes it explicit: this
//! module measures and breaks, and it never decides what a thing looks like.

use misa_proto::view::{Span, SpanKind};
use unicode_width::UnicodeWidthStr;

use crate::Style;

/// The display width of text, in columns.
///
/// Wide characters are two columns, combining marks are none. This is the number
/// a terminal and a fixed-advance layout both mean.
pub fn width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

/// Cut text to at most `columns`, never inside a character.
pub fn clip(text: &str, columns: usize) -> String {
    split_at_width(text, columns).0
}

/// Allocate column widths within `total` cells.
///
/// As many columns as fit at `minimum` wide, no more than `maximum`, separated by
/// `gap`. Any remainder goes to the leading columns, so a split of an odd number of
/// cells never drops one. This is the previous system's `layout.columns`.
pub fn columns(total: usize, minimum: usize, maximum: usize, gap: usize) -> Vec<usize> {
    let total = total.max(1);
    let minimum = minimum.max(1);
    let maximum = maximum.max(1);
    let count = maximum.min((total + gap) / (minimum + gap)).max(1);
    let usable = total.saturating_sub(gap * (count - 1)).max(count);
    let base = usable / count;
    let extra = usable % count;
    (0..count)
        .map(|index| base + usize::from(index < extra))
        .collect()
}

/// Pad text on the right to exactly `columns`, clipping when it is longer.
pub fn pad(text: &str, columns: usize) -> String {
    let mut out = clip(text, columns);
    let used = width(&out);
    for _ in used..columns {
        out.push(' ');
    }
    out
}

/// Break inline content into lines of at most `columns`, keeping each run's kind.
///
/// Three rules, in order, and each one exists because a real transcript needed it:
///
/// 1. A word that fits on the current line stays there.
/// 2. A word that does not fit moves to the next line, and the whitespace it moved
///    across is dropped rather than left at the end of the previous line.
/// 3. A word longer than a whole line is broken at the column. A URL, a path, or a
///    base64 blob must widen the layout, not overflow it.
///
/// A newline inside a run is a break, not a word. Emphasis spanning a break stays
/// on both halves, because a renderer that dropped it would silently change what
/// the author emphasised.
pub fn wrap_spans(spans: &[Span], columns: usize) -> Vec<Vec<Span>> {
    if columns == 0 {
        return vec![spans.to_vec()];
    }
    let mut out = Lines::new(columns);
    // Whitespace at the end of one run separates it from the first word of the
    // next; that boundary is data, not padding, so it is carried across runs —
    // together with the kind of the run it came from, so `a **bold**` does not
    // paint the space before `bold` bold.
    let mut pending: Option<SpanKind> = None;
    for span in spans {
        for piece in split_pieces(&span.text, &span.kind, &mut pending) {
            match piece {
                Piece::Break => out.break_line(),
                Piece::Word { text, space } => out.word(&text, space.as_ref(), &span.kind),
            }
        }
    }
    out.finish()
}

/// Wrap already-styled terminal text without throwing its styles away.
///
/// Pickers and chrome have presentation spans rather than protocol spans, so
/// routing them through [`wrap_spans`] would either leak a fake semantic kind or
/// clip an entire row. This is the same word/wide-token policy, with styles
/// carried through the break.
pub fn wrap_styled(spans: &[(Style, String)], columns: usize) -> Vec<Vec<(Style, String)>> {
    if columns == 0 {
        return vec![spans.to_vec()];
    }
    let mut lines = Vec::new();
    let mut current: Vec<(Style, String)> = Vec::new();
    let mut used = 0;
    for (style, text) in spans {
        for character in text.chars() {
            if character == '\n' {
                lines.push(std::mem::take(&mut current));
                used = 0;
                continue;
            }
            let value = character.to_string();
            let cells = width(&value).max(1);
            if used > 0 && used + cells > columns {
                lines.push(std::mem::take(&mut current));
                used = 0;
            }
            // Whitespace at a physical line boundary is layout noise; whitespace inside a
            // row is data and must survive. In particular, picker marker/key/label spans
            // are separately styled and cannot be reconstructed from word boundaries.
            if used == 0 && character.is_whitespace() {
                continue;
            }
            if let Some((last_style, last)) = current.last_mut()
                && *last_style == *style
            {
                last.push(character);
            } else {
                current.push((*style, value));
            }
            used += cells;
        }
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(current);
    }
    lines
}

#[cfg(test)]
mod styled_tests {
    use super::*;

    #[test]
    fn styled_layout_preserves_separately_styled_spaces() {
        let rows = wrap_styled(
            &[
                (Style::PLAIN.bold(), "a".into()),
                (Style::PLAIN, " ".into()),
                (Style::PLAIN.dim(), "b".into()),
            ],
            8,
        );
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0].1, "a");
        assert_eq!(rows[0][1], (Style::PLAIN, " ".into()));
        assert_eq!(rows[0][2].1, "b");
    }
}

enum Piece {
    Break,
    Word {
        text: String,
        /// The kind of the whitespace that separated this word from the one before
        /// it, when there was any. It belongs to the run the space came from.
        space: Option<SpanKind>,
    },
}

/// Split a run into words, remembering which had whitespace before them and which
/// run that whitespace came from.
///
/// `pending` is threaded in and out so that a run boundary is not a word boundary:
/// `hello *world*` keeps the space between its plain and emphasised runs, and
/// `a**b**` does not gain one. A space that ends a run keeps that run's kind as it
/// crosses into the next, which is what keeps `a **bold**`'s space plain.
fn split_pieces(text: &str, kind: &SpanKind, pending: &mut Option<SpanKind>) -> Vec<Piece> {
    let mut out = Vec::new();
    for (index, segment) in text.split('\n').enumerate() {
        if index > 0 {
            out.push(Piece::Break);
        }
        let mut rest = segment;
        while !rest.is_empty() {
            let trimmed = rest.trim_start_matches(char::is_whitespace);
            if trimmed.len() != rest.len() {
                *pending = Some(kind.clone());
                rest = trimmed;
            }
            if rest.is_empty() {
                break;
            }
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            out.push(Piece::Word {
                text: rest[..end].to_string(),
                space: pending.take(),
            });
            rest = &rest[end..];
        }
    }
    out
}

struct Lines {
    columns: usize,
    lines: Vec<Vec<Span>>,
    current: Vec<Span>,
    used: usize,
}

impl Lines {
    fn new(columns: usize) -> Self {
        Lines {
            columns,
            lines: Vec::new(),
            current: Vec::new(),
            used: 0,
        }
    }

    fn break_line(&mut self) {
        self.lines.push(std::mem::take(&mut self.current));
        self.used = 0;
    }

    fn word(&mut self, text: &str, space: Option<&SpanKind>, kind: &SpanKind) {
        let size = width(text);
        let separator = usize::from(space.is_some() && self.used > 0);
        if self.used + separator + size <= self.columns {
            if separator == 1 {
                // The separator keeps the kind of the run it came from rather than
                // the kind of the word that follows it.
                self.push(" ", space.expect("a separator has a kind"));
            }
            self.push(text, kind);
            self.used += separator + size;
            return;
        }
        if self.used > 0 {
            self.break_line();
        }
        // The line is empty now. Either the word fits, or it has to be broken.
        if size <= self.columns {
            self.push(text, kind);
            self.used = size;
            return;
        }
        let mut rest = text.to_string();
        while width(&rest) > self.columns {
            let (take, kept) = split_at_width(&rest, self.columns);
            if take.is_empty() {
                break;
            }
            self.push(&take, kind);
            self.lines.push(std::mem::take(&mut self.current));
            rest = kept;
            self.used = 0;
        }
        if !rest.is_empty() {
            self.push(&rest, kind);
            self.used = width(&rest);
        }
    }

    fn push(&mut self, text: &str, kind: &SpanKind) {
        if text.is_empty() {
            return;
        }
        if let Some(last) = self.current.last_mut()
            && &last.kind == kind
        {
            last.text.push_str(text);
            return;
        }
        self.current.push(Span {
            text: text.to_string(),
            kind: kind.clone(),
        });
    }

    fn finish(mut self) -> Vec<Vec<Span>> {
        if !self.current.is_empty() || self.lines.is_empty() {
            self.lines.push(self.current);
        }
        self.lines
    }
}

/// Break a single token at the column, since it cannot be moved to another line.
///
/// Returns what fits and the remainder, both on character boundaries.
fn split_at_width(text: &str, columns: usize) -> (String, String) {
    if width(text) <= columns {
        return (text.to_string(), String::new());
    }
    let mut used = 0usize;
    let mut end = 0usize;
    for (index, ch) in text.char_indices() {
        let size = width(&ch.to_string());
        if used + size > columns {
            break;
        }
        used += size;
        end = index + ch.len_utf8();
    }
    (text[..end].to_string(), text[end..].to_string())
}

/// Whether a span carries no semantic marking at all.
pub fn is_plain(span: &Span) -> bool {
    matches!(span.kind, SpanKind::Plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Span {
        Span::plain(text)
    }

    fn text_of(lines: &[Vec<Span>]) -> Vec<String> {
        lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|span| span.text.clone())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn width_counts_wide_characters_as_two() {
        assert_eq!(width("abc"), 3);
        assert_eq!(width("日本"), 4);
        assert_eq!(width(""), 0);
    }

    #[test]
    fn clip_never_splits_a_character() {
        assert_eq!(clip("hello", 3), "hel");
        assert_eq!(clip("日本語", 3), "日");
        assert_eq!(clip("hi", 9), "hi");
    }

    #[test]
    fn pad_fills_and_clips() {
        assert_eq!(pad("ab", 5), "ab   ");
        assert_eq!(pad("abcdef", 3), "abc");
    }

    #[test]
    fn columns_fit_as_many_as_possible_and_share_the_remainder() {
        assert_eq!(columns(100, 28, 3, 2), vec![32, 32, 32]);
        assert_eq!(columns(80, 28, 3, 2), vec![39, 39]);
        assert_eq!(columns(20, 28, 3, 2), vec![20]);
        // An odd remainder goes to the leading column, never dropped.
        assert_eq!(columns(100, 10, 3, 3), vec![32, 31, 31]);
        assert!(columns(1, 1, 4, 0).len() <= 4);
    }

    #[test]
    fn a_sentence_wraps_on_whitespace() {
        let lines = wrap_spans(&[plain("one two three four")], 9);
        assert_eq!(text_of(&lines), vec!["one two", "three", "four"]);
    }

    #[test]
    fn the_whitespace_a_word_moved_across_is_not_left_behind() {
        let lines = wrap_spans(&[plain("aaaa bbbb cccc")], 4);
        assert_eq!(text_of(&lines), vec!["aaaa", "bbbb", "cccc"]);
    }

    #[test]
    fn an_unbreakable_token_is_broken_rather_than_overflowing() {
        let lines = wrap_spans(&[plain("averylongword")], 4);
        assert_eq!(text_of(&lines), vec!["aver", "ylon", "gwor", "d"]);
        for line in &lines {
            let width: usize = line.iter().map(|span| width(&span.text)).sum();
            assert!(width <= 4);
        }
    }

    #[test]
    fn a_long_token_after_a_short_one_still_fits() {
        let lines = wrap_spans(&[plain("a supercalifragilistic")], 6);
        assert_eq!(
            text_of(&lines),
            vec!["a", "superc", "alifra", "gilist", "ic"]
        );
    }

    #[test]
    fn a_newline_in_a_run_ends_a_line() {
        let lines = wrap_spans(&[plain("first\nsecond")], 80);
        assert_eq!(text_of(&lines), vec!["first", "second"]);
    }

    #[test]
    fn emphasis_that_spans_a_break_stays_on_both_halves() {
        let spans = vec![Span {
            text: "one two three".into(),
            kind: SpanKind::Strong,
        }];
        let lines = wrap_spans(&spans, 7);
        assert_eq!(text_of(&lines), vec!["one two", "three"]);
        for line in &lines {
            for span in line {
                assert_eq!(span.kind, SpanKind::Strong);
            }
        }
    }

    #[test]
    fn whitespace_between_styled_runs_is_not_lost() {
        // A space that ends a plain run belongs between it and the next run.
        let lines = wrap_spans(
            &[
                plain("hello "),
                Span {
                    text: "world".into(),
                    kind: SpanKind::Emphasis,
                },
                plain(" again"),
            ],
            80,
        );
        assert_eq!(text_of(&lines), vec!["hello world again"]);
        // And a run that does not end in whitespace stays glued to the next.
        let lines = wrap_spans(
            &[
                plain("a"),
                Span {
                    text: "b".into(),
                    kind: SpanKind::Strong,
                },
                plain("c"),
            ],
            80,
        );
        assert_eq!(text_of(&lines), vec!["abc"]);
    }

    #[test]
    fn the_space_before_a_styled_run_keeps_the_kind_of_its_own_run() {
        // `a **bold**` is a plain run that owns the trailing space followed by a bold
        // run that owns none of it; the space must not be painted bold.
        let lines = wrap_spans(
            &[
                plain("a "),
                Span {
                    text: "bold".into(),
                    kind: SpanKind::Strong,
                },
            ],
            80,
        );
        assert_eq!(text_of(&lines), vec!["a bold"]);
        let bold = lines[0]
            .iter()
            .find(|span| span.kind == SpanKind::Strong)
            .expect("a bold run");
        assert_eq!(bold.text, "bold");
        assert!(!bold.text.starts_with(' '), "{:?}", lines[0]);
        // The space and the word it precedes are not glued into the bold run.
        assert!(
            lines[0]
                .iter()
                .any(|span| span.kind == SpanKind::Plain && span.text == "a "),
            "{:?}",
            lines[0]
        );
    }

    #[test]
    fn adjacent_runs_of_the_same_kind_merge_and_different_ones_do_not() {
        let lines = wrap_spans(&[plain("a"), plain("b")], 80);
        assert_eq!(lines[0].len(), 1);
        assert_eq!(lines[0][0].text, "ab");

        let lines = wrap_spans(
            &[
                plain("a"),
                Span {
                    text: "b".into(),
                    kind: SpanKind::Code,
                },
            ],
            80,
        );
        assert_eq!(lines[0].len(), 2);
    }

    #[test]
    fn a_link_keeps_its_target_across_a_break() {
        let spans = vec![Span::link(
            "a very long link label",
            "https://example.invalid",
        )];
        let lines = wrap_spans(&spans, 8);
        assert!(lines.len() >= 3);
        for span in lines.iter().flatten() {
            assert_eq!(
                span.kind,
                SpanKind::Link {
                    href: "https://example.invalid".into()
                }
            );
        }
    }

    #[test]
    fn wrapping_at_zero_columns_returns_the_content_unchanged() {
        let lines = wrap_spans(&[plain("anything")], 0);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0][0].text, "anything");
    }

    #[test]
    fn whitespace_only_content_produces_one_empty_line() {
        let lines = wrap_spans(&[plain("   ")], 10);
        assert_eq!(text_of(&lines), vec![""]);
    }
}
