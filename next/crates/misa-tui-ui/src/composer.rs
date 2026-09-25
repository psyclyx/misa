//! Editable input, local command catalogues and picker decisions.
use crate::catalog::ComposerCatalog;
use crate::prefs::{KeymapSettings, PickerSettings};
use crate::{Action, Catalog, Key, KeyOut, display_keys, ed};
use misa_kit::intent as line;
use misa_kit::intent::{Command, Intent};
use misa_kit::picker::{Accept, Effect as PickerEffect, Frecency, Picker};
use misa_proto::view::Choice;

struct HistorySearch {
    draft: String,
    query: String,
    before: usize,
}

#[derive(Default)]
pub(crate) struct Settings {
    pub picker: PickerSettings,
    pub keymap: KeymapSettings,
    pub frecency: Frecency,
    pub favorites: Vec<String>,
}
pub(crate) enum Effect {
    Frecency(Frecency),
    Remembered(String),
    Favorite(String, bool),
}

/// Decisions for the owning screen to apply; the composer never writes preferences.
pub(crate) struct Decision {
    pub out: KeyOut,
    pub action: Option<Action>,
    pub notice: Option<String>,
    pub save_needed: bool,
    pub preferences: Vec<Effect>,
}

/// Scope-local input only. Declarations, source cache, preferences, and pending
/// decisions belong to the live composer, not to a parked scope.
pub struct ParkedInput {
    editor: ed::Editor,
    picker: Option<Picker>,
    pending_command: Option<String>,
}

pub struct Composer {
    editor: ed::Editor,
    operator: Option<char>,
    history_search: Option<HistorySearch>,
    picker: Option<Picker>,
    pending_command: Option<String>,
    catalog: ComposerCatalog,
    settings: Settings,
    notice: Option<String>,
    save_needed: bool,
    action: Option<Action>,
    effects: Vec<Effect>,
}
impl Default for Composer {
    fn default() -> Self {
        Self {
            editor: ed::Editor::new(),
            operator: None,
            history_search: None,
            picker: None,
            pending_command: None,
            catalog: ComposerCatalog::default(),
            settings: Settings::default(),
            notice: None,
            save_needed: false,
            action: None,
            effects: Vec::new(),
        }
    }
}
impl Composer {
    pub fn text(&self) -> &str {
        self.editor.text()
    }
    pub fn is_empty(&self) -> bool {
        self.editor.is_empty()
    }
    pub fn mode(&self) -> ed::Mode {
        self.editor.mode()
    }
    pub fn split_at_cursor(&self) -> (&str, &str) {
        self.editor.split_at_cursor()
    }
    pub fn cursor(&self) -> usize {
        self.editor.cursor()
    }
    pub fn visual_range(&self) -> Option<(usize, usize)> {
        self.editor.visual_range()
    }
    pub fn set_text(&mut self, text: impl Into<String>) {
        self.editor.set_text(text.into());
    }
    pub fn set_mode(&mut self, mode: ed::Mode) {
        self.editor.set_mode(mode);
    }
    pub fn has_picker(&self) -> bool {
        self.picker.is_some()
    }
    pub fn picker(&self) -> Option<&Picker> {
        self.picker.as_ref()
    }
    pub fn clear_picker(&mut self) {
        self.picker = None;
        self.pending_command = None;
    }
    #[cfg(test)]
    pub(crate) fn preview_picker_items(&mut self, items: Vec<Choice>, truncated: bool) {
        self.picker
            .as_mut()
            .expect("picker open")
            .set_items(items, truncated);
    }
    #[cfg(test)]
    pub(crate) fn commit_history(&mut self) {
        self.editor.submit();
    }
    pub fn park(&mut self) -> ParkedInput {
        ParkedInput {
            editor: std::mem::replace(&mut self.editor, ed::Editor::new()),
            picker: self.picker.take(),
            pending_command: self.pending_command.take(),
        }
    }
    pub fn restore(&mut self, parked: ParkedInput) {
        self.editor = parked.editor;
        self.picker = parked.picker;
        self.pending_command = parked.pending_command;
    }
    pub fn prepend(&mut self, text: &str) {
        if self.is_empty() {
            self.set_text(text);
        } else if !text.is_empty() {
            self.set_text(format!("{text}\n{}", self.text()));
        }
    }
    pub(crate) fn settings(&mut self, settings: Settings) {
        self.settings = settings;
    }
    pub(crate) fn decide(
        &mut self,
        notice: Option<String>,
        f: impl FnOnce(&mut Self) -> KeyOut,
    ) -> Decision {
        self.notice = notice;
        let out = f(self);
        Decision {
            out,
            action: self.action.take(),
            notice: self.notice.take(),
            save_needed: std::mem::take(&mut self.save_needed),
            preferences: std::mem::take(&mut self.effects),
        }
    }
    pub(crate) fn searching(&self) -> bool {
        self.history_search.is_some()
    }
    /// A reader scroll cancels reverse search, but viewport movement belongs to Screen.
    pub(crate) fn cancel_history_search(&mut self) {
        self.history_search = None;
    }
    pub(crate) fn reader_key(&self, key: &Key) -> bool {
        self.mode() == ed::Mode::Normal
            && (matches!(key, Key::StartSelection | Key::Char('v'))
                || matches!(key, Key::Char('y')) && self.is_empty() && self.operator.is_none())
    }
    pub fn open_host_picker(&mut self, picker: Picker, text: &str) {
        self.set_text(text);
        self.pending_command = None;
        self.picker = Some(picker);
    }

    /// Replace local composer declarations after the selected scope's catalogs load.
    /// Owner metadata and connection status are supplied independently.
    pub(crate) fn declare(&mut self, info: &Catalog) {
        self.declare_with_raw(info, &[]);
    }

    /// Add host-owned commands whose arguments are interpreted by the host rather
    /// than the session's positional command parser. Session declarations win on id collisions.
    pub(crate) fn declare_with_raw(&mut self, info: &Catalog, raw: &[Command]) {
        self.catalog.declare_with_raw(info, raw);
    }

    /// The commands as candidates, built from the declaration.
    ///
    /// The declaration carries everything a candidate needs, so this costs nothing
    /// and works before any subscription has arrived. `completion.commands` exists
    /// as well, for a frontend that renders server-side and has no declaration in
    /// hand; the two say the same thing.
    pub(crate) fn command_candidates(&self) -> Vec<Choice> {
        self.catalog.command_candidates()
    }

    /// Open the picker a command's next argument needs.
    fn open_argument_picker(&mut self, command: &str, argument: &str, source: &str) -> KeyOut {
        let words = line::words(self.editor.text().trim_start_matches('/'));
        let index = self.catalog.argument_index(command, argument).unwrap_or(0);
        if words.values.first().map(String::as_str) != Some(command) {
            self.editor.set_text(format!("/{command} "));
        } else if words.values.len() <= index + 1 && !words.trailing_space {
            self.editor.set_text(format!("{} ", self.editor.text()));
        }
        self.pending_command = Some(command.to_string());
        let accept = Accept::Argument {
            command: command.to_string(),
            argument: argument.to_string(),
        };
        // Ranked by what this client remembers: a list that started from nothing every run
        // would be a list that learned nothing.
        self.picker = Some(
            Picker::inline(source, format!("/{command} {argument}"), accept)
                .with_views(
                    self.settings
                        .picker
                        .picker_views(source, misa_kit::picker::PickerPlacement::Inline),
                )
                .with_frecency(self.settings.frecency.clone())
                .with_favorites(self.settings.favorites.iter().cloned()),
        );
        if let Some((items, truncated)) = self.catalog.held(source) {
            self.picker
                .as_mut()
                .unwrap()
                .set_items(items.clone(), *truncated);
        }
        if let Some(picker) = self.picker.as_mut() {
            picker.set_query(Self::picker_query(
                self.editor.text(),
                &picker.accept,
                &self.catalog,
            ));
        }
        let needs_catalog = self.catalog.request(source);
        if needs_catalog {
            KeyOut::Complete {
                source: source.to_string(),
                prefix: self
                    .picker
                    .as_ref()
                    .map(|picker| picker.query.clone())
                    .unwrap_or_default(),
            }
        } else {
            KeyOut::Local
        }
    }

    /// Give the picker the items a source produced.
    pub(crate) fn candidates(&mut self, source: &str, items: Vec<Choice>, truncated: bool) {
        let resident = self.catalog.received(source, &items, truncated);
        if let Some(picker) = self.picker.as_mut()
            && picker.source.as_deref() == Some(source)
            && resident
        {
            picker.set_items(items, truncated);
        }
    }

    /// Commit an asynchronous completion to the source cache without allowing a
    /// stale answer to replace the query currently on screen. Resident sources
    /// are subscriptions from the client's point of view: one successful answer
    /// is enough for every later picker in this scope.
    pub(crate) fn completion(
        &mut self,
        source: &str,
        prefix: &str,
        items: Vec<Choice>,
        truncated: bool,
    ) {
        if !self.catalog.completed(source, &items, truncated) {
            return;
        }
        if let Some(picker) = self.picker.as_mut()
            && picker.source.as_deref() == Some(source)
            && picker.query == prefix
        {
            picker.set_items(items, truncated);
        }
    }

    /// Let a failed resident request be retried by the next local query. A
    /// failure is different from a successful empty catalogue: the former is a
    /// recoverable transport state, not an empty list that should trap the picker.
    pub(crate) fn completion_failed(&mut self, source: &str) {
        self.catalog.failed(source);
    }

    fn search_history(&mut self, restart: bool) {
        let search = self.history_search.as_mut().expect("search is active");
        if restart {
            search.before = self.editor.history().len();
        }
        if let Some(at) = (0..search.before)
            .rev()
            .find(|&at| self.editor.history()[at].contains(&search.query))
        {
            let found = self.editor.history()[at].clone();
            search.before = at;
            self.editor.set_text(found);
            self.notice = Some(format!("reverse search: {}", search.query));
        } else {
            self.notice = Some(format!("reverse search: {} — no match", search.query));
        }
    }

    pub(crate) fn key(&mut self, key: Key) -> KeyOut {
        if self.history_search.is_some() {
            match key {
                Key::HistorySearch => self.search_history(false),
                Key::Char(character) => {
                    self.history_search.as_mut().unwrap().query.push(character);
                    self.search_history(true);
                }
                Key::Backspace => {
                    self.history_search.as_mut().unwrap().query.pop();
                    self.search_history(true);
                }
                Key::Escape => {
                    let search = self.history_search.take().unwrap();
                    self.editor.set_text(search.draft);
                    self.notice = None;
                }
                Key::Submit => {
                    self.history_search = None;
                    self.notice = None;
                }
                Key::Quit => return KeyOut::Quit,
                _ => {
                    self.history_search = None;
                    self.notice = None;
                    return self.key(key);
                }
            }
            return KeyOut::Local;
        }

        // A picker in front of the editor takes everything except the way out.
        if self.picker.is_some() {
            return self.picker_key(if key == Key::InterruptSubmit {
                Key::Submit
            } else {
                key
            });
        }
        match key {
            Key::HistorySearch => {
                self.history_search = Some(HistorySearch {
                    draft: self.editor.text().into(),
                    query: String::new(),
                    before: self.editor.history().len(),
                });
                self.search_history(false);
                KeyOut::Local
            }
            // Alt-P is a history binding in the editor and a previous-view
            // binding only while a picker owns the input. `picker_key` has
            // already consumed the latter case above.
            Key::HistoryPrevious | Key::PickerPreviousView => {
                self.editor.history_step(true);
                KeyOut::Local
            }
            Key::HistoryNext => {
                self.editor.history_step(false);
                KeyOut::Local
            }
            Key::Newline => {
                self.editor.insert("\n");
                KeyOut::Local
            }
            Key::InterruptSubmit => match self.submit() {
                KeyOut::Intent(Intent::Prompt { text, attachments }) => {
                    KeyOut::Intent(Intent::Interrupt { text, attachments })
                }
                other => other,
            },
            Key::Quit => KeyOut::Quit,
            Key::Escape => {
                self.operator = None;
                self.editor.set_mode(ed::Mode::Normal);
                KeyOut::Local
            }
            Key::Interrupt => {
                // The draft is kept: an interrupt is about the model, not about what
                // somebody has typed. Kept, and written down, because that is the whole point
                // of keeping it.
                self.editor.interrupt();
                self.save_needed = true;
                KeyOut::Intent(Intent::Cancel { target: None })
            }
            Key::Submit => self.submit(),
            Key::Char('/') if self.editor.is_empty() => self.open_command_picker_inline(),
            // `:` lists what *this program* can do, which is a different question
            // from what the session can do and is answered without asking it.
            Key::Char(':') if self.editor.is_empty() => self.open_action_palette(),
            Key::Fill(text) => {
                // A picker's answer, arriving as the line it completes.
                self.editor.set_text(text);
                self.submit()
            }
            Key::Char(character) if self.editor.mode() == ed::Mode::Normal => {
                self.normal_char(character)
            }
            Key::Char(character) if self.editor.mode() == ed::Mode::Visual => {
                self.visual_char(character)
            }
            Key::Char(character) => {
                self.editor.type_char(character);
                KeyOut::Local
            }
            Key::Backspace => {
                self.editor.backspace();
                KeyOut::Local
            }
            Key::Eof if self.editor.is_empty() => KeyOut::Quit,
            Key::Delete | Key::Eof => {
                self.editor.delete();
                KeyOut::Local
            }
            Key::Tab => self.complete_argument(),
            Key::Motion(motion) => {
                if let Some(operator) = self.operator.take() {
                    let text = self.editor.operate(operator, Some(motion));
                    if operator == 'y' {
                        return KeyOut::Copy(text);
                    }
                } else if motion == ed::Motion::Up && self.editor.on_first_line() {
                    self.editor.history_step(true);
                } else if motion == ed::Motion::Down && self.editor.on_last_line() {
                    self.editor.history_step(false);
                } else {
                    self.editor.move_cursor(motion);
                }
                KeyOut::Local
            }
            // A picker owns this key; otherwise Screen routes it to the viewport.
            Key::ScrollPage(_) => KeyOut::Local,
            Key::Action(action) => {
                self.action = Some(action);
                KeyOut::Local
            }
            Key::Favorite
            | Key::PickerSlot(_)
            | Key::StartSelection
            | Key::Extend(_)
            | Key::Alt(_) => KeyOut::Local,
            Key::QueueEdit => KeyOut::Intent(Intent::Action {
                node: "queue".into(),
                action: "queue.edit".into(),
                args: misa_value::Value::Null,
                fields: vec![],
            }),
        }
    }

    fn normal_char(&mut self, character: char) -> KeyOut {
        let motion = match character {
            'h' => Some(ed::Motion::Left),
            'l' => Some(ed::Motion::Right),
            'w' => Some(ed::Motion::WordNext),
            'b' => Some(ed::Motion::WordPrevious),
            'e' => Some(ed::Motion::WordEnd),
            '0' => Some(ed::Motion::LineStart),
            '$' => Some(ed::Motion::LineEnd),
            'g' => Some(ed::Motion::First),
            'G' => Some(ed::Motion::Last),
            'j' => Some(ed::Motion::Down),
            'k' => Some(ed::Motion::Up),
            _ => None,
        };
        if let Some(operator) = self.operator.take() {
            if character == operator || motion.is_some() {
                let text = self.editor.operate(operator, motion);
                if operator == 'y' {
                    return KeyOut::Copy(text);
                }
            }
        } else if let Some(motion) = motion {
            return self.key(Key::Motion(motion));
        } else {
            match character {
                'd' | 'c' | 'y' => self.operator = Some(character),
                'i' => {
                    self.editor.set_mode(ed::Mode::Insert);
                }
                'I' => {
                    self.editor.move_cursor(ed::Motion::LineStart);
                    self.editor.set_mode(ed::Mode::Insert);
                }
                'A' => {
                    self.editor.move_cursor(ed::Motion::LineEnd);
                    self.editor.set_mode(ed::Mode::Insert);
                }
                'v' => {
                    self.editor.set_mode(ed::Mode::Visual);
                }
                'V' => {
                    self.editor.visual_line();
                }
                'a' => {
                    self.editor.move_cursor(ed::Motion::Right);
                    self.editor.set_mode(ed::Mode::Insert);
                }
                'o' | 'O' => self.editor.open_line(character == 'O'),
                'p' => {
                    self.editor.paste();
                }
                'x' => {
                    self.editor.delete();
                }
                'u' => {
                    self.editor.undo();
                }
                'U' => {
                    self.editor.redo();
                }
                _ => {}
            }
        }
        KeyOut::Local
    }

    fn visual_char(&mut self, character: char) -> KeyOut {
        match character {
            'y' | 'd' | 'c' => {
                let text = self.editor.visual_operation(character);
                if character == 'y' {
                    KeyOut::Copy(text)
                } else {
                    KeyOut::Local
                }
            }
            _ => KeyOut::Local,
        }
    }

    /// Enter: submit what is there, or open the picker a declaration asks for.
    fn submit(&mut self) -> KeyOut {
        let text = self.editor.text().to_string();
        // Raw commands are declared explicitly by the host. Only the command word
        // is inspected here; the host owns the syntax of the rest of the line.
        let trimmed = text.trim();
        if let Some(word) = trimmed
            .strip_prefix('/')
            .filter(|word| !word.contains('\n'))
            .and_then(|word| word.split_whitespace().next())
            && self.catalog.is_raw(word)
        {
            self.editor.submit();
            self.notice = None;
            self.save_needed = true;
            return KeyOut::Submitted(text);
        }
        match line::parse(&text, self.catalog.commands()) {
            line::Parsed::Invalid { message } => {
                self.notice = Some(message);
                KeyOut::Local
            }
            line::Parsed::Empty => KeyOut::Local,
            line::Parsed::Needs {
                command,
                argument,
                source,
                ..
            } => {
                // A command that cannot run yet is not sent. The declaration said
                // where its value comes from, so the client opens its own picker.
                match source {
                    Some(source) => self.open_argument_picker(&command, &argument, &source),
                    None => {
                        self.notice = Some(format!("/{command} needs a value for {argument}"));
                        KeyOut::Local
                    }
                }
            }
            line::Parsed::Unknown { name } => {
                self.notice = Some(format!("no command named `/{name}`"));
                KeyOut::Local
            }
            parsed => match line::intent(&parsed) {
                Some(intent) => {
                    self.editor.submit();
                    self.notice = None;
                    // The draft is spent; what is left is written down, so a client that is
                    // killed after this does not resurrect what was just sent.
                    self.save_needed = true;
                    KeyOut::Intent(intent)
                }
                None => KeyOut::Local,
            },
        }
    }

    /// Tab: complete the argument the cursor is in, if the declaration says one can
    /// be completed.
    fn complete_argument(&mut self) -> KeyOut {
        let text = self.editor.text().trim_start().to_string();
        let Some(rest) = text.strip_prefix('/') else {
            return KeyOut::Local;
        };
        let words = line::words(rest);
        let name = words.values.first().cloned().unwrap_or_default();
        let Some(command) = self.catalog.command(&name).cloned() else {
            return KeyOut::Local;
        };
        let count = words.values.len().saturating_sub(1);
        let position = if words.trailing_space {
            count
        } else {
            count.saturating_sub(1)
        };
        let Some(argument) = command.args.get(position) else {
            return KeyOut::Local;
        };
        match argument.source.clone() {
            Some(source) => self.open_argument_picker(&name, &argument.name, &source),
            None => KeyOut::Local,
        }
    }

    fn open_command_picker_inline(&mut self) -> KeyOut {
        self.open_command_picker_with(misa_kit::picker::PickerPlacement::Inline)
    }

    pub(crate) fn open_command_picker(&mut self) -> KeyOut {
        self.open_command_picker_with(misa_kit::picker::PickerPlacement::Overlay)
    }

    fn open_command_picker_with(&mut self, placement: misa_kit::picker::PickerPlacement) -> KeyOut {
        self.pending_command = None;
        self.editor.set_text("/");
        // `/` on an empty line is a request for the session's commands, and the
        // promise the declaration made is that they can be listed without asking.
        let mut picker = if self.catalog.has_source("commands") {
            match placement {
                misa_kit::picker::PickerPlacement::Inline => {
                    Picker::inline("commands", "Commands", Accept::Run)
                }
                misa_kit::picker::PickerPlacement::Overlay => {
                    Picker::over("commands", "Commands", Accept::Run)
                }
            }
        } else {
            Picker::new("Commands", Accept::Run)
        };
        picker.placement = placement;
        picker.set_items(self.command_candidates(), false);
        self.picker = Some(
            picker
                .with_views(self.settings.picker.picker_views("commands", placement))
                .with_frecency(self.settings.frecency.clone())
                .with_favorites(self.settings.favorites.iter().cloned()),
        );
        KeyOut::Local
    }

    /// Open the client's own actions.
    pub(crate) fn open_action_palette(&mut self) -> KeyOut {
        self.editor.set_text(":");
        let mut picker = Picker::over("actions", "Actions", Accept::Run)
            .with_views(
                self.settings
                    .picker
                    .picker_views("actions", misa_kit::picker::PickerPlacement::Overlay),
            )
            .with_frecency(self.settings.frecency.clone())
            .with_favorites(self.settings.favorites.iter().cloned());
        picker.set_items(
            Action::ALL
                .iter()
                .filter(|action| match action.command_name() {
                    Some(command) => self.catalog.has_command(command),
                    None => true,
                })
                .map(|action| {
                    let keys = action.keys(&self.settings.keymap);
                    Choice {
                        value: action.id().to_string(),
                        label: action.label().to_string(),
                        detail: Some(if keys.is_empty() {
                            String::new()
                        } else {
                            display_keys(&keys)
                        }),
                        metadata: None,
                    }
                })
                .collect(),
            false,
        );
        self.picker = Some(picker);
        KeyOut::Local
    }

    fn picker_key(&mut self, key: Key) -> KeyOut {
        // The slash is a lexical boundary, not a character for the current
        // command or argument query. It starts a fresh command path, so `/model`
        // can be abandoned directly for `/login` without clearing the draft first.
        if key == Key::Char('/')
            && self.editor.text() != "/"
            && matches!(
                self.picker.as_ref().map(|picker| &picker.accept),
                Some(Accept::Run | Accept::Argument { .. })
            )
        {
            return self.open_command_picker_inline();
        }
        if key == Key::Action(Action::OpenCommands) {
            // Alt-/ is the reference picker's replace-view binding: it returns to
            // the command catalog, with a fresh lexical query and the command
            // picker as the owner of the input.
            return self.open_command_picker();
        }
        // Completing the command word moves completion to its argument while
        // keeping the composer as the only editable text.
        if key == Key::Char(' ')
            && self.pending_command.is_none()
            && self.editor.text().starts_with('/')
        {
            let name = self.editor.text().trim_start_matches('/');
            if self.catalog.has_command(name) {
                self.editor.type_char(' ');
                self.picker = None;
                return match line::parse(self.editor.text(), self.catalog.commands()) {
                    line::Parsed::Needs {
                        source: Some(_), ..
                    } => self.submit(),
                    _ => KeyOut::Local,
                };
            }
        }
        if key == Key::Tab {
            let query = self.picker.as_mut().and_then(Picker::complete);
            if let Some(query) = query {
                self.apply_picker_query(&query);
            }
            return KeyOut::Local;
        }
        let Some(picker) = self.picker.as_mut() else {
            return KeyOut::Local;
        };
        let effect = match key {
            Key::Escape => picker.cancel(),
            Key::Submit => picker.accept(),
            Key::Motion(ed::Motion::Right) => {
                picker.cycle_view(1);
                PickerEffect::None
            }
            Key::PickerPreviousView => {
                picker.cycle_view(-1);
                PickerEffect::None
            }
            Key::Motion(ed::Motion::Down) | Key::HistoryNext => {
                picker.move_selection(1);
                PickerEffect::None
            }
            Key::Favorite => picker.favorite(),
            Key::Motion(ed::Motion::Up) | Key::HistoryPrevious => {
                picker.move_selection(-1);
                PickerEffect::None
            }
            Key::PickerSlot(index) => {
                if picker.select_index(index) {
                    picker.accept()
                } else {
                    PickerEffect::None
                }
            }
            // The reference choice context owns Alt-letter sequences. In the
            // normal editor those same physical keys may be global actions;
            // routing them here keeps the context boundary explicit.
            Key::Action(Action::ScrollUp) => picker.shortcut_key('k'),
            Key::Action(Action::ScrollDown) => picker.shortcut_key('j'),
            Key::QueueEdit => picker.shortcut_key('e'),
            Key::Alt(character) => picker.shortcut_key(character),
            Key::Char(character) if picker.shortcut_pending() => picker.shortcut_key(character),
            Key::Char(character) => {
                self.editor.type_char(character);
                picker.set_query(Self::picker_query(
                    self.editor.text(),
                    &picker.accept,
                    &self.catalog,
                ))
            }
            Key::Backspace | Key::Delete => {
                if key == Key::Backspace {
                    self.editor.backspace();
                } else {
                    self.editor.delete();
                }
                if self.editor.is_empty() {
                    picker.cancel()
                } else {
                    picker.set_query(Self::picker_query(
                        self.editor.text(),
                        &picker.accept,
                        &self.catalog,
                    ))
                }
            }
            Key::Eof => picker.cancel(),
            Key::Motion(ed::Motion::Left) => {
                picker.cycle_view(-1);
                PickerEffect::None
            }
            Key::Quit => return KeyOut::Quit,
            Key::Interrupt => picker.cancel(),
            _ => PickerEffect::None,
        };
        self.picker_effect(effect)
    }

    fn picker_query(text: &str, accept: &Accept, catalog: &ComposerCatalog) -> String {
        match accept {
            Accept::Argument { command, argument } => {
                let index = catalog.argument_index(command, argument);
                index
                    .and_then(|index| {
                        line::words(text.trim_start_matches('/'))
                            .values
                            .get(index + 1)
                            .cloned()
                    })
                    .unwrap_or_default()
            }
            _ => text
                .strip_prefix('/')
                .or_else(|| text.strip_prefix(':'))
                .unwrap_or(text)
                .into(),
        }
    }

    fn apply_picker_query(&mut self, query: &str) {
        let Some(picker) = self.picker.as_ref() else {
            return;
        };
        match &picker.accept {
            Accept::Argument { command, argument } => {
                let index = self.catalog.argument_index(command, argument).unwrap_or(0);
                let mut values = line::words(self.editor.text().trim_start_matches('/'))
                    .values
                    .into_iter()
                    .skip(1)
                    .collect::<Vec<_>>();
                values.resize(values.len().max(index + 1), String::new());
                values[index] = query.to_string();
                self.editor.set_text(format!(
                    "/{command} {}",
                    values
                        .iter()
                        .map(|value| line::quote(value))
                        .collect::<Vec<_>>()
                        .join(" ")
                ));
            }
            Accept::Run => {
                let prefix = if self.editor.text().starts_with(':') {
                    ':'
                } else {
                    '/'
                };
                self.editor.set_text(format!("{prefix}{query}"));
            }
            Accept::Action { .. } => self.editor.set_text(query),
        }
    }

    pub(crate) fn paste(&mut self, text: &str) -> KeyOut {
        self.editor.insert(text);
        let effect = self.picker.as_mut().map(|picker| {
            picker.set_query(Self::picker_query(
                self.editor.text(),
                &picker.accept,
                &self.catalog,
            ))
        });
        effect.map_or(KeyOut::Local, |effect| self.picker_effect(effect))
    }

    fn picker_effect(&mut self, effect: PickerEffect) -> KeyOut {
        match effect {
            PickerEffect::None => KeyOut::Local,
            PickerEffect::Cancelled => {
                self.picker = None;
                self.pending_command = None;
                self.editor.set_text("");
                KeyOut::Local
            }
            PickerEffect::Ask { source, prefix } => {
                if self.catalog.should_ask(&source) {
                    KeyOut::Complete { source, prefix }
                } else {
                    KeyOut::Local
                }
            }
            PickerEffect::Accepted(accepted) => {
                // An accepted candidate is what frecency is *for*: the next list is ranked by
                // it, and a count a restart forgets is a list ranked by nothing. Taken from
                // the picker before it is dropped, because that is where the counts are.
                if let Some(picker) = &self.picker {
                    let frecency = picker.frecency().clone();
                    self.effects.push(Effect::Frecency(frecency));
                }
                self.picker = None;
                self.effects
                    .push(Effect::Remembered(accepted.value.clone()));
                self.pending_command = None;
                self.save_needed = true;
                // An accepted argument completes the line rather than sending it, so
                // somebody can add the next argument or edit what they got.
                if let Accept::Argument { command, argument } = &accepted.accept {
                    let index = self.catalog.argument_index(command, argument).unwrap_or(0);
                    let mut values = line::words(self.editor.text().trim_start_matches('/'))
                        .values
                        .into_iter()
                        .skip(1)
                        .collect::<Vec<_>>();
                    values.resize(values.len().max(index + 1), String::new());
                    values[index] = accepted.value.clone();
                    let text = format!(
                        "/{command} {}",
                        values
                            .iter()
                            .map(|value| line::quote(value))
                            .collect::<Vec<_>>()
                            .join(" ")
                    );
                    self.editor.set_text(text);
                    self.notice = Some(format!("{} → {}", accepted.label, accepted.value));
                    // A command whose arguments are all filled is complete, so it is
                    // sent: somebody who picked a model has said what they meant. One
                    // that is not complete leaves the line for the next argument.
                    match line::parse(self.editor.text(), self.catalog.commands()) {
                        line::Parsed::Command { .. } => self.submit(),
                        _ => KeyOut::Local,
                    }
                } else if let Some(action) = Action::find(&accepted.value) {
                    // A candidate without a slash is one of this program's own actions:
                    // there is nobody to ask, so it is done here.
                    self.notice = None;
                    {
                        self.action = Some(action);
                        KeyOut::Local
                    }
                } else {
                    // A command candidate is not sent here: it is put into the line and
                    // submitted through the same path as a typed one, so a command with
                    // an argument opens that argument's picker instead of being refused.
                    self.editor.set_text(Picker::fill_text(&accepted));
                    self.submit()
                }
            }
            PickerEffect::Favorited { value, favorite } => {
                self.effects.push(Effect::Favorite(value.clone(), favorite));
                self.notice = Some(if favorite {
                    format!("favorite: {value}")
                } else {
                    format!("unfavorite: {value}")
                });
                self.save_needed = true;
                KeyOut::Local
            }
        }
    }

    /// Open the model chooser as a focused overlay. The typed `/model` path is
    /// still editor completion; the global model action is the direct chooser.
    pub(crate) fn open_model_picker(&mut self) -> KeyOut {
        let command = "model";
        let source = "models";
        self.pending_command = Some(command.into());
        self.editor.set_text("/model ");
        let mut picker = Picker::over(
            source,
            "Model",
            Accept::Argument {
                command: command.into(),
                argument: "model".into(),
            },
        )
        .with_views(
            self.settings
                .picker
                .picker_views(source, misa_kit::picker::PickerPlacement::Overlay),
        )
        .with_frecency(self.settings.frecency.clone())
        .with_favorites(self.settings.favorites.iter().cloned());
        if let Some((items, truncated)) = self.catalog.held(source) {
            picker.set_items(items.clone(), *truncated);
        }
        self.picker = Some(picker);
        if self.catalog.should_ask(source) {
            KeyOut::Complete {
                source: source.into(),
                prefix: String::new(),
            }
        } else {
            KeyOut::Local
        }
    }
}
