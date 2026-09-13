//! Selection, structural navigation, and copy.
//!
//! # Why this lives here, and not in a session
//!
//! Nothing about a selection is a fact about the agent. Which characters somebody
//! highlighted, which row their cursor is on, and what they copied are decisions the
//! client makes and the client forgets; the previous system sent a `clipboard/write`
//! effect, which meant a session could write to a clipboard. Only the client knows
//! what was selected, so only the client can act on it — and that is a property of
//! this module having no `Intent` in it, not a rule anyone has to remember.
//!
//! # What it moves over
//!
//! The *rendered* body: the rows the renderer produced, and the node each came from.
//! Working on rows rather than on the tree is what makes byte offsets mean something
//! (a byte offset in a tree is a byte offset in what? — the answer is the text), and
//! the node ids on the rows are what makes structural navigation possible without a
//! second copy of the tree.
//!
//! # The three kinds
//!
//! - [`Kind::Char`] selects between the anchor and the head, as a reader expects;
//! - [`Kind::Line`] selects whole rows;
//! - [`Kind::Block`] selects the byte columns between them on every row in between,
//!   which is what a vim reader's `Ctrl-V` means.
//!
//! A byte column is exact only where the text is ASCII; where it is not, the
//! selection is cut back to a character boundary rather than panicking on a slice.

use misa_render::Line;

/// A place in what is on screen: a row, and a byte offset into that row's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Spot {
    pub row: usize,
    /// A byte offset into the row. Always on a character boundary.
    pub byte: usize,
}

impl Spot {
    pub fn new(row: usize, byte: usize) -> Spot {
        Spot { row, byte }
    }
}

/// What a selection covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    /// The characters between the anchor and the head.
    #[default]
    Char,
    /// Whole rows, from the anchor's row to the head's.
    Line,
    /// A rectangle: the same byte columns on every row between the two.
    Block,
}

impl Kind {
    /// The next kind, for a key that cycles them.
    pub fn next(self) -> Kind {
        match self {
            Kind::Char => Kind::Line,
            Kind::Line => Kind::Block,
            Kind::Block => Kind::Char,
        }
    }
}

/// The rendered body a selection moves over.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Body {
    rows: Vec<String>,
    nodes: Vec<Option<String>>,
}

impl Body {
    /// Snapshot rendered lines, so navigation never needs the tree again.
    pub fn of(lines: &[Line]) -> Body {
        Body {
            rows: lines.iter().map(Line::text).collect(),
            nodes: lines.iter().map(|line| line.node.clone()).collect(),
        }
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }

    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// A row's text. An index past the end is the empty string, not a panic: a
    /// selection is not allowed to be a way to crash a client.
    pub fn row(&self, index: usize) -> &str {
        self.rows.get(index).map(String::as_str).unwrap_or("")
    }

    /// The node a row came from, when the renderer knew.
    pub fn node(&self, index: usize) -> Option<&str> {
        self.nodes.get(index).and_then(Option::as_deref)
    }

    /// A spot that exists: the row clamped, the offset cut back to the row's end and
    /// to a character boundary.
    pub fn clamp(&self, spot: Spot) -> Spot {
        let row = spot.row.min(self.rows.len().saturating_sub(1));
        let text = self.row(row);
        Spot { row, byte: floor_boundary(text, spot.byte) }
    }
}

/// A selection: an anchor that stays put and a head that moves.
#[derive(Clone, Debug, PartialEq)]
pub struct Selection {
    anchor: Spot,
    head: Spot,
    kind: Kind,
}

impl Selection {
    /// A selection that selects nothing yet: one spot, and `v` from there.
    pub fn caret(at: Spot) -> Selection {
        Selection { anchor: at, head: at, kind: Kind::Char }
    }

    /// A selection anchored at one end of a node and headed at the other.
    pub fn of_node(body: &Body, at: Spot) -> Selection {
        let mut selection = Selection::caret(at);
        selection.select_node(body);
        selection
    }

    pub fn anchor(&self) -> Spot {
        self.anchor
    }

    pub fn head(&self) -> Spot {
        self.head
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// Which kind a range is. Changing it changes what is selected, not where the
    /// ends are, which is why a reader can switch kinds mid-selection.
    pub fn set_kind(&mut self, kind: Kind) {
        self.kind = kind;
    }

    /// Whether anything is selected: a caret selects nothing.
    pub fn is_caret(&self) -> bool {
        self.anchor == self.head
    }

    /// Drop the anchor onto the head, which ends the selection in place.
    pub fn collapse(&mut self) {
        self.anchor = self.head;
    }

    /// Put the anchor back where the head is, and move the head by hand.
    pub fn restart(&mut self) {
        self.anchor = self.head;
    }

    // --- motions -----------------------------------------------------------------

    pub fn left(&mut self, body: &Body) {
        let spot = body.clamp(self.head);
        self.head = if spot.byte == 0 {
            Spot::new(spot.row, 0)
        } else {
            Spot::new(spot.row, prev_boundary(body.row(spot.row), spot.byte))
        };
    }

    pub fn right(&mut self, body: &Body) {
        let spot = body.clamp(self.head);
        let text = body.row(spot.row);
        self.head = if spot.byte >= text.len() {
            Spot::new(spot.row, text.len())
        } else {
            Spot::new(spot.row, next_boundary(text, spot.byte))
        };
    }

    pub fn word_left(&mut self, body: &Body) {
        let spot = body.clamp(self.head);
        // At the start of a row, the word before is the last one of the row above.
        if spot.byte == 0 && spot.row > 0 {
            let previous = spot.row - 1;
            let text = body.row(previous);
            self.head = Spot::new(previous, prev_word(text, text.len()));
            return;
        }
        self.head = Spot::new(spot.row, prev_word(body.row(spot.row), spot.byte));
    }

    pub fn word_right(&mut self, body: &Body) {
        let spot = body.clamp(self.head);
        let text = body.row(spot.row);
        let index = next_word(text, spot.byte);
        self.head = if index >= text.len() && spot.row + 1 < body.len() {
            Spot::new(spot.row + 1, 0)
        } else {
            Spot::new(spot.row, index)
        };
    }

    pub fn up(&mut self, body: &Body) {
        let spot = body.clamp(self.head);
        self.head = body.clamp(Spot::new(spot.row.saturating_sub(1), spot.byte));
    }

    pub fn down(&mut self, body: &Body) {
        let spot = body.clamp(self.head);
        self.head = body.clamp(Spot::new(spot.row + 1, spot.byte));
    }

    pub fn line_start(&mut self, body: &Body) {
        self.head = Spot::new(body.clamp(self.head).row, 0);
    }

    pub fn line_end(&mut self, body: &Body) {
        let row = body.clamp(self.head).row;
        self.head = Spot::new(row, body.row(row).len());
    }

    pub fn document_start(&mut self, body: &Body) {
        self.head = body.clamp(Spot::new(0, 0));
    }

    pub fn document_end(&mut self, body: &Body) {
        let row = body.len().saturating_sub(1);
        self.head = Spot::new(row, body.row(row).len());
    }

    // --- structural navigation ---------------------------------------------------

    /// The start of the next node: the next row whose node is not this row's.
    ///
    /// The rows carry the ids the session gave the nodes, so this is a scan of what is
    /// already on screen — the client needs no tree and asks nobody.
    pub fn node_forward(&mut self, body: &Body) {
        let row = body.clamp(self.head).row;
        let node = body.node(row);
        let mut next = row + 1;
        while next < body.len() && body.node(next) == node {
            next += 1;
        }
        self.head = Spot::new(next.min(body.len().saturating_sub(1)), 0);
    }

    /// The start of the node before this one, skipping what is left of the current
    /// node first — so pressing it twice leaves the node rather than its second row.
    pub fn node_back(&mut self, body: &Body) {
        let row = body.clamp(self.head).row;
        let node = body.node(row);
        let mut first = row;
        while first > 0 && body.node(first - 1) == node {
            first -= 1;
        }
        if first == 0 {
            self.head = Spot::new(0, 0);
            return;
        }
        let previous = body.node(first - 1);
        let mut start = first - 1;
        while start > 0 && body.node(start - 1) == previous {
            start -= 1;
        }
        self.head = Spot::new(start, 0);
    }

    /// Select the whole node the head is in.
    pub fn select_node(&mut self, body: &Body) {
        let row = body.clamp(self.head).row;
        let node = body.node(row);
        let mut first = row;
        while first > 0 && body.node(first - 1) == node {
            first -= 1;
        }
        let mut last = row;
        while last + 1 < body.len() && body.node(last + 1) == node {
            last += 1;
        }
        self.anchor = Spot::new(first, 0);
        self.head = Spot::new(last, body.row(last).len());
    }

    // --- what was selected -------------------------------------------------------

    /// The range, ordered, as the two spots a copy reads between.
    pub fn range(&self, body: &Body) -> (Spot, Spot) {
        let (start, end) = if self.anchor <= self.head { (self.anchor, self.head) } else { (self.head, self.anchor) };
        let (start, end) = (body.clamp(start), body.clamp(end));
        match self.kind {
            Kind::Char => (start, end),
            Kind::Line => (Spot::new(start.row, 0), Spot::new(end.row, body.row(end.row).len())),
            // A block keeps its columns and takes every row between.
            Kind::Block => {
                let from = start.byte.min(end.byte);
                let to = start.byte.max(end.byte);
                (Spot::new(start.row, from), Spot::new(end.row, to))
            }
        }
    }

    /// The byte range covered on one row, for a client that wants to draw it.
    /// `None` when the row is outside the selection.
    pub fn on_row(&self, body: &Body, row: usize) -> Option<(usize, usize)> {
        let (start, end) = self.range(body);
        if row < start.row || row > end.row {
            return None;
        }
        let text = body.row(row);
        let (from, to) = match self.kind {
            Kind::Block => (start.byte, end.byte),
            _ => (if row == start.row { start.byte } else { 0 }, if row == end.row { end.byte } else { text.len() }),
        };
        let from = floor_boundary(text, from.min(text.len()));
        let to = floor_boundary(text, to.max(from).min(text.len()));
        Some((from, to))
    }

    /// The text a copy would put on a clipboard.
    pub fn text(&self, body: &Body) -> String {
        let (start, end) = self.range(body);
        let mut out = String::new();
        for row in start.row..=end.row {
            let Some((from, to)) = self.on_row(body, row) else { continue };
            if row > start.row {
                out.push('\n');
            }
            out.push_str(&body.row(row)[from..to]);
        }
        out
    }

    /// The nodes the selection touches, in order, without repeats.
    pub fn nodes(&self, body: &Body) -> Vec<String> {
        let (start, end) = self.range(body);
        let mut out: Vec<String> = Vec::new();
        for row in start.row..=end.row {
            if let Some(node) = body.node(row)
                && !out.iter().any(|seen| seen == node)
            {
                out.push(node.to_string());
            }
        }
        out
    }
}

// --- text, by the byte ----------------------------------------------------------

fn char_at(text: &str, index: usize) -> Option<char> {
    text.get(index..)?.chars().next()
}

fn char_before(text: &str, index: usize) -> Option<char> {
    text.get(..index)?.chars().next_back()
}

/// Cut `index` back to a character boundary at or before it.
fn floor_boundary(text: &str, index: usize) -> usize {
    let mut index = index.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// The offset after the character starting at `index`.
fn next_boundary(text: &str, index: usize) -> usize {
    match char_at(text, index) {
        Some(ch) => index + ch.len_utf8(),
        None => text.len(),
    }
}

/// The offset before the character ending at `index`.
fn prev_boundary(text: &str, index: usize) -> usize {
    match char_before(text, index) {
        Some(ch) => index - ch.len_utf8(),
        None => 0,
    }
}

fn is_word(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_'
}

/// The offset at or after `index` where the next word begins.
fn next_word(text: &str, index: usize) -> usize {
    let mut index = floor_boundary(text, index);
    while char_at(text, index).is_some_and(is_word) {
        index = next_boundary(text, index);
    }
    while char_at(text, index).is_some_and(|ch| !is_word(ch)) {
        index = next_boundary(text, index);
    }
    index
}

/// The offset where the word before `index` begins.
fn prev_word(text: &str, index: usize) -> usize {
    let mut index = floor_boundary(text, index);
    while let Some(ch) = char_before(text, index) {
        if is_word(ch) {
            break;
        }
        index -= ch.len_utf8();
    }
    while let Some(ch) = char_before(text, index) {
        if !is_word(ch) {
            break;
        }
        index -= ch.len_utf8();
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_render::Style;

    /// Three rows, two of them from one node, which is the shape a transcript has.
    fn body() -> Body {
        let line = |text: &str, node: &str| Line {
            indent: 0,
            spans: vec![(Style::PLAIN, text.to_string())],
            node: Some(node.to_string()),
        };
        Body::of(&[
            line("first line", "msg.1"),
            line("second line", "msg.1"),
            line("the result", "call.1"),
        ])
    }

    #[test]
    fn a_row_is_its_text_and_the_node_it_came_from() {
        let body = body();
        assert_eq!(body.len(), 3);
        assert_eq!(body.row(1), "second line");
        assert_eq!(body.node(1), Some("msg.1"));
        assert_eq!(body.node(2), Some("call.1"));
        // Past the end is empty, not a panic.
        assert_eq!(body.row(99), "");
        assert_eq!(body.node(99), None);
    }

    #[test]
    fn the_rendered_text_includes_the_indent_a_row_was_drawn_with() {
        let body = Body::of(&[Line {
            indent: 2,
            spans: vec![(Style::PLAIN, "railed".to_string())],
            node: None,
        }]);
        assert_eq!(body.row(0), "  railed");
        assert_eq!(body.clamp(Spot::new(0, 99)).byte, 8);
    }

    #[test]
    fn a_clamped_spot_lands_on_a_character_boundary() {
        let body = Body::of(&[Line {
            indent: 0,
            spans: vec![(Style::PLAIN, "héllo".to_string())],
            node: None,
        }]);
        // `é` is two bytes; offset 2 is inside it.
        assert_eq!(body.clamp(Spot::new(0, 2)).byte, 1);
        assert_eq!(body.clamp(Spot::new(9, 99)).row, 0);
        assert_eq!(body.clamp(Spot::new(0, 99)).byte, 6);
    }

    #[test]
    fn char_motion_walks_and_stops_at_the_ends() {
        let body = body();
        let mut selection = Selection::caret(Spot::new(0, 0));
        selection.right(&body);
        assert_eq!(selection.head(), Spot::new(0, 1));
        selection.left(&body);
        assert_eq!(selection.head(), Spot::new(0, 0));
        // Left at the start of the document stays there; right at the end of a row
        // stays at the row's end rather than wrapping into the next one.
        selection.left(&body);
        assert_eq!(selection.head(), Spot::new(0, 0));
        selection.line_end(&body);
        assert_eq!(selection.head(), Spot::new(0, "first line".len()));
        selection.right(&body);
        assert_eq!(selection.head(), Spot::new(0, "first line".len()));
    }

    #[test]
    fn word_motion_crosses_a_row() {
        let body = body();
        let mut selection = Selection::caret(Spot::new(0, 0));
        selection.word_right(&body);
        assert_eq!(selection.head(), Spot::new(0, "first ".len()));
        selection.word_right(&body);
        // "line" ends the row, so the next word is the next row's first.
        assert_eq!(selection.head(), Spot::new(1, 0));
        selection.word_left(&body);
        assert_eq!(selection.head(), Spot::new(0, "first ".len()));
    }

    #[test]
    fn up_and_down_keep_the_column() {
        let body = body();
        let mut selection = Selection::caret(Spot::new(0, 4));
        selection.down(&body);
        assert_eq!(selection.head(), Spot::new(1, 4));
        selection.down(&body);
        // "the result" is shorter than the column was, so it clamps.
        assert_eq!(selection.head(), Spot::new(2, 4));
        selection.down(&body);
        assert_eq!(selection.head().row, 2, "the caret left the document");
    }

    #[test]
    fn a_char_selection_is_the_text_between_the_ends() {
        let body = body();
        let mut selection = Selection::caret(Spot::new(0, 0));
        selection.head = Spot::new(1, 6);
        assert_eq!(selection.text(&body), "first line\nsecond");
    }

    #[test]
    fn a_selection_taken_backwards_is_the_same_text() {
        let body = body();
        let mut forwards = Selection::caret(Spot::new(2, 3));
        forwards.head = Spot::new(2, 7);
        let mut backwards = Selection::caret(Spot::new(2, 7));
        backwards.head = Spot::new(2, 3);
        assert_eq!(forwards.text(&body), " res");
        assert_eq!(backwards.text(&body), forwards.text(&body));
    }

    #[test]
    fn a_line_selection_takes_whole_rows() {
        let body = body();
        let mut selection = Selection::caret(Spot::new(0, 4));
        selection.set_kind(Kind::Line);
        selection.head = Spot::new(1, 2);
        assert_eq!(selection.text(&body), "first line\nsecond line");
    }

    #[test]
    fn a_block_selection_keeps_its_columns_on_every_row() {
        let body = body();
        let mut selection = Selection::caret(Spot::new(0, 2));
        selection.set_kind(Kind::Block);
        selection.head = Spot::new(2, 5);
        // Every row contributes the same byte columns, and the short last row clamps.
        assert_eq!(selection.text(&body), "rst\ncon\ne r");
    }

    #[test]
    fn a_kind_cycles_through_the_three() {
        assert_eq!(Kind::Char.next(), Kind::Line);
        assert_eq!(Kind::Line.next(), Kind::Block);
        assert_eq!(Kind::Block.next(), Kind::Char);
    }

    #[test]
    fn a_caret_selects_nothing_and_collapsing_ends_a_selection() {
        let body = body();
        let mut selection = Selection::caret(Spot::new(0, 3));
        assert!(selection.is_caret());
        assert_eq!(selection.text(&body), "");
        selection.head = Spot::new(1, 3);
        assert!(!selection.is_caret());
        selection.collapse();
        assert!(selection.is_caret());
        assert_eq!(selection.text(&body), "");
    }

    #[test]
    fn node_navigation_uses_the_rows_the_session_named() {
        let body = body();
        let mut selection = Selection::caret(Spot::new(0, 2));
        selection.node_forward(&body);
        assert_eq!(selection.head(), Spot::new(2, 0), "the next node is the tool result");
        // Twice from the start of the second node leaves it rather than moving within it.
        selection.node_back(&body);
        assert_eq!(selection.head(), Spot::new(0, 0));
    }

    #[test]
    fn selecting_a_node_takes_all_of_it_and_no_more() {
        let body = body();
        let mut selection = Selection::caret(Spot::new(1, 3));
        selection.select_node(&body);
        assert_eq!(selection.anchor(), Spot::new(0, 0));
        assert_eq!(selection.head(), Spot::new(1, "second line".len()));
        assert_eq!(selection.text(&body), "first line\nsecond line");
        assert_eq!(selection.nodes(&body), vec!["msg.1".to_string()]);
    }

    #[test]
    fn a_selection_can_be_made_over_a_node_in_one_step() {
        let body = body();
        let selection = Selection::of_node(&body, Spot::new(2, 2));
        assert_eq!(selection.text(&body), "the result");
    }

    #[test]
    fn the_nodes_under_a_selection_are_what_copied_bytes_belong_to() {
        let body = body();
        let mut selection = Selection::caret(Spot::new(1, 0));
        selection.head = Spot::new(2, 2);
        assert_eq!(selection.nodes(&body), vec!["msg.1".to_string(), "call.1".to_string()]);
    }

    #[test]
    fn a_selection_over_a_row_with_no_node_names_none() {
        // A notice or the composer is drawn without a node; a selection may cover it,
        // and then there is nothing to attribute the bytes to.
        let body = Body::of(&[Line { indent: 0, spans: vec![(Style::PLAIN, "the composer".into())], node: None }]);
        let mut selection = Selection::caret(Spot::new(0, 0));
        selection.head = Spot::new(0, 4);
        assert_eq!(selection.nodes(&body), Vec::<String>::new());
        assert_eq!(selection.text(&body), "the ");
    }

    #[test]
    fn a_row_the_selection_does_not_touch_covers_nothing() {
        let body = body();
        let mut selection = Selection::caret(Spot::new(0, 2));
        selection.head = Spot::new(0, 5);
        assert_eq!(selection.on_row(&body, 0), Some((2, 5)));
        assert_eq!(selection.on_row(&body, 1), None);
    }

    #[test]
    fn document_motion_goes_to_the_two_ends() {
        let body = body();
        let mut selection = Selection::caret(Spot::new(1, 5));
        selection.document_start(&body);
        assert_eq!(selection.head(), Spot::new(0, 0));
        selection.document_end(&body);
        assert_eq!(selection.head(), Spot::new(2, "the result".len()));
    }

    #[test]
    fn an_empty_body_is_navigable_rather_than_fatal() {
        let body = Body::of(&[]);
        let mut selection = Selection::caret(Spot::new(0, 0));
        selection.down(&body);
        selection.word_right(&body);
        selection.node_forward(&body);
        selection.select_node(&body);
        assert!(selection.text(&body).is_empty());
    }
}
