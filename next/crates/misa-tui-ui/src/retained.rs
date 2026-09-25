//! Retained terminal viewport, selection and frame assembly.
mod document;
use crate::Screen;
use document::image_max_rows;
use document::{DocumentIndex, Mutation};
use misa_lines::Line;
use misa_proto::Node;
use misa_proto::sync::{Stream, StreamUpdate, ViewOp};
use misa_terminal_ui::viewport::{Head, Request, Viewport};
use std::ops::AddAssign;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Work {
    pub formatted_nodes: usize,
    pub appended_bytes: usize,
    pub copied_rows: usize,
    pub index_steps: usize,
}
impl AddAssign for Work {
    fn add_assign(&mut self, rhs: Self) {
        self.formatted_nodes += rhs.formatted_nodes;
        self.appended_bytes += rhs.appended_bytes;
        self.copied_rows += rhs.copied_rows;
        self.index_steps += rhs.index_steps;
    }
}
pub struct Retained {
    document: DocumentIndex,
    selection_body: Option<crate::select::Body>,
    viewport: Viewport<String, crate::select::Spot>,
    pub work: Work,
}
impl Retained {
    pub fn new(view: Node, screen: &Screen) -> Self {
        let (document, work) = DocumentIndex::new(view, screen);
        Self {
            document,
            selection_body: None,
            viewport: Viewport::new(screen.follow),
            work,
        }
    }
    pub fn interaction(&self) -> Node {
        self.document.interaction()
    }
    pub fn has_turn(&self) -> bool {
        self.document.has_turn()
    }
    pub fn image_blobs(&self) -> Vec<misa_proto::view::BlobRef> {
        self.document.image_blobs()
    }
    /// Kitty placements for the image rows present in a finished frame.
    ///
    /// Only the first reserved row of an image anchors it, so a placement is
    /// emitted once per image however many rows it covers. Rows outside the frame
    /// produce no placement, which is how a scrolled-away image is deleted.
    fn image_placements(
        &self,
        screen: &Screen,
        lines: &[Line],
    ) -> Vec<misa_terminal_ui::graphics::Placement> {
        if !screen.graphics.enabled() {
            return Vec::new();
        }
        let max_rows = image_max_rows(screen);
        let mut seen = std::collections::HashSet::new();
        lines
            .iter()
            .enumerate()
            .filter_map(|(row, line)| {
                let id = line.node.as_deref()?;
                if !seen.insert(id) {
                    return None;
                }
                let hash = self.document.image_hash(id)?;
                screen.graphics.place(
                    hash,
                    (row as u16, line.indent as u16),
                    screen.width.saturating_sub(line.indent as u16),
                    max_rows,
                )
            })
            .collect()
    }
    pub fn selection_key(
        &mut self,
        screen: &mut Screen,
        key: &crate::Key,
    ) -> Option<crate::KeyOut> {
        if !screen.reading_key(key) {
            return None;
        }
        // A key can extend the selection into text that arrived since its last
        // movement. Passive stream repaint reuses the prior selection body.
        self.selection_body = None;
        self.cache_selection();
        screen.selection_in(self.selection_body.as_ref().unwrap(), key)
    }
    fn cache_selection(&mut self) {
        if self.selection_body.is_none() {
            let all = self.document.selection_lines();
            self.selection_body = Some(crate::select::Body::of(&all));
        }
    }
    fn viewport_start(&mut self, screen: &Screen, room: usize) -> usize {
        let request = Request {
            scroll: screen.scroll,
            follow: screen.follow,
            intent: screen.scroll_intent,
            room,
            head: screen.selection.as_ref().map(|selection| {
                let identity = selection.head();
                Head {
                    identity,
                    row: identity.row,
                }
            }),
        };
        let document = &self.document;
        self.viewport.resolve(
            request,
            document.total_rows(),
            |row| document.row_node(row),
            |key, offset| document.node_row(key, offset),
        )
    }
    fn apply_mutation(&mut self, mutation: Mutation) {
        if mutation.rebuilt {
            self.work = mutation.work;
        } else {
            self.work += mutation.work;
        }
        if mutation.layout_changed {
            self.viewport.layout_changed();
            self.selection_body = None;
        }
    }
    pub fn resolved_scroll(&self) -> usize {
        self.viewport.resolved_scroll()
    }
    pub fn following(&self) -> bool {
        self.viewport.following()
    }
    pub fn apply_ops(&mut self, ops: &[ViewOp], screen: &Screen) -> Result<(), String> {
        let mutation = self.document.apply_ops(ops, screen)?;
        self.apply_mutation(mutation);
        Ok(())
    }
    pub fn changed(
        &mut self,
        tree: &[ViewOp],
        live: &[StreamUpdate],
        reset_live: bool,
        screen: &Screen,
    ) -> Result<(), String> {
        let mutation = self.document.changed(tree, live, reset_live, screen)?;
        self.apply_mutation(mutation);
        Ok(())
    }
    pub fn reset(&mut self, tree: Node, streams: &[Stream], screen: &Screen) {
        let mutation = self.document.reset(tree, streams, screen);
        self.work = mutation.work;
        self.selection_body = None;
        self.viewport = Viewport::new(screen.follow);
        self.viewport.layout_changed();
    }
    pub fn local(&mut self, screen: &Screen) {
        let mutation = self.document.local(screen);
        if mutation.rebuilt {
            self.viewport = Viewport::new(screen.follow);
        }
        self.apply_mutation(mutation);
        self.selection_body = None;
    }
    pub fn placed_lines(&self, limit: usize) -> (Vec<Line>, Vec<Line>) {
        self.document.placed_lines(limit)
    }
    #[cfg(test)]
    fn current(&mut self, stream: Stream, screen: &Screen) {
        self.changed(&[], &[StreamUpdate::Current { stream }], false, screen)
            .unwrap();
    }
    #[cfg(test)]
    fn op(&mut self, op: &ViewOp, screen: &Screen) -> Result<(), String> {
        self.apply_ops(std::slice::from_ref(op), screen)
    }
    #[cfg(test)]
    pub fn draw(&mut self, screen: &Screen) -> Vec<Line> {
        self.frame(screen, None).lines
    }
    #[cfg(test)]
    pub fn frame(&mut self, screen: &Screen, staging: Option<&str>) -> crate::chrome::Frame {
        self.frame_with(screen, staging, &[], &[])
    }
    pub fn frame_with(
        &mut self,
        screen: &Screen,
        staging: Option<&str>,
        extra_document: &[Line],
        extra_footer: &[Line],
    ) -> crate::chrome::Frame {
        let mut top = crate::chrome::top(screen, self.document.attachments(), None);
        if screen.dialogs.modal() || self.document.has_panel() {
            // Dialogs and session panels are overlays. They own input focus, but
            // do not erase the transcript or the status bar beneath them.
            let status: Vec<_> = self
                .document
                .footer_lines(false)
                .chain(extra_footer.iter().cloned())
                .take(1)
                .collect();
            top.truncate(screen.height as usize);
            let available = (screen.height as usize).saturating_sub(top.len() + status.len());
            let overlay = if screen.dialogs.modal() {
                screen.dialogs.lines(
                    &screen.theme,
                    screen.width as usize,
                    screen.dialog_settings(),
                )
            } else {
                self.document.surface_lines(screen)
            };
            let overlay = crate::chrome::physical(overlay, screen.width as usize);
            let overlay_len = overlay.len().min(available);
            let room = available.saturating_sub(overlay_len);
            let start = self.viewport_start(screen, room);
            let (mut middle, steps) = self.document.visible_rows(start, room);
            self.work.index_steps += steps;
            middle.extend_from_slice(
                &extra_document[..extra_document.len().min(room.saturating_sub(middle.len()))],
            );
            let cursor_row = top.len() + middle.len();
            let cursor_column = overlay
                .first()
                .map(|line| {
                    misa_render::width(&line.text()).min(screen.width.saturating_sub(1) as usize)
                })
                .unwrap_or(0);
            middle.extend(overlay.into_iter().take(overlay_len));
            let mut lines = top;
            lines.extend(middle);
            lines.extend(status);
            let images = self.image_placements(screen, &lines);
            return crate::chrome::Frame {
                lines,
                cursor_row,
                cursor_column,
                images,
            };
        }
        if screen
            .picker
            .as_ref()
            .is_some_and(misa_kit::picker::Picker::is_overlay)
        {
            // The reference picker is an overlay: the transcript remains visible
            // above it, the picker occupies the available middle rows, and status
            // remains the bottom region. The normal composer is intentionally not
            // part of this layout while the picker owns input focus.
            let status: Vec<_> = self.document.footer_lines(false).collect();
            let status = status
                .into_iter()
                .chain(extra_footer.iter().cloned())
                .take(1)
                .collect::<Vec<_>>();
            let picker = crate::chrome::physical(
                crate::picker_lines(screen, screen.picker.as_ref().unwrap()),
                screen.width as usize,
            );
            let available = (screen.height as usize).saturating_sub(top.len() + status.len());
            let picker_len = picker.len().min(available);
            let room = available.saturating_sub(picker_len);
            let start = self.viewport_start(screen, room);
            let (mut lines, steps) = self.document.visible_rows(start, room);
            self.work.index_steps += steps;
            // Contributions are document furniture, so an overlay must not make
            // them disappear. They consume the same document room as the retained
            // transcript, just as they do in the normal frame.
            lines.extend_from_slice(
                &extra_document[..extra_document.len().min(room.saturating_sub(lines.len()))],
            );
            let cursor_column = screen
                .picker
                .as_ref()
                .map(|picker| {
                    let prefix = match picker.accept {
                        misa_kit::picker::Accept::Run => "/",
                        misa_kit::picker::Accept::Action { .. } => ":",
                        misa_kit::picker::Accept::Argument { .. } => "",
                    };
                    let query = format!("  {}: {}{}", picker.title, prefix, picker.query);
                    misa_render::width(&query).min(screen.width.saturating_sub(1) as usize)
                })
                .unwrap_or(0);
            let cursor_row = top.len() + lines.len();
            lines.extend(picker.into_iter().take(picker_len));
            lines.extend(status);
            let mut frame_lines = top;
            frame_lines.extend(lines);
            let images = self.image_placements(screen, &frame_lines);
            return crate::chrome::Frame {
                lines: frame_lines,
                cursor_row,
                cursor_column,
                images,
            };
        }
        let preferred_input = crate::chrome::composer(screen);
        let dock: Vec<_> = self.document.footer_lines(true).collect();
        let dock = dock
            .into_iter()
            .chain(staging.into_iter().map(|text| Line {
                surface: None,
                indent: 0,
                node: None,
                spans: vec![(screen.theme.role("keybinding"), text.to_string())],
            }))
            .collect::<Vec<_>>();
        let dock = crate::chrome::physical(dock, screen.width as usize);
        let status: Vec<_> = self.document.footer_lines(false).collect();
        // The reference reserves one status row, the queue immediately above the
        // editor, and at least one row for the transcript when the terminal allows
        // it. The editor itself is capped at half the terminal height.
        let status = status
            .into_iter()
            .chain(extra_footer.iter().cloned())
            .take(1)
            .collect::<Vec<_>>();
        let available = (screen.height as usize).saturating_sub(top.len() + status.len());
        let editor_rows = preferred_input
            .lines
            .len()
            .min((screen.height as usize / 2).max(1))
            .min(available);
        let input = crate::chrome::composer_with_budget(screen, editor_rows);
        // One blank row separates the transcript (and its dock) from the composer.
        // It is reserved before the transcript so the composer's rows are fixed.
        let separator = usize::from(available > editor_rows);
        let remaining = available
            .saturating_sub(editor_rows)
            .saturating_sub(separator);
        let dock = &dock[..dock.len().min(remaining.saturating_sub(1))];
        let remaining = remaining.saturating_sub(dock.len());
        let completions = screen
            .picker
            .as_ref()
            .filter(|picker| picker.is_inline())
            .map(|picker| crate::completion_lines(screen, picker))
            .unwrap_or_default();
        // The reference gives inline completions the second half of the space
        // left after the dock, leaving the first half for transcript rows.
        let completion_count = completions.len().min(remaining / 2);
        let room = remaining.saturating_sub(completion_count);
        let extra_document = &extra_document[..extra_document.len().min(room)];
        let room = room.saturating_sub(extra_document.len());
        let start = self.viewport_start(screen, room);
        let (mut lines, steps) = self.document.visible_rows(start, room);
        self.work.index_steps += steps;
        self.work.copied_rows += lines.len();
        if let Some(selection) = &screen.selection {
            // Selection is explicit local work. It uses cached rows, never tree formatting.
            if self.selection_body.is_none() {
                let all = self.document.selection_lines();
                self.selection_body = Some(crate::select::Body::of(&all));
            }
            for (i, line) in lines.iter_mut().enumerate() {
                if let Some((from, to)) =
                    selection.on_row(self.selection_body.as_ref().unwrap(), start + i)
                {
                    crate::select_highlight(line, from, to, &screen.theme);
                }
            }
        }
        lines.extend_from_slice(extra_document);
        // Session-owned input docks (for example the queued-prompt preview) belong
        // immediately above the composer. Status is a bottom bar in the reference
        // layout, so independently composed status documents stay after the chrome.
        let cursor_row = top.len()
            + lines.len()
            + dock.len()
            + separator
            + input.cursor_row.min(editor_rows.saturating_sub(1));
        lines.extend_from_slice(dock);
        if separator > 0 {
            lines.push(Line::default());
        }
        lines.extend(input.lines.into_iter().take(editor_rows));
        lines.extend(completions.into_iter().take(completion_count));
        let mut frame_lines = top;
        frame_lines.extend(lines);
        frame_lines.extend(status);
        let images = self.image_placements(screen, &frame_lines);
        crate::chrome::Frame {
            lines: frame_lines,
            cursor_row,
            cursor_column: input.cursor_column,
            images,
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn stream_growth_invalidates_selection_without_rebuilding_owners() {
        use super::*;
        let screen = Screen::new(20, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "reply.body".into(),
                role: "assistant".into(),
                text: "hello".into(),
            },
            &screen,
        );
        retained.cache_selection();
        assert!(retained.selection_body.is_some());
        retained.work = Work::default();
        retained
            .changed(
                &[],
                &[StreamUpdate::Append {
                    id: "reply.body".into(),
                    offset: 5,
                    text: " world".into(),
                }],
                false,
                &screen,
            )
            .unwrap();
        assert!(retained.selection_body.is_none());
        assert_eq!(retained.work.formatted_nodes, 0);
        assert_eq!(retained.work.appended_bytes, 6);
        assert_eq!(retained.document.stream_text("reply.body"), "hello world");
    }

    #[test]
    fn independent_documents_keep_their_own_node_identities_and_placement() {
        let screen = crate::Screen::new(80, 20);
        let make = |text: &str| {
            misa_proto::Node::text("notice", [misa_proto::view::Span::plain(text)]).id("same")
        };
        let mut left = super::Retained::new(make("left"), &screen);
        let right = super::Retained::new(make("right"), &screen);
        left.op(
            &misa_proto::sync::ViewOp::Replace {
                id: "same".into(),
                node: make("changed"),
            },
            &screen,
        )
        .unwrap();

        assert!(
            left.placed_lines(20)
                .0
                .iter()
                .any(|line| line.text().contains("changed"))
        );
        assert!(
            right
                .placed_lines(20)
                .0
                .iter()
                .any(|line| line.text().contains("right"))
        );
        let mut main = super::Retained::new(make("conversation"), &screen);
        let extras = left
            .placed_lines(20)
            .0
            .into_iter()
            .chain(right.placed_lines(20).0)
            .collect::<Vec<_>>();
        let frame = main.frame_with(&screen, None, &extras, &[]);
        assert!(
            frame
                .lines
                .iter()
                .any(|line| line.text().contains("conversation"))
        );
        assert!(
            frame
                .lines
                .iter()
                .any(|line| line.text().contains("changed"))
        );
        assert!(frame.lines.iter().any(|line| line.text().contains("right")));
    }
    use super::*;
    use misa_linear::{Paint, THINKING_TAIL_LINES};
    use misa_proto::view::Kind;
    use misa_proto::view::Span;
    fn text(id: &str, value: &str) -> Node {
        Node::text("assistant", [Span::plain(value)]).id(id)
    }
    fn document(history: usize) -> Node {
        let mut transcript = Node::section("transcript").id("transcript");
        for i in 0..history {
            transcript.children.push(
                Node::section("message")
                    .id(format!("msg{i}"))
                    .child(text(&format!("msg{i}.body"), "settled content")),
            );
        }
        Node::section("session")
            .id("session")
            .child(text("header", "Header"))
            .child(transcript)
    }
    fn all(retained: &Retained) -> Vec<Line> {
        let mut lines = retained.document.all_lines();
        while lines.last().is_some_and(Line::is_blank) {
            lines.pop();
        }
        lines
    }
    fn oracle(retained: &Retained, screen: &Screen) {
        assert_eq!(
            all(retained),
            misa_lines::render(
                &screen.resolve(&retained.document.snapshot()),
                &screen.theme,
                screen.width as usize
            )
        );
    }
    fn image_node(width: u32, height: u32) -> Node {
        Node::new(
            "screenshot",
            Kind::Image {
                blob: misa_proto::view::BlobRef {
                    hash: "b".repeat(64),
                    len: 1,
                    media: Some("image/png".into()),
                },
                alt: "a wide chart".into(),
                width,
                height,
            },
        )
        .id("picture")
    }
    #[test]
    fn a_supported_image_reserves_its_placement_rows() {
        let mut screen = Screen::new(80, 24);
        screen.graphics = misa_terminal_ui::graphics::Kitty::new(
            true,
            misa_terminal_ui::graphics::CellSize {
                width: 10,
                height: 20,
            },
        );
        let hash = "b".repeat(64);
        screen.graphics.insert(
            &hash,
            image::RgbaImage::from_raw(400, 100, vec![0; 400 * 100 * 4]).unwrap(),
        );
        let mut retained = Retained::new(image_node(400, 100), &screen);
        // 400px at 10px/cell is 40 columns wide, 100px at 20px/cell is 5 rows.
        assert_eq!(retained.document.total_rows(), 5);
        let frame = retained.frame(&screen, None);
        assert_eq!(frame.images.len(), 1);
        assert_eq!(frame.images[0].rows, 5);
        assert!(frame.images[0].escape.contains("a=T"));
    }
    #[test]
    fn an_unsupported_image_keeps_its_single_placeholder_row() {
        let screen = Screen::new(80, 24);
        let retained = Retained::new(image_node(400, 100), &screen);
        assert_eq!(retained.document.total_rows(), 1);
        assert!(
            all(&retained)
                .iter()
                .any(|line| line.text().contains("[image: a wide chart]"))
        );
    }
    #[test]
    fn a_scrolled_viewport_keeps_its_node_when_content_arrives_above() {
        use misa_proto::sync::ViewOp;
        let mut screen = Screen::new(40, 12);
        let document = |count: usize| {
            let mut root = Node::section("session").id("session");
            for index in 0..count {
                root.children
                    .push(text(&format!("t{index}"), &format!("line {index}")));
            }
            root
        };
        let mut retained = Retained::new(document(12), &screen);
        screen.follow = false;
        screen.scroll = 3;
        screen.scroll_intent = 1;
        let start = retained.viewport_start(&screen, 5);
        assert_eq!(start, 3);
        assert_eq!(
            retained
                .document
                .row_line(start)
                .and_then(|line| line.node.as_deref()),
            Some("t3")
        );
        // A line arrives above the reader. The physical row shifts; the anchor does
        // not, so the frame starts at the same node it did before.
        retained
            .op(
                &ViewOp::Insert {
                    parent: "session".into(),
                    before: Some("t0".into()),
                    node: text("new", "line new"),
                },
                &screen,
            )
            .unwrap();

        let after = retained.viewport_start(&screen, 5);
        assert_eq!(after, 4, "the anchor moved down with its node");
        assert_eq!(
            retained
                .document
                .row_line(after)
                .and_then(|line| line.node.as_deref()),
            Some("t3")
        );
    }

    #[test]
    fn a_wrapped_semantic_row_keeps_its_run_offset_after_an_insert() {
        let mut screen = Screen::new(20, 12);
        let mut root = Node::section("session").id("session");
        root.children.push(text("wrapped", &"word ".repeat(30)));
        for i in 0..10 {
            root.children.push(text(&format!("tail{i}"), "tail"));
        }
        let mut retained = Retained::new(root, &screen);
        let first = (0..retained.document.total_rows())
            .find(|&row| {
                retained
                    .document
                    .row_line(row)
                    .and_then(|line| line.node.as_deref())
                    == Some("wrapped")
            })
            .unwrap();
        assert_eq!(
            retained
                .document
                .row_line(first + 1)
                .unwrap()
                .node
                .as_deref(),
            Some("wrapped")
        );
        screen.follow = false;
        screen.scroll = first + 1;
        screen.scroll_intent = 1;
        assert_eq!(retained.viewport_start(&screen, 2), first + 1);
        retained
            .apply_ops(
                &[ViewOp::Insert {
                    parent: "session".into(),
                    before: Some("wrapped".into()),
                    node: text("inserted", "new row"),
                }],
                &screen,
            )
            .unwrap();
        let start = retained.viewport_start(&screen, 2);
        assert_eq!(start, first + 2);
        assert_eq!(
            retained.document.row_line(start).unwrap().node.as_deref(),
            Some("wrapped")
        );
    }

    #[test]
    fn a_plain_repaint_does_not_rewalk_the_document_and_a_user_scroll_reanchors() {
        let mut screen = Screen::new(40, 12);
        let mut retained = Retained::new(document(6), &screen);
        screen.follow = false;
        screen.scroll = 2;
        screen.scroll_intent = 1;
        let first = retained.viewport_start(&screen, 3);
        assert_eq!(first, 2);
        // No new intent and no layout change: the resolution is reused.
        retained.work = Work::default();
        let again = retained.viewport_start(&screen, 3);
        assert_eq!(again, 2);
        assert_eq!(retained.work, Work::default());
        // A reader delta re-anchors against the row they asked for.
        screen.scroll = 4;
        screen.scroll_intent = 2;
        let scrolled = retained.viewport_start(&screen, 3);
        assert_eq!(scrolled, 4);
        assert_eq!(
            retained
                .document
                .row_line(scrolled)
                .and_then(|line| line.node.as_deref()),
            Some("msg3.body")
        );
    }

    #[test]
    fn moving_the_selection_reveals_it_once_without_fighting_a_later_scroll() {
        let mut screen = Screen::new(40, 12);
        let mut retained = Retained::new(document(20), &screen);
        screen.follow = false;
        screen.scroll = 0;
        screen.scroll_intent = 1;
        assert_eq!(retained.viewport_start(&screen, 4), 0);
        // The caret moves below the viewport; the frame scrolls it into view.
        screen.selection = Some(crate::select::Selection::caret(crate::select::Spot::new(
            10, 0,
        )));
        assert_eq!(retained.viewport_start(&screen, 4), 7);
        // The same selection on the next repaint does not move the viewport again.
        assert_eq!(retained.viewport_start(&screen, 4), 7);
        // A reader's scroll is respected; the unchanged caret does not pull back.
        screen.scroll = 2;
        screen.scroll_intent = 2;
        assert_eq!(retained.viewport_start(&screen, 4), 2);
        // Moving the caret again reveals the new position.
        screen.selection = Some(crate::select::Selection::caret(crate::select::Spot::new(
            2, 0,
        )));
        assert_eq!(retained.viewport_start(&screen, 4), 2);
    }

    #[test]
    fn following_tracks_the_tail_and_ignores_the_anchor() {
        let screen = Screen::new(40, 12);
        let mut retained = Retained::new(document(6), &screen);
        let total = retained.document.total_rows();
        assert!(screen.follow);
        assert_eq!(retained.viewport_start(&screen, 3), total.saturating_sub(3));
        assert!(retained.following());
    }

    #[test]
    fn a_scroll_clamps_to_the_last_page_and_resumes_following_at_the_bottom() {
        let mut screen = Screen::new(40, 12);
        let mut retained = Retained::new(document(20), &screen);
        let room = 4;
        let total = retained.document.total_rows();
        let bottom = total.saturating_sub(room);
        screen.follow = false;
        // A scroll well past the end lands on the last full page, never on
        // `total - 1` with a nearly empty viewport.
        screen.scroll = total + 5;
        screen.scroll_intent = 1;
        assert_eq!(retained.viewport_start(&screen, room), bottom);
        assert!(
            retained.following(),
            "reaching the bottom must resume following"
        );
        // New output then tracks the tail, because following is back on.
        retained
            .op(
                &ViewOp::Insert {
                    parent: "transcript".into(),
                    before: None,
                    node: text("extra", "extra"),
                },
                &screen,
            )
            .unwrap();

        let grown = retained.document.total_rows();
        assert_eq!(
            retained.viewport_start(&screen, room),
            grown.saturating_sub(room)
        );
    }

    #[test]
    fn the_wheel_scroll_handler_clamps_at_the_ends_and_resumes_following() {
        // The mouse wheel maps to `Key::ScrollPage`, the same handler the keyboard
        // scroll path uses. Driving it here is driving the wheel.
        let mut screen = Screen::new(40, 12);
        let mut retained = Retained::new(document(20), &screen);
        let room = 4;
        let total = retained.document.total_rows();
        let bottom = total.saturating_sub(room);
        screen.follow = false;
        // A wheel-up at the top cannot scroll past row zero.
        screen.key(crate::Key::ScrollPage(-30));
        assert_eq!(retained.viewport_start(&screen, room), 0);
        assert!(!retained.following());
        // Walking down to the last page resumes following.
        loop {
            screen.key(crate::Key::ScrollPage(3));
            let start = retained.viewport_start(&screen, room);
            screen.scroll = start;
            if start == bottom {
                break;
            }
        }
        assert!(retained.following());
        // One more notch stays clamped to the last page.
        screen.key(crate::Key::ScrollPage(3));
        assert_eq!(retained.viewport_start(&screen, room), bottom);
    }

    #[test]
    fn the_composer_keeps_its_row_when_the_transcript_scrolls() {
        let mut screen = Screen::new(40, 16);
        screen.editor.set_text("DRAFT");
        let mut retained = Retained::new(document(50), &screen);
        let composer_row = |frame: &crate::chrome::Frame| {
            frame
                .lines
                .iter()
                .position(|line| line.text().contains("DRAFT"))
                .expect("the composer")
        };
        let following = composer_row(&retained.frame(&screen, None));
        // Scrolling up and then back to the tail must not move the composer: it
        // is reserved at the bottom before the transcript gets its room.
        screen.follow = false;
        screen.scroll = 4;
        screen.scroll_intent = 1;
        assert_eq!(composer_row(&retained.frame(&screen, None)), following);
        screen.scroll = retained.document.total_rows();
        screen.scroll_intent = 2;
        assert_eq!(composer_row(&retained.frame(&screen, None)), following);
    }

    #[test]
    fn shared_document_updates_keep_token_work_incremental_and_settle_atomically() {
        for history in [100, 1000] {
            let screen = Screen::new(40, 16);
            let mut retained = Retained::new(document(history), &screen);
            retained.current(
                Stream {
                    id: "new.body".into(),
                    role: "assistant".into(),
                    text: String::new(),
                },
                &screen,
            );

            retained.work = Work::default();
            retained
                .changed(
                    &[],
                    &[StreamUpdate::Append {
                        id: "new.body".into(),
                        offset: 0,
                        text: "é🙂".into(),
                    }],
                    false,
                    &screen,
                )
                .unwrap();
            assert_eq!(retained.work.formatted_nodes, 0);
            assert_eq!(retained.work.appended_bytes, 6);
            assert!(retained.work.index_steps <= 24);
            retained
                .changed(
                    &[ViewOp::Insert {
                        parent: "transcript".into(),
                        before: None,
                        node: text("new", "é🙂"),
                    }],
                    &[StreamUpdate::End {
                        id: "new.body".into(),
                    }],
                    false,
                    &screen,
                )
                .unwrap();
            assert!(retained.document.stream_count() == 0);
            assert!(retained.document.contains("new"));
            oracle(&retained, &screen);
        }
    }
    #[test]
    fn canonical_operations_match_full_render_after_every_operation() {
        for _ in 0..6 {
            let mut screen = Screen::new(37, 24);
            let mut retained = Retained::new(document(20), &screen);
            oracle(&retained, &screen);
            let operations = [
                ViewOp::Insert {
                    parent: "transcript".into(),
                    before: Some("msg4".into()),
                    node: text("inserted", "inserted at a stable anchor"),
                },
                ViewOp::Replace {
                    id: "msg3.body".into(),
                    node: text(
                        "msg3.body",
                        "a much longer replacement that wraps into multiple lines at this width",
                    ),
                },
                ViewOp::Remove { id: "msg7".into() },
                ViewOp::Insert {
                    parent: "msg4".into(),
                    before: None,
                    node: text("msg4.extra", "another body"),
                },
                ViewOp::Replace {
                    id: "msg4".into(),
                    node: text("msg4", "section becomes content"),
                },
            ];
            for op in operations {
                retained.op(&op, &screen).unwrap();

                oracle(&retained, &screen);
            }
            screen.width = 19;
            retained.local(&screen);
            oracle(&retained, &screen);
        }
    }

    #[test]
    fn message_sections_are_rendered_as_one_rail_aware_owner() {
        let screen = Screen::new(40, 12);
        let view = Node::section("session").id("session").child(
            Node::section("transcript").id("transcript").child(
                Node::section("message.assistant").id("message.1").child(
                    Node::text(
                        "message.assistant.markdown.paragraph",
                        [Span::plain("hello")],
                    )
                    .id("message.1.body"),
                ),
            ),
        );
        let retained = Retained::new(view.clone(), &screen);
        let lines = all(&retained);
        assert!(
            lines.iter().any(|line| line.text().contains("┃ hello")),
            "{lines:?}"
        );
        assert_eq!(
            lines,
            misa_lines::render(&screen.resolve(&view), &screen.theme, screen.width as usize)
        );
    }

    #[test]
    fn a_live_stream_is_drawn_as_the_block_it_will_settle_into() {
        let screen = Screen::new(40, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.thinking".into(),
                role: "message.assistant.thinking".into(),
                text: "pondering".into(),
            },
            &screen,
        );

        let lines = all(&retained);
        let body = lines
            .iter()
            .find(|line| line.text().contains("pondering"))
            .expect("the streamed body");
        assert!(body.text().starts_with("┃ "), "{:?}", body.text());
        assert_eq!(
            body.surface,
            screen.theme.surface("message.assistant.thinking")
        );
    }

    #[test]
    fn a_pending_messages_thinking_stream_renders_before_its_text() {
        let screen = Screen::new(40, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.text".into(),
                role: "message.assistant".into(),
                text: "answer".into(),
            },
            &screen,
        );
        retained.current(
            Stream {
                id: "msg1.thinking".into(),
                role: "message.assistant.thinking".into(),
                text: "reason".into(),
            },
            &screen,
        );

        let rows: Vec<String> = all(&retained).iter().map(Line::text).collect();
        let thinking = rows.iter().position(|row| row.contains("reason")).unwrap();
        let answer = rows.iter().position(|row| row.contains("answer")).unwrap();
        assert!(thinking < answer, "{rows:?}");
    }

    #[test]
    fn a_streamed_answer_renders_markdown_not_plain_text() {
        let screen = Screen::new(60, 20);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.text".into(),
                role: "message.assistant".into(),
                text: "a **bold** word\n\n```rust\nlet x = 1;\n```".into(),
            },
            &screen,
        );

        let lines = all(&retained);
        let rows: Vec<String> = lines.iter().map(Line::text).collect();
        // The bold run is styled, not shown as `**bold**`.
        assert!(
            lines.iter().any(|line| line
                .spans
                .iter()
                .any(|(style, text)| style.bold && text.trim() == "bold")),
            "{:?}",
            lines
                .iter()
                .flat_map(|line| line
                    .spans
                    .iter()
                    .map(|(style, text)| (format!("{style:?}"), text.clone())))
                .collect::<Vec<_>>()
        );
        assert!(!rows.iter().any(|row| row.contains("**")), "{rows:?}");
        // The fence is a code block, with its language label and body.
        assert!(rows.iter().any(|row| row.contains("rust")), "{rows:?}");
        assert!(rows.iter().any(|row| row.contains("let x")), "{rows:?}");
    }

    #[test]
    fn an_incrementally_streamed_answer_rewraps_only_its_last_segment() {
        let screen = Screen::new(20, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.body".into(),
                role: "message.assistant".into(),
                text: String::new(),
            },
            &screen,
        );

        let paint = Paint::of(&screen.theme, screen.width, "message.assistant");
        for delta in ["alpha ", "beta\n", "gamma ", "delta"] {
            retained
                .document
                .append_unindexed("msg1.body", delta, &paint);
        }
        assert_eq!(
            retained
                .document
                .stream_lines("msg1.body")
                .iter()
                .map(Line::text)
                .collect::<Vec<_>>(),
            vec!["┃ alpha beta", "┃ gamma delta"]
        );
    }

    #[test]
    fn a_collapsed_thinking_tail_renders_markdown_like_the_expanded_stream() {
        let text = "intro line one\nintro line two\n\na **bold** word\n\n```rust\nlet x = 1;\n```";
        let stream = || Stream {
            id: "msg1.thinking".into(),
            role: "message.assistant.thinking".into(),
            text: text.into(),
        };
        let screen = Screen::new(40, 20);
        let mut collapsed = Retained::new(Node::section("session").id("session"), &screen);
        collapsed.current(stream(), &screen);

        let tail = all(&collapsed);

        let mut opened = Screen::new(40, 20);
        opened.set_opened(vec!["msg1.thinking".into()]);
        let mut expanded = Retained::new(Node::section("session").id("session"), &opened);
        expanded.current(stream(), &opened);

        let full = all(&expanded);
        // The collapsed window is exactly the last rows of the same rendering.
        assert_eq!(tail, full[full.len() - THINKING_TAIL_LINES..].to_vec());
        assert_eq!(tail.len(), THINKING_TAIL_LINES);
        // The window still carries markdown: the bold run is styled.
        assert!(
            tail.iter().any(|line| line
                .spans
                .iter()
                .any(|(style, text)| style.bold && text.trim() == "bold")),
            "{tail:?}"
        );
        // And the fenced block is laid out, language label and code row included.
        let rows: Vec<String> = tail.iter().map(Line::text).collect();
        assert!(rows.iter().any(|row| row.contains("rust")), "{rows:?}");
        assert!(
            rows.iter().any(|row| row.contains("let x = 1;")),
            "{rows:?}"
        );
    }

    #[test]
    fn resizing_a_collapsed_markdown_tail_reflows_and_keeps_the_last_rows() {
        let text = "```rust\nlet x = 1;\n```\n\nalpha **bold** beta gamma delta epsilon zeta eta theta iota kappa";
        let stream = || Stream {
            id: "msg1.thinking".into(),
            role: "message.assistant.thinking".into(),
            text: text.into(),
        };
        let mut screen = Screen::new(40, 20);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(stream(), &screen);

        let wide = all(&retained);

        // A narrower terminal rewraps the unified rendering and re-applies the
        // window, so the visible rows are still the most recent reasoning.
        screen.width = 24;
        retained.local(&screen);
        let narrow = all(&retained);
        assert_ne!(wide, narrow, "the stream must reflow at the new width");
        assert_eq!(narrow.len(), THINKING_TAIL_LINES);

        let mut opened = Screen::new(24, 20);
        opened.set_opened(vec!["msg1.thinking".into()]);
        let mut expanded = Retained::new(Node::section("session").id("session"), &opened);
        expanded.current(stream(), &opened);

        let full = all(&expanded);
        assert_eq!(narrow, full[full.len() - THINKING_TAIL_LINES..].to_vec());
    }

    #[test]
    fn a_collapsed_thinking_stream_follows_its_tail() {
        let screen = Screen::new(40, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.thinking".into(),
                role: "message.assistant.thinking".into(),
                text: "one\ntwo\nthree\nfour\nfive".into(),
            },
            &screen,
        );

        let rows: Vec<String> = all(&retained).iter().map(Line::text).collect();
        assert_eq!(rows, vec!["┃ three", "┃ four", "┃ five"]);
    }

    #[test]
    fn resizing_a_tailed_thinking_stream_reflows_and_keeps_the_tail() {
        let mut screen = Screen::new(40, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.thinking".into(),
                role: "message.assistant.thinking".into(),
                text: "alpha alpha\nalpha beta\nalpha gamma".into(),
            },
            &screen,
        );

        assert_eq!(
            all(&retained).iter().map(Line::text).collect::<Vec<_>>(),
            vec!["┃ alpha alpha", "┃ alpha beta", "┃ alpha gamma"]
        );
        // A narrower terminal rewraps the buffered text, word-aware, and re-applies
        // the tail, so the visible window is still the most recent reasoning.
        screen.width = 10;
        retained.local(&screen);
        assert_eq!(
            all(&retained).iter().map(Line::text).collect::<Vec<_>>(),
            vec!["┃ beta", "┃ alpha", "┃ gamma"]
        );
    }

    #[test]
    fn a_collapsed_thinking_tail_wraps_at_word_boundaries() {
        let screen = Screen::new(10, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.thinking".into(),
                role: "message.assistant.thinking".into(),
                text: "alpha gamma".into(),
            },
            &screen,
        );

        assert_eq!(
            all(&retained).iter().map(Line::text).collect::<Vec<_>>(),
            vec!["┃ alpha", "┃ gamma"]
        );
    }

    #[test]
    fn an_opened_thinking_stream_keeps_every_line() {
        let mut screen = Screen::new(40, 12);
        screen.set_opened(vec!["msg1.thinking".into()]);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.thinking".into(),
                role: "message.assistant.thinking".into(),
                text: "one\ntwo\nthree\nfour\nfive".into(),
            },
            &screen,
        );

        let rows: Vec<String> = all(&retained).iter().map(Line::text).collect();
        assert_eq!(rows, vec!["┃ one", "┃ two", "┃ three", "┃ four", "┃ five"]);
    }

    #[test]
    fn an_opened_thinking_block_keeps_one_rail_not_the_parents() {
        let mut screen = Screen::new(60, 16);
        screen.set_opened(vec!["msg.1.thinking".into()]);
        let view = Node::section("session").id("session").child(
            Node::section("transcript").id("transcript").child(
                Node::section("message.assistant").id("msg.1").child(
                    Node::new(
                        "message.assistant.thinking",
                        Kind::Collapsible {
                            summary: vec![Span::strong("thinking")],
                        },
                    )
                    .id("msg.1.thinking")
                    .child(Node::text(
                        "message.assistant.thinking.text",
                        [Span::plain("one\ntwo")],
                    )),
                ),
            ),
        );
        let retained = Retained::new(view, &screen);
        let lines = all(&retained);
        let rows: Vec<String> = lines.iter().map(Line::text).collect();
        // The assistant's padding, then the thinking block's own padding and body.
        assert_eq!(rows, vec!["┃ ", "┃ ", "┃ one", "┃ two", "┃ ", "", "┃ "]);
        // The rail is the thinking block's own, not the assistant's stacked on it.
        assert_eq!(
            lines[1].spans[0].0,
            screen.theme.role("message.assistant.thinking.rail")
        );
    }

    #[test]
    fn a_closed_tool_call_shows_one_name_under_its_own_rail() {
        let screen = Screen::new(60, 16);
        let view = Node::section("session").id("session").child(
            Node::section("transcript").id("transcript").child(
                Node::section("message.assistant").id("msg.1").child(
                    Node::new(
                        "tool.call",
                        Kind::Collapsible {
                            summary: vec![Span::strong("echo"), Span::plain(" ")],
                        },
                    )
                    .id("msg.1.call.1")
                    .label("echo")
                    .child(Node::text("tool.call.args", [Span::plain("hi")])),
                ),
            ),
        );
        let retained = Retained::new(view, &screen);
        let rows: Vec<String> = all(&retained).iter().map(Line::text).collect();
        assert_eq!(rows, vec!["┃ ", "┃ ◇ echo", "┃ "]);
    }

    #[test]
    fn a_closed_thinking_block_previews_a_few_lines_under_the_rail() {
        let screen = Screen::new(60, 16);
        let view = Node::section("session").id("session").child(
            Node::section("transcript").id("transcript").child(
                Node::section("message.assistant").id("msg.1").child(
                    Node::new(
                        "message.assistant.thinking",
                        Kind::Collapsible {
                            summary: vec![
                                Span::strong("thinking"),
                                Span::plain(" · one\ntwo\nthree\n… 3 lines hidden"),
                            ],
                        },
                    )
                    .id("msg.1.thinking")
                    .child(Node::text(
                        "message.assistant.thinking.text",
                        [Span::plain("one\ntwo\nthree\nfour\nfive\nsix")],
                    )),
                ),
            ),
        );
        let retained = Retained::new(view, &screen);
        let lines = all(&retained);
        let rows = lines.iter().map(Line::text).collect::<Vec<_>>();
        assert_eq!(
            rows,
            vec![
                "┃ ",
                "┃ ",
                "┃ thinking · one",
                "┃ two",
                "┃ three",
                "┃ … 3 lines hidden",
                "┃ ",
                "",
                "┃ ",
            ]
        );
        assert_eq!(
            lines[1].spans[0].0,
            screen.theme.role("message.assistant.thinking.rail")
        );
    }

    #[test]
    fn a_canonical_panel_is_exclusive_and_does_not_become_transcript_content() {
        let screen = Screen::new(40, 12);
        let view = Node::section("session")
            .id("session")
            .child(
                Node::section("transcript")
                    .id("transcript")
                    .child(text("body", "conversation")),
            )
            .child(
                Node::section("panel")
                    .id("login")
                    .label("Credential")
                    .action(misa_proto::view::Action {
                        id: "panel.close".into(),
                        on: misa_proto::view::ActionOn::Click,
                        label: Some("Close".into()),
                        args: misa_value::Value::Null,
                    })
                    .child(Node::text("panel.copy", [Span::plain("enter a token")])),
            );
        let mut retained = Retained::new(view, &screen);
        let frame = retained.frame(&screen, None);
        let rows = frame.lines.iter().map(Line::text).collect::<Vec<_>>();
        assert!(rows.iter().any(|row| row.contains("Credential")));
        assert!(rows.iter().any(|row| row.contains("enter a token")));
        assert!(rows.iter().any(|row| row.contains("esc Close")), "{rows:?}");
        assert!(
            rows.iter().position(|row| row.contains("Credential"))
                > rows.iter().position(|row| row.contains("misa"))
        );
        assert!(
            !retained
                .placed_lines(64)
                .0
                .iter()
                .any(|line| line.text().contains("Credential"))
        );
    }
    #[test]
    fn append_formatting_and_viewport_copy_work_do_not_grow_with_history() {
        let mut results = vec![];
        let mut baselines = vec![];
        for history in [100, 1000] {
            for _ in 0..6 {
                let screen = Screen::new(40, 16);
                let mut retained = Retained::new(document(history), &screen);
                retained.current(
                    Stream {
                        id: "msg9999.body".into(),
                        role: "assistant".into(),
                        text: "current ".into(),
                    },
                    &screen,
                );

                retained.work = Work::default();
                let mut offset = "current ".len();
                let mut baseline_visits = 0;
                let mut baseline_output = String::new();
                for text in ["one ", "two ", "界🙂", "\n", "final"] {
                    retained
                        .changed(
                            &[],
                            &[StreamUpdate::Append {
                                id: "msg9999.body".into(),
                                offset,
                                text: text.into(),
                            }],
                            false,
                            &screen,
                        )
                        .unwrap();
                    offset += text.len();
                    let rendered = retained.draw(&screen);
                    assert!(
                        rendered
                            .iter()
                            .any(|line| line.node.as_deref() == Some("msg9999.body"))
                    );
                    // The previous full-tree presentation path remains callable as
                    // a baseline. Count its actual recursive resolve visits.
                    let legacy = retained.interaction();
                    crate::RESOLVE_VISITS.with(|visits| visits.set(0));
                    baseline_output.push_str(&misa_lines::to_plain(&crate::draw(&screen, &legacy)));
                    baseline_visits += crate::RESOLVE_VISITS.with(|visits| visits.get());
                }
                assert_eq!(
                    retained.document.stream_text("msg9999.body"),
                    "current one two 界🙂\nfinal"
                );
                assert_eq!(retained.work.formatted_nodes, 0);
                assert_eq!(retained.work.appended_bytes, 21);
                results.push(retained.work);
                baselines.push((baseline_visits, baseline_output));
            }
        }
        assert!(
            results.iter().all(|work| (
                work.formatted_nodes,
                work.appended_bytes,
                work.copied_rows
            ) == (
                results[0].formatted_nodes,
                results[0].appended_bytes,
                results[0].copied_rows
            )),
            "{results:?}"
        );
        assert!(results.iter().all(|work| work.index_steps <= 5 * 24));
        assert!(baselines[..6].iter().all(|run| run == &baselines[0]));
        assert!(baselines[6..].iter().all(|run| run == &baselines[6]));
        eprintln!(
            "baseline actual resolve visits: 100-history {}, 1000-history {}; one output per size across six runs",
            baselines[0].0, baselines[6].0
        );
        eprintln!("100/1000 history, six runs each: {results:?}");
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::{Key, KeyOut};
    use misa_proto::view::Kind;
    use misa_proto::view::Span;
    #[test]
    fn live_selection_copies_the_displayed_wrapped_row() {
        let mut screen = Screen::new(10, 12);
        let mut retained = Retained::new(Node::section("session").id("session"), &screen);
        retained.current(
            Stream {
                id: "msg1.body".into(),
                role: "assistant".into(),
                text: "hello world".into(),
            },
            &screen,
        );

        // A live row wraps at the word, exactly as the settled block will.
        assert_eq!(
            retained
                .document
                .stream_lines("msg1.body")
                .iter()
                .map(Line::text)
                .collect::<Vec<_>>(),
            ["hello", "world"]
        );
        screen.key(Key::Escape);
        assert_eq!(
            retained.selection_key(&mut screen, &Key::Char('v')),
            Some(KeyOut::Local)
        );
        retained.selection_key(&mut screen, &Key::Motion(crate::ed::Motion::LineEnd));
        assert_eq!(
            retained.selection_key(&mut screen, &Key::Char('y')),
            Some(KeyOut::Copy("world".into()))
        );
    }
    #[test]
    fn full_viewport_reserves_staged_attachment_and_multiline_editor_rows() {
        let mut root = Node::section("session").id("session");
        for i in 0..100 {
            root.children
                .push(Node::text("text", [Span::plain("history")]).id(format!("n{i}")));
        }
        let mut screen = Screen::new(30, 12);
        screen.editor.set_text("first\nsecond");
        let mut retained = Retained::new(root, &screen);
        let frame = retained.frame(&screen, Some("1 clipboard attachments · 1 uploading"));
        assert_eq!(frame.lines.len(), 12);
        assert!(
            frame
                .lines
                .iter()
                .all(|line| !line.text().contains('\n') && misa_render::width(&line.text()) <= 30)
        );
        assert_eq!(frame.cursor_row, 11);
        assert_eq!(frame.cursor_column, 8);
    }
    #[test]
    fn folded_content_edits_and_trailing_blanks_match_the_settled_renderer() {
        let mut screen = Screen::new(40, 40);
        let folded = Node::new(
            "call",
            Kind::Collapsible {
                summary: vec![Span::plain("summary")],
            },
        )
        .id("call")
        .child(
            Node::new("quote", Kind::Quote)
                .id("quote")
                .child(Node::text("text", [Span::plain("before")]).id("body")),
        );
        let root = Node::section("session")
            .id("session")
            .child(folded)
            .child(Node::text("text", [Span::plain("")]).id("blank"));
        let mut retained = Retained::new(root, &screen);
        retained
            .op(
                &ViewOp::Replace {
                    id: "body".into(),
                    node: Node::text("text", [Span::plain("after")]).id("body"),
                },
                &screen,
            )
            .unwrap();

        screen.open_all();
        retained.local(&screen);
        let expected = misa_lines::render(
            &screen.resolve(&retained.document.snapshot()),
            &screen.theme,
            40,
        );
        let frame = retained.frame(&screen, None);
        let top = crate::chrome::top(&screen, 0, None);
        assert_eq!(
            &frame.lines[top.len()..top.len() + expected.len()],
            expected.as_slice()
        );
        // The transcript, the blank separator, and then the composer.
        assert_eq!(
            frame.lines.len(),
            expected.len() + top.len() + 1 + crate::chrome::composer(&screen).lines.len()
        );
    }
}
