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

mod buttons;
mod chrome;
pub mod clipboard;
mod dialogs;
mod event_loop;
pub use misa_linear::graphics;
pub mod output;
pub mod prefs;
pub mod presentation;
pub mod print;
mod retained;
pub mod save;
pub mod scoped_remote;
pub mod storage;
pub mod workspace;

#[cfg(test)]
thread_local! { static RESOLVE_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

use std::path::PathBuf;

use crate::prefs::Prefs;
use crate::retained::Retained;
use misa_kit::intent::Command;
use misa_kit::intent::Intent;
use misa_kit::intent::Source;
use misa_kit::picker::{Accept, Effect as PickerEffect, Picker};
use misa_kit::{editor as ed, intent as line};
use misa_lines::Line;
use misa_lines::select;
use misa_proto::preparation::SourceKind;
use misa_proto::view::{ActionOn, Choice, Field, Kind, Node};
use misa_render::{Theme, ThemeOverrides};

/// One action this program can take on its own display.
///
/// Client-side by definition, and the reason `:` is a different palette from `/`: a
/// session has no opinion about whether a transcript is expanded or which theme
/// somebody prefers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    ToggleVerbose,
    CycleEffort,
    ScrollUp,
    ScrollDown,
    ScrollTop,
    ScrollBottom,
    OpenModel,
    OpenActionPalette,
    ThemeDark,
    ThemeLight,
    ThemePlain,
    OpenCommands,
    OpenSelection,
    Compact,
    OpenUsage,
    Quit,
}

impl Action {
    pub const ALL: &'static [Action] = &[
        Action::ToggleVerbose,
        Action::CycleEffort,
        Action::ScrollUp,
        Action::ScrollDown,
        Action::ScrollTop,
        Action::ScrollBottom,
        Action::OpenModel,
        Action::OpenActionPalette,
        Action::ThemeDark,
        Action::ThemeLight,
        Action::ThemePlain,
        Action::OpenCommands,
        Action::OpenSelection,
        Action::Compact,
        Action::OpenUsage,
    ];

    pub fn id(&self) -> &'static str {
        match self {
            Action::ToggleVerbose => "transcript.verbose",
            Action::CycleEffort => "effort.cycle",
            Action::ScrollUp => "transcript.up",
            Action::ScrollDown => "transcript.down",
            Action::ScrollTop => "transcript.top",
            Action::ScrollBottom => "transcript.bottom",
            Action::OpenModel => "model.open",
            Action::OpenActionPalette => "actions.open",
            Action::ThemeDark => "theme.dark",
            Action::ThemeLight => "theme.light",
            Action::ThemePlain => "theme.plain",
            Action::OpenCommands => "commands.open",
            Action::OpenSelection => "selection.open",
            Action::Compact => "compaction.compact",
            Action::OpenUsage => "usage.open",
            Action::Quit => "app.quit",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Action::ToggleVerbose => "Toggle transcript detail",
            Action::CycleEffort => "Cycle reasoning effort",
            Action::ScrollUp => "Scroll transcript up",
            Action::ScrollDown => "Scroll transcript down",
            Action::ScrollTop => "Go to the top of the transcript",
            Action::ScrollBottom => "Follow the newest output",
            Action::OpenModel => "Choose active model",
            Action::OpenActionPalette => "Open action palette / key reference",
            Action::ThemeDark => "Use the dark theme",
            Action::ThemeLight => "Use the light theme",
            Action::ThemePlain => "Use no colour",
            Action::OpenCommands => "Open session commands",
            Action::OpenSelection => "Navigate transcript",
            Action::Compact => "Compact conversation",
            Action::OpenUsage => "Show usage",
            Action::Quit => "Leave",
        }
    }

    /// The keys this action is bound to, for the palette to show beside it.
    pub fn keys(&self, keymap: &crate::prefs::KeymapSettings) -> String {
        keymap.keys(self.id())
    }

    fn command_name(&self) -> Option<&'static str> {
        match self {
            Action::Compact => Some("compact"),
            Action::OpenUsage => Some("usage"),
            _ => None,
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
    DaemonInvoke {
        daemon: String,
        scope: misa_proto::observation::Scope,
        command: String,
        input: misa_value::Value,
    },
    Invoke {
        command: String,
        input: misa_value::Value,
    },
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
    /// Start the reader selection over the current retained document.
    ///
    /// The view is required to build byte offsets, so the screen can announce
    /// this intent but the retained owner performs it at the event boundary.
    StartSelection,
    /// Leave.
    Quit,
}

/// The client's whole state.
struct HistorySearch {
    draft: String,
    query: String,
    before: usize,
}

/// The named theme with the client's overrides applied.
fn theme_named(name: &str, overrides: &ThemeOverrides) -> Theme {
    let base = match name {
        "plain" => Theme::plain(),
        "light" => Theme::light(),
        _ => Theme::dark(),
    };
    base.overlay(overrides)
}

pub struct Screen {
    pub dialogs: dialogs::Dialogs,
    pub local_presentation: presentation::Local,
    pub components: misa_lines::components::Registry,
    pub values: misa_render::fact::Registry,
    operator: Option<char>,
    history_search: Option<HistorySearch>,
    pub theme: Theme,
    pub editor: ed::Editor,
    draft_scope: Option<String>,
    saved_prefs: Prefs,
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
    /// Resident sources have a loading state at the client boundary. This is not
    /// picker state: it prevents a slow/empty catalog from turning every keystroke
    /// into another request while still allowing the editor to remain usable.
    resident_requests: std::collections::HashSet<String>,
    pub scroll: usize,
    /// Whether the viewport follows new output. Scrolling away stops it, which is
    /// what lets somebody read while a model is still writing.
    pub follow: bool,
    /// A monotonic count of explicit scroll commands, so the retained viewport can
    /// tell a reader's scroll from a layout shift under the same row number.
    pub scroll_intent: u64,
    pub width: u16,
    pub height: u16,
    /// The kitty graphics cache and planner. Terminal-specific, so it lives on the
    /// client, not in the retained layout or the shared line renderer.
    pub graphics: graphics::Kitty,
}

impl Screen {
    pub fn new(width: u16, height: u16) -> Screen {
        Screen {
            draft_scope: None,
            saved_prefs: Prefs::default(),
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
            resident_requests: Default::default(),
            scroll: 0,
            follow: true,
            scroll_intent: 0,
            width,
            height,
            graphics: graphics::Kitty::new(false, graphics::CellSize::default()),
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
        screen.theme = theme_named(&prefs.theme, &prefs.theme_overrides);
        screen.editor.set_text(prefs.draft.clone());
        screen.saved_prefs = prefs.clone();
        screen.prefs = prefs;
        screen.prefs_path = Some(path);
        screen
    }

    fn remember_draft(&mut self) {
        if let Some(scope) = &self.draft_scope {
            self.prefs
                .drafts
                .insert(scope.clone(), self.editor.text().into());
        } else {
            self.prefs.draft = self.editor.text().into();
        }
    }
    fn enter_draft_scope(&mut self, scope: String) {
        self.remember_draft();
        self.editor
            .set_text(self.prefs.drafts.get(&scope).cloned().unwrap_or_default());
        self.draft_scope = Some(scope);
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
        self.remember_draft();
        match storage::File::at(path).update(&self.saved_prefs, &self.prefs) {
            Ok(()) => self.saved_prefs = self.prefs.clone(),
            Err(error) => self.notice = Some(error),
        }
    }

    /// Replace local composer declarations after the selected scope's catalogs load.
    /// Owner metadata and connection status are supplied independently.
    pub fn declare(&mut self, info: &Catalog) {
        self.resident.clear();
        self.resident_requests.clear();
        self.commands = info.commands.clone();
        self.commands.push(Command::new(
            "save",
            "Save attachment",
            "/save [number] <local path>",
        ));
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
                        command
                            .args
                            .iter()
                            .map(|arg| arg.label.clone())
                            .collect::<Vec<_>>()
                            .join(" ")
                    )
                }),
                metadata: None,
            })
            .collect()
    }

    fn source(&self, id: &str) -> Option<&Source> {
        self.sources.iter().find(|source| source.id == id)
    }

    /// Whether a source can be answered from what this client already holds.
    fn is_resident(&self, id: &str) -> bool {
        self.source(id)
            .map(|source| source.kind == SourceKind::Resident)
            .unwrap_or(false)
    }

    /// Open the picker a command's next argument needs.
    fn open_argument_picker(&mut self, command: &str, argument: &str, source: &str) -> KeyOut {
        let words = line::words(self.editor.text().trim_start_matches('/'));
        let index = self
            .commands
            .iter()
            .find(|item| item.id == command)
            .and_then(|item| item.args.iter().position(|item| item.name == argument))
            .unwrap_or(0);
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
                    self.prefs
                        .picker
                        .picker_views(source, misa_kit::picker::PickerPlacement::Inline),
                )
                .with_frecency(self.prefs.frecency())
                .with_favorites(self.prefs.favorites()),
        );
        if let Some((items, truncated)) = self.resident.get(source) {
            self.picker
                .as_mut()
                .unwrap()
                .set_items(items.clone(), *truncated);
        }
        if let Some(picker) = self.picker.as_mut() {
            picker.set_query(Self::picker_query(
                self.editor.text(),
                &picker.accept,
                &self.commands,
            ));
        }
        let needs_catalog = self.is_resident(source)
            && !self.resident.contains_key(source)
            && self.resident_requests.insert(source.to_string());
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
    pub fn candidates(&mut self, source: &str, items: Vec<Choice>, truncated: bool) {
        let resident = self.is_resident(source);
        if resident {
            self.resident_requests.remove(source);
            self.resident
                .insert(source.into(), (items.clone(), truncated));
        }
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
    pub fn completion(&mut self, source: &str, prefix: &str, items: Vec<Choice>, truncated: bool) {
        if self.is_resident(source) {
            self.resident_requests.remove(source);
            self.resident
                .insert(source.to_string(), (items.clone(), truncated));
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
    pub fn completion_failed(&mut self, source: &str) {
        self.resident_requests.remove(source);
    }

    /// The rendered body a selection moves over. Nothing here reaches a session.
    fn body(&self, view: &Node) -> select::Body {
        let resolved = self.resolve(view);
        select::Body::of(&misa_lines::render(
            &resolved,
            &self.theme,
            self.width as usize,
        ))
    }

    /// A key that concerns the reader's selection, if it concerns one at all.
    ///
    /// `None` means "not about the selection", and the caller falls through to
    /// [`Screen::key`]. It takes the view because a selection is over the *rendered*
    /// body, and only a caller holding the tree can compute that — which is the price
    /// of byte offsets meaning something.
    pub fn selection_key(&mut self, view: &Node, key: &Key) -> Option<KeyOut> {
        if !self.reading_key(key) {
            return None;
        }
        self.selection_in(&self.body(view), key)
    }
    fn reading_key(&self, key: &Key) -> bool {
        self.selection.is_some()
            || self.editor.mode() == ed::Mode::Normal
                && (matches!(key, Key::StartSelection | Key::Char('v'))
                    || matches!(key, Key::Char('y'))
                        && self.editor.is_empty()
                        && self.operator.is_none())
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
            Key::StartSelection | Key::Char('v') => {
                self.begin_selection(body);
                Some(KeyOut::Local)
            }
            Key::Char('y') if self.editor.is_empty() && self.operator.is_none() => {
                Some(self.copy_body(body))
            }
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
            let field = panel_field(panel)
                .map(|field| field.id.clone())
                .unwrap_or_default();
            // The panel's actions are the source of truth for its affordances. The surface
            // renders them as buttons; putting a second, client-authored prose hint in the
            // status/notice lane makes the interaction disagree with the action model.
            self.notice = None;
            self.panel = Some(PanelInput {
                panel: asking,
                field,
                text: String::new(),
            });
        }
        if let Some(action) = panel.actions.iter().find(|action| {
            action.on == ActionOn::Click && self.prefs.dialogs.matches(&action.id, key)
        }) {
            self.panel = None;
            self.notice = None;
            return Some(KeyOut::Intent(Intent::Action {
                node: panel.id.clone(),
                action: action.id.clone(),
                args: action.args.clone(),
                fields: Vec::new(),
            }));
        }
        if panel
            .children
            .iter()
            .flat_map(|child| child.actions.iter())
            .any(|action| {
                action.on == ActionOn::Submit && self.prefs.dialogs.matches(&action.id, key)
            })
        {
            return Some(self.submit_panel(panel));
        }
        Some(match key {
            Key::Eof
                if self
                    .panel
                    .as_ref()
                    .is_some_and(|panel| panel.text.is_empty()) =>
            {
                KeyOut::Quit
            }
            Key::Escape => self.dismiss_panel(panel),
            Key::Submit if self.prefs.dialogs.matches("panel.submit", key) => {
                self.submit_panel(panel)
            }
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
        let close = panel.actions.iter().find(|action| {
            action.on == ActionOn::Click
                && self
                    .prefs
                    .dialogs
                    .key(&action.id)
                    .is_some_and(|key| key == "escape")
        });
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
        let Some(form) = panel
            .children
            .iter()
            .find(|child| matches!(&child.kind, Kind::Fields { fields } if !fields.is_empty()))
        else {
            return KeyOut::Local;
        };
        let Some(action) = form
            .actions
            .iter()
            .find(|action| action.on == ActionOn::Submit)
        else {
            return KeyOut::Local;
        };
        let Kind::Fields { fields: declared } = &form.kind else {
            return KeyOut::Local;
        };
        let typed = self
            .panel
            .as_ref()
            .map(|state| state.text.clone())
            .unwrap_or_default();
        let focused = self
            .panel
            .as_ref()
            .map(|state| state.field.clone())
            .unwrap_or_default();
        let fields = declared
            .iter()
            .map(|field| Field {
                value: if field.id == focused {
                    typed.clone()
                } else {
                    field.value.clone()
                },
                ..field.clone()
            })
            .collect::<Vec<_>>();
        // What was typed goes out of this client's hands as it leaves the screen: a secret
        // that stays in a field after it has been sent is a secret on a screen.
        if let Some(state) = self.panel.as_mut() {
            state.text.clear();
        }
        self.notice = None;
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
            Key::Escape | Key::Char('q') | Key::Interrupt | Key::Eof => {
                self.notice = None;
                return KeyOut::Local;
            }
            Key::Char('y') => {
                let text = selection.text(&body);
                self.notice = Some(format!("copied {} bytes", text.len()));
                return KeyOut::Copy(text);
            }
            // `o` drops the anchor where the caret is; `v` cycles what the range means.
            Key::Char('o') => selection.restart(),
            Key::Char('v') => selection.set_kind(selection.kind().next()),
            Key::Char('a') => selection.select_node(&body),
            Key::Char('j') => selection.down(&body),
            Key::Char('k') => selection.up(&body),
            Key::Char('g') => selection.document_start(&body),
            Key::Char('G') => selection.document_end(&body),
            Key::Char('h') => selection.left(&body),
            Key::Char('l') => selection.right(&body),
            Key::Char('J') | Key::Submit | Key::Newline => selection.select_node(&body),
            Key::Char('K') | Key::Backspace => selection.node_back(&body),
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
            Key::Extend(ed::Motion::Left) => selection.left(&body),
            Key::Extend(ed::Motion::Right) => selection.right(&body),
            Key::Extend(ed::Motion::Up) => selection.up(&body),
            Key::Extend(ed::Motion::Down) => selection.down(&body),
            _ => {}
        }
        self.selection = Some(selection);
        KeyOut::Local
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
                self.save();
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
            Key::ScrollPage(delta) => {
                self.scroll_by(delta);
                KeyOut::Local
            }
            Key::Action(action) => self.action(action),
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
        if let Some(parsed) = save::parse(&text) {
            return match parsed {
                Ok(request) => {
                    self.editor.submit();
                    self.save();
                    KeyOut::Save(request)
                }
                Err(error) => {
                    self.notice = Some(error);
                    KeyOut::Local
                }
            };
        }
        match line::parse(&text, &self.commands) {
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
        let text = self.editor.text().trim_start().to_string();
        let Some(rest) = text.strip_prefix('/') else {
            return KeyOut::Local;
        };
        let words = line::words(rest);
        let name = words.values.first().cloned().unwrap_or_default();
        let Some(command) = self
            .commands
            .iter()
            .find(|command| command.id == name)
            .cloned()
        else {
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

    fn open_command_picker(&mut self) -> KeyOut {
        self.open_command_picker_with(misa_kit::picker::PickerPlacement::Overlay)
    }

    fn open_command_picker_with(&mut self, placement: misa_kit::picker::PickerPlacement) -> KeyOut {
        self.pending_command = None;
        self.editor.set_text("/");
        // `/` on an empty line is a request for the session's commands, and the
        // promise the declaration made is that they can be listed without asking.
        let mut picker = if self.sources.iter().any(|source| source.id == "commands") {
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
                .with_views(self.prefs.picker.picker_views("commands", placement))
                .with_frecency(self.prefs.frecency())
                .with_favorites(self.prefs.favorites()),
        );
        KeyOut::Local
    }

    /// Open the client's own actions.
    fn open_action_palette(&mut self) -> KeyOut {
        self.editor.set_text(":");
        let mut picker = Picker::over("actions", "Actions", Accept::Run)
            .with_views(
                self.prefs
                    .picker
                    .picker_views("actions", misa_kit::picker::PickerPlacement::Overlay),
            )
            .with_frecency(self.prefs.frecency())
            .with_favorites(self.prefs.favorites());
        picker.set_items(
            Action::ALL
                .iter()
                .filter(|action| match action.command_name() {
                    Some(command) => self.commands.iter().any(|entry| entry.id == command),
                    None => true,
                })
                .map(|action| {
                    let keys = action.keys(&self.prefs.keymap);
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
            if self.commands.iter().any(|command| command.id == name) {
                self.editor.type_char(' ');
                self.picker = None;
                return match line::parse(self.editor.text(), &self.commands) {
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
                    &self.commands,
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
                        &self.commands,
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

    fn picker_query(text: &str, accept: &Accept, commands: &[line::Command]) -> String {
        match accept {
            Accept::Argument { command, argument } => {
                let index = commands
                    .iter()
                    .find(|item| &item.id == command)
                    .and_then(|item| item.args.iter().position(|item| &item.name == argument));
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
                let index = self
                    .commands
                    .iter()
                    .find(|item| &item.id == command)
                    .and_then(|item| item.args.iter().position(|item| &item.name == argument))
                    .unwrap_or(0);
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

    pub fn paste(&mut self, text: &str) -> KeyOut {
        self.editor.insert(text);
        let effect = self.picker.as_mut().map(|picker| {
            picker.set_query(Self::picker_query(
                self.editor.text(),
                &picker.accept,
                &self.commands,
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
                if self.is_resident(&source) {
                    self.resident_requests.insert(source);
                    KeyOut::Local
                } else {
                    KeyOut::Complete { source, prefix }
                }
            }
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
                if let Accept::Argument { command, argument } = &accepted.accept {
                    let index = self
                        .commands
                        .iter()
                        .find(|item| &item.id == command)
                        .and_then(|item| item.args.iter().position(|item| &item.name == argument))
                        .unwrap_or(0);
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
            PickerEffect::Favorited { value, favorite } => {
                if favorite {
                    self.prefs.favorites.insert(value.clone());
                } else {
                    self.prefs.favorites.remove(&value);
                }
                self.notice = Some(if favorite {
                    format!("favorite: {value}")
                } else {
                    format!("unfavorite: {value}")
                });
                self.save();
                KeyOut::Local
            }
        }
    }

    fn action(&mut self, action: Action) -> KeyOut {
        match action {
            Action::ToggleVerbose => {
                if self.prefs.any_open() {
                    self.prefs.close_all();
                } else {
                    self.prefs.open_all();
                }
                self.save();
                KeyOut::Local
            }
            Action::CycleEffort => KeyOut::Invoke {
                command: "session.effort.cycle".into(),
                input: misa_value::Value::map([]),
            },
            Action::ScrollUp => {
                self.scroll_by(-10);
                KeyOut::Local
            }
            Action::ScrollDown => {
                self.scroll_by(10);
                KeyOut::Local
            }
            Action::ScrollTop => {
                self.follow = false;
                self.scroll = 0;
                self.scroll_intent = self.scroll_intent.wrapping_add(1);
                KeyOut::Local
            }
            Action::ScrollBottom => {
                self.follow = true;
                self.scroll_intent = self.scroll_intent.wrapping_add(1);
                KeyOut::Local
            }
            Action::OpenModel => self.open_model_picker(),
            Action::OpenActionPalette => self.open_action_palette(),
            Action::ThemeDark => {
                self.theme = theme_named("dark", &self.prefs.theme_overrides);
                self.prefs.theme = "dark".into();
                self.save();
                KeyOut::Local
            }
            Action::ThemeLight => {
                self.theme = theme_named("light", &self.prefs.theme_overrides);
                self.prefs.theme = "light".into();
                self.save();
                KeyOut::Local
            }
            Action::ThemePlain => {
                self.theme = theme_named("plain", &self.prefs.theme_overrides);
                self.prefs.theme = "plain".into();
                self.save();
                KeyOut::Local
            }
            Action::OpenCommands => self.open_command_picker(),
            Action::OpenSelection => KeyOut::StartSelection,
            Action::Compact => KeyOut::Intent(Intent::Command {
                name: "compact".into(),
                args: misa_value::Value::Null,
            }),
            Action::OpenUsage => KeyOut::Intent(Intent::Command {
                name: "usage".into(),
                args: misa_value::Value::Null,
            }),
            Action::Quit => KeyOut::Quit,
        }
    }

    /// Open the model chooser as a focused overlay. The typed `/model` path is
    /// still editor completion; the global model action is the direct chooser.
    fn open_model_picker(&mut self) -> KeyOut {
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
            self.prefs
                .picker
                .picker_views(source, misa_kit::picker::PickerPlacement::Overlay),
        )
        .with_frecency(self.prefs.frecency())
        .with_favorites(self.prefs.favorites());
        if let Some((items, truncated)) = self.resident.get(source) {
            picker.set_items(items.clone(), *truncated);
        }
        self.picker = Some(picker);
        if self.resident.contains_key(source) {
            KeyOut::Local
        } else {
            self.resident_requests.insert(source.into());
            KeyOut::Complete {
                source: source.into(),
                prefix: String::new(),
            }
        }
    }

    fn scroll_by(&mut self, delta: isize) {
        let next = self.scroll as isize + delta;
        self.scroll = next.max(0) as usize;
        self.follow = false;
        self.scroll_intent = self.scroll_intent.wrapping_add(1);
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
        node.children = node
            .children
            .iter()
            .map(|child| self.resolve(child))
            .collect();
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
            } else if node.label.is_none() && self.theme.rail(&node.role).is_some() {
                // A label-less short form whose role names a rail stays a railed block
                // when it closes. Losing the rail would flatten a thinking block into
                // ordinary assistant prose, which is exactly what identifies it. A
                // labelled disclosure (a tool call) is rendered by its own title.
                node.children = vec![
                    Node::text(format!("{}.summary", node.role), summary.clone())
                        .id(format!("{}.summary", node.id)),
                ];
                node.kind = Kind::Section;
            } else {
                node.kind = Kind::Text {
                    spans: summary.clone(),
                };
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
        Kind::Fields { fields }
            if child
                .actions
                .iter()
                .any(|action| action.on == ActionOn::Submit) =>
        {
            fields.iter().find(|field| !field.read_only)
        }
        _ => None,
    })
}

/// A key, in the vocabulary the client cares about.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    HistorySearch,
    HistoryPrevious,
    HistoryNext,
    StartSelection,
    Newline,
    InterruptSubmit,
    Char(char),
    Backspace,
    Delete,
    Eof,
    Submit,
    Tab,
    Favorite,
    PickerPreviousView,
    PickerSlot(usize),
    /// An Alt-letter event not claimed by a global binding. Pickers use it for
    /// their stable multi-key choice shortcuts.
    Alt(char),
    QueueEdit,
    Escape,
    Interrupt,
    Quit,
    /// Text from a picker, to be submitted if it is ready.
    Fill(String),
    Motion(ed::Motion),
    Extend(ed::Motion),
    ScrollPage(isize),
    Action(Action),
}

/// Draw a screen: the transcript, the picker when one is open, and the input line.
pub fn draw(screen: &Screen, view: &Node) -> Vec<Line> {
    // There is one terminal frame policy. The retained path is the production path, while
    // this helper is used by print/tests and must not quietly implement a second geometry:
    // a second layout is how header, dock, status, picker, and message rails drift apart.
    let mut retained = Retained::new(view.clone(), screen);
    retained.frame_with(screen, None, &[], &[]).lines
}

/// The picker, as lines. A frontend with a window would draw this as a panel.
fn choice_preview(
    choice: &misa_proto::view::Choice,
    settings: &crate::prefs::PickerPreviewSettings,
) -> Vec<String> {
    let Some(metadata) = &choice.metadata else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    if let Some(context) = metadata.context_window {
        lines.push(format!("{}: {context} tokens", settings.context_label));
    }
    if !metadata.efforts.is_empty() {
        lines.push(format!(
            "{}: {}",
            settings.effort_label,
            metadata.efforts.join(" · ")
        ));
    }
    if let Some(pricing) = &metadata.pricing {
        let rate = |value: Option<i64>| {
            value
                .map(|value| format!("${}", format_rate(value)))
                .unwrap_or_else(|| "?".into())
        };
        if pricing.input_micros_per_thousand.is_some()
            || pricing.output_micros_per_thousand.is_some()
        {
            lines.push(format!(
                "{}: {} input · {} output",
                settings.price_label,
                rate(pricing.input_micros_per_thousand),
                rate(pricing.output_micros_per_thousand)
            ));
        }
        if pricing.cache_read_micros_per_thousand.is_some()
            || pricing.cache_write_micros_per_thousand.is_some()
        {
            lines.push(format!(
                "{}: {} read · {} write per 1M tokens",
                settings.cache_label,
                rate(pricing.cache_read_micros_per_thousand),
                rate(pricing.cache_write_micros_per_thousand)
            ));
        }
        if let Some(request) = pricing.request_micros.filter(|value| *value > 0) {
            lines.push(format!(
                "{}: ${}",
                settings.request_label,
                format_money(request)
            ));
        }
        lines.push(settings.estimate_note.clone());
    } else {
        lines.push(settings.unavailable_label.clone());
    }
    if let Some(peak) = &metadata.peak {
        let windows = peak
            .windows
            .iter()
            .map(|window| format!("{:02}:00–{:02}:00", window.start_hour, window.end_hour))
            .collect::<Vec<_>>();
        let days = peak
            .weekdays
            .iter()
            .map(|day| match day {
                1 => "Sun",
                2 => "Mon",
                3 => "Tue",
                4 => "Wed",
                5 => "Thu",
                6 => "Fri",
                7 => "Sat",
                _ => "?",
            })
            .collect::<Vec<_>>();
        lines.push(format!(
            "{} {:.3}× at {} UTC{}",
            settings.peak_label,
            peak.multiplier_ppm as f64 / 1_000_000.0,
            windows.join(", "),
            if days.is_empty() {
                String::new()
            } else {
                format!(" on {}", days.join(", "))
            }
        ));
    }
    lines
}

fn format_rate(rate: i64) -> String {
    let value = format!("{:.6}", rate as f64 / 1_000.0);
    value
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn format_money(micros: i64) -> String {
    let value = format!("{:.6}", micros as f64 / 1_000_000.0);
    value
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn picker_lines(screen: &Screen, picker: &Picker) -> Vec<Line> {
    let theme = &screen.theme;
    let settings = &screen.prefs.picker;
    let width = screen.width as usize;
    let padding = settings.padding.min(width.saturating_sub(1));
    let content_width = width.saturating_sub(padding * 2).max(1);
    let selected_preview = picker
        .selected()
        .map(|choice| choice_preview(choice, &settings.preview))
        .unwrap_or_default()
        .into_iter()
        .take(settings.preview_lines)
        .collect::<Vec<_>>();
    let selected_detail = selected_preview.is_empty()
        && picker
            .selected()
            .and_then(|choice| choice.detail.as_deref())
            .is_some_and(|detail| !detail.is_empty());
    let view_count = picker.views().len().max(1);
    // The previous system's `layout.columns`: as many panels as fit, with the
    // remainder shared by the leading ones. The last cell of an odd split is not
    // dropped.
    let widths = misa_render::columns(
        content_width,
        settings.minimum_panel_width,
        view_count.min(3).max(1),
        settings.gap,
    );
    let column_count = widths.len();
    let fixed = 1 + selected_preview.len() + usize::from(selected_detail) + column_count + 1;
    let height = settings
        .preferred_height
        .clamp(settings.min_height, settings.max_height)
        .min(screen.height as usize);
    let panel_budget = height.saturating_sub(fixed).max(1);
    let first_view = (picker.view_index() / column_count) * column_count;
    let panels = picker
        .views()
        .iter()
        .skip(first_view)
        .take(column_count)
        .enumerate()
        .map(|(panel, view)| {
            let panel_width = widths[panel];
            let choices = picker.matches_for(*view);
            let active_view = *view == picker.view();
            let selected = if active_view {
                picker.selected_index()
            } else {
                0
            };
            let start = selected
                .saturating_sub(panel_budget / 2)
                .min(choices.len().saturating_sub(1));
            let mut rows = Vec::new();
            let mut used = 0;
            let mut shown = 0;
            for (index, candidate) in choices.iter().enumerate().skip(start) {
                let active = active_view && index == picker.selected_index();
                let choice = choice_spans(theme, &settings.row, candidate, index, active, picker);
                let wrapped = misa_render::wrap_styled(&choice, panel_width);
                if used + wrapped.len() > panel_budget && !rows.is_empty() {
                    break;
                }
                used += wrapped.len();
                shown += 1;
                rows.extend(wrapped.into_iter().map(|row| {
                    pad_styled(
                        row,
                        panel_width,
                        theme.role(if active {
                            "choice.row.selected"
                        } else {
                            "choice.row"
                        }),
                    )
                }));
                if used >= panel_budget {
                    break;
                }
            }
            if choices.is_empty() {
                rows.push(vec![(
                    theme.role("choice.empty"),
                    misa_render::pad(&settings.empty_label, panel_width),
                )]);
            }
            let more = choices.len() > start + shown || picker.is_truncated();
            let overflow = more.then(|| {
                overflow_text(
                    &settings.overflow_format,
                    start + 1,
                    start + shown,
                    choices.len(),
                )
            });
            (
                *view,
                settings
                    .view_titles
                    .get(view.id())
                    .map(String::as_str)
                    .unwrap_or_else(|| view.title()),
                rows,
                more,
                overflow,
            )
        })
        .collect::<Vec<_>>();
    let has_more = panels.iter().any(|(_, _, _, more, _)| *more);
    let prefix = match picker.accept {
        Accept::Run => "/",
        Accept::Action { .. } => ":",
        Accept::Argument { .. } => "",
    };
    let input_prefix = format!("{}{}: {}", " ".repeat(padding), picker.title, prefix);
    let mut lines = vec![Line {
        surface: None,
        indent: 0,
        spans: vec![
            (theme.role("choice.prompt"), input_prefix.clone()),
            (
                theme.role("choice.query"),
                misa_render::clip(
                    &picker.query,
                    width.saturating_sub(misa_render::width(&input_prefix)),
                ),
            ),
        ],
        node: None,
    }];
    if selected_detail {
        let detail = picker
            .selected()
            .and_then(|choice| choice.detail.as_deref())
            .unwrap_or_default();
        lines.push(Line {
            surface: None,
            indent: 0,
            spans: vec![(
                theme.role("choice.hint"),
                misa_render::pad(&format!("{}{}", " ".repeat(padding), detail), width),
            )],
            node: None,
        });
    }
    for preview in &selected_preview {
        lines.push(Line {
            surface: None,
            indent: 0,
            spans: vec![(
                theme.role("choice.preview"),
                misa_render::pad(&format!("{}{}", " ".repeat(padding), preview), width),
            )],
            node: None,
        });
    }
    let mut heading_spans = vec![(theme.role("plain"), " ".repeat(padding))];
    for (panel, (view, title, _, _, _)) in panels.iter().enumerate() {
        let style = theme.role(if *view == picker.view() {
            "choice.view.active"
        } else {
            "choice.view"
        });
        heading_spans.push((style, misa_render::pad(title, widths[panel])));
        if panel + 1 < panels.len() {
            heading_spans.push((theme.role("plain"), " ".repeat(settings.gap)));
        }
    }
    lines.push(Line {
        surface: None,
        indent: 0,
        spans: heading_spans,
        node: None,
    });
    let max_rows = panels
        .iter()
        .map(|(_, _, rows, _, _)| rows.len())
        .max()
        .unwrap_or(1);
    for row in 0..max_rows {
        let mut spans = vec![(theme.role("plain"), " ".repeat(padding))];
        for (panel, (_, _, rows, _, _)) in panels.iter().enumerate() {
            if let Some(panel_line) = rows.get(row) {
                spans.extend(panel_line.iter().cloned());
            } else {
                spans.push((theme.role("plain"), " ".repeat(widths[panel])));
            }
            if panel + 1 < panels.len() {
                spans.push((theme.role("plain"), " ".repeat(settings.gap)));
            }
        }
        lines.push(Line {
            surface: None,
            indent: 0,
            spans,
            node: None,
        });
    }
    let hints = picker_hints(settings, picker, false);
    lines.push(Line {
        surface: None,
        indent: 0,
        spans: {
            let mut spans = vec![(theme.role("plain"), " ".repeat(padding))];
            spans.extend(crate::buttons::key_reference(
                theme,
                &settings.hint_separator,
                hints
                    .iter()
                    .map(|hint| (hint.key.as_str(), hint.label.as_str())),
            ));
            spans
        },
        node: None,
    });
    if has_more {
        lines.push(Line {
            surface: None,
            indent: 0,
            spans: vec![(
                theme.role("choice.hint"),
                format!(
                    "{}{} {}",
                    " ".repeat(padding),
                    panels
                        .iter()
                        .filter_map(|(_, _, _, _, overflow)| overflow.as_deref())
                        .collect::<Vec<_>>()
                        .join(&settings.hint_separator),
                    settings.more_label
                ),
            )],
            node: None,
        });
    }
    crate::chrome::physical(lines, width)
}

fn overflow_text(template: &str, start: usize, last: usize, total: usize) -> String {
    template
        .replace("{start}", &start.to_string())
        .replace("{last}", &last.to_string())
        .replace("{total}", &total.to_string())
}

fn choice_spans(
    theme: &misa_render::Theme,
    row: &crate::prefs::PickerRowSettings,
    candidate: &misa_proto::view::Choice,
    index: usize,
    selected: bool,
    picker: &Picker,
) -> Vec<(misa_render::Style, String)> {
    let row_style = theme.role(if selected {
        "choice.row.selected"
    } else {
        "choice.row"
    });
    let mut spans = vec![
        (
            row_style,
            if selected {
                row.selected_marker.clone()
            } else {
                row.marker.clone()
            },
        ),
        (row_style, row.marker_separator.clone()),
        (
            if selected {
                row_style
            } else {
                theme.role("keybinding")
            },
            display_keys(&picker.shortcut_for(index)),
        ),
        (row_style, row.marker_separator.clone()),
        (row_style, candidate.label.clone()),
    ];
    if let Some(detail) = &candidate.detail
        && !detail.is_empty()
    {
        spans.push((
            if selected {
                row_style
            } else {
                theme.role("choice.hint")
            },
            format!("{}{detail}", row.detail_separator),
        ));
    }
    spans
}

fn pad_styled(
    mut spans: Vec<(misa_render::Style, String)>,
    width: usize,
    style: misa_render::Style,
) -> Vec<(misa_render::Style, String)> {
    let used = spans
        .iter()
        .map(|(_, text)| misa_render::width(text))
        .sum::<usize>();
    if used < width {
        spans.push((style, " ".repeat(width - used)));
    }
    spans
}

/// The compact editor-owned projection of the same picker state. Inline choices
/// do not repeat the query/title—the composer already displays that—and therefore
/// consume only the rows needed by candidate labels and the overflow hint.
pub(crate) fn completion_lines(screen: &Screen, picker: &Picker) -> Vec<Line> {
    let matches = picker.matches();
    let width = screen.width as usize;
    let mut lines = Vec::new();
    for (index, candidate) in matches.iter().enumerate() {
        let selected = index == picker.selected_index();
        let mut spans = vec![(
            screen.theme.role("plain"),
            screen.prefs.picker.row.inline_prefix.clone(),
        )];
        spans.extend(choice_spans(
            &screen.theme,
            &screen.prefs.picker.row,
            candidate,
            index,
            selected,
            picker,
        ));
        let wrapped = misa_render::wrap_styled(&spans, width.max(1));
        for row in wrapped {
            lines.push(Line {
                surface: None,
                indent: 0,
                spans: pad_styled(
                    row,
                    width,
                    screen.theme.role(if selected {
                        "choice.row.selected"
                    } else {
                        "choice.row"
                    }),
                ),
                node: None,
            });
        }
    }
    if picker.is_truncated() {
        let hints = picker_hints(&screen.prefs.picker, picker, true);
        let mut spans = vec![(
            screen.theme.role("plain"),
            screen.prefs.picker.row.inline_prefix.clone(),
        )];
        spans.extend(crate::buttons::key_reference(
            &screen.theme,
            &screen.prefs.picker.hint_separator,
            hints
                .iter()
                .map(|hint| (hint.key.as_str(), hint.label.as_str())),
        ));
        spans.push((
            screen.theme.role("plain"),
            screen.prefs.picker.hint_separator.clone(),
        ));
        spans.push((
            screen.theme.role("choice.hint"),
            screen.prefs.picker.more_label.clone(),
        ));
        lines.push(Line {
            surface: None,
            indent: 0,
            spans,
            node: None,
        });
    }
    lines
}

fn picker_hints<'a>(
    settings: &'a crate::prefs::PickerSettings,
    picker: &Picker,
    inline: bool,
) -> Vec<&'a crate::prefs::PickerHint> {
    settings
        .hints
        .iter()
        .filter(|hint| if inline { hint.inline } else { hint.overlay })
        .filter(|hint| !hint.only_with_views || picker.views().len() > 1)
        .collect()
}

/// Use the same compact key vocabulary in local palettes and the status bar.
/// Binding identity stays in the action table; this is only presentation.
fn display_keys(keys: &str) -> String {
    keys.split(" · ")
        .map(|key| {
            key.split('+')
                .map(|part| match part {
                    "alt" => "⌥",
                    "ctrl" => "⌃",
                    "shift" => "⇧",
                    "enter" => "↵",
                    "escape" => "esc",
                    "tab" => "⇥",
                    "pageup" => "pgup",
                    "pagedown" => "pgdn",
                    other => other,
                })
                .collect::<Vec<_>>()
                .concat()
        })
        .collect::<Vec<_>>()
        .join(" · ")
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
            spans.push((
                selected,
                text[overlap_start - start..overlap_end - start].to_string(),
            ));
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
    fn turn_settled(&self) -> Option<bool> {
        None
    }
    /// The next view, if one changed.
    async fn next(&mut self) -> Result<Option<Node>, String>;
    /// Incremental consumers retain presentation owners between these updates.
    async fn next_presentation(&mut self) -> Result<Option<Presentation>, String> {
        Ok(self.next().await?.map(Presentation::Snapshot))
    }
    async fn request(&mut self, request: SessionRequest) -> Option<SessionReply> {
        Some(match request {
            SessionRequest::RefreshRequests => {
                SessionReply::Notice("This session has no input request catalog".into())
            }
            SessionRequest::DaemonInvoke { .. } | SessionRequest::Invoke { .. } => {
                SessionReply::Notice("This session does not support installed invocations".into())
            }
            SessionRequest::Intent(intent) => {
                let draft = match &intent {
                    Intent::Prompt { text, attachments }
                    | Intent::Interrupt { text, attachments } => {
                        Some((text.clone(), attachments.clone()))
                    }
                    _ => None,
                };
                SessionReply::Sent {
                    draft,
                    result: self.send(intent).await,
                }
            }
            SessionRequest::Upload {
                generation,
                bytes,
                media,
            } => SessionReply::Uploaded {
                generation,
                result: self.upload(bytes, &media).await,
            },
            SessionRequest::Download { reference } => SessionReply::Downloaded {
                reference: reference.clone(),
                result: self.download(&reference).await,
            },
            SessionRequest::Complete { source, prefix } => {
                let result = self.complete(&source, &prefix).await;
                SessionReply::Complete {
                    source,
                    prefix,
                    result,
                }
            }
            SessionRequest::Save { node, destination } => {
                SessionReply::Notice(match self.save_attachment(&node, &destination).await {
                    Ok(()) => format!("Saved {destination}"),
                    Err(error) => error,
                })
            }
        })
    }
    async fn send(&mut self, intent: Intent) -> Result<(), String>;
    async fn upload(
        &mut self,
        _bytes: Vec<u8>,
        _media: &str,
    ) -> Result<misa_proto::view::BlobRef, String> {
        Err("This client has no blob connection".into())
    }
    async fn save_attachment(&mut self, _node: &str, _destination: &str) -> Result<(), String> {
        Err("This client has no blob connection".into())
    }
    /// Fetch the bytes a [`misa_proto::view::BlobRef`] names. The generic client
    /// renders a placeholder until it returns, so an implementation is free to be
    /// slow — but it must not be called on the render path.
    async fn download(
        &mut self,
        _reference: &misa_proto::view::BlobRef,
    ) -> Result<Vec<u8>, String> {
        Err("This client has no blob connection".into())
    }
    /// Candidates a session holds, for a source this client asked about.
    async fn complete(&mut self, source: &str, prefix: &str)
    -> Result<(Vec<Choice>, bool), String>;
    fn catalog(&self) -> Catalog {
        Catalog::default()
    }
    fn location(&self) -> String {
        "Session".into()
    }
    fn selected(&self) -> Option<misa_proto::directory::Entry> {
        None
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog {
    pub commands: Vec<Command>,
    pub sources: Vec<Source>,
}

pub enum Presentation {
    Attention {
        id: String,
        generation: i64,
    },
    Documents(Vec<(String, misa_client::document::Update)>),
    Activate(String),
    Forget(String),
    Contribution {
        id: String,
        update: misa_client::document::Update,
    },
    TurnOutput(Node),
    Document(misa_client::document::Update),
    Declaration {
        catalog: Catalog,
        location: String,
    },
    Candidates {
        source: String,
        items: Vec<Choice>,
        truncated: bool,
    },
    Snapshot(Node),
    Reply(SessionReply),
}
pub enum SessionRequest {
    DaemonInvoke {
        daemon: String,
        scope: misa_proto::observation::Scope,
        command: String,
        input: misa_value::Value,
    },
    RefreshRequests,
    Invoke {
        command: String,
        input: misa_value::Value,
    },
    Intent(Intent),
    Upload {
        generation: u64,
        bytes: Vec<u8>,
        media: String,
    },
    Download {
        reference: misa_proto::view::BlobRef,
    },
    Complete {
        source: String,
        prefix: String,
    },
    Save {
        node: String,
        destination: String,
    },
}
pub enum SessionReply {
    DaemonForm {
        daemon: String,
        scope: misa_proto::observation::Scope,
        form: misa_client::form::Form,
        drafts: std::collections::BTreeMap<String, String>,
    },
    Form(misa_client::form::Form),
    Request {
        id: String,
        generation: i64,
        model: Option<misa_client::request::Model>,
    },
    Report(Node),
    Uploaded {
        generation: u64,
        result: Result<misa_proto::view::BlobRef, String>,
    },
    Downloaded {
        reference: misa_proto::view::BlobRef,
        result: Result<Vec<u8>, String>,
    },
    Sent {
        draft: Option<(String, Vec<misa_proto::view::BlobRef>)>,
        result: Result<(), String>,
    },
    Complete {
        source: String,
        prefix: String,
        result: Result<(Vec<Choice>, bool), String>,
    },
    Notice(String),
}

/// The interactive loop.
pub async fn run(session: &mut dyn Session) -> Result<(), String> {
    event_loop::run(session).await
}

fn translate(
    code: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
    keymap: &crate::prefs::KeymapSettings,
) -> Option<Key> {
    use crossterm::event::KeyCode;
    let alt = modifiers.contains(crossterm::event::KeyModifiers::ALT);
    let bound = |id| keymap.matches(id, code, modifiers);
    if bound("history.search") {
        return Some(Key::HistorySearch);
    }
    if bound("picker.previous_view") {
        return Some(Key::PickerPreviousView);
    }
    if bound("history.previous") {
        return Some(Key::HistoryPrevious);
    }
    if bound("history.next") {
        return Some(Key::HistoryNext);
    }
    if bound("app.quit") {
        return Some(Key::Quit);
    }
    if bound("input.interrupt") {
        return Some(Key::Interrupt);
    }
    if bound("transcript.verbose") {
        return Some(Key::Action(Action::ToggleVerbose));
    }
    if bound("effort.cycle") {
        return Some(Key::Action(Action::CycleEffort));
    }
    if bound("transcript.up") {
        return Some(Key::Action(Action::ScrollUp));
    }
    if bound("transcript.down") {
        return Some(Key::Action(Action::ScrollDown));
    }
    if bound("model.open") {
        return Some(Key::Action(Action::OpenModel));
    }
    if bound("commands.open") {
        return Some(Key::Action(Action::OpenCommands));
    }
    if bound("picker.favorite") {
        return Some(Key::Favorite);
    }
    for index in 0..9 {
        let id = format!("picker.slot.{}", index + 1);
        if keymap.matches(&id, code, modifiers) {
            return Some(Key::PickerSlot(index));
        }
    }
    if bound("queue.edit") {
        return Some(Key::QueueEdit);
    }
    if bound("selection.open") {
        return Some(Key::StartSelection);
    }
    if bound("input.eof") {
        return Some(Key::Eof);
    }
    if bound("input.interrupt_submit") {
        return Some(Key::InterruptSubmit);
    }
    if bound("input.newline") {
        return Some(Key::Newline);
    }
    if bound("transcript.top") {
        return Some(Key::Action(Action::ScrollTop));
    }
    if bound("transcript.bottom") {
        return Some(Key::Action(Action::ScrollBottom));
    }
    if bound("actions.open") {
        return Some(Key::Action(Action::OpenActionPalette));
    }
    Some(match code {
        // Crossterm reports shifted letters as a character plus SHIFT. Preserve
        // the case used by the semantic selection vocabulary (J/K select the
        // child/parent node) instead of collapsing it into ordinary typing.
        KeyCode::Char('j') if modifiers.contains(crossterm::event::KeyModifiers::SHIFT) => {
            Key::Char('J')
        }
        KeyCode::Char('k') if modifiers.contains(crossterm::event::KeyModifiers::SHIFT) => {
            Key::Char('K')
        }
        KeyCode::Char(character) if alt => Key::Alt(character),
        KeyCode::Char(character) => Key::Char(character),
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::Enter => Key::Submit,
        KeyCode::Tab => Key::Tab,
        KeyCode::Esc => Key::Escape,
        KeyCode::Left if modifiers.contains(crossterm::event::KeyModifiers::SHIFT) => {
            Key::Extend(ed::Motion::Left)
        }
        KeyCode::Right if modifiers.contains(crossterm::event::KeyModifiers::SHIFT) => {
            Key::Extend(ed::Motion::Right)
        }
        KeyCode::Up if modifiers.contains(crossterm::event::KeyModifiers::SHIFT) => {
            Key::Extend(ed::Motion::Up)
        }
        KeyCode::Down if modifiers.contains(crossterm::event::KeyModifiers::SHIFT) => {
            Key::Extend(ed::Motion::Down)
        }
        KeyCode::Left => Key::Motion(ed::Motion::Left),
        KeyCode::Right => Key::Motion(ed::Motion::Right),
        KeyCode::Up => Key::Motion(ed::Motion::Up),
        KeyCode::Down => Key::Motion(ed::Motion::Down),
        KeyCode::Home => Key::Motion(ed::Motion::LineStart),
        KeyCode::End => Key::Motion(ed::Motion::LineEnd),
        // These are the global transcript bindings in the reference. Their
        // amount is chosen by the viewport owner, not baked into translation.
        KeyCode::PageUp => Key::Motion(ed::Motion::Up),
        KeyCode::PageDown => Key::Motion(ed::Motion::Down),
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
            sources: vec![
                Source::resident("models", "Models"),
                Source::resident("effort", "Effort"),
            ],
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
                Node::new(
                    "tool.call",
                    Kind::Collapsible {
                        summary: vec![misa_proto::view::Span::plain("echo (collapsed)")],
                    },
                )
                .id("call.1")
                .label("echo")
                .child(Node::text(
                    "tool.result",
                    [misa_proto::view::Span::plain("the result")],
                )),
            )
    }

    fn text_of(screen: &Screen, view: &Node) -> String {
        misa_lines::to_plain(&draw(screen, view))
    }

    /// A view with a panel in it, the shape the session opens for `/login`.
    fn panel_view(with_field: bool) -> Node {
        let mut session = view();
        let mut panel = Node::section("panel")
            .id("login")
            .label("Credential for `anthropic`");
        panel.children.push(Node::text(
            "panel.text",
            [misa_proto::view::Span::plain("the daemon stores it")],
        ));
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
            assert_eq!(
                screen.panel_key(&view, &Key::Char(character)),
                Some(KeyOut::Local)
            );
        }
        // The composer never saw a key, and the secret is masked on screen.
        assert_eq!(
            screen.editor.text(),
            "",
            "the panel's keys went into the composer"
        );
        assert!(
            !text_of(&screen, &view).contains("sk-a-secret"),
            "{}",
            text_of(&screen, &view)
        );

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
        assert!(
            !text_of(&screen, &view).contains("sk-a-secret"),
            "{}",
            text_of(&screen, &view)
        );
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
        assert_eq!(
            reading.panel_key(&report, &Key::Char('x')),
            Some(KeyOut::Local)
        );
        assert!(matches!(
            reading.panel_key(&report, &Key::Escape),
            Some(KeyOut::Intent(_))
        ));
    }

    #[test]
    fn panel_actions_use_configured_keys_for_both_close_and_submit() {
        let asking = panel_view(true);
        let mut screen = screen();
        screen
            .prefs
            .dialogs
            .action_keys
            .insert("panel.close".into(), "q".into());
        screen
            .prefs
            .dialogs
            .action_keys
            .insert("panel.submit".into(), "s".into());

        for character in "abc".chars() {
            assert_eq!(
                screen.panel_key(&asking, &Key::Char(character)),
                Some(KeyOut::Local)
            );
        }
        assert!(matches!(
            screen.panel_key(&asking, &Key::Char('s')),
            Some(KeyOut::Intent(_))
        ));
        assert!(matches!(
            screen.panel_key(&asking, &Key::Char('q')),
            Some(KeyOut::Intent(_))
        ));
        assert_eq!(screen.panel_key(&asking, &Key::Escape), Some(KeyOut::Local));
    }

    #[test]
    fn ordinary_text_is_submitted_as_a_prompt() {
        let mut screen = screen();
        type_text(&mut screen, "hello");
        match screen.key(Key::Submit) {
            KeyOut::Intent(Intent::Prompt { text, .. }) => assert_eq!(text, "hello"),
            other => panic!("expected a prompt, got {other:?}"),
        }
        assert!(
            screen.editor.is_empty(),
            "the line was not cleared after sending"
        );
    }

    #[test]
    fn a_slash_opens_a_picker_built_from_the_declaration() {
        let mut screen = screen();
        assert_eq!(screen.key(Key::Char('/')), KeyOut::Local);
        let picker = screen.picker.as_ref().expect("a picker");
        assert_eq!(picker.items().len(), 4);
        assert_eq!(picker.views(), &[misa_kit::picker::PickerView::All]);
        assert!(picker.items().iter().any(|item| item.value == "/save"));
        assert!(picker.items().iter().any(|item| item.value == "/model"));
    }

    #[test]
    fn completion_edits_the_composer_and_resident_candidates_survive_closed_pickers() {
        let mut screen = screen();
        screen.candidates(
            "models",
            vec![Choice {
                value: "chosen".into(),
                label: "Chosen".into(),
                detail: None,
                metadata: None,
            }],
            false,
        );
        type_text(&mut screen, "/model ");
        assert_eq!(screen.editor.text(), "/model ");
        assert_eq!(
            screen.picker.as_ref().unwrap().source.as_deref(),
            Some("models")
        );
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
        assert_eq!(
            screen.key(Key::Char('m')),
            KeyOut::Local,
            "a command picker asked a session"
        );
        assert_eq!(screen.key(Key::Char('o')), KeyOut::Local);
        assert_eq!(screen.picker.as_ref().expect("a picker").query, "mo");
    }

    #[test]
    fn accepting_a_command_runs_it() {
        let mut screen = screen();
        screen.key(Key::Char('/'));
        assert_eq!(
            screen.key(Key::Submit),
            KeyOut::Intent(Intent::Command {
                name: "clear".into(),
                args: misa_value::Value::Map(std::sync::Arc::new(Default::default())),
            })
        );
        assert!(screen.picker.is_none());
    }

    #[test]
    fn a_command_that_needs_a_value_opens_the_picker_its_declaration_named() {
        let mut screen = screen();
        screen.editor.set_text("/model");
        assert_eq!(
            screen.key(Key::Submit),
            KeyOut::Complete {
                source: "models".into(),
                prefix: String::new()
            }
        );
        let picker = screen.picker.as_ref().expect("a picker");
        assert_eq!(picker.source.as_deref(), Some("models"));
        assert_eq!(
            picker.accept,
            Accept::Argument {
                command: "model".into(),
                argument: "model".into()
            }
        );
    }

    #[test]
    fn a_resident_source_with_nothing_held_is_asked_for_once() {
        let mut screen = screen();
        screen.editor.set_text("/model");
        // The first ask is deliberate; after the items arrive, nothing more is sent.
        assert_eq!(
            screen.key(Key::Tab),
            KeyOut::Complete {
                source: "models".into(),
                prefix: String::new()
            }
        );
        screen.candidates(
            "models",
            vec![Choice {
                value: "scripted-1".into(),
                label: "Scripted".into(),
                detail: None,
                metadata: None,
            }],
            false,
        );
        screen.key(Key::Char('s'));
        assert_eq!(
            screen.key(Key::Char('c')),
            KeyOut::Local,
            "a held source was asked again"
        );
    }

    #[test]
    fn quoted_argument_completion_keeps_prior_arguments_and_decoded_prefix() {
        let mut screen = screen();
        screen.commands.push(
            line::Command::new("visit", "Visit", "")
                .arg(line::Arg::new("daemon", "Daemon").required())
                .arg(
                    line::Arg::new("session", "Session")
                        .required()
                        .from("models"),
                ),
        );
        screen.editor.set_text("/visit 'daemon one' 'child se");
        assert_eq!(
            screen.key(Key::Tab),
            KeyOut::Complete {
                source: "models".into(),
                prefix: "child se".into()
            }
        );
        screen.candidates(
            "models",
            vec![Choice {
                value: "child session's name".into(),
                label: "Child".into(),
                detail: None,
                metadata: None,
            }],
            false,
        );
        let KeyOut::Intent(Intent::Command { name, args }) = screen.key(Key::Submit) else {
            panic!("expected completed visit");
        };
        assert_eq!(name, "visit");
        assert_eq!(
            args.get("daemon").and_then(misa_value::Value::as_str),
            Some("daemon one")
        );
        assert_eq!(
            args.get("session").and_then(misa_value::Value::as_str),
            Some("child session's name")
        );
    }

    #[test]
    fn unfinished_quoted_argument_stays_in_the_composer() {
        let mut screen = screen();
        screen.editor.set_text("/model 'two words");
        assert_eq!(screen.submit(), KeyOut::Local);
        assert_eq!(screen.editor.text(), "/model 'two words");
        assert!(screen.notice.as_ref().unwrap().contains("quoted"));
    }

    #[test]
    fn accepting_a_value_completes_the_command_and_sends_it() {
        let mut screen = screen();
        screen.editor.set_text("/model");
        assert_eq!(
            screen.key(Key::Submit),
            KeyOut::Complete {
                source: "models".into(),
                prefix: String::new()
            }
        );
        screen.candidates(
            "models",
            vec![Choice {
                value: "scripted-1".into(),
                label: "Scripted".into(),
                detail: None,
                metadata: None,
            }],
            false,
        );
        // Picking the value is the last thing to say, so the command goes.
        assert_eq!(
            screen.key(Key::Submit),
            KeyOut::Intent(Intent::Command {
                name: "model".into(),
                args: misa_value::Value::map([("model", misa_value::Value::str("scripted-1"))]),
            })
        );
        assert!(
            screen.editor.is_empty(),
            "the line was not cleared after sending"
        );
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
        assert!(
            screen
                .notice
                .as_deref()
                .expect("a notice")
                .contains("nonsense")
        );
        assert!(
            screen.editor.text().contains("nonsense"),
            "the line was lost"
        );
    }

    #[test]
    fn an_interrupt_keeps_the_draft_and_asks_the_session_to_stop() {
        let mut screen = screen();
        type_text(&mut screen, "half written");
        assert_eq!(
            screen.key(Key::Interrupt),
            KeyOut::Intent(Intent::Cancel { target: None })
        );
        assert_eq!(screen.editor.text(), "half written");
    }

    /// A path for one test's memory, removed first so a rerun does not read a stale one.
    fn memory(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("misa-tui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("client.json")
    }

    #[test]
    fn persisted_drafts_follow_exact_scope_and_independent_windows_merge() {
        let path = memory("scoped-drafts");
        let mut first = Screen::remembering(Prefs::default(), path.clone());
        let mut second = Screen::remembering(Prefs::default(), path.clone());
        first.enter_draft_scope("peer-a:session:epoch1".into());
        first.editor.set_text("first");
        first.save();
        second.enter_draft_scope("peer-b:session:epoch1".into());
        second.editor.set_text("second");
        second.save();
        let prefs = Prefs::load(&storage::File::at(path.clone()));
        assert_eq!(prefs.drafts["peer-a:session:epoch1"], "first");
        assert_eq!(prefs.drafts["peer-b:session:epoch1"], "second");
        let mut reopened = Screen::remembering(prefs, path);
        reopened.enter_draft_scope("peer-a:session:epoch2".into());
        assert_eq!(reopened.editor.text(), "");
        reopened.enter_draft_scope("peer-a:session:epoch1".into());
        assert_eq!(reopened.editor.text(), "first");
    }
    #[test]
    fn a_client_that_remembers_starts_where_somebody_left_off() {
        // What "a restart forgets where somebody was" meant: the theme, the nodes they had
        // opened, and the draft were in memory and nowhere else.
        let path = memory("remember");
        let mut first = Screen::remembering(Prefs::default(), path.clone());
        assert_eq!(
            first.theme.name, "dark",
            "a client that remembers nothing opens dark"
        );
        first.key(Key::Action(Action::ThemePlain));
        first.key(Key::Action(Action::ToggleVerbose));
        type_text(&mut first, "half a question");
        first.save();

        let second =
            Screen::remembering(Prefs::load(&storage::File::at(path.clone())), path.clone());
        // The theme somebody chose is the one they are drawn with next time.
        assert_eq!(second.theme.name, "plain");
        assert_eq!(second.editor.text(), "half a question");
        // And the opened node is open in what it draws, not only in what it remembers.
        assert!(
            text_of(&second, &view()).contains("the result"),
            "{}",
            text_of(&second, &view())
        );
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
                .map(|value| Choice {
                    value: value.into(),
                    label: value.into(),
                    detail: None,
                    metadata: None,
                })
                .collect::<Vec<_>>()
        };
        first.candidates("models", candidates(), false);
        // The second one, deliberately: the first is what a list would offer anyway.
        first.key(Key::Motion(ed::Motion::Down));
        assert!(matches!(first.key(Key::Submit), KeyOut::Intent(_)));

        let mut second =
            Screen::remembering(Prefs::load(&storage::File::at(path.clone())), path.clone());
        second.declare(&declaration());
        second.editor.set_text("/model");
        second.key(Key::Submit);
        second.candidates("models", candidates(), false);
        assert_eq!(
            second
                .picker
                .as_ref()
                .expect("a picker")
                .selected()
                .map(|choice| choice.value.as_str()),
            Some("scripted-chatty")
        );
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
        assert!(
            screen.notice.is_none(),
            "nothing failed, so there is nothing to say"
        );
    }

    #[test]
    fn a_reader_may_open_a_tool_call_and_the_session_is_not_told() {
        let mut screen = screen();
        let before = text_of(&screen, &view());
        assert!(before.contains("◇ echo"), "{before}");
        assert!(!before.contains("the result"), "{before}");
        assert_eq!(
            screen.key(Key::Action(Action::ToggleVerbose)),
            KeyOut::Local
        );
        let after = text_of(&screen, &view());
        assert!(after.contains("the result"), "{after}");
    }

    #[test]
    fn a_client_override_reaches_the_theme_it_draws_with() {
        let path = memory("theme-override");
        let mut prefs = Prefs::default();
        prefs.theme_overrides.roles.insert(
            "error".into(),
            misa_render::StylePatch {
                fg: Some(misa_render::Color::Rgb(1, 2, 3)),
                ..misa_render::StylePatch::default()
            },
        );
        let screen = Screen::remembering(prefs, path.clone());
        assert_eq!(
            screen.theme.role("error").fg,
            misa_render::Color::Rgb(1, 2, 3)
        );
        // Switching the base theme keeps the override.
        let mut screen = screen;
        screen.key(Key::Action(Action::ThemeLight));
        assert_eq!(
            screen.theme.role("error").fg,
            misa_render::Color::Rgb(1, 2, 3)
        );
        let _ = std::fs::remove_dir_all(path.parent().expect("a parent"));
    }

    #[test]
    fn the_theme_is_the_clients_and_switching_it_touches_nothing_else() {
        let mut screen = screen();
        screen.key(Key::Action(Action::ThemePlain));
        assert_eq!(screen.theme.name, "plain");
        screen.key(Key::Action(Action::ThemeLight));
        assert_eq!(screen.theme.name, "light");
        screen.key(Key::Action(Action::ThemeDark));
        assert_eq!(screen.theme.name, "dark");
    }

    #[test]
    fn a_colon_opens_the_clients_own_palette_and_an_action_needs_no_session() {
        let mut screen = screen();
        assert_eq!(screen.key(Key::Char(':')), KeyOut::Local);
        let picker = screen.picker.as_ref().expect("a palette");
        assert_eq!(picker.title, "Actions");
        assert!(
            picker
                .items()
                .iter()
                .any(|item| item.value == Action::ToggleVerbose.id())
        );
        // Choosing one acts here: no intent, and nothing to ask.
        let chosen = picker
            .items()
            .iter()
            .position(|item| item.value == Action::ToggleVerbose.id())
            .unwrap();
        let picker = screen.picker.as_mut().expect("a palette");
        for _ in 0..chosen {
            picker.move_selection(1);
        }
        assert_eq!(screen.key(Key::Submit), KeyOut::Local);
        assert!(screen.prefs.any_open());
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
        assert!(text_of(&screen, &view()).contains("│"));
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
        assert!(screen.picker.as_ref().is_some_and(Picker::is_inline));
        assert!(
            text.contains("more"),
            "a partial list was not reported: {text}"
        );
        assert!(text.contains("/model"), "{text}");
    }

    #[test]
    fn picker_copy_and_row_grammar_are_client_composition() {
        let mut screen = screen();
        screen.prefs.picker.hints = vec![crate::prefs::PickerHint {
            id: "accept".into(),
            key: "enter".into(),
            label: "choose it".into(),
            only_with_views: false,
            inline: true,
            overlay: true,
        }];
        screen.prefs.picker.row.selected_marker = "[x]".into();
        screen.prefs.picker.row.marker = "[ ]".into();
        screen.prefs.picker.row.marker_separator = "|".into();
        screen.prefs.picker.row.detail_separator = " :: ".into();
        screen.key(Key::Char('/'));
        let candidates = screen.command_candidates();
        screen
            .picker
            .as_mut()
            .expect("command picker")
            .set_items(candidates, true);
        let text = text_of(&screen, &view());
        assert!(text.contains("[x]|"), "{text}");
        assert!(text.contains("choose it"), "{text}");
        assert!(!text.contains("↑↓ more"), "{text}");
    }

    #[test]
    fn an_open_picker_overlays_the_transcript_and_owns_the_input_region() {
        let mut screen = screen();
        screen.key(Key::Action(Action::OpenCommands));
        let text = text_of(&screen, &view());
        assert!(
            text.contains("misa  ·  coding agent\n/ conversation   : actions   F1 help"),
            "{text}"
        );
        assert!(text.contains("Commands:"), "{text}");
        assert!(
            text.contains("hello"),
            "the picker hid the transcript instead of overlaying it: {text}"
        );
    }

    #[test]
    fn copying_is_the_clients_and_needs_no_session() {
        let mut screen = screen();
        screen.editor.set_mode(ed::Mode::Normal);
        match screen.selection_key(&view(), &Key::Char('y')) {
            Some(KeyOut::Copy(text)) => {
                assert!(text.contains("hello"), "{text}");
                assert!(text.contains("◇ echo"), "{text}");
            }
            other => panic!("expected a copy, got {other:?}"),
        }
        assert!(
            screen
                .notice
                .as_deref()
                .expect("a notice")
                .contains("copied")
        );
    }

    #[test]
    fn a_selection_covers_the_rendered_rows_it_was_dragged_over() {
        let mut screen = screen();
        screen.editor.set_mode(ed::Mode::Normal);
        let view = view();
        // `v` anchors at the bottom, which is where somebody following the tail is
        // looking; the first motion is what says how far back the range goes.
        assert_eq!(
            screen.selection_key(&view, &Key::Char('v')),
            Some(KeyOut::Local)
        );
        assert!(screen.selection.is_some(), "v opened no selection");
        screen.selection_key(&view, &Key::Motion(ed::Motion::First));
        match screen.selection_key(&view, &Key::Char('y')) {
            Some(KeyOut::Copy(text)) => {
                assert!(text.contains("hello"), "{text}");
                assert!(
                    !text.contains("echo (collapsed)"),
                    "the range covered more than it had: {text}"
                );
            }
            other => panic!("expected a copy, got {other:?}"),
        }
        assert!(
            screen.selection.is_none(),
            "the selection outlived the copy"
        );
    }

    #[test]
    fn a_selection_ends_on_escape_and_leaves_the_composer_alone() {
        let mut screen = screen();
        screen.editor.set_mode(ed::Mode::Normal);
        let view = view();
        screen.selection_key(&view, &Key::Char('v'));
        assert_eq!(
            screen.selection_key(&view, &Key::Escape),
            Some(KeyOut::Local)
        );
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
        assert_eq!(
            screen.selection_key(&view, &Key::Char('y')),
            None,
            "insert mode lost a keystroke"
        );
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
        assert_eq!(
            painted.as_deref(),
            Some("hello"),
            "the wrong bytes were highlighted"
        );
        assert!(
            text_of(&screen, &view).contains("hello"),
            "the highlight ate the text"
        );
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
        let key = translate(
            KeyCode::Enter,
            KeyModifiers::SHIFT,
            &crate::prefs::KeymapSettings::default(),
        )
        .unwrap();
        assert_eq!(screen.key(key), KeyOut::Local);
        assert_eq!(screen.editor.text(), "first\n");
        let key = translate(
            KeyCode::Enter,
            KeyModifiers::ALT,
            &crate::prefs::KeymapSettings::default(),
        )
        .unwrap();
        assert_eq!(
            screen.key(key),
            KeyOut::Intent(Intent::Interrupt {
                text: "first\n".into(),
                attachments: vec![]
            })
        );
        screen.editor.set_text("/clear");
        assert!(
            matches!(screen.key(Key::InterruptSubmit), KeyOut::Intent(Intent::Command { ref name, .. }) if name == "clear")
        );
        assert_eq!(
            translate(
                KeyCode::Char('r'),
                KeyModifiers::CONTROL,
                &crate::prefs::KeymapSettings::default(),
            ),
            Some(Key::HistorySearch)
        );
        assert_eq!(
            translate(
                KeyCode::Char('f'),
                KeyModifiers::ALT,
                &crate::prefs::KeymapSettings::default(),
            ),
            Some(Key::Action(Action::CycleEffort))
        );
        assert_eq!(
            translate(
                KeyCode::Char('m'),
                KeyModifiers::ALT,
                &crate::prefs::KeymapSettings::default(),
            ),
            Some(Key::Action(Action::OpenModel))
        );
        assert_eq!(
            translate(
                KeyCode::Char('t'),
                KeyModifiers::ALT,
                &crate::prefs::KeymapSettings::default(),
            ),
            Some(Key::Action(Action::ToggleVerbose))
        );
        assert_eq!(
            translate(
                KeyCode::Char('/'),
                KeyModifiers::ALT,
                &crate::prefs::KeymapSettings::default(),
            ),
            Some(Key::Action(Action::OpenCommands))
        );
        assert_eq!(
            translate(
                KeyCode::Home,
                KeyModifiers::CONTROL,
                &crate::prefs::KeymapSettings::default(),
            ),
            Some(Key::Action(Action::ScrollTop))
        );
        assert_eq!(
            translate(
                KeyCode::End,
                KeyModifiers::CONTROL,
                &crate::prefs::KeymapSettings::default(),
            ),
            Some(Key::Action(Action::ScrollBottom))
        );
    }

    #[test]
    fn configured_keymaps_drive_translation_and_palette_labels_together() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut keymap = crate::prefs::KeymapSettings::default();
        keymap
            .bindings
            .insert("model.open".into(), vec!["ctrl+x".into()]);
        assert_eq!(
            translate(KeyCode::Char('x'), KeyModifiers::CONTROL, &keymap),
            Some(Key::Action(Action::OpenModel))
        );
        assert_eq!(keymap.keys("model.open"), "ctrl+x");
        assert_eq!(
            translate(KeyCode::Char('m'), KeyModifiers::ALT, &keymap,),
            Some(Key::Alt('m'))
        );
    }

    #[test]
    fn a_slash_restarts_the_command_path_from_both_command_and_argument_pickers() {
        let mut command = screen();
        assert_eq!(command.key(Key::Char('/')), KeyOut::Local);
        for character in "model".chars() {
            assert_eq!(command.key(Key::Char(character)), KeyOut::Local);
        }
        assert_eq!(command.key(Key::Char('/')), KeyOut::Local);
        assert_eq!(command.editor.text(), "/");

        let mut argument = screen();
        argument.editor.set_text("/model");
        assert!(
            matches!(argument.key(Key::Tab), KeyOut::Complete { source, .. } if source == "models")
        );
        assert_eq!(argument.key(Key::Char('/')), KeyOut::Local);
        assert_eq!(argument.editor.text(), "/");
    }

    #[test]
    fn a_visual_editor_range_can_be_yanked_or_changed() {
        let mut screen = screen();
        screen.editor.set_text("hello");
        screen.key(Key::Escape);
        screen.key(Key::Char('v'));
        screen.key(Key::Motion(ed::Motion::Left));
        assert_eq!(screen.key(Key::Char('y')), KeyOut::Copy("o".into()));
        assert_eq!(screen.editor.mode(), ed::Mode::Normal);
    }

    #[test]
    fn the_reference_normal_mode_bindings_reach_the_shared_editor() {
        let mut screen = screen();
        type_text(&mut screen, "one two");
        screen.key(Key::Escape);
        screen.key(Key::Char('g'));
        screen.key(Key::Char('e'));
        assert_eq!(screen.editor.cursor(), 3);
        screen.key(Key::Char('0'));
        screen.key(Key::Char('y'));
        screen.key(Key::Char('w'));
        screen.key(Key::Char('G'));
        screen.key(Key::Char('p'));
        assert_eq!(screen.editor.text(), "one twoone ");
    }

    #[test]
    fn the_visual_editor_range_is_painted_in_the_composer() {
        let mut screen = screen();
        screen.editor.set_text("hello");
        screen.key(Key::Escape);
        screen.key(Key::Char('v'));
        screen.key(Key::Motion(ed::Motion::Left));
        let lines = draw(&screen, &view());
        let selection = screen.theme.role("selection");
        assert!(
            lines
                .iter()
                .flat_map(|line| line.spans.iter())
                .any(|(style, text)| style.bg == selection.bg && text == "o")
        );
    }
}

#[cfg(test)]
fn test_unique_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}
