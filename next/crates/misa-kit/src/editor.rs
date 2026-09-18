//! Composing an input line.
//!
//! Modes, motions, history, and undo live here because they are interaction policy:
//! they decide what a keypress *means* to the thing being typed into, they differ
//! between platforms, and no session has any business in them.
//!
//! What is deliberately not here: the meaning of a submission. Submitting produces
//! an [`Intent`], and what happens next is a session's.
//!
//! # Modes
//!
//! [`Mode::Insert`] and [`Mode::Normal`] are here because the previous system's
//! editor is modal and people rely on it. A frontend that wants a single mode uses
//! [`Mode::Insert`] throughout and never switches; the same state machine serves
//! both, and a `plain` policy is one line rather than a second implementation.

use crate::intent::Intent;

/// How a keypress is interpreted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Keys insert text.
    Insert,
    /// Keys are motions and edits.
    Normal,
    /// Keys move the cursor while retaining the text range selected from the
    /// visual anchor.
    Visual,
}

/// A movement within the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    WordNext,
    WordPrevious,
    WordEnd,
    LineStart,
    LineEnd,
    First,
    Last,
    Up,
    Down,
}

/// What a keypress caused.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    /// The editor changed and nothing else needs to know.
    Changed,
    /// Nothing happened.
    Ignored,
    /// The mode changed, which a frontend shows (the previous system drew it in the
    /// prompt: `┌`, `◆`, `◇`).
    ModeChanged(Mode),
    /// A submission, as the intent it becomes. The text is kept in history.
    Submit(Intent),
    /// The draft was interrupted but kept: the previous system's Ctrl-C, which
    /// preserves what somebody typed rather than throwing it away.
    Interrupted,
}

/// An input line with a cursor, a mode, and a history.
pub struct Editor {
    text: String,
    /// A byte offset into `text`, always on a character boundary.
    cursor: usize,
    mode: Mode,
    visual_anchor: Option<usize>,
    history: Vec<String>,
    /// Where a person is browsing, and what they were typing before they started.
    history_at: Option<usize>,
    draft: String,
    /// Removed text, for an undo that is worth having.
    undone: Vec<(String, usize, String)>,
    /// States moved out of the undo history by `U`, invalidated by a new edit.
    redone: Vec<(String, usize)>,
    /// The register used by normal-mode yank and paste. It belongs to the editor,
    /// alongside the text it edits, so every client gets the same semantics.
    register: String,
}

impl Default for Editor {
    fn default() -> Self {
        Editor::new()
    }
}

impl Editor {
    pub fn new() -> Editor {
        Editor {
            text: String::new(),
            cursor: 0,
            mode: Mode::Insert,
            visual_anchor: None,
            history: Vec::new(),
            history_at: None,
            draft: String::new(),
            undone: Vec::new(),
            redone: Vec::new(),
            register: String::new(),
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn history(&self) -> &[String] {
        &self.history
    }

    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    /// Whether the cursor is on the last line, which is where a history recall
    /// belongs: pressing up in the middle of a message should move the cursor.
    pub fn on_last_line(&self) -> bool {
        !self.text[self.cursor..].contains('\n')
    }

    pub fn on_first_line(&self) -> bool {
        !self.text[..self.cursor].contains('\n')
    }

    /// Replace the whole line. Used by a picker filling an argument.
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = self.text.len();
        self.history_at = None;
        self.visual_anchor = None;
    }

    /// Append text at the cursor.
    pub fn insert(&mut self, text: &str) -> Effect {
        self.remember_for_undo();
        self.text.insert_str(self.cursor, text);
        self.cursor += text.len();
        self.history_at = None;
        Effect::Changed
    }

    pub fn type_char(&mut self, character: char) -> Effect {
        let mut buffer = [0u8; 4];
        self.insert(character.encode_utf8(&mut buffer))
    }

    /// Remove the character before the cursor.
    pub fn backspace(&mut self) -> Effect {
        if self.cursor == 0 {
            return Effect::Ignored;
        }
        self.remember_for_undo();
        let previous = self.text[..self.cursor]
            .chars()
            .next_back()
            .map(char::len_utf8)
            .unwrap_or(1);
        self.text
            .replace_range(self.cursor - previous..self.cursor, "");
        self.cursor -= previous;
        Effect::Changed
    }

    pub fn delete(&mut self) -> Effect {
        if self.cursor >= self.text.len() {
            return Effect::Ignored;
        }
        self.remember_for_undo();
        let next = self.text[self.cursor..]
            .chars()
            .next()
            .map(char::len_utf8)
            .unwrap_or(1);
        self.text.replace_range(self.cursor..self.cursor + next, "");
        Effect::Changed
    }

    pub fn move_cursor(&mut self, motion: Motion) -> Effect {
        let target = match motion {
            Motion::Left => self.text[..self.cursor]
                .chars()
                .next_back()
                .map(|c| self.cursor - c.len_utf8()),
            Motion::Right => self.text[self.cursor..]
                .chars()
                .next()
                .map(|c| self.cursor + c.len_utf8()),
            // On a single line, the start and the end of the line are the ends of the
            // text: a motion that did nothing because there was no newline would be a
            // key that silently depends on something unrelated.
            Motion::LineStart => self.text[..self.cursor]
                .rfind('\n')
                .map(|at| at + 1)
                .or(Some(0)),
            Motion::LineEnd => self.text[self.cursor..]
                .find('\n')
                .map(|at| self.cursor + at)
                .or(Some(self.text.len())),
            Motion::First => Some(0),
            Motion::Last => Some(self.text.len()),
            // The start of the next word, not the end of this one: a frontend that
            // wants the other convention adds a motion rather than changing this.
            Motion::WordNext => Some(word_next(&self.text, self.cursor)),
            Motion::WordPrevious => Some(word_previous(&self.text, self.cursor)),
            Motion::WordEnd => Some(word_end(&self.text, self.cursor)),
            Motion::Up | Motion::Down => {
                let start = self.text[..self.cursor].rfind('\n').map_or(0, |at| at + 1);
                let column = self.text[start..self.cursor].chars().count();
                let next_start = if motion == Motion::Up {
                    start
                        .checked_sub(1)
                        .map(|end| self.text[..end].rfind('\n').map_or(0, |at| at + 1))
                } else {
                    self.text[self.cursor..]
                        .find('\n')
                        .map(|at| self.cursor + at + 1)
                };
                next_start.map(|start| {
                    let end = self.text[start..]
                        .find('\n')
                        .map_or(self.text.len(), |at| start + at);
                    start
                        + self.text[start..end]
                            .char_indices()
                            .nth(column)
                            .map_or(end - start, |(at, _)| at)
                })
            }
        };
        match target {
            Some(target) if target != self.cursor => {
                self.cursor = target;
                Effect::Changed
            }
            _ => Effect::Ignored,
        }
    }

    /// Toggle between inserting and moving.
    pub fn set_mode(&mut self, mode: Mode) -> Effect {
        if self.mode == mode {
            return Effect::Ignored;
        }
        self.mode = mode;
        self.visual_anchor = (mode == Mode::Visual).then_some(self.cursor);
        Effect::ModeChanged(mode)
    }

    /// Enter visual mode selecting the current line, the `V` binding in the
    /// reference editor. The range remains ordinary editor state, so rendering
    /// and operations do not need a second selection implementation.
    pub fn visual_line(&mut self) -> Effect {
        let start = self.text[..self.cursor].rfind('\n').map_or(0, |at| at + 1);
        let end = self.text[self.cursor..]
            .find('\n')
            .map_or(self.text.len(), |at| self.cursor + at);
        self.mode = Mode::Visual;
        self.visual_anchor = Some(start);
        self.cursor = end;
        Effect::ModeChanged(Mode::Visual)
    }

    pub fn visual_range(&self) -> Option<(usize, usize)> {
        (self.mode == Mode::Visual).then(|| {
            let anchor = self.visual_anchor.unwrap_or(self.cursor);
            (anchor.min(self.cursor), anchor.max(self.cursor))
        })
    }

    /// Apply a visual operation and leave visual mode, as Vim does. The caller
    /// decides whether the returned text is sent to a clipboard.
    pub fn visual_operation(&mut self, operator: char) -> String {
        let Some((start, end)) = self.visual_range() else {
            return String::new();
        };
        let selected = self.text[start..end].to_string();
        if operator == 'y' {
            self.register = selected.clone();
        }
        if operator != 'y' {
            self.remember_for_undo();
            self.text.replace_range(start..end, "");
            self.cursor = start;
        }
        self.mode = if operator == 'c' {
            Mode::Insert
        } else {
            Mode::Normal
        };
        self.visual_anchor = None;
        selected
    }

    /// Paste the most recently yanked text after the cursor, the normal-mode `p`
    /// operation. Empty registers are a no-op, just like an empty clipboard.
    pub fn paste(&mut self) -> Effect {
        if self.register.is_empty() {
            return Effect::Ignored;
        }
        let at = self.text[self.cursor..]
            .chars()
            .next()
            .map_or(self.text.len(), |character| {
                self.cursor + character.len_utf8()
            });
        let value = self.register.clone();
        self.remember_for_undo();
        self.text.insert_str(at, &value);
        self.cursor = at + value.len();
        Effect::Changed
    }

    /// Submit, if there is anything to submit.
    ///
    /// Consecutive identical submissions are not recorded twice: the previous system
    /// made the same decision, and it is the difference between a usable history and
    /// a list of the same line.
    pub fn submit(&mut self) -> Effect {
        let text = std::mem::take(&mut self.text);
        self.cursor = 0;
        self.history_at = None;
        self.draft.clear();
        self.visual_anchor = None;
        self.mode = Mode::Insert;
        if text.trim().is_empty() {
            return Effect::Ignored;
        }
        if self.history.last().map(String::as_str) != Some(text.as_str()) {
            self.history.push(text.clone());
        }
        Effect::Submit(Intent::Prompt {
            text,
            attachments: Vec::new(),
        })
    }

    /// Interrupt, keeping what was typed.
    pub fn interrupt(&mut self) -> Effect {
        Effect::Interrupted
    }

    /// Step back through history, or forward towards the draft.
    ///
    /// Moving forward past the newest entry restores what was being typed before the
    /// browsing started, which is what makes a history worth browsing rather than a
    /// way to lose a half-written message.
    pub fn history_step(&mut self, backwards: bool) -> Effect {
        if self.history.is_empty() {
            return Effect::Ignored;
        }
        let at = match (self.history_at, backwards) {
            (None, true) => {
                self.draft = self.text.clone();
                Some(self.history.len() - 1)
            }
            (None, false) => return Effect::Ignored,
            (Some(0), true) => return Effect::Ignored,
            (Some(at), true) => Some(at - 1),
            (Some(at), false) if at + 1 < self.history.len() => Some(at + 1),
            (Some(_), false) => {
                self.history_at = None;
                let draft = std::mem::take(&mut self.draft);
                self.set_text(draft);
                return Effect::Changed;
            }
        };
        self.history_at = at;
        if let Some(at) = at {
            let text = self.history[at].clone();
            self.text = text;
            self.cursor = self.text.len();
        }
        Effect::Changed
    }

    /// Apply an operator to a motion, or to the current line when no motion is given.
    /// Returns the affected text so a frontend can copy it to its clipboard.
    pub fn operate(&mut self, operator: char, motion: Option<Motion>) -> String {
        let original = self.cursor;
        let linewise = motion.is_none() || matches!(motion, Some(Motion::Up | Motion::Down));
        let (mut start, mut end) = if let Some(motion) = motion {
            self.move_cursor(motion);
            let target = self.cursor;
            self.cursor = original;
            (original.min(target), original.max(target))
        } else {
            let start = self.text[..self.cursor].rfind('\n').map_or(0, |at| at + 1);
            let end = self.text[self.cursor..]
                .find('\n')
                .map_or(self.text.len(), |at| self.cursor + at + 1);
            (start, end)
        };
        if linewise {
            start = self.text[..start].rfind('\n').map_or(0, |at| at + 1);
            if motion.is_some() {
                end = self.text[end..]
                    .find('\n')
                    .map_or(self.text.len(), |at| end + at + 1);
            }
            if operator == 'c' && end > start && self.text[..end].ends_with('\n') {
                end -= 1;
            }
        }
        let copied = self.text[start..end].to_string();
        if operator == 'y' {
            self.register = copied.clone();
        }
        if operator != 'y' {
            self.remember_for_undo();
            // Deleting the last line also removes its preceding separator.
            if linewise && end == self.text.len() && start > 0 && operator == 'd' {
                start -= 1;
            }
            self.text.replace_range(start..end, "");
            self.cursor = start;
            if operator == 'c' {
                self.mode = Mode::Insert;
            }
        }
        copied
    }

    pub fn open_line(&mut self, above: bool) {
        self.move_cursor(if above {
            Motion::LineStart
        } else {
            Motion::LineEnd
        });
        self.insert("\n");
        if above {
            self.cursor -= 1;
        }
        self.mode = Mode::Insert;
    }

    pub fn undo(&mut self) -> Effect {
        match self.undone.pop() {
            Some((text, cursor, _)) => {
                self.redone.push((self.text.clone(), self.cursor));
                self.text = text;
                self.cursor = cursor.min(self.text.len());
                Effect::Changed
            }
            None => Effect::Ignored,
        }
    }

    /// Move forward through the editor's undo history, the normal-mode `U`
    /// binding in the reference editor.
    pub fn redo(&mut self) -> Effect {
        match self.redone.pop() {
            Some((text, cursor)) => {
                self.undone
                    .push((self.text.clone(), self.cursor, String::new()));
                self.text = text;
                self.cursor = cursor.min(self.text.len());
                Effect::Changed
            }
            None => Effect::Ignored,
        }
    }

    fn remember_for_undo(&mut self) {
        // One entry per contiguous edit run: an undo that walked back a character at
        // a time would make the coherent unit of work unreachable.
        if self.undone.len() > 64 {
            self.undone.remove(0);
        }
        self.undone
            .push((self.text.clone(), self.cursor, String::new()));
        self.redone.clear();
    }

    /// The text around the cursor, for a frontend that draws it.
    pub fn split_at_cursor(&self) -> (&str, &str) {
        (&self.text[..self.cursor], &self.text[self.cursor..])
    }
}

/// The start of the word after an offset, or the end of the text.
fn word_next(text: &str, cursor: usize) -> usize {
    let mut at = cursor;
    // The rest of the word the cursor is in.
    while let Some(character) = text[at..].chars().next() {
        if character.is_whitespace() {
            break;
        }
        at += character.len_utf8();
    }
    // Then the whitespace after it.
    while let Some(character) = text[at..].chars().next() {
        if !character.is_whitespace() {
            break;
        }
        at += character.len_utf8();
    }
    at
}

/// The start of the word before an offset, or zero.
fn word_previous(text: &str, cursor: usize) -> usize {
    let mut at = cursor;
    while at > 0 {
        let character = text[..at].chars().next_back().expect("in range");
        if !character.is_whitespace() {
            break;
        }
        at -= character.len_utf8();
    }
    while at > 0 {
        let character = text[..at].chars().next_back().expect("in range");
        if character.is_whitespace() {
            break;
        }
        at -= character.len_utf8();
    }
    at
}

/// The end boundary of the word at or after the cursor.
fn word_end(text: &str, cursor: usize) -> usize {
    let mut at = cursor;
    while let Some(character) = text[at..].chars().next() {
        if !character.is_whitespace() {
            break;
        }
        at += character.len_utf8();
    }
    while let Some(character) = text[at..].chars().next() {
        if character.is_whitespace() {
            break;
        }
        at += character.len_utf8();
    }
    at
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Editor {
        let mut editor = Editor::new();
        for character in text.chars() {
            editor.type_char(character);
        }
        editor
    }

    #[test]
    fn typing_places_characters_and_moves_the_cursor() {
        let mut editor = typed("hi");
        assert_eq!(editor.text(), "hi");
        assert_eq!(editor.cursor(), 2);
        editor.type_char('!');
        assert_eq!(editor.text(), "hi!");
    }

    #[test]
    fn a_multibyte_character_is_one_keypress_and_one_backspace() {
        let mut editor = typed("héllo");
        assert_eq!(editor.cursor(), 6, "the cursor is a byte offset");
        editor.backspace();
        assert_eq!(editor.text(), "héll");
        assert_eq!(editor.cursor(), 5);
        // And the boundary is never split.
        editor.backspace();
        assert_eq!(editor.text(), "hél");
    }

    #[test]
    fn a_newline_is_content_and_the_cursor_can_cross_it() {
        let mut editor = typed("one\ntwo");
        assert!(editor.on_last_line());
        assert!(!editor.on_first_line());
        editor.move_cursor(Motion::First);
        assert!(editor.on_first_line());
        assert!(!editor.on_last_line());
    }

    #[test]
    fn motions_stop_at_the_ends_rather_than_wrapping() {
        let mut editor = typed("abc");
        assert_eq!(editor.move_cursor(Motion::LineStart), Effect::Changed);
        assert_eq!(editor.cursor(), 0);
        assert_eq!(editor.move_cursor(Motion::Left), Effect::Ignored);
        assert_eq!(editor.move_cursor(Motion::LineEnd), Effect::Changed);
        assert_eq!(editor.cursor(), 3);
        assert_eq!(editor.move_cursor(Motion::Right), Effect::Ignored);
    }

    #[test]
    fn word_motions_skip_whitespace_rather_than_stopping_in_it() {
        let mut editor = typed("one two three");
        editor.move_cursor(Motion::First);
        editor.move_cursor(Motion::WordNext);
        assert_eq!(&editor.text()[editor.cursor()..], "two three");
        // And back to the start of the word before it, which is where `w` and `b`
        // are inverses.
        editor.move_cursor(Motion::WordPrevious);
        assert_eq!(editor.cursor(), 0);
        assert_eq!(editor.move_cursor(Motion::WordPrevious), Effect::Ignored);
    }

    #[test]
    fn submitting_clears_the_line_and_records_it() {
        let mut editor = typed("hello");
        match editor.submit() {
            Effect::Submit(Intent::Prompt { text, .. }) => assert_eq!(text, "hello"),
            other => panic!("expected a prompt, got {other:?}"),
        }
        assert!(editor.text().is_empty());
        assert_eq!(editor.history(), ["hello"]);
    }

    #[test]
    fn submitting_nothing_is_not_an_intent_and_does_not_touch_history() {
        let mut editor = Editor::new();
        assert_eq!(editor.submit(), Effect::Ignored);
        editor.type_char(' ');
        assert_eq!(editor.submit(), Effect::Ignored);
        assert!(editor.history().is_empty());
    }

    #[test]
    fn a_repeated_submission_is_recorded_once() {
        let mut editor = Editor::new();
        editor.set_text("same");
        editor.submit();
        editor.set_text("same");
        editor.submit();
        assert_eq!(editor.history().len(), 1, "history kept a duplicate");
    }

    #[test]
    fn history_browsing_restores_the_draft_it_interrupted() {
        let mut editor = Editor::new();
        editor.set_text("first");
        editor.submit();
        editor.set_text("second");
        editor.submit();
        editor.type_char('d');
        editor.type_char('r');

        assert_eq!(editor.history_step(true), Effect::Changed);
        assert_eq!(editor.text(), "second");
        assert_eq!(editor.history_step(true), Effect::Changed);
        assert_eq!(editor.text(), "first");
        assert_eq!(
            editor.history_step(true),
            Effect::Ignored,
            "history walked past its start"
        );
        assert_eq!(editor.history_step(false), Effect::Changed);
        assert_eq!(editor.text(), "second");
        // Past the newest entry is the draft that was being typed.
        assert_eq!(editor.history_step(false), Effect::Changed);
        assert_eq!(editor.text(), "dr");
    }

    #[test]
    fn history_recalled_text_can_be_edited_and_submitted() {
        let mut editor = Editor::new();
        editor.set_text("a previous message");
        editor.submit();
        editor.history_step(true);
        editor.backspace();
        editor.type_char('!');
        match editor.submit() {
            Effect::Submit(Intent::Prompt { text, .. }) => assert_eq!(text, "a previous messag!"),
            other => panic!("expected a prompt, got {other:?}"),
        }
    }

    #[test]
    fn a_mode_change_is_visible_to_a_frontend_and_insertion_is_the_default() {
        let mut editor = Editor::new();
        assert_eq!(editor.mode(), Mode::Insert);
        assert_eq!(
            editor.set_mode(Mode::Normal),
            Effect::ModeChanged(Mode::Normal)
        );
        assert_eq!(editor.set_mode(Mode::Normal), Effect::Ignored);
        assert_eq!(
            editor.set_mode(Mode::Insert),
            Effect::ModeChanged(Mode::Insert)
        );
    }

    #[test]
    fn undo_walks_back_a_run_of_edits() {
        let mut editor = typed("one");
        editor.type_char(' ');
        editor.type_char('t');
        assert_eq!(editor.text(), "one t");
        editor.undo();
        assert_eq!(editor.text(), "one ");
        assert_eq!(editor.undo(), Effect::Changed);
    }

    #[test]
    fn redo_restores_an_undo_and_a_new_edit_invalidates_that_branch() {
        let mut editor = typed("one");
        editor.type_char('!');
        editor.undo();
        assert_eq!(editor.text(), "one");
        assert_eq!(editor.redo(), Effect::Changed);
        assert_eq!(editor.text(), "one!");
        editor.undo();
        editor.type_char('?');
        assert_eq!(editor.redo(), Effect::Ignored);
        assert_eq!(editor.text(), "one?");
    }

    #[test]
    fn a_picker_can_replace_the_line_and_the_cursor_follows() {
        let mut editor = typed("half-written");
        editor.set_text("/model scripted-1");
        assert_eq!(editor.cursor(), editor.text().len());
        editor.set_text("/");
        assert_eq!(editor.cursor(), 1);
    }

    #[test]
    fn an_interrupt_keeps_the_draft() {
        let mut editor = typed("important");
        assert_eq!(editor.interrupt(), Effect::Interrupted);
        assert_eq!(
            editor.text(),
            "important",
            "an interrupt threw away the draft"
        );
    }
    #[test]
    fn vertical_motions_keep_character_columns_and_clip_short_lines() {
        let mut editor = typed("ééé\nx\n水水水");
        editor.move_cursor(Motion::First);
        editor.move_cursor(Motion::Right);
        editor.move_cursor(Motion::Right);
        editor.move_cursor(Motion::Down);
        assert_eq!(editor.split_at_cursor(), ("ééé\nx", "\n水水水"));
        editor.move_cursor(Motion::Down);
        assert_eq!(editor.split_at_cursor(), ("ééé\nx\n水", "水水"));
        editor.move_cursor(Motion::Up);
        assert_eq!(editor.split_at_cursor(), ("ééé\nx", "\n水水水"));
    }

    #[test]
    fn deleting_a_middle_line_preserves_its_neighbors_and_undo_restores_it() {
        let mut editor = typed("one\ntwo\nthree");
        editor.move_cursor(Motion::First);
        editor.move_cursor(Motion::Down);
        assert_eq!(editor.operate('d', None), "two\n");
        assert_eq!(editor.text(), "one\nthree");
        editor.undo();
        assert_eq!(editor.text(), "one\ntwo\nthree");
    }

    #[test]
    fn normal_register_yanks_and_pastes_and_word_end_is_a_motion() {
        let mut editor = typed("one two");
        editor.move_cursor(Motion::LineStart);
        assert_eq!(editor.move_cursor(Motion::WordEnd), Effect::Changed);
        assert_eq!(editor.cursor(), 3);
        editor.move_cursor(Motion::LineStart);
        editor.operate('y', Some(Motion::WordNext));
        editor.move_cursor(Motion::Last);
        assert_eq!(editor.paste(), Effect::Changed);
        assert_eq!(editor.text(), "one twoone ");
    }
}
