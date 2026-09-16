//! The terminal frontend.
//!
//! It renders observed semantic documents and owns local themes and drafts.
//! Shared client catalogs prepare reads and invocations; owners validate and
//! execute them. Daemon relationships remain independent of the selected session.
//!
//! # Two palettes, and why they are different things
//!
//! `:` opens the **client's** actions — the things this program can do to its own
//! display. `/` opens the **session's** commands, read from the declarations the
//! session sent. That split is not cosmetic: an action is a key that changes what
//! somebody is looking at, and a command is a request that changes a conversation,
//! and only one of those belongs on the other side of a network.
//!
//! # What the picker needs from a session
//!
//! Nothing, in the common case. The declarations say a command needs a model and
//! that models come from a source; the items arrive as a subscription the client
//! holds; the matching happens here. A session is asked only when a source has no
//! items to hold — see [`misa_kit::picker`].

pub mod print;
pub mod workspace;
pub mod scoped_remote;
mod dialogs;
pub mod presentation;
pub mod output;
mod event_loop;
mod retained;
mod chrome;
pub mod clipboard;
pub mod storage;
pub mod prefs;
pub mod save;

#[cfg(test)]
thread_local! { static RESOLVE_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

use std::path::PathBuf;

use misa_kit::picker::{Accept, Effect as PickerEffect, Picker};
use crate::prefs::Prefs;
use misa_kit::{editor as ed, intent as line};
use misa_render::select;
use misa_proto::view::{ActionOn, Choice, Field, Kind, Node};
use misa_kit::intent::Command;
use misa_proto::preparation::SourceKind;
use misa_kit::intent::Intent;
use misa_kit::intent::Source;
use misa_render::{Line, Theme};

/// One action this program can take on its own display.
///
/// Client-side by definition, and the reason `:` is a different palette from `/`: a
/// session has no opinion about whether a transcript is expanded or which theme
/// somebody prefers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ToggleDetail,
    ScrollTop,
    ScrollBottom,
    ThemeDark,
    ThemePlain,
    OpenCommands,
    Quit,
}

impl Action {
    pub const ALL: &'static [Action] = &[
        Action::ToggleDetail,
        Action::ScrollTop,
        Action::ScrollBottom,
        Action::ThemeDark,
        Action::ThemePlain,
        Action::OpenCommands,
        Action::Quit,
    ];

    pub fn id(&self) -> &'static str {
        match self {
            Action::ToggleDetail => "transcript.detail",
            Action::ScrollTop => "transcript.top",
            Action::ScrollBottom => "transcript.bottom",
            Action::ThemeDark => "theme.dark",
            Action::ThemePlain => "theme.plain",
            Action::OpenCommands => "commands.open",
            Action::Quit => "app.quit",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Action::ToggleDetail => "Expand or collapse every tool call",
            Action::ScrollTop => "Go to the top of the transcript",
            Action::ScrollBottom => "Follow the newest output",
            Action::ThemeDark => "Use the dark theme",
            Action::ThemePlain => "Use no colour",
            Action::OpenCommands => "List the session's commands",
            Action::Quit => "Leave",
        }
    }

    /// The keys this action is bound to, for the palette to show beside it.
    pub fn keys(&self) -> &'static str {
        match self {
            Action::ToggleDetail => "ctrl-t",
            Action::ScrollTop => "ctrl-home",
            Action::ScrollBottom => "ctrl-end",
            Action::ThemeDark => "",
            Action::ThemePlain => "",
            Action::OpenCommands => "/",
            Action::Quit => "ctrl-q",
        }
    }

    pub fn find(id: &str) -> Option<Action> {
        Action::ALL.iter().copied().find(|action| action.id() == id)
    }
}

/// What somebody has typed into a panel.
///
/// A panel is the session's state — what question is open, what field it has — and this is the
/// client's: the text in the field before it is submitted, which belongs to whoever is typing
/// and is never sent until they say so.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PanelInput {
    /// The id of the panel node this is an answer to.
    pub panel: String,
    /// The field being typed into, by id. Empty when the panel has nothing to type into.
    pub field: String,
    pub text: String,
}

/// What a keypress caused.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyOut {
    DaemonInvoke { daemon:String,scope:misa_proto::observation::Scope,command:String,input:misa_value::Value },
    Invoke { command: String, input: misa_value::Value },
    /// Choose a local destination, then request the attachment the session offered.
    Save(save::Request),
    /// The client handled it alone.
    Local,
    /// Something to ask a session.
    Intent(Intent),
    /// The picker needs candidates a session has and this client does not.
    Complete { source: String, prefix: String },
    /// Text to put on the terminal's clipboard: a reader's selection, copied.
    Copy(String),
    /// Leave.
    Quit,
}

/// The client's whole state.
struct HistorySearch {
    draft: String,
    query: String,
    before: usize,
}

pub struct Screen {
    pub dialogs: dialogs::Dialogs,
    pub local_presentation: presentation::Local,
    pub components: misa_render::components::Registry,
    pub values: misa_render::fact::Registry,
    operator: Option<char>,
    history_search: Option<HistorySearch>,
    pub theme: Theme,
    pub editor: ed::Editor,
    /// The picker in front of the editor, when one is open.
    pub picker: Option<Picker>,
    /// Which command an accepted argument belongs to.
    pending_command: Option<String>,
    /// What this client remembers between runs: the theme, the nodes somebody opened, the
    /// draft, and which choices they reach for. Presentation state, all of it, and the reason
    /// a restart no longer forgets where somebody was.
    pub prefs: Prefs,
    /// Where that memory is written. `None` for a client that was not told, which is what a
    /// test is: a client with no home directory should still run.
    pub prefs_path: Option<PathBuf>,
    pub notice: Option<String>,
    /// The reader's selection, when one is open. It is over the rendered body, so
    /// moving it needs the view — which is why `selection_key` takes one.
    pub selection: Option<select::Selection>,
    /// What has been typed into an open panel, and which panel it is for.
    pub panel: Option<PanelInput>,
    pub commands: Vec<Command>,
    pub sources: Vec<Source>,
    pub location: String,
    resident: std::collections::HashMap<String, (Vec<Choice>, bool)>,
    pub scroll: usize,
    /// Whether the viewport follows new output. Scrolling away stops it, which is
    /// what lets somebody read while a model is still writing.
    pub follow: bool,
    pub width: u16,
    pub height: u16,
}

impl Screen {
    pub fn new(width: u16, height: u16) -> Screen {
        Screen {
            local_presentation: presentation::stock(),
            components: Default::default(),
            values: Default::default(),
            operator: None,
            history_search: None,
            theme: Theme::dark(),
            editor: ed::Editor::new(),
            picker: None,
            pending_command: None,
            prefs: Prefs::default(),
            prefs_path: None,
            notice: None,
            dialogs: Default::default(),
            selection: None,
            panel: None,
            commands: Vec::new(),
            sources: Vec::new(),
            location: String::new(),
            resident: Default::default(),
            scroll: 0,
            follow: true,
            width,
            height,
        }
    }

    /// A screen that remembers, which is the one a person gets.
    ///
    /// Separate from [`Screen::new`] because a test wants a screen with no state directory:
    /// a client that wrote to somebody's home during a test would be a client whose tests
    /// depend on the order they ran in.
    pub fn durable() -> Screen {
        let path = storage::File::default_path();
        Screen::remembering(Prefs::load(&storage::File::at(path.clone())), path)
    }

    /// A screen remembering a document somebody else decided where to keep.
    ///
    /// The seam a test needs: the default path is somebody's home directory, and a test
    /// that wrote there would be a test that depends on the machine it ran on.
    pub fn remembering(prefs: Prefs, path: PathBuf) -> Screen {
        let mut screen = Screen::new(100, 40);
        screen.theme = match prefs.theme.as_str() {
            "plain" => Theme::plain(),
            _ => Theme::dark(),
        };
        screen.editor.set_text(prefs.draft.clone());
        screen.prefs = prefs;
        screen.prefs_path = Some(path);
        screen
    }

    /// Write what this client knows, and say so when it cannot.
    ///
    /// Called where a decision changed something and once on the way out — not on every
    /// keystroke: a write per character is a write per character. The draft is the one thing
    /// that waits for somebody to stop typing, which is the price of this being a file.
    pub fn save(&mut self) {
        let Some(path) = self.prefs_path.clone() else {
            return;
        };
        self.prefs.draft = self.editor.text().to_string();
        if let Err(error) = self.prefs.save(&storage::File::at(path)) {
            self.notice = Some(error);
        }
    }


    /// Replace local composer declarations after the selected scope's catalogs load.
    /// Owner metadata and connection status are supplied independently.
    pub fn declare(&mut self, info: &Catalog) {
        self.resident.clear();
        self.commands = info.commands.clone();
        self.commands.push(Command::new("save", "Save attachment", "/save [number] <local path>"));
        self.sources = info.sources.clone();
    }

    /// The commands as candidates, built from the declaration.
    ///
    /// The declaration carries everything a candidate needs, so this costs nothing
    /// and works before any subscription has arrived. `completion.commands` exists
    /// as well, for a frontend that renders server-side and has no declaration in
    /// hand; the two say the same thing.
    pub fn command_candidates(&self) -> Vec<Choice> {
        self.commands
            .iter()
            .map(|command| Choice {
                value: format!("/{}", command.id),
                label: format!("/{}", command.id),
                detail: Some(if command.args.is_empty() {
                    command.description.clone()
                } else {
                    format!(
                        "{} — {}",
                        command.description,
                        command.args.iter().map(|arg| arg.label.clone()).collect::<Vec<_>>().join(" ")
                    )
                }),
            })
            .collect()
    }

    fn source(&self, id: &str) -> Option<&Source> {
        self.sources.iter().find(|source| source.id == id)
    }

    /// Whether a source can be answered from what this client already holds.
    fn is_resident(&self, id: &str) -> bool {
        self.source(id).map(|source| source.kind == SourceKind::Resident).unwrap_or(false)
    }

    /// Open the picker a command's next argument needs.
    fn open_argument_picker(&mut self, command: &str, argument: &str, source: &str) {
        if self.editor.text().trim() == format!("/{command}") || !self.editor.text().starts_with(&format!("/{command} ")) {
            self.editor.set_text(format!("/{command} "));
        }
        self.pending_command = Some(command.to_string());
        let accept = Accept::Argument { command: command.to_string(), argument: argument.to_string() };
        // Ranked by what this client remembers: a list that started from nothing every run
        // would be a list that learned nothing.
        self.picker = Some(
            Picker::over(source, format!("/{command} {argument}"), accept)
                .with_frecency(self.prefs.frecency()),
        );
        if let Some((items, truncated)) = self.resident.get(source) {
            self.picker.as_mut().unwrap().set_items(items.clone(), *truncated);
        }
    }

    /// Give the picker the items a source produced.
    pub fn candidates(&mut self, source: &str, items: Vec<Choice>, truncated: bool) {
        let resident = self.is_resident(source);
        if resident { self.resident.insert(source.into(), (items.clone(), truncated)); }
        if let Some(picker) = self.picker.as_mut()
            && picker.source.as_deref() == Some(source)
            && resident
        {
            picker.set_items(items, truncated);
        }
    }

    /// The rendered body a selection moves over. Nothing here reaches a session.
    fn body(&self, view: &Node) -> select::Body {
        let resolved = self.resolve(view);
        select::Body::of(&misa_render::render(&resolved, &self.theme, self.width as usize))
    }

    /// A key that concerns the reader's selection, if it concerns one at all.
    ///
    /// `None` means "not about the selection", and the caller falls through to
    /// [`Screen::key`]. It takes the view because a selection is over the *rendered*
    /// body, and only a caller holding the tree can compute that — which is the price
    /// of byte offsets meaning something.
    pub fn selection_key(&mut self, view: &Node, key: &Key) -> Option<KeyOut> {
        if !self.reading_key(key) { return None; }
        self.selection_in(&self.body(view), key)
    }
    fn reading_key(&self, key: &Key) -> bool {
        self.selection.is_some() || self.editor.mode() == ed::Mode::Normal &&
            (matches!(key, Key::Char('v')) || matches!(key, Key::Char('y')) && self.editor.is_empty() && self.operator.is_none())
    }
    fn selection_in(&mut self, body: &select::Body, key: &Key) -> Option<KeyOut> {
        if self.selection.is_some() {
            return Some(self.selecting(body, key));
        }
        // `v` and `y` belong to a reader, but only where a vim reader expects them: in
        // normal mode, so typing into the composer is never stolen.
        if self.editor.mode() != ed::Mode::Normal {
            return None;
        }
        match key {
            Key::Char('v') => {
                self.begin_selection(body);
                Some(KeyOut::Local)
            }
            Key::Char('y') if self.editor.is_empty() && self.operator.is_none() => Some(self.copy_body(body)),
            _ => None,
        }
    }

    /// A key that concerns an open panel, if one is open.
    ///
    /// The panel is modal in the terminal, the way the picker is: it is the one thing on the
    /// screen that is a question rather than something to read, so while it is up every key
    /// that is not the way out belongs to it. The two exceptions are the ways out of the
    /// *program*, because a person may always stop.
    ///
    /// A client that wanted a non-modal panel would put the field somewhere else; what it may
    /// not do is decide the panel is not a question.
    pub fn panel_key(&mut self, view: &Node, key: &Key) -> Option<KeyOut> {
        let panel = panel_of(view)?;
        if matches!(key, Key::Quit | Key::Interrupt) {
            return None;
        }
        let asking = panel.id.clone();
        if self.panel.as_ref().map(|state| &state.panel) != Some(&asking) {
            // A draft belongs to the question that asked for it: a value carried into the next
            // panel is a value nobody wrote.
            let field = panel_field(panel).map(|field| field.id.clone()).unwrap_or_default();
            self.notice = if field.is_empty() {
                Some(format!("{} — esc dismisses", panel.label.clone().unwrap_or_default()))
            } else {
                Some("typing into the panel — enter sends it, esc dismisses".to_string())
            };
            self.panel = Some(PanelInput { panel: asking, field, text: String::new() });
        }
        Some(match key {
            Key::Eof if self.panel.as_ref().is_some_and(|panel| panel.text.is_empty()) => KeyOut::Quit,
            Key::Escape => self.dismiss_panel(panel),
            Key::Submit => self.submit_panel(panel),
            Key::Backspace | Key::Delete => {
                if let Some(state) = self.panel.as_mut() {
                    state.text.pop();
                }
                KeyOut::Local
            }
            Key::Char(character) => {
                if let Some(state) = self.panel.as_mut() {
                    state.text.push(*character);
                }
                KeyOut::Local
            }
            _ => KeyOut::Local,
        })
    }

    /// Take the panel away, by the action the session offered for it.
    fn dismiss_panel(&mut self, panel: &Node) -> KeyOut {
        let close = panel.actions.iter().find(|action| action.on == ActionOn::Click);
        let Some(close) = close else {
            // A panel nobody can dismiss is the session's decision; this client will not
            // invent one, and it says so rather than eating the key in silence.
            self.notice = Some("this panel has no way out".to_string());
            return KeyOut::Local;
        };
        self.panel = None;
        self.notice = None;
        KeyOut::Intent(Intent::Action {
            node: panel.id.clone(),
            action: close.id.clone(),
            args: misa_value::Value::Null,
            fields: Vec::new(),
        })
    }

    /// Send what is in the field, if the panel asked for something.
    fn submit_panel(&mut self, panel: &Node) -> KeyOut {
        let Some(form) = panel.children.iter().find(|child| matches!(&child.kind, Kind::Fields { fields } if !fields.is_empty()))
        else {
            return KeyOut::Local;
        };
        let Some(action) = form.actions.iter().find(|action| action.on == ActionOn::Submit) else {
            return KeyOut::Local;
        };
        let Kind::Fields { fields: declared } = &form.kind else {
            return KeyOut::Local;
        };
        let typed = self.panel.as_ref().map(|state| state.text.clone()).unwrap_or_default();
        let focused = self.panel.as_ref().map(|state| state.field.clone()).unwrap_or_default();
        let fields = declared
            .iter()
            .map(|field| Field {
                value: if field.id == focused { typed.clone() } else { field.value.clone() },
                ..field.clone()
            })
            .collect::<Vec<_>>();
        // What was typed goes out of this client's hands as it leaves the screen: a secret
        // that stays in a field after it has been sent is a secret on a screen.
        if let Some(state) = self.panel.as_mut() {
            state.text.clear();
        }
        self.notice = Some("sent".to_string());
        KeyOut::Intent(Intent::Action {
            node: form.id.clone(),
            action: action.id.clone(),
            args: misa_value::Value::Null,
            fields,
        })
    }

    /// Start a selection at the bottom, which is where somebody following the tail is
    /// already looking.
    fn begin_selection(&mut self, body: &select::Body) {
        let row = body.len().saturating_sub(1);
        self.selection = Some(select::Selection::caret(select::Spot::new(row, 0)));
        self.notice = Some("copying — y takes it, esc stops".to_string());
    }

    /// Copy the whole body, for a reader who did not bother to select anything.
    fn copy_body(&mut self, body: &select::Body) -> KeyOut {
        let mut everything = select::Selection::caret(select::Spot::new(0, 0));
        everything.document_end(&body);
        let text = everything.text(&body);
        self.notice = Some(format!("copied {} bytes", text.len()));
        KeyOut::Copy(text)
    }

    fn selecting(&mut self, body: &select::Body, key: &Key) -> KeyOut {
        let Some(mut selection) = self.selection.take() else {
            return KeyOut::Local;
        };
        match key {
            Key::Escape => {
                self.notice = None;
                return KeyOut::Local;
            }
            Key::Char('y') | Key::Submit => {
                let text = selection.text(&body);
                self.notice = Some(format!("copied {} bytes", text.len()));
                return KeyOut::Copy(text);
            }
            // `o` drops the anchor where the caret is; `v` cycles what the range means.
            Key::Char('o') => selection.restart(),
            Key::Char('v') => selection.set_kind(selection.kind().next()),
            Key::Char('a') => selection.select_node(&body),
            Key::Char('w') => selection.word_right(&body),
            Key::Char('b') => selection.word_left(&body),
            Key::Char('n') => selection.node_forward(&body),
            Key::Char('p') => selection.node_back(&body),
            Key::Motion(ed::Motion::Left) => selection.left(&body),
            Key::Motion(ed::Motion::Right) => selection.right(&body),
            Key::Motion(ed::Motion::Up) => selection.up(&body),
            Key::Motion(ed::Motion::Down) => selection.down(&body),
            Key::Motion(ed::Motion::LineStart) => selection.line_start(&body),
            Key::Motion(ed::Motion::LineEnd) => selection.line_end(&body),
            Key::Motion(ed::Motion::WordNext) => selection.word_right(&body),
            Key::Motion(ed::Motion::WordPrevious) => selection.word_left(&body),
            Key::Motion(ed::Motion::First) => selection.document_start(&body),
            Key::Motion(ed::Motion::Last) => selection.document_end(&body),
            _ => {}
        }
        self.selection = Some(selection);
        KeyOut::Local
    }

    fn search_history(&mut self, restart: bool) {
        let search = self.history_search.as_mut().expect("search is active");
        if restart { search.before = self.editor.history().len(); }
        if let Some(at) = (0..search.before).rev().find(|&at| self.editor.history()[at].contains(&search.query)) {
            let found = self.editor.history()[at].clone();
            search.before = at;
            self.editor.set_text(found);
            self.notice = Some(format!("reverse search: {}", search.query));
        } else {
            self.notice = Some(format!("reverse search: {} — no match", search.query));
        }
    }

    pub fn key(&mut self, key: Key) -> KeyOut {
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
                Key::Submit => { self.history_search = None; self.notice = None; }
                Key::Quit => return KeyOut::Quit,
                _ => { self.history_search = None; self.notice = None; return self.key(key); }
            }
            return KeyOut::Local;
        }

        // A picker in front of the editor takes everything except the way out.
        if self.picker.is_some() {
            return self.picker_key(if key == Key::InterruptSubmit { Key::Submit } else { key });
        }
        match key {
            Key::HistorySearch => {
                self.history_search = Some(HistorySearch { draft: self.editor.text().into(), query: String::new(), before: self.editor.history().len() });
                self.search_history(false);
                KeyOut::Local
            }
            Key::Newline => { self.editor.insert("\n"); KeyOut::Local }
            Key::InterruptSubmit => match self.submit() {
                KeyOut::Intent(Intent::Prompt { text, attachments }) => KeyOut::Intent(Intent::Interrupt { text, attachments }),
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
                self.save();
                KeyOut::Intent(Intent::Cancel { target: None })
            }
            Key::Submit => self.submit(),
            Key::Char('/') if self.editor.is_empty() => self.open_command_picker(),
            // `:` lists what *this program* can do, which is a different question
            // from what the session can do and is answered without asking it.
            Key::Char(':') if self.editor.is_empty() => self.open_action_palette(),
            Key::Fill(text) => {
                // A picker's answer, arriving as the line it completes.
                self.editor.set_text(text);
                self.submit()
            }
            Key::Char(character) if self.editor.mode() == ed::Mode::Normal => self.normal_char(character),
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
                    if operator == 'y' { return KeyOut::Copy(text); }
                } else if motion == ed::Motion::Up && self.editor.on_first_line() {
                    self.editor.history_step(true);
                } else if motion == ed::Motion::Down && self.editor.on_last_line() {
                    self.editor.history_step(false);
                } else { self.editor.move_cursor(motion); }
                KeyOut::Local
            }
            Key::ScrollPage(delta) => {
                self.scroll_by(delta);
                KeyOut::Local
            }
            Key::Action(action) => self.action(action),
        }
    }

    fn normal_char(&mut self, character: char) -> KeyOut {
        let motion = match character {
            'h' => Some(ed::Motion::Left), 'l' => Some(ed::Motion::Right),
            'w' => Some(ed::Motion::WordNext), 'b' => Some(ed::Motion::WordPrevious),
            '0' => Some(ed::Motion::LineStart), '$' => Some(ed::Motion::LineEnd),
            'j' => Some(ed::Motion::Down), 'k' => Some(ed::Motion::Up),
            _ => None,
        };
        if let Some(operator) = self.operator.take() {
            if character == operator || motion.is_some() {
                let text = self.editor.operate(operator, motion);
                if operator == 'y' { return KeyOut::Copy(text); }
            }
        } else if let Some(motion) = motion {
            return self.key(Key::Motion(motion));
        } else {
            match character {
                'd' | 'c' | 'y' => self.operator = Some(character),
                'i' => { self.editor.set_mode(ed::Mode::Insert); }
                'a' => { self.editor.move_cursor(ed::Motion::Right); self.editor.set_mode(ed::Mode::Insert); }
                'o' | 'O' => self.editor.open_line(character == 'O'),
                'x' => { self.editor.delete(); }
                'u' => { self.editor.undo(); }
                _ => {}
            }
        }
        KeyOut::Local
    }

    /// Enter: submit what is there, or open the picker a declaration asks for.
    fn submit(&mut self) -> KeyOut {
        let text = self.editor.text().to_string();
        if let Some(parsed) = save::parse(&text) {
            return match parsed {
                Ok(request) => { self.editor.submit(); self.save(); KeyOut::Save(request) }
                Err(error) => { self.notice = Some(error); KeyOut::Local }
            };
        }
        match line::parse(&text, &self.commands) {
            line::Parsed::Empty => KeyOut::Local,
            line::Parsed::Needs { command, argument, source, .. } => {
                // A command that cannot run yet is not sent. The declaration said
                // where its value comes from, so the client opens its own picker.
                match source {
                    Some(source) => {
                        self.open_argument_picker(&command, &argument, &source);
                        KeyOut::Local
                    }
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
                    self.save();
                    KeyOut::Intent(intent)
                }
                None => KeyOut::Local,
            },
        }
    }

    /// Tab: complete the argument the cursor is in, if the declaration says one can
    /// be completed.
    fn complete_argument(&mut self) -> KeyOut {
        let text = self.editor.text().trim().to_string();
        let Some(rest) = text.strip_prefix('/') else {
            return KeyOut::Local;
        };
        let mut words = rest.split_whitespace();
        let name = words.next().unwrap_or_default().to_string();
        let Some(command) = self.commands.iter().find(|command| command.id == name).cloned() else {
            return KeyOut::Local;
        };
        let position = words.count();
        let Some(argument) = command.args.get(position) else {
            return KeyOut::Local;
        };
        match argument.source.clone() {
            Some(source) => {
                self.open_argument_picker(&name, &argument.name, &source);
                if self.is_resident(&source) && self.picker.as_ref().is_some_and(Picker::is_empty) {
                    // A resident source the client holds nothing for yet: the first
                    // prefix is not a keystroke of latency, it is the one ask that
                    // makes every later keystroke local.
                    return KeyOut::Complete { source, prefix: String::new() };
                }
                KeyOut::Local
            }
            None => KeyOut::Local,
        }
    }

    fn open_command_picker(&mut self) -> KeyOut {
        self.pending_command = None;
        self.editor.set_text("/");
        // `/` on an empty line is a request for the session's commands, and the
        // promise the declaration made is that they can be listed without asking.
        let mut picker = if self.sources.iter().any(|source| source.id == "commands") {
            Picker::over("commands", "Commands", Accept::Run)
        } else {
            Picker::new("Commands", Accept::Run)
        };
        picker.set_items(self.command_candidates(), false);
        self.picker = Some(picker.with_frecency(self.prefs.frecency()));
        KeyOut::Local
    }

    /// Open the client's own actions.
    fn open_action_palette(&mut self) -> KeyOut {
        self.editor.set_text(":");
        let mut picker = Picker::new("Actions", Accept::Run).with_frecency(self.prefs.frecency());
        picker.set_items(
            Action::ALL
                .iter()
                .map(|action| {
                    let keys = action.keys();
                    Choice {
                        value: action.id().to_string(),
                        label: action.label().to_string(),
                        detail: Some(if keys.is_empty() { String::new() } else { keys.to_string() }),
                    }
                })
                .collect(),
            false,
        );
        self.picker = Some(picker);
        KeyOut::Local
    }

    fn picker_key(&mut self, key: Key) -> KeyOut {
        // Completing the command word moves completion to its argument while
        // keeping the composer as the only editable text.
        if key == Key::Char(' ') && self.pending_command.is_none() && self.editor.text().starts_with('/') {
            let name = self.editor.text().trim_start_matches('/');
            if self.commands.iter().any(|command| command.id == name) {
                self.editor.type_char(' ');
                self.picker = None;
                return match line::parse(self.editor.text(), &self.commands) {
                    line::Parsed::Needs { source: Some(_), .. } => self.submit(),
                    _ => KeyOut::Local,
                };
            }
        }
        let Some(picker) = self.picker.as_mut() else {
            return KeyOut::Local;
        };
        let effect = match key {
            Key::Escape => picker.cancel(),
            Key::Submit => picker.accept(),
            Key::Tab | Key::Motion(ed::Motion::Down) => {
                picker.move_selection(1);
                PickerEffect::None
            }
            Key::Motion(ed::Motion::Up) => {
                picker.move_selection(-1);
                PickerEffect::None
            }
            Key::Char(character) => {
                self.editor.type_char(character);
                picker.set_query(Self::picker_query(self.editor.text(), &picker.accept))
            }
            Key::Backspace | Key::Delete | Key::Eof => {
                if key == Key::Backspace { self.editor.backspace(); } else { self.editor.delete(); }
                if self.editor.is_empty() { picker.cancel() }
                else { picker.set_query(Self::picker_query(self.editor.text(), &picker.accept)) }
            }
            Key::Motion(motion) => { self.editor.move_cursor(motion); PickerEffect::None }
            Key::Quit => return KeyOut::Quit,
            Key::Interrupt => picker.cancel(),
            _ => PickerEffect::None,
        };
        self.picker_effect(effect)
    }

    fn picker_query<'a>(text: &'a str, accept: &Accept) -> &'a str {
        match accept {
            Accept::Argument { .. } => text.split_once(char::is_whitespace).map_or("", |(_, rest)| rest),
            _ => text.strip_prefix('/').or_else(|| text.strip_prefix(':')).unwrap_or(text),
        }
    }

    pub fn paste(&mut self, text: &str) -> KeyOut {
        self.editor.insert(text);
        let effect = self.picker.as_mut().map(|picker|
            picker.set_query(Self::picker_query(self.editor.text(), &picker.accept)));
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
            PickerEffect::Ask { source, prefix } => KeyOut::Complete { source, prefix },
            PickerEffect::Accepted(accepted) => {
                // An accepted candidate is what frecency is *for*: the next list is ranked by
                // it, and a count a restart forgets is a list ranked by nothing. Taken from
                // the picker before it is dropped, because that is where the counts are.
                if let Some(picker) = &self.picker {
                    let frecency = picker.frecency().clone();
                    self.prefs.remember_frecency(&frecency);
                }
                self.picker = None;
                self.prefs.remembered(&accepted.value);
                self.pending_command = None;
                self.save();
                // An accepted argument completes the line rather than sending it, so
                // somebody can add the next argument or edit what they got.
                if matches!(accepted.accept, Accept::Argument { .. }) {
                    let text = Picker::fill_text(&accepted);
                    self.editor.set_text(text);
                    self.notice = Some(format!("{} → {}", accepted.label, accepted.value));
                    // A command whose arguments are all filled is complete, so it is
                    // sent: somebody who picked a model has said what they meant. One
                    // that is not complete leaves the line for the next argument.
                    match line::parse(self.editor.text(), &self.commands) {
                        line::Parsed::Command { .. } => self.submit(),
                        _ => KeyOut::Local,
                    }
                } else if let Some(action) = Action::find(&accepted.value) {
                    // A candidate without a slash is one of this program's own actions:
                    // there is nobody to ask, so it is done here.
                    self.notice = None;
                    self.action(action)
                } else {
                    // A command candidate is not sent here: it is put into the line and
                    // submitted through the same path as a typed one, so a command with
                    // an argument opens that argument's picker instead of being refused.
                    self.editor.set_text(Picker::fill_text(&accepted));
                    self.submit()
                }
            }
        }
    }

    fn action(&mut self, action: Action) -> KeyOut {
        match action {
            Action::ToggleDetail => {
                // Presentation state, and only presentation state: which nodes are
                // showing their long form is the client's to remember — and now the
                // client remembers it between runs, which is what this file is for.
                if self.prefs.any_open() {
                    self.prefs.close_all();
                } else {
                    self.prefs.open_all();
                }
                self.save();
                KeyOut::Local
            }
            Action::ScrollTop => {
                self.follow = false;
                self.scroll = 0;
                KeyOut::Local
            }
            Action::ScrollBottom => {
                self.follow = true;
                KeyOut::Local
            }
            Action::ThemeDark => {
                self.theme = Theme::dark();
                self.prefs.theme = "dark".into();
                self.save();
                KeyOut::Local
            }
            Action::ThemePlain => {
                self.theme = Theme::plain();
                self.prefs.theme = "plain".into();
                self.save();
                KeyOut::Local
            }
            Action::OpenCommands => self.open_command_picker(),
            Action::Quit => KeyOut::Quit,
        }
    }

    fn scroll_by(&mut self, delta: isize) {
        let next = self.scroll as isize + delta;
        self.scroll = next.max(0) as usize;
        self.follow = false;
    }

    /// Apply the client's own decisions to the tree the session sent.
    ///
    /// A collapsible the reader opened keeps its children and drops its summary; one
    /// they closed does the opposite. Expansion belongs to the client;
    /// this is why a theme change or a re-render never loses somebody's place.
    pub fn resolve(&self, node: &Node) -> Node {
        #[cfg(test)]
        RESOLVE_VISITS.with(|visits| visits.set(visits.get() + 1));
        let mut node = node.clone();
        node.children = node.children.iter().map(|child| self.resolve(child)).collect();
        // What is being typed into a panel is drawn in the panel's field. The session sent an
        // empty one and knows nothing about the draft, which is the whole of why a secret can
        // be typed into a terminal and still never reach a log.
        if let (Kind::Fields { fields }, Some(state)) = (&mut node.kind, &self.panel)
            && let Some(field) = fields.iter_mut().find(|field| field.id == state.field)
        {
            field.value = state.text.clone();
        }
        if let Kind::Collapsible { summary } = &node.kind {
            let open = self.prefs.is_open(&node.id);
            if open {
                node.kind = Kind::Section;
            } else {
                node.kind = Kind::Text { spans: summary.clone() };
                node.children.clear();
            }
        }
        node
    }
}

/// The panel in a view, if the session has one open.
///
/// By role rather than by id: the id is the session's name for the panel — `login`,
/// `authorize` — and that is what an action has to name, so the role is what says what a node
/// *is*.
fn panel_of(view: &Node) -> Option<&Node> {
    if view.role == "panel" {
        return Some(view);
    }
    view.children.iter().find_map(panel_of)
}

/// The field a panel wants typed into, if it wants one.
fn panel_field(panel: &Node) -> Option<&Field> {
    panel.children.iter().find_map(|child| match &child.kind {
        Kind::Fields { fields } if child.actions.iter().any(|action| action.on == ActionOn::Submit) => fields.iter().find(|field| !field.read_only),
        _ => None,
    })
}

/// A key, in the vocabulary the client cares about.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    HistorySearch,
    Newline,
    InterruptSubmit,
    Char(char),
    Backspace,
    Delete,
    Eof,
    Submit,
    Tab,
    Escape,
    Interrupt,
    Quit,
    /// Text from a picker, to be submitted if it is ready.
    Fill(String),
    Motion(ed::Motion),
    ScrollPage(isize),
    Action(Action),
}

/// Draw a screen: the transcript, the picker when one is open, and the input line.
pub fn draw(screen: &Screen, view: &Node) -> Vec<Line> {
    let resolved = screen.resolve(view);
    let rendered = misa_render::render(&resolved, &screen.theme, screen.width as usize);
    // The body is what a selection moves over, so it is kept whole and the window is
    // taken from it afterwards: a row's index must not depend on where the viewport is.
    let body = select::Body::of(&rendered);
    let attachment_count = save::attachments(view).len();
    let chrome = (if screen.picker.is_some() { 9 } else { 3 }) + usize::from(attachment_count > 0);
    let room = (screen.height as usize).saturating_sub(chrome);

    // Scrolling is presentation, so the client does it and nobody is told. Following
    // is the default and scrolling away stops it, which is what lets somebody read
    // while a model is still writing.
    let start = if screen.follow {
        rendered.len().saturating_sub(room)
    } else {
        screen.scroll.min(rendered.len().saturating_sub(1))
    };
    let mut lines: Vec<Line> = rendered.into_iter().skip(start).take(room).collect();

    // A selection is painted from byte offsets this client holds itself.
    if let Some(selection) = &screen.selection {
        for (offset, line) in lines.iter_mut().enumerate() {
            if let Some((from, to)) = selection.on_row(&body, start + offset) {
                select_highlight(line, from, to, &screen.theme);
            }
        }
    }

    self::chrome(screen, attachment_count, &mut lines);
    lines
}

fn chrome(screen: &Screen, attachment_count: usize, lines: &mut Vec<Line>) {
    if let Some(picker) = &screen.picker {
        lines.extend(picker_lines(screen, picker));
    }
    if attachment_count > 0 {
        lines.push(Line { node: None, indent: 0, spans: vec![(screen.theme.role("notice"), format!("{attachment_count} attachments · /save [1–{attachment_count}] <local path> · latest by default"))] });
    }
    if let Some(notice) = &screen.notice {
        lines.push(Line {
            indent: 0,
            spans: vec![(screen.theme.role("notice"), notice.clone())],
            node: None,
        });
    }
    lines.push(Line { indent: 0, spans: Vec::new(), node: None });
    lines.push(input_line(screen));
}

/// The picker, as lines. A frontend with a window would draw this as a panel.
fn picker_lines(screen: &Screen, picker: &Picker) -> Vec<Line> {
    let mut lines = Vec::new();
    let theme = &screen.theme;
    let matches = picker.matches();
    let visible = matches.len().min(6);
    let first = picker
        .selected_index()
        .saturating_sub(visible.saturating_sub(1))
        .min(matches.len().saturating_sub(1));
    let detail = format!(
        "{}{}{}",
        picker.title,
        String::new(),
        if picker.is_truncated() { " · partial" } else { "" }
    );
    lines.push(Line { indent: 0, spans: vec![(theme.role("palette.title"), detail)], node: None });
    for (offset, candidate) in matches.iter().skip(first).take(visible).enumerate() {
        let index = first + offset;
        let selected = index == picker.selected_index();
        let role = if selected { "palette.item.selected" } else { "palette.item" };
        let mut text = format!("  {} ", candidate.label);
        if let Some(detail) = &candidate.detail
            && !detail.is_empty()
        {
            text.push_str(&format!("· {detail}"));
        }
        lines.push(Line {
            indent: 0,
            spans: vec![(theme.role(role), misa_render::clip(&text, screen.width as usize))],
            node: None,
        });
    }
    if matches.is_empty() {
        lines.push(Line {
            indent: 0,
            spans: vec![(theme.role("palette.hint"), "  no matches".to_string())],
            node: None,
        });
    }
    lines
}

/// The input line, with the mode the previous system drew in its prompt.
fn input_line(screen: &Screen) -> Line {
    let theme = &screen.theme;
    let (mode, role) = match screen.editor.mode() {
        ed::Mode::Insert => ("┌", "mode.insert"),
        ed::Mode::Normal => ("◆", "mode.normal"),
    };
    let (before, after) = screen.editor.split_at_cursor();
    let mut spans = vec![(theme.role(role), format!("{mode} "))];
    if before.is_empty() && after.is_empty() {
        spans.push((theme.role("composer"), "".to_string()));
    } else {
        spans.push((theme.role("composer"), before.to_string()));
    }
    spans.push((theme.role("composer"), after.to_string()));
    Line { indent: 0, spans, node: None }
}

/// Re-style the byte range a selection covers on one line.
///
/// The offsets are into [`Line::text`], which starts with the indent, so the indent comes
/// off first. A range that lands mid-character is cut back rather than slicing a `str`
/// where nobody can see.
fn select_highlight(line: &mut Line, from: usize, to: usize, theme: &Theme) {
    let indent = line.indent as usize;
    let (from, to) = (from.saturating_sub(indent), to.saturating_sub(indent));
    if to <= from {
        return;
    }
    let selected = theme.role("selection");
    let mut spans: Vec<(misa_render::Style, String)> = Vec::new();
    let mut cursor = 0usize;
    for (style, text) in line.spans.drain(..) {
        let start = cursor;
        let end = cursor + text.len();
        let overlap_start = start.max(from.min(end));
        let overlap_end = end.min(to.max(start));
        if overlap_start >= overlap_end {
            spans.push((style, text));
        } else {
            if overlap_start > start {
                spans.push((style, text[..overlap_start - start].to_string()));
            }
            spans.push((selected, text[overlap_start - start..overlap_end - start].to_string()));
            if overlap_end < end {
                spans.push((style, text[overlap_end - start..].to_string()));
            }
        }
        cursor = end;
    }
    line.spans = spans;
}

/// What a frontend needs from a transport, so this binary can be tested and the
/// transport can be swapped.
#[async_trait::async_trait]
pub trait Session: Send {
    /// Scoped clients correlate completion to the accepted operation.
    fn turn_settled(&self) -> Option<bool> { None }
    /// The next view, if one changed.
    async fn next(&mut self) -> Result<Option<Node>, String>;
    /// Incremental consumers retain presentation owners between these updates.
    async fn next_presentation(&mut self) -> Result<Option<Presentation>, String> {
        Ok(self.next().await?.map(Presentation::Snapshot))
    }
    async fn request(&mut self, request: SessionRequest) -> Option<SessionReply> {
        Some(match request {
            SessionRequest::RefreshRequests => SessionReply::Notice("This session has no input request catalog".into()),
            SessionRequest::DaemonInvoke { .. } | SessionRequest::Invoke { .. } => SessionReply::Notice("This session does not support installed invocations".into()),
            SessionRequest::Intent(intent) => {
                let draft = match &intent { Intent::Prompt { text, attachments } | Intent::Interrupt { text, attachments } => Some((text.clone(), attachments.clone())), _ => None };
                SessionReply::Sent { draft, result: self.send(intent).await }
            },
            SessionRequest::Upload { generation, bytes, media } => SessionReply::Uploaded { generation, result: self.upload(bytes, &media).await },
            SessionRequest::Complete { source, prefix } => {
                let result = self.complete(&source, &prefix).await;
                SessionReply::Complete { source, prefix, result }
            }
            SessionRequest::Save { node, destination } => SessionReply::Notice(match self.save_attachment(&node, &destination).await {
                Ok(()) => format!("Saved {destination}"), Err(error) => error,
            }),
        })
    }
    async fn send(&mut self, intent: Intent) -> Result<(), String>;
    async fn upload(&mut self, _bytes: Vec<u8>, _media: &str) -> Result<misa_proto::view::BlobRef, String> {
        Err("This client has no blob connection".into())
    }
    async fn save_attachment(&mut self, _node: &str, _destination: &str) -> Result<(), String> {
        Err("This client has no blob connection".into())
    }
    /// Candidates a session holds, for a source this client asked about.
    async fn complete(&mut self, source: &str, prefix: &str) -> Result<(Vec<Choice>, bool), String>;
    fn catalog(&self) -> Catalog { Catalog::default() }
    fn location(&self) -> String { "Session".into() }
    fn selected(&self) -> Option<misa_proto::directory::Entry> { None }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog { pub commands: Vec<Command>, pub sources: Vec<Source> }

pub enum Presentation {
    Attention { id: String, generation: i64 },
    Documents(Vec<(String,misa_client::document::Update)>),
    Activate(String),
    Forget(String),
    Contribution { id: String, update: misa_client::document::Update },
    TurnOutput(Node),
    Document(misa_client::document::Update),
    Declaration { catalog: Catalog, location: String },
    Candidates { source: String, items: Vec<Choice>, truncated: bool },
    Snapshot(Node),
    Reply(SessionReply),
}
pub enum SessionRequest {
    DaemonInvoke { daemon:String,scope:misa_proto::observation::Scope,command:String,input:misa_value::Value },
    RefreshRequests,
    Invoke { command: String, input: misa_value::Value },
    Intent(Intent),
    Upload { generation: u64, bytes: Vec<u8>, media: String },
    Complete { source: String, prefix: String },
    Save { node: String, destination: String },
}
pub enum SessionReply {
    DaemonForm { daemon:String,scope:misa_proto::observation::Scope,form:misa_client::form::Form,drafts:std::collections::BTreeMap<String,String> },
    Form(misa_client::form::Form),
    Request { id: String, generation: i64, model: Option<misa_client::request::Model> },
    Report(Node),
    Uploaded { generation: u64, result: Result<misa_proto::view::BlobRef, String> },
    Sent { draft: Option<(String, Vec<misa_proto::view::BlobRef>)>, result: Result<(), String> },
    Complete { source: String, prefix: String, result: Result<(Vec<Choice>, bool), String> },
    Notice(String),
}

/// The interactive loop.
pub async fn run(session: &mut dyn Session) -> Result<(), String> {
    event_loop::run(session).await
}

fn translate(code: crossterm::event::KeyCode, modifiers: crossterm::event::KeyModifiers) -> Option<Key> {
    use crossterm::event::KeyCode;
    let control = modifiers.contains(crossterm::event::KeyModifiers::CONTROL);
    Some(match code {
        KeyCode::Char('r') if control => Key::HistorySearch,
        KeyCode::Char('q') if control => Key::Quit,
        KeyCode::Char('c') if control => Key::Interrupt,
        KeyCode::Char('t') if control => Key::Action(Action::ToggleDetail),
        KeyCode::Char('d') if control => Key::Eof,
        KeyCode::Char('u') if control => Key::ScrollPage(-10),
        KeyCode::Char(character) => Key::Char(character),
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Enter if modifiers.contains(crossterm::event::KeyModifiers::ALT) => Key::InterruptSubmit,
        KeyCode::Enter if modifiers.contains(crossterm::event::KeyModifiers::SHIFT) => Key::Newline,
        KeyCode::Enter => Key::Submit,
        KeyCode::Tab => Key::Tab,
        KeyCode::Esc => Key::Escape,
        KeyCode::Left => Key::Motion(ed::Motion::Left),
        KeyCode::Right => Key::Motion(ed::Motion::Right),
        KeyCode::Up => Key::Motion(ed::Motion::Up),
        KeyCode::Down => Key::Motion(ed::Motion::Down),
        KeyCode::Home => Key::Motion(ed::Motion::LineStart),
        KeyCode::End => Key::Motion(ed::Motion::LineEnd),
        KeyCode::PageUp => Key::ScrollPage(-20),
        KeyCode::PageDown => Key::ScrollPage(20),
        _ => return None,
    })
}

pub fn sgr(style: &misa_render::Style) -> String {
    use misa_render::Color;
    let mut codes: Vec<String> = Vec::new();
    match style.fg {
        Color::Default => {}
        Color::Indexed(index) => codes.push(format!("38;5;{index}")),
        Color::Rgb(r, g, b) => codes.push(format!("38;2;{r};{g};{b}")),
    }
    match style.bg {
        Color::Default => {}
        Color::Indexed(index) => codes.push(format!("48;5;{index}")),
        Color::Rgb(r, g, b) => codes.push(format!("48;2;{r};{g};{b}")),
    }
    if style.bold {
        codes.push("1".into());
    }
    if style.dim {
        codes.push("2".into());
    }
    if style.italic {
        codes.push("3".into());
    }
    if style.underline {
        codes.push("4".into());
    }
    if style.strikethrough {
        codes.push("9".into());
    }
    if codes.is_empty() {
        String::new()
    } else {
        format!("\u{1b}[{}m", codes.join(";"))
    }
}

/// What the composer's field is called.
pub const PROMPT_FIELD: &str = "prompt";

/// The composer's field, if the view has one.
pub fn prompt_field(view: &Node) -> Option<&Field> {
    if let Kind::Fields { fields } = &view.kind
        && let Some(field) = fields.iter().find(|field| field.id == PROMPT_FIELD)
    {
        return Some(field);
    }
    view.children.iter().find_map(prompt_field)
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::preparation::Arg;

    fn declaration() -> Catalog {
        Catalog {
            commands: vec![
                Command::new("clear", "Clear", "forget this branch"),
                Command::new("model", "Model", "choose a model")
                    .arg(Arg::new("model", "Model").required().from("models")),
                Command::new("effort", "Effort", "choose how hard to think")
                    .arg(Arg::new("level", "Level").required().from("effort")),
            ],
            sources: vec![Source::resident("models", "Models"), Source::resident("effort", "Effort")],
        }
    }

    fn screen() -> Screen {
        let mut screen = Screen::new(80, 24);
        screen.declare(&declaration());
        screen
    }

    fn type_text(screen: &mut Screen, text: &str) {
        for character in text.chars() {
            screen.key(Key::Char(character));
        }
    }

    fn view() -> Node {
        Node::section("session")
            .id("session")
            .child(Node::text("message.user", [misa_proto::view::Span::plain("hello")]).id("msg.1"))
            .child(
                Node::new("tool.call", Kind::Collapsible {
                    summary: vec![misa_proto::view::Span::plain("echo (collapsed)")],

                })
                .id("call.1")
                .child(Node::text("tool.result", [misa_proto::view::Span::plain("the result")])),
            )
    }

    fn text_of(screen: &Screen, view: &Node) -> String {
        misa_render::to_plain(&draw(screen, view))
    }

    /// A view with a panel in it, the shape the session opens for `/login`.
    fn panel_view(with_field: bool) -> Node {
        let mut session = view();
        let mut panel = Node::section("panel").id("login").label("Credential for `anthropic`");
        panel.children.push(Node::text("panel.text", [misa_proto::view::Span::plain("the daemon stores it")]));
        if with_field {
            panel.children.push(
                Node::new(
                    "panel.input",
                    Kind::Fields {
                        fields: vec![Field {
                            id: "value".into(),
                            label: "Token".into(),
                            value: String::new(),
                            hint: None,
                            read_only: false,
                            secret: true,
                            kind: misa_proto::view::FieldKind::Inline,
                        }],
                    },
                )
                .id("panel.input")
                .action(misa_proto::view::Action {
                    id: "panel.submit".into(),
                    on: ActionOn::Submit,
                    label: Some("Store".into()),
                    args: misa_value::Value::Null,
                }),
            );
        }
        panel.actions.push(misa_proto::view::Action {
            id: "panel.close".into(),
            on: ActionOn::Click,
            label: Some("Cancel".into()),
            args: misa_value::Value::Null,
        });
        session.children.push(panel);
        session
    }

    #[test]
    fn a_panel_takes_the_keyboard_and_a_secret_does_not_stay_on_the_screen() {
        // The terminal could offer the login panel and nothing else: there was no way to type
        // into it, so `/login <provider>` was a command a person could send and not finish.
        let mut screen = screen();
        let view = panel_view(true);
        for character in "sk-a-secret".chars() {
            assert_eq!(screen.panel_key(&view, &Key::Char(character)), Some(KeyOut::Local));
        }
        // The composer never saw a key, and the secret is masked on screen.
        assert_eq!(screen.editor.text(), "", "the panel's keys went into the composer");
        assert!(!text_of(&screen, &view).contains("sk-a-secret"), "{}", text_of(&screen, &view));

        match screen.panel_key(&view, &Key::Submit) {
            Some(KeyOut::Intent(Intent::Action { action, fields, .. })) => {
                assert_eq!(action, "panel.submit");
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].id, "value");
                assert_eq!(fields[0].value, "sk-a-secret");
            }
            other => panic!("expected a submit, got {other:?}"),
        }
        // A secret that stays in a field after it has gone is a secret on a screen.
        assert!(!text_of(&screen, &view).contains("sk-a-secret"), "{}", text_of(&screen, &view));
    }

    #[test]
    fn a_panel_is_left_by_the_action_the_session_offered_and_by_nothing_this_client_invents() {
        let asking = panel_view(true);
        let mut typing = screen();
        match typing.panel_key(&asking, &Key::Escape) {
            Some(KeyOut::Intent(Intent::Action { node, action, .. })) => {
                assert_eq!(node, "login");
                assert_eq!(action, "panel.close");
            }
            other => panic!("expected a dismissal, got {other:?}"),
        }

        // A view with no panel in it is none of the panel's business, and the ways out of the
        // program are never the panel's either: a person may always stop.
        let mut composer = screen();
        assert_eq!(composer.panel_key(&view(), &Key::Char('a')), None);
        assert_eq!(composer.panel_key(&asking, &Key::Quit), None);
        assert_eq!(composer.panel_key(&asking, &Key::Interrupt), None);

        // A report has nothing to type into, and esc still means "the session's way out".
        let report = panel_view(false);
        let mut reading = screen();
        assert_eq!(reading.panel_key(&report, &Key::Char('x')), Some(KeyOut::Local));
        assert!(matches!(reading.panel_key(&report, &Key::Escape), Some(KeyOut::Intent(_))));
    }

    #[test]
    fn ordinary_text_is_submitted_as_a_prompt() {
        let mut screen = screen();
        type_text(&mut screen, "hello");
        match screen.key(Key::Submit) {
            KeyOut::Intent(Intent::Prompt { text, .. }) => assert_eq!(text, "hello"),
            other => panic!("expected a prompt, got {other:?}"),
        }
        assert!(screen.editor.is_empty(), "the line was not cleared after sending");
    }

    #[test]
    fn a_slash_opens_a_picker_built_from_the_declaration() {
        let mut screen = screen();
        assert_eq!(screen.key(Key::Char('/')), KeyOut::Local);
        let picker = screen.picker.as_ref().expect("a picker");
        assert_eq!(picker.items().len(), 4);
        assert!(picker.items().iter().any(|item| item.value == "/save"));
        assert!(picker.items().iter().any(|item| item.value == "/model"));
    }

    #[test]
    fn completion_edits_the_composer_and_resident_candidates_survive_closed_pickers() {
        let mut screen = screen();
        screen.candidates("models", vec![Choice { value: "chosen".into(), label: "Chosen".into(), detail: None }], false);
        type_text(&mut screen, "/model ");
        assert_eq!(screen.editor.text(), "/model ");
        assert_eq!(screen.picker.as_ref().unwrap().source.as_deref(), Some("models"));
        assert_eq!(screen.picker.as_ref().unwrap().items()[0].value, "chosen");
        screen.paste("cho");
        assert_eq!(screen.editor.text(), "/model cho");
        assert_eq!(screen.picker.as_ref().unwrap().query, "cho");
        screen.key(Key::Backspace);
        assert_eq!(screen.editor.text(), "/model ch");
        assert_eq!(screen.picker.as_ref().unwrap().query, "ch");
        assert_eq!(screen.key(Key::Quit), KeyOut::Quit);
    }

    #[test]
    fn control_d_is_eof_only_on_an_empty_composer() {
        let mut screen = screen();
        assert_eq!(screen.key(Key::Eof), KeyOut::Quit);
        screen.editor.set_text("éx");
        screen.key(Key::Motion(ed::Motion::LineStart));
        assert_eq!(screen.key(Key::Eof), KeyOut::Local);
        assert_eq!(screen.editor.text(), "x");
        screen.key(Key::Motion(ed::Motion::LineEnd));
        assert_eq!(screen.key(Key::Eof), KeyOut::Local);
        assert_eq!(screen.editor.text(), "x");
    }

    #[test]
    fn typing_in_a_command_picker_filters_without_asking_anything() {
        let mut screen = screen();
        screen.key(Key::Char('/'));
        assert_eq!(screen.key(Key::Char('m')), KeyOut::Local, "a command picker asked a session");
        assert_eq!(screen.key(Key::Char('o')), KeyOut::Local);
        assert_eq!(screen.picker.as_ref().expect("a picker").query, "mo");
    }

    #[test]
    fn accepting_a_command_runs_it() {
        let mut screen = screen();
        screen.key(Key::Char('/'));
        assert_eq!(screen.key(Key::Submit), KeyOut::Intent(Intent::Command {
            name: "clear".into(),
            args: misa_value::Value::Map(std::sync::Arc::new(Default::default())),
        }));
        assert!(screen.picker.is_none());
    }

    #[test]
    fn a_command_that_needs_a_value_opens_the_picker_its_declaration_named() {
        let mut screen = screen();
        screen.editor.set_text("/model");
        assert_eq!(screen.key(Key::Submit), KeyOut::Local, "an unready command was sent");
        let picker = screen.picker.as_ref().expect("a picker");
        assert_eq!(picker.source.as_deref(), Some("models"));
        assert_eq!(picker.accept, Accept::Argument { command: "model".into(), argument: "model".into() });
    }

    #[test]
    fn a_resident_source_with_nothing_held_is_asked_for_once() {
        let mut screen = screen();
        screen.editor.set_text("/model");
        // The first ask is deliberate; after the items arrive, nothing more is sent.
        assert_eq!(screen.key(Key::Tab), KeyOut::Complete { source: "models".into(), prefix: String::new() });
        screen.candidates("models", vec![Choice {
            value: "scripted-1".into(),
            label: "Scripted".into(),
            detail: None,
        }], false);
        screen.key(Key::Char('s'));
        assert_eq!(screen.key(Key::Char('c')), KeyOut::Local, "a held source was asked again");
    }

    #[test]
    fn accepting_a_value_completes_the_command_and_sends_it() {
        let mut screen = screen();
        screen.editor.set_text("/model");
        assert_eq!(screen.key(Key::Submit), KeyOut::Local, "the picker should be open");
        screen.candidates("models", vec![Choice {
            value: "scripted-1".into(),
            label: "Scripted".into(),
            detail: None,
        }], false);
        // Picking the value is the last thing to say, so the command goes.
        assert_eq!(
            screen.key(Key::Submit),
            KeyOut::Intent(Intent::Command {
                name: "model".into(),
                args: misa_value::Value::map([("model", misa_value::Value::str("scripted-1"))]),
            })
        );
        assert!(screen.editor.is_empty(), "the line was not cleared after sending");
    }

    #[test]
    fn escape_closes_a_picker_and_clears_the_half_typed_command() {
        let mut screen = screen();
        screen.editor.set_text("/model");
        screen.key(Key::Submit);
        assert!(screen.picker.is_some());
        assert_eq!(screen.key(Key::Escape), KeyOut::Local);
        assert!(screen.picker.is_none());
        assert!(screen.editor.is_empty());
    }

    #[test]
    fn a_command_that_does_not_exist_is_answered_here_rather_than_sent() {
        let mut screen = screen();
        screen.editor.set_text("/nonsense");
        assert_eq!(screen.key(Key::Submit), KeyOut::Local);
        assert!(screen.notice.as_deref().expect("a notice").contains("nonsense"));
        assert!(screen.editor.text().contains("nonsense"), "the line was lost");
    }

    #[test]
    fn an_interrupt_keeps_the_draft_and_asks_the_session_to_stop() {
        let mut screen = screen();
        type_text(&mut screen, "half written");
        assert_eq!(screen.key(Key::Interrupt), KeyOut::Intent(Intent::Cancel { target: None }));
        assert_eq!(screen.editor.text(), "half written");
    }

    /// A path for one test's memory, removed first so a rerun does not read a stale one.
    fn memory(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("misa-tui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("client.json")
    }

    #[test]
    fn a_client_that_remembers_starts_where_somebody_left_off() {
        // What "a restart forgets where somebody was" meant: the theme, the nodes they had
        // opened, and the draft were in memory and nowhere else.
        let path = memory("remember");
        let mut first = Screen::remembering(Prefs::default(), path.clone());
        assert_eq!(first.theme.name, "dark", "a client that remembers nothing opens dark");
        first.key(Key::Action(Action::ThemePlain));
        first.key(Key::Action(Action::ToggleDetail));
        type_text(&mut first, "half a question");
        first.save();

        let second = Screen::remembering(Prefs::load(&storage::File::at(path.clone())), path.clone());
        // The theme somebody chose is the one they are drawn with next time.
        assert_eq!(second.theme.name, "plain");
        assert_eq!(second.editor.text(), "half a question");
        // And the opened node is open in what it draws, not only in what it remembers.
        assert!(text_of(&second, &view()).contains("the result"), "{}", text_of(&second, &view()));
        let _ = std::fs::remove_dir_all(path.parent().expect("a parent"));
    }

    #[test]
    fn what_was_sent_is_not_resurrected_by_the_next_run() {
        let path = memory("spent");
        let mut screen = Screen::remembering(Prefs::default(), path.clone());
        type_text(&mut screen, "send me");
        assert!(matches!(screen.key(Key::Submit), KeyOut::Intent(_)));
        assert!(screen.editor.is_empty());
        // The line went out, so the memory of it goes out with it.
        let next = Screen::remembering(Prefs::load(&storage::File::at(path.clone())), path.clone());
        assert_eq!(next.editor.text(), "");
        let _ = std::fs::remove_dir_all(path.parent().expect("a parent"));
    }

    #[test]
    fn what_somebody_reaches_for_ranks_the_next_list() {
        // Frecency was a picker's own memory, which meant it was ranked by nothing the second
        // time a client started. It is the client's memory now.
        let path = memory("frecency");
        let mut first = Screen::remembering(Prefs::default(), path.clone());
        // A remembering client is a client like any other: it has the session's declarations
        // and it knows how to parse a line.
        first.declare(&declaration());
        first.editor.set_text("/model");
        first.key(Key::Submit);
        let candidates = || {
            ["scripted-1", "scripted-chatty"]
                .into_iter()
                .map(|value| Choice { value: value.into(), label: value.into(), detail: None })
                .collect::<Vec<_>>()
        };
        first.candidates("models", candidates(), false);
        // The second one, deliberately: the first is what a list would offer anyway.
        first.key(Key::Motion(ed::Motion::Down));
        assert!(matches!(first.key(Key::Submit), KeyOut::Intent(_)));

        let mut second = Screen::remembering(Prefs::load(&storage::File::at(path.clone())), path.clone());
        second.declare(&declaration());
        second.editor.set_text("/model");
        second.key(Key::Submit);
        second.candidates("models", candidates(), false);
        assert_eq!(second.picker.as_ref().expect("a picker").selected().map(|choice| choice.value.as_str()), Some("scripted-chatty"));
        let _ = std::fs::remove_dir_all(path.parent().expect("a parent"));
    }

    #[test]
    fn a_client_with_nowhere_to_write_still_runs() {
        // A screen with no path is a screen that keeps its memory in memory, which is what a
        // test has and what a client with no home directory has.
        let mut screen = Screen::new(80, 24);
        type_text(&mut screen, "a draft");
        screen.save();
        assert_eq!(screen.editor.text(), "a draft");
        assert!(screen.notice.is_none(), "nothing failed, so there is nothing to say");
    }

    #[test]
    fn a_reader_may_open_a_tool_call_and_the_session_is_not_told() {
        let mut screen = screen();
        let before = text_of(&screen, &view());
        assert!(before.contains("echo (collapsed)"), "{before}");
        assert!(!before.contains("the result"), "{before}");
        assert_eq!(screen.key(Key::Action(Action::ToggleDetail)), KeyOut::Local);
        let after = text_of(&screen, &view());
        assert!(after.contains("the result"), "{after}");
    }

    #[test]
    fn the_theme_is_the_clients_and_switching_it_touches_nothing_else() {
        let mut screen = screen();
        screen.key(Key::Action(Action::ThemePlain));
        assert_eq!(screen.theme.name, "plain");
        screen.key(Key::Action(Action::ThemeDark));
        assert_eq!(screen.theme.name, "dark");
    }

    #[test]
    fn a_colon_opens_the_clients_own_palette_and_an_action_needs_no_session() {
        let mut screen = screen();
        assert_eq!(screen.key(Key::Char(':')), KeyOut::Local);
        let picker = screen.picker.as_ref().expect("a palette");
        assert_eq!(picker.title, "Actions");
        assert!(picker.items().iter().any(|item| item.value == Action::ThemePlain.id()));
        // Choosing one acts here: no intent, and nothing to ask.
        let chosen = picker.items().iter().position(|item| item.value == Action::ThemePlain.id()).unwrap();
        let picker = screen.picker.as_mut().expect("a palette");
        for _ in 0..chosen {
            picker.move_selection(1);
        }
        assert_eq!(screen.key(Key::Submit), KeyOut::Local);
        assert_eq!(screen.theme.name, "plain");
        assert!(screen.picker.is_none());
    }

    #[test]
    fn scrolling_away_stops_following_the_tail() {
        let mut screen = screen();
        assert!(screen.follow);
        screen.key(Key::ScrollPage(-5));
        assert!(!screen.follow, "the viewport kept following after a scroll");
        screen.key(Key::Action(Action::ScrollBottom));
        assert!(screen.follow);
    }

    #[test]
    fn the_input_line_shows_the_mode_the_previous_system_drew() {
        let mut screen = screen();
        assert!(text_of(&screen, &view()).contains("┌"));
        screen.editor.set_mode(ed::Mode::Normal);
        assert!(text_of(&screen, &view()).contains("◆"));
    }

    #[test]
    fn a_picker_is_drawn_with_its_selection_and_its_partial_state() {
        let mut screen = screen();
        screen.key(Key::Char('/'));
        screen.candidates("commands", screen.command_candidates(), true);
        let candidates = screen.command_candidates();
        let picker = screen.picker.as_mut().expect("a picker");
        picker.set_items(candidates, true);
        let text = text_of(&screen, &view());
        assert!(text.contains("Commands"), "{text}");
        assert!(text.contains("partial"), "a partial list was not reported: {text}");
        assert!(text.contains("/model"), "{text}");
    }

    #[test]
    fn copying_is_the_clients_and_needs_no_session() {
        let mut screen = screen();
        screen.editor.set_mode(ed::Mode::Normal);
        match screen.selection_key(&view(), &Key::Char('y')) {
            Some(KeyOut::Copy(text)) => {
                assert!(text.contains("hello"), "{text}");
                assert!(text.contains("echo (collapsed)"), "{text}");
            }
            other => panic!("expected a copy, got {other:?}"),
        }
        assert!(screen.notice.as_deref().expect("a notice").contains("copied"));
    }

    #[test]
    fn a_selection_covers_the_rendered_rows_it_was_dragged_over() {
        let mut screen = screen();
        screen.editor.set_mode(ed::Mode::Normal);
        let view = view();
        // `v` anchors at the bottom, which is where somebody following the tail is
        // looking; the first motion is what says how far back the range goes.
        assert_eq!(screen.selection_key(&view, &Key::Char('v')), Some(KeyOut::Local));
        assert!(screen.selection.is_some(), "v opened no selection");
        screen.selection_key(&view, &Key::Motion(ed::Motion::First));
        match screen.selection_key(&view, &Key::Char('y')) {
            Some(KeyOut::Copy(text)) => {
                assert!(text.contains("hello"), "{text}");
                assert!(!text.contains("echo (collapsed)"), "the range covered more than it had: {text}");
            }
            other => panic!("expected a copy, got {other:?}"),
        }
        assert!(screen.selection.is_none(), "the selection outlived the copy");
    }

    #[test]
    fn a_selection_ends_on_escape_and_leaves_the_composer_alone() {
        let mut screen = screen();
        screen.editor.set_mode(ed::Mode::Normal);
        let view = view();
        screen.selection_key(&view, &Key::Char('v'));
        assert_eq!(screen.selection_key(&view, &Key::Escape), Some(KeyOut::Local));
        assert!(screen.selection.is_none());
        assert!(screen.notice.is_none(), "the notice outlived the selection");
        assert_eq!(screen.editor.text(), "", "escape reached the composer");
    }

    #[test]
    fn a_reader_who_is_typing_never_loses_a_key_to_a_selection() {
        let mut screen = screen();
        let view = view();
        // In insert mode `y` is a letter, and nothing about a reader's selection may
        // take it: that is the whole reason the selection is asked second.
        assert_eq!(screen.selection_key(&view, &Key::Char('y')), None, "insert mode lost a keystroke");
        assert_eq!(screen.key(Key::Char('y')), KeyOut::Local);
        assert_eq!(screen.editor.text(), "y");
    }

    #[test]
    fn a_selection_is_painted_from_offsets_the_client_holds() {
        let mut screen = screen();
        screen.editor.set_mode(ed::Mode::Normal);
        let view = view();
        screen.selection_key(&view, &Key::Char('v'));
        screen.selection_key(&view, &Key::Motion(ed::Motion::First));
        let selected = screen.theme.role("selection");
        let lines = draw(&screen, &view);
        let painted = lines
            .iter()
            .flat_map(|line| line.spans.iter())
            .find(|(style, _)| *style == selected)
            .map(|(_, text)| text.clone());
        assert_eq!(painted.as_deref(), Some("hello"), "the wrong bytes were highlighted");
        assert!(text_of(&screen, &view).contains("hello"), "the highlight ate the text");
    }
    #[test]
    fn modal_operators_edit_unicode_and_yank_without_changing_text() {
        let mut screen = screen();
        screen.editor.set_text("héllo world");
        screen.key(Key::Escape);
        screen.key(Key::Char('0'));
        screen.key(Key::Char('y'));
        assert_eq!(screen.key(Key::Char('w')), KeyOut::Copy("héllo ".into()));
        assert_eq!(screen.editor.text(), "héllo world");
        screen.key(Key::Char('d'));
        screen.key(Key::Char('w'));
        assert_eq!(screen.editor.text(), "world");
        screen.key(Key::Char('u'));
        assert_eq!(screen.editor.text(), "héllo world");
        screen.key(Key::Char('c'));
        screen.key(Key::Char('$'));
        assert_eq!(screen.editor.text(), "");
        assert_eq!(screen.editor.mode(), ed::Mode::Insert);
    }

    #[test]
    fn line_operators_open_lines_and_escape_cancels_pending_edit() {
        let mut screen = screen();
        screen.editor.set_text("first\nlast");
        screen.key(Key::Escape);
        screen.key(Key::Char('d'));
        screen.key(Key::Char('d'));
        assert_eq!(screen.editor.text(), "first");
        screen.key(Key::Char('O'));
        assert_eq!(screen.editor.text(), "\nfirst");
        screen.key(Key::Char('a'));
        screen.key(Key::Escape);
        screen.key(Key::Char('o'));
        assert_eq!(screen.editor.text(), "a\n\nfirst");
        screen.key(Key::Escape);
        screen.key(Key::Char('d'));
        screen.key(Key::Escape);
        screen.key(Key::Char('w'));
        assert_eq!(screen.editor.text(), "a\n\nfirst");
    }

    #[test]
    fn reverse_search_refines_cycles_accepts_and_restores_draft() {
        let mut screen = screen();
        for text in ["old cat", "dog", "new cat"] {
            screen.editor.set_text(text);
            screen.editor.submit();
        }
        screen.editor.set_text("draft");
        screen.key(Key::HistorySearch);
        screen.key(Key::Char('c'));
        assert_eq!(screen.editor.text(), "new cat");
        screen.key(Key::HistorySearch);
        assert_eq!(screen.editor.text(), "old cat");
        screen.key(Key::Escape);
        assert_eq!(screen.editor.text(), "draft");
        screen.key(Key::HistorySearch);
        screen.key(Key::Char('d'));
        assert_eq!(screen.key(Key::Submit), KeyOut::Local);
        assert_eq!(screen.editor.text(), "dog");
        assert!(matches!(screen.key(Key::Submit), KeyOut::Intent(_)));
    }

    #[test]
    fn shift_enter_inserts_a_newline_and_alt_enter_interrupts_with_the_draft() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut screen = screen();
        screen.editor.set_text("first");
        let key = translate(KeyCode::Enter, KeyModifiers::SHIFT).unwrap();
        assert_eq!(screen.key(key), KeyOut::Local);
        assert_eq!(screen.editor.text(), "first\n");
        let key = translate(KeyCode::Enter, KeyModifiers::ALT).unwrap();
        assert_eq!(screen.key(key), KeyOut::Intent(Intent::Interrupt { text: "first\n".into(), attachments: vec![] }));
        screen.editor.set_text("/clear");
        assert!(matches!(screen.key(Key::InterruptSubmit), KeyOut::Intent(Intent::Command { ref name, .. }) if name == "clear"));
        assert_eq!(translate(KeyCode::Char('r'), KeyModifiers::CONTROL), Some(Key::HistorySearch));
    }

}

#[cfg(test)]
fn test_unique_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
