//! Reader selection and requested viewport state. The retained viewport owns the
//! resolved position; only explicit reader commands advance the intent epoch.
use crate::{Action, Key, KeyOut, ed};
use misa_lines::select::{Body, Selection, Spot};
use misa_terminal_ui::viewport::{Head, Request};

pub struct Reader {
    selection: Option<Selection>,
    scroll: usize,
    follow: bool,
    scroll_intent: u64,
}

impl Default for Reader {
    fn default() -> Self {
        Self::new()
    }
}

impl Reader {
    pub fn new() -> Self {
        Self {
            selection: None,
            scroll: 0,
            follow: true,
            scroll_intent: 0,
        }
    }

    pub fn has_selection(&self) -> bool {
        self.selection.is_some()
    }

    pub fn clear_selection(&mut self) {
        self.selection = None;
    }

    pub fn request(&self, room: usize) -> Request<Spot> {
        Request {
            scroll: self.scroll,
            follow: self.follow,
            intent: self.scroll_intent,
            room,
            head: self.selection.as_ref().map(|selection| {
                let identity = selection.head();
                Head {
                    identity,
                    row: identity.row,
                }
            }),
        }
    }

    pub fn following(&self) -> bool {
        self.follow
    }

    /// Read back the physically resolved viewport after a frame. This is not a
    /// reader command: it must not advance the scroll intent epoch.
    pub fn resolved(&mut self, scroll: usize, follow: bool) {
        self.scroll = scroll;
        self.follow = follow;
    }

    pub fn reset_viewport(&mut self) {
        self.scroll = 0;
        self.follow = true;
    }

    pub fn scroll_by(&mut self, delta: isize) {
        let next = self.scroll as isize + delta;
        self.scroll = next.max(0) as usize;
        self.follow = false;
        self.scroll_intent = self.scroll_intent.wrapping_add(1);
    }

    pub fn action(&mut self, action: Action) -> bool {
        match action {
            Action::ScrollUp => self.scroll_by(-10),
            Action::ScrollDown => self.scroll_by(10),
            Action::ScrollTop => {
                self.follow = false;
                self.scroll = 0;
                self.scroll_intent = self.scroll_intent.wrapping_add(1);
            }
            Action::ScrollBottom => {
                self.follow = true;
                self.scroll_intent = self.scroll_intent.wrapping_add(1);
            }
            _ => return false,
        }
        true
    }

    /// Returns whether the coordinator should dismiss an active history search.
    /// The picker has first refusal, so Screen calls this only without a picker.
    pub fn page(&mut self, delta: isize, searching: bool) -> bool {
        self.scroll_by(delta);
        searching
    }

    /// `v` and `y` are reader keys only in normal mode, unless a selection is
    /// already active. The composer decides which unselected keys are reader keys.
    pub fn accepts(&self, reader_key: bool) -> bool {
        self.has_selection() || reader_key
    }

    pub fn key(
        &mut self,
        body: &Body,
        key: &Key,
        mode: ed::Mode,
        empty: bool,
        notice: &mut Option<String>,
    ) -> Option<KeyOut> {
        if self.has_selection() {
            return Some(self.selecting(body, key, notice));
        }
        if mode != ed::Mode::Normal {
            return None;
        }
        match key {
            Key::StartSelection | Key::Char('v') => {
                self.begin(body, notice);
                Some(KeyOut::Local)
            }
            Key::Char('y') if empty => Some(Self::copy_body(body, notice)),
            _ => None,
        }
    }

    fn begin(&mut self, body: &Body, notice: &mut Option<String>) {
        let row = body.len().saturating_sub(1);
        self.selection = Some(Selection::caret(Spot::new(row, 0)));
        *notice = Some("copying — y takes it, esc stops".to_string());
    }

    fn copy_body(body: &Body, notice: &mut Option<String>) -> KeyOut {
        let mut everything = Selection::caret(Spot::new(0, 0));
        everything.document_end(body);
        let text = everything.text(body);
        *notice = Some(format!("copied {} bytes", text.len()));
        KeyOut::Copy(text)
    }

    fn selecting(&mut self, body: &Body, key: &Key, notice: &mut Option<String>) -> KeyOut {
        let Some(mut selection) = self.selection.take() else {
            return KeyOut::Local;
        };
        match key {
            Key::Escape | Key::Char('q') | Key::Interrupt | Key::Eof => {
                *notice = None;
                return KeyOut::Local;
            }
            Key::Char('y') => {
                let text = selection.text(body);
                *notice = Some(format!("copied {} bytes", text.len()));
                return KeyOut::Copy(text);
            }
            Key::Char('o') => selection.restart(),
            Key::Char('v') => selection.set_kind(selection.kind().next()),
            Key::Char('a') => selection.select_node(body),
            Key::Char('j') => selection.down(body),
            Key::Char('k') => selection.up(body),
            Key::Char('g') => selection.document_start(body),
            Key::Char('G') => selection.document_end(body),
            Key::Char('h') => selection.left(body),
            Key::Char('l') => selection.right(body),
            Key::Char('J') | Key::Submit | Key::Newline => selection.select_node(body),
            Key::Char('K') | Key::Backspace => selection.node_back(body),
            Key::Char('w') => selection.word_right(body),
            Key::Char('b') => selection.word_left(body),
            Key::Char('n') => selection.node_forward(body),
            Key::Char('p') => selection.node_back(body),
            Key::Motion(ed::Motion::Left) => selection.left(body),
            Key::Motion(ed::Motion::Right) => selection.right(body),
            Key::Motion(ed::Motion::Up) => selection.up(body),
            Key::Motion(ed::Motion::Down) => selection.down(body),
            Key::Motion(ed::Motion::LineStart) => selection.line_start(body),
            Key::Motion(ed::Motion::LineEnd) => selection.line_end(body),
            Key::Motion(ed::Motion::WordNext) => selection.word_right(body),
            Key::Motion(ed::Motion::WordPrevious) => selection.word_left(body),
            Key::Motion(ed::Motion::First) => selection.document_start(body),
            Key::Motion(ed::Motion::Last) => selection.document_end(body),
            Key::Extend(ed::Motion::Left) => selection.left(body),
            Key::Extend(ed::Motion::Right) => selection.right(body),
            Key::Extend(ed::Motion::Up) => selection.up(body),
            Key::Extend(ed::Motion::Down) => selection.down(body),
            _ => {}
        }
        self.selection = Some(selection);
        KeyOut::Local
    }

    pub fn selected_row(&self, body: &Body, row: usize) -> Option<(usize, usize)> {
        self.selection
            .as_ref()
            .and_then(|selection| selection.on_row(body, row))
    }

    #[cfg(test)]
    pub(crate) fn fixture(&mut self, scroll: usize, follow: bool, intent: u64) {
        self.scroll = scroll;
        self.follow = follow;
        self.scroll_intent = intent;
    }
    #[cfg(test)]
    pub(crate) fn fixture_selection(&mut self, selection: Selection) {
        self.selection = Some(selection);
    }
}
