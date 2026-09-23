//! The incremental live-stream renderer.
//!
//! A live stream is text that arrived in pieces. [`Live`] parses the accumulated
//! text as markdown, reuses the blocks a previous parse already laid out, and
//! produces the same [`Line`] rows a settled body would. It is deliberately
//! independent of the screen: the caller supplies the resolved [`Paint`] for the
//! stream's role and the current width.

use misa_lines::Line;
use misa_proto::Node;
use misa_proto::sync::Stream;
use misa_render::{Style, Theme};

/// How many trailing lines of a collapsed thinking stream stay visible while it
/// grows. The settled block replaces them with its head preview.
pub const THINKING_TAIL_LINES: usize = 3;

/// Whether a stream role belongs to the model reasoning rather than to its answer.
pub fn thinking_stream(role: &str) -> bool {
    role.starts_with("message.assistant.thinking") || role.starts_with("message.thinking")
}

/// The order two live streams of one pending message render in.
///
/// A message has a thinking stream and a text stream, and the answer must not
/// appear above the reasoning that produced it. The suffix decides that, not the
/// stream id's alphabetical order (`text` sorts before `thinking`).
pub fn stream_order(id: &str) -> (String, u8) {
    let (owner, suffix) = id.rsplit_once('.').unwrap_or((id, ""));
    let rank = match suffix {
        "thinking" => 0,
        "text" => 1,
        _ => 2,
    };
    (owner.to_string(), rank)
}

/// Resolved presentation of one live stream at a terminal width.
pub struct Paint<'a> {
    /// The theme the stream's blocks are laid out against.
    pub theme: &'a Theme,
    /// The rail glyph and style for the stream's role, when the theme names one.
    pub rail: Option<(String, Style)>,
    /// The surface colour painted behind the stream's rows.
    pub surface: Option<Style>,
    /// The wrapping width left after the rail.
    pub width: usize,
}

impl<'a> Paint<'a> {
    /// Resolve the presentation of `role` against `theme` at `width` columns.
    pub fn of(theme: &'a Theme, width: u16, role: &str) -> Self {
        let rail = theme.rail(role);
        let inset = rail
            .as_ref()
            .map_or(0, |(glyph, _)| misa_render::width(glyph));
        Self {
            theme,
            surface: theme.surface(role),
            rail,
            width: (width as usize).saturating_sub(inset).max(1),
        }
    }
}

/// One stream being rendered as it grows.
pub struct Live {
    pub stream: Stream,
    pub lines: Vec<Line>,
    /// The parsed markdown of the in-flight text, reused block by block as the
    /// stream grows, so streaming renders through the same path as a settled body.
    document: Option<misa_markdown::Document>,
    /// Each block's laid-out lines, so only the blocks an append changed are
    /// rendered again. Invalidated by a width change.
    rendered: Vec<(Node, Vec<Line>)>,
    rendered_width: usize,
    pub last_nonblank: usize,
    /// Some(limit) while a collapsed thinking stream follows its own tail: the
    /// reader is watching the current reasoning, not the beginning of a summary
    /// that has not been written yet. An opened stream keeps every line.
    tail: Option<usize>,
}

impl Live {
    /// An empty live stream over `stream`. When `tail` is `Some(limit)` the
    /// rendering is windowed to its last `limit` lines, which is how a collapsed
    /// thinking stream follows its own tail.
    pub fn of(stream: Stream, tail: Option<usize>) -> Self {
        Self {
            stream,
            lines: Vec::new(),
            document: None,
            rendered: Vec::new(),
            rendered_width: 0,
            last_nonblank: 0,
            tail,
        }
    }

    /// Append a delta and re-render through the markdown path.
    pub fn append(&mut self, text: &str, paint: &Paint) {
        self.stream.text.push_str(text);
        // Every live stream, collapsed thinking included, renders through the
        // markdown path a settled body uses. The tail is a view over the result.
        self.markdown(paint);
    }

    /// Parse the in-flight text as markdown and lay it out, reusing the previous
    /// parse's blocks where it can.
    pub fn markdown(&mut self, paint: &Paint) {
        let text = std::mem::take(&mut self.stream.text);
        let id = self.stream.id.clone();
        let document = misa_markdown::document(&self.stream.role, &text, self.document.as_ref());
        let changed = self
            .document
            .as_ref()
            .is_none_or(|previous| previous.source != document.source);
        if changed {
            let width = paint.width;
            let reuse = self.rendered_width == width;
            let mut rendered = Vec::with_capacity(document.blocks.len());
            for (index, block) in document.blocks.iter().enumerate() {
                if let Some((node, lines)) = self.rendered.get(index)
                    && reuse
                    && node == block
                {
                    rendered.push((node.clone(), lines.clone()));
                    continue;
                }
                let lines = misa_lines::render_block(block, paint.theme, width, 0)
                    .into_iter()
                    .map(|mut line| {
                        line.node = Some(id.clone());
                        if let Some((glyph, rail_style)) = &paint.rail {
                            line.spans.insert(0, (*rail_style, glyph.clone()));
                        }
                        line.surface = line.surface.or(paint.surface);
                        line
                    })
                    .collect();
                rendered.push((block.clone(), lines));
            }
            self.lines = rendered
                .iter()
                .flat_map(|(_, lines)| lines.iter().cloned())
                .collect();
            self.rendered = rendered;
            self.rendered_width = width;
            // A collapsed thinking stream is a window onto the tail of the same
            // lines every other stream renders. Keeping only the last rows here
            // means the window holds no rows the viewport can scroll to.
            if let Some(limit) = self.tail {
                let skip = self.lines.len().saturating_sub(limit);
                self.lines.drain(..skip);
            }
            self.last_nonblank = self
                .lines
                .iter()
                .rposition(|line| !line.is_blank())
                .map_or(0, |index| index + 1);
        }
        self.document = Some(document);
        self.stream.text = text;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(id: &str, role: &str, text: &str) -> Stream {
        Stream {
            id: id.into(),
            role: role.into(),
            text: text.into(),
        }
    }

    fn paint<'a>(theme: &'a Theme, width: u16, role: &str) -> Paint<'a> {
        Paint::of(theme, width, role)
    }

    #[test]
    fn a_streamed_answer_renders_markdown_not_plain_text() {
        let theme = Theme::dark();
        let paint = paint(&theme, 60, "message.assistant");
        let mut live = Live::of(stream("msg1.text", "message.assistant", ""), None);
        live.append("a **bold** word\n\n```rust\nlet x = 1;\n```", &paint);
        let rows: Vec<String> = live.lines.iter().map(Line::text).collect();
        assert!(
            live.lines.iter().any(|line| line
                .spans
                .iter()
                .any(|(style, text)| style.bold && text.trim() == "bold")),
            "{:?}",
            live.lines
        );
        assert!(!rows.iter().any(|row| row.contains("**")), "{rows:?}");
        assert!(rows.iter().any(|row| row.contains("rust")), "{rows:?}");
        assert!(rows.iter().any(|row| row.contains("let x")), "{rows:?}");
    }

    #[test]
    fn an_incrementally_streamed_answer_rewraps_only_its_last_segment() {
        let theme = Theme::dark();
        let paint = paint(&theme, 20, "message.assistant");
        let mut live = Live::of(stream("msg1.body", "message.assistant", ""), None);
        for delta in ["alpha ", "beta\n", "gamma ", "delta"] {
            live.append(delta, &paint);
        }
        assert_eq!(
            live.lines.iter().map(Line::text).collect::<Vec<_>>(),
            vec!["┃ alpha beta", "┃ gamma delta"]
        );
    }

    #[test]
    fn a_collapsed_thinking_stream_follows_its_tail() {
        let theme = Theme::dark();
        let paint = paint(&theme, 40, "message.assistant.thinking");
        let mut live = Live::of(
            stream(
                "msg1.thinking",
                "message.assistant.thinking",
                "one\ntwo\nthree\nfour\nfive",
            ),
            Some(THINKING_TAIL_LINES),
        );
        live.append("", &paint);
        let rows: Vec<String> = live.lines.iter().map(Line::text).collect();
        assert_eq!(rows, vec!["┃ three", "┃ four", "┃ five"]);
    }

    #[test]
    fn an_opened_thinking_stream_keeps_every_line() {
        let theme = Theme::dark();
        let paint = paint(&theme, 40, "message.assistant.thinking");
        let mut live = Live::of(
            stream(
                "msg1.thinking",
                "message.assistant.thinking",
                "one\ntwo\nthree\nfour\nfive",
            ),
            None,
        );
        live.append("", &paint);
        let rows: Vec<String> = live.lines.iter().map(Line::text).collect();
        assert_eq!(rows, vec!["┃ one", "┃ two", "┃ three", "┃ four", "┃ five"]);
    }

    #[test]
    fn thinking_roles_and_stream_order_are_recognised() {
        assert!(thinking_stream("message.assistant.thinking"));
        assert!(thinking_stream("message.thinking.text"));
        assert!(!thinking_stream("message.assistant"));
        assert!(stream_order("msg1.thinking") < stream_order("msg1.text"));
        assert_eq!(stream_order("msg1.thinking"), ("msg1".into(), 0));
        assert_eq!(stream_order("msg1.text"), ("msg1".into(), 1));
        assert_eq!(stream_order("msg1.tool"), ("msg1".into(), 2));
    }

    #[test]
    fn paint_insets_the_width_by_the_rail() {
        let theme = Theme::dark();
        let painted = paint(&theme, 40, "message.assistant.thinking");
        assert!(painted.rail.is_some());
        assert!(painted.width < 40);
        let bare = paint(&theme, 40, "assistant");
        assert!(bare.rail.is_none());
        assert_eq!(bare.width, 40);
    }
}
