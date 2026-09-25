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

mod catalog;
pub use catalog::Catalog;

pub mod buttons;
pub mod chrome;
pub mod offline;
pub mod prefs;
pub mod presentation;
pub mod retained;
pub mod terminal_loop;

#[cfg(test)]
thread_local! { static RESOLVE_VISITS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

pub use crate::prefs::PreferencePersistence;
use crate::prefs::{PreferenceState, Prefs};
use crate::retained::Retained;
use catalog::ComposerCatalog;
pub use misa_kit::editor as ed;
use misa_kit::intent as line;
use misa_kit::intent::Command;
use misa_kit::intent::Intent;
use misa_kit::picker::Effect as PickerEffect;
pub use misa_kit::picker::{Accept, Picker};
use misa_lines::Line;
use misa_lines::select;
use misa_proto::view::{ActionOn, Choice, Field, Kind, Node};
use misa_render::{Theme, ThemeOverrides};
use misa_terminal_ui::graphics;

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
    Invoke {
        command: String,
        input: misa_value::Value,
    },
    /// A submitted line for a host-owned raw command. The host interprets it.
    Submitted(String),
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

/// UI-owned rendering of a dialog provided by a connected host.
#[derive(Default, Clone)]
pub struct DialogSurface {
    pub modal: bool,
    pub content: Vec<Line>,
}
impl DialogSurface {
    pub fn modal(&self) -> bool {
        self.modal
    }
    pub fn lines(
        &self,
        _theme: &Theme,
        _width: usize,
        _settings: &prefs::DialogSettings,
    ) -> Vec<Line> {
        self.content.clone()
    }
}

pub struct Screen {
    pub dialogs: DialogSurface,
    pub local_presentation: presentation::Local,
    pub components: misa_lines::components::Registry,
    pub values: misa_render::fact::Registry,
    operator: Option<char>,
    history_search: Option<HistorySearch>,
    pub theme: Theme,
    pub editor: ed::Editor,
    preferences: PreferenceState,
    /// The picker in front of the editor, when one is open.
    pub picker: Option<Picker>,
    /// Which command an accepted argument belongs to.
    pub pending_command: Option<String>,
    pub notice: Option<String>,
    /// The reader's selection, when one is open. It is over the rendered body, so
    /// moving it needs the view — which is why `selection_key` takes one.
    pub selection: Option<select::Selection>,
    /// What has been typed into an open panel, and which panel it is for.
    pub panel: Option<PanelInput>,
    catalog: ComposerCatalog,
    pub location: String,
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
            preferences: PreferenceState::default(),
            local_presentation: presentation::stock(),
            components: Default::default(),
            values: Default::default(),
            operator: None,
            history_search: None,
            theme: Theme::dark(),
            editor: ed::Editor::new(),
            picker: None,
            pending_command: None,
            notice: None,
            dialogs: Default::default(),
            selection: None,
            panel: None,
            catalog: ComposerCatalog::default(),
            location: String::new(),
            scroll: 0,
            follow: true,
            scroll_intent: 0,
            width,
            height,
            graphics: graphics::Kitty::new(false, graphics::CellSize::default()),
        }
    }

    /// Restore a document and attach the frontend's persistence capability.
    /// Neither construction nor saving chooses a filesystem path in the UI.
    pub fn remembering(prefs: Prefs, persistence: Box<dyn PreferencePersistence>) -> Screen {
        let mut screen = Screen::new(100, 40);
        screen.preferences = PreferenceState::remembering(prefs, persistence);
        screen.theme = screen.preferences.theme();
        screen.editor.set_text(screen.preferences.initial_draft());
        screen
    }

    pub fn remember_draft(&mut self) {
        self.preferences.remember_draft(self.editor.text());
    }
    pub fn enter_draft_scope(&mut self, scope: String) {
        let draft = self.preferences.enter_scope(scope, self.editor.text());
        self.editor.set_text(draft);
    }
    /// Activate a parked editor without overwriting its text with the saved draft.
    pub fn activate_draft_scope(&mut self, scope: String) {
        self.preferences.activate_scope(scope);
    }
    /// A newly visited scope has no parked editor; restore only its durable draft.
    pub fn restore_draft_scope(&mut self, scope: String) {
        let draft = self.preferences.restore_scope(scope);
        self.editor.set_text(draft);
    }
    pub fn set_draft_for(&mut self, scope: String, text: String) {
        self.preferences.set_draft_for(scope, text);
    }
    pub fn keymap(&self) -> &prefs::KeymapSettings {
        self.preferences.keymap()
    }
    pub fn dialog_settings(&self) -> &prefs::DialogSettings {
        self.preferences.dialogs()
    }
    pub fn component_settings(&self) -> &misa_lines::components::Settings {
        self.preferences.components()
    }
    pub fn picker_settings(&self) -> &prefs::PickerSettings {
        self.preferences.picker()
    }
    pub fn is_open(&self, id: &str) -> bool {
        self.preferences.is_open(id)
    }
    pub fn any_open(&self) -> bool {
        self.preferences.any_open()
    }
    pub fn opened(&self) -> &[String] {
        self.preferences.opened()
    }
    pub fn set_opened(&mut self, opened: Vec<String>) {
        self.preferences.set_opened(opened);
    }
    pub fn open_all(&mut self) {
        self.preferences.open_all();
    }

    /// Write what this client knows, and say so when it cannot.
    ///
    /// Called where a decision changed something and once on the way out — not on every
    /// keystroke: a write per character is a write per character. The draft is the one thing
    /// that waits for somebody to stop typing, which is the price of this being a file.
    pub fn save(&mut self) {
        if let Err(error) = self.preferences.save(self.editor.text()) {
            self.notice = Some(error);
        }
    }

    /// Replace local composer declarations after the selected scope's catalogs load.
    /// Owner metadata and connection status are supplied independently.
    pub fn declare(&mut self, info: &Catalog) {
        self.declare_with_raw(info, &[]);
    }

    /// Add host-owned commands whose arguments are interpreted by the host rather
    /// than the session's positional command parser. Session declarations win on id collisions.
    pub fn declare_with_raw(&mut self, info: &Catalog, raw: &[Command]) {
        self.catalog.declare_with_raw(info, raw);
    }

    /// The commands as candidates, built from the declaration.
    ///
    /// The declaration carries everything a candidate needs, so this costs nothing
    /// and works before any subscription has arrived. `completion.commands` exists
    /// as well, for a frontend that renders server-side and has no declaration in
    /// hand; the two say the same thing.
    pub fn command_candidates(&self) -> Vec<Choice> {
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
                    self.picker_settings()
                        .picker_views(source, misa_kit::picker::PickerPlacement::Inline),
                )
                .with_frecency(self.preferences.frecency())
                .with_favorites(self.preferences.favorites()),
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
    pub fn candidates(&mut self, source: &str, items: Vec<Choice>, truncated: bool) {
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
    pub fn completion(&mut self, source: &str, prefix: &str, items: Vec<Choice>, truncated: bool) {
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
    pub fn completion_failed(&mut self, source: &str) {
        self.catalog.failed(source);
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
            action.on == ActionOn::Click && self.dialog_settings().matches(&action.id, key)
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
                action.on == ActionOn::Submit && self.dialog_settings().matches(&action.id, key)
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
            Key::Submit if self.dialog_settings().matches("panel.submit", key) => {
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
                    .dialog_settings()
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
            self.save();
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

    fn open_command_picker(&mut self) -> KeyOut {
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
                .with_views(self.picker_settings().picker_views("commands", placement))
                .with_frecency(self.preferences.frecency())
                .with_favorites(self.preferences.favorites()),
        );
        KeyOut::Local
    }

    /// Open the client's own actions.
    fn open_action_palette(&mut self) -> KeyOut {
        self.editor.set_text(":");
        let mut picker = Picker::over("actions", "Actions", Accept::Run)
            .with_views(
                self.picker_settings()
                    .picker_views("actions", misa_kit::picker::PickerPlacement::Overlay),
            )
            .with_frecency(self.preferences.frecency())
            .with_favorites(self.preferences.favorites());
        picker.set_items(
            Action::ALL
                .iter()
                .filter(|action| match action.command_name() {
                    Some(command) => self.catalog.has_command(command),
                    None => true,
                })
                .map(|action| {
                    let keys = action.keys(self.keymap());
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

    pub fn paste(&mut self, text: &str) -> KeyOut {
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
                    self.preferences.remember_frecency(&frecency);
                }
                self.picker = None;
                self.preferences.remembered(&accepted.value);
                self.pending_command = None;
                self.save();
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
                self.preferences.set_favorite(&value, favorite);
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
                if self.any_open() {
                    self.preferences.close_all();
                } else {
                    self.open_all();
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
                self.theme = self.preferences.select_theme("dark");
                self.save();
                KeyOut::Local
            }
            Action::ThemeLight => {
                self.theme = self.preferences.select_theme("light");
                self.save();
                KeyOut::Local
            }
            Action::ThemePlain => {
                self.theme = self.preferences.select_theme("plain");
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
            self.picker_settings()
                .picker_views(source, misa_kit::picker::PickerPlacement::Overlay),
        )
        .with_frecency(self.preferences.frecency())
        .with_favorites(self.preferences.favorites());
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
            let open = self.is_open(&node.id);
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
pub fn panel_of(view: &Node) -> Option<&Node> {
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
    let settings = screen.picker_settings();
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
) -> Vec<(misa_style::Style, String)> {
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
    mut spans: Vec<(misa_style::Style, String)>,
    width: usize,
    style: misa_style::Style,
) -> Vec<(misa_style::Style, String)> {
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
            screen.picker_settings().row.inline_prefix.clone(),
        )];
        spans.extend(choice_spans(
            &screen.theme,
            &screen.picker_settings().row,
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
        let hints = picker_hints(screen.picker_settings(), picker, true);
        let mut spans = vec![(
            screen.theme.role("plain"),
            screen.picker_settings().row.inline_prefix.clone(),
        )];
        spans.extend(crate::buttons::key_reference(
            &screen.theme,
            &screen.picker_settings().hint_separator,
            hints
                .iter()
                .map(|hint| (hint.key.as_str(), hint.label.as_str())),
        ));
        spans.push((
            screen.theme.role("plain"),
            screen.picker_settings().hint_separator.clone(),
        ));
        spans.push((
            screen.theme.role("choice.hint"),
            screen.picker_settings().more_label.clone(),
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
pub fn select_highlight(line: &mut Line, from: usize, to: usize, theme: &Theme) {
    let indent = line.indent as usize;
    let (from, to) = (from.saturating_sub(indent), to.saturating_sub(indent));
    if to <= from {
        return;
    }
    let selected = theme.role("selection");
    let mut spans: Vec<(misa_style::Style, String)> = Vec::new();
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

pub fn translate(
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
    use misa_kit::intent::Source;
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
        screen.preferences.configure_dialogs(|dialogs| {
            dialogs.action_keys.insert("panel.close".into(), "q".into());
            dialogs
                .action_keys
                .insert("panel.submit".into(), "s".into());
        });

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
        assert_eq!(picker.items().len(), 3);
        assert_eq!(picker.views(), &[misa_kit::picker::PickerView::All]);
        assert!(!picker.items().iter().any(|item| item.value == "/save"));
        assert!(picker.items().iter().any(|item| item.value == "/model"));
    }

    #[test]
    fn host_raw_command_uses_the_same_picker_and_editor_submission_path() {
        let mut screen = screen();
        screen.declare_with_raw(
            &declaration(),
            &[Command::new("export", "Export", "host-owned syntax")],
        );
        assert_eq!(screen.key(Key::Char('/')), KeyOut::Local);
        assert!(
            screen
                .picker
                .as_ref()
                .unwrap()
                .items()
                .iter()
                .any(|item| item.value == "/export")
        );
        screen.picker = None;
        screen.editor.set_text("/export any unquoted path");
        assert_eq!(
            screen.key(Key::Submit),
            KeyOut::Submitted("/export any unquoted path".into())
        );
        assert!(screen.editor.is_empty());
        screen.editor.set_text("/unknown");
        assert_eq!(screen.key(Key::Submit), KeyOut::Local);
        assert_eq!(screen.editor.text(), "/unknown");
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
    fn empty_success_is_held_but_failure_can_retry_and_declaration_resets_requests() {
        let mut screen = screen();
        screen.editor.set_text("/model");
        assert!(matches!(screen.key(Key::Tab), KeyOut::Complete { .. }));
        screen.completion_failed("models");
        // A keystroke in the still-open picker retries after failure.
        assert_eq!(
            screen.key(Key::Char('x')),
            KeyOut::Complete {
                source: "models".into(),
                prefix: "x".into(),
            }
        );
        screen.completion("models", "x", vec![], false);
        screen.picker = None;
        assert_eq!(screen.key(Key::Tab), KeyOut::Local);
        screen.declare(&declaration());
        screen.picker = None;
        assert!(matches!(screen.key(Key::Tab), KeyOut::Complete { .. }));
        screen.declare(&declaration());
        // An answer from the abandoned request cannot fill the new scope's cache.
        screen.completion("models", "", vec![], false);
        screen.picker = None;
        assert!(matches!(screen.key(Key::Tab), KeyOut::Complete { .. }));
    }

    #[test]
    fn stale_completion_keeps_current_query_but_populates_resident_cache() {
        let mut screen = screen();
        screen.editor.set_text("/model");
        assert!(matches!(screen.key(Key::Tab), KeyOut::Complete { .. }));
        screen.key(Key::Char('n'));
        screen.completion(
            "models",
            "",
            vec![Choice {
                value: "new-model".into(),
                label: "New model".into(),
                detail: None,
                metadata: None,
            }],
            false,
        );
        assert_eq!(screen.picker.as_ref().unwrap().query, "n");
        assert!(screen.picker.as_ref().unwrap().items().is_empty());
        screen.picker = None;
        assert_eq!(screen.key(Key::Tab), KeyOut::Local);
        assert_eq!(
            screen.picker.as_ref().unwrap().items()[0].value,
            "new-model"
        );
    }

    #[test]
    fn session_command_wins_over_colliding_host_raw_command() {
        let mut screen = screen();
        screen.declare_with_raw(&declaration(), &[Command::new("clear", "Raw", "raw")]);
        screen.editor.set_text("/clear");
        assert_eq!(
            screen.key(Key::Submit),
            KeyOut::Intent(Intent::Command {
                name: "clear".into(),
                args: misa_value::Value::map([]),
            })
        );
    }

    #[test]
    fn quoted_argument_completion_keeps_prior_arguments_and_decoded_prefix() {
        let mut screen = screen();
        let mut info = declaration();
        info.commands.push(
            line::Command::new("visit", "Visit", "")
                .arg(line::Arg::new("daemon", "Daemon").required())
                .arg(
                    line::Arg::new("session", "Session")
                        .required()
                        .from("models"),
                ),
        );
        screen.declare(&info);
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

    #[derive(Clone, Default)]
    struct Memory(std::rc::Rc<std::cell::RefCell<Prefs>>);
    impl Memory {
        fn prefs(&self) -> Prefs {
            self.0.borrow().clone()
        }
        fn screen(&self, prefs: Prefs) -> Screen {
            Screen::remembering(prefs, Box::new(self.clone()))
        }
    }
    impl PreferencePersistence for Memory {
        fn update(&self, base: &Prefs, next: &Prefs) -> Result<(), String> {
            let mut current = self.0.borrow_mut();
            if base.theme != next.theme {
                current.theme = next.theme.clone();
            }
            if base.opened != next.opened {
                current.opened = next.opened.clone();
            }
            if base.draft != next.draft {
                current.draft = next.draft.clone();
            }
            if base.theme_overrides != next.theme_overrides {
                current.theme_overrides = next.theme_overrides.clone();
            }
            for (key, value) in &next.drafts {
                if base.drafts.get(key) != Some(value) {
                    current.drafts.insert(key.clone(), value.clone());
                }
            }
            for (key, value) in &next.frecency {
                if base.frecency.get(key) != Some(value) {
                    current.frecency.insert(key.clone(), *value);
                }
            }
            Ok(())
        }
    }

    #[test]
    fn persisted_drafts_follow_exact_scope_and_independent_windows_merge() {
        let memory = Memory::default();
        let mut first = memory.screen(Prefs::default());
        let mut second = memory.screen(Prefs::default());
        first.enter_draft_scope("peer-a:session:epoch1".into());
        first.editor.set_text("first");
        first.save();
        second.enter_draft_scope("peer-b:session:epoch1".into());
        second.editor.set_text("second");
        second.save();
        let prefs = memory.prefs();
        assert_eq!(prefs.drafts["peer-a:session:epoch1"], "first");
        assert_eq!(prefs.drafts["peer-b:session:epoch1"], "second");
        let mut reopened = memory.screen(prefs);
        reopened.enter_draft_scope("peer-a:session:epoch2".into());
        assert_eq!(reopened.editor.text(), "");
        reopened.enter_draft_scope("peer-a:session:epoch1".into());
        assert_eq!(reopened.editor.text(), "first");
    }
    #[test]
    fn a_client_that_remembers_starts_where_somebody_left_off() {
        // What "a restart forgets where somebody was" meant: the theme, the nodes they had
        // opened, and the draft were in memory and nowhere else.
        let memory = Memory::default();
        let mut first = memory.screen(Prefs::default());
        assert_eq!(
            first.theme.name, "dark",
            "a client that remembers nothing opens dark"
        );
        first.key(Key::Action(Action::ThemePlain));
        first.key(Key::Action(Action::ToggleVerbose));
        type_text(&mut first, "half a question");
        first.save();

        let second = memory.screen(memory.prefs());
        // The theme somebody chose is the one they are drawn with next time.
        assert_eq!(second.theme.name, "plain");
        assert_eq!(second.editor.text(), "half a question");
        // And the opened node is open in what it draws, not only in what it remembers.
        assert!(
            text_of(&second, &view()).contains("the result"),
            "{}",
            text_of(&second, &view())
        );
    }

    #[test]
    fn what_was_sent_is_not_resurrected_by_the_next_run() {
        let memory = Memory::default();
        let mut screen = memory.screen(Prefs::default());
        type_text(&mut screen, "send me");
        assert!(matches!(screen.key(Key::Submit), KeyOut::Intent(_)));
        assert!(screen.editor.is_empty());
        // The line went out, so the memory of it goes out with it.
        let next = memory.screen(memory.prefs());
        assert_eq!(next.editor.text(), "");
    }

    #[test]
    fn what_somebody_reaches_for_ranks_the_next_list() {
        // Frecency was a picker's own memory, which meant it was ranked by nothing the second
        // time a client started. It is the client's memory now.
        let memory = Memory::default();
        let mut first = memory.screen(Prefs::default());
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

        let mut second = memory.screen(memory.prefs());
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
        let memory = Memory::default();
        let mut prefs = Prefs::default();
        prefs.theme_overrides.roles.insert(
            "error".into(),
            misa_style::StylePatch {
                fg: Some(misa_style::Color::Rgb(1, 2, 3)),
                ..misa_style::StylePatch::default()
            },
        );
        let screen = memory.screen(prefs);
        assert_eq!(
            screen.theme.role("error").fg,
            misa_style::Color::Rgb(1, 2, 3)
        );
        // Switching the base theme keeps the override.
        let mut screen = screen;
        screen.key(Key::Action(Action::ThemeLight));
        assert_eq!(
            screen.theme.role("error").fg,
            misa_style::Color::Rgb(1, 2, 3)
        );
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
        assert!(screen.any_open());
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
        screen.preferences.configure_picker(|picker| {
            picker.hints = vec![crate::prefs::PickerHint {
                id: "accept".into(),
                key: "enter".into(),
                label: "choose it".into(),
                only_with_views: false,
                inline: true,
                overlay: true,
            }];
            picker.row.selected_marker = "[x]".into();
            picker.row.marker = "[ ]".into();
            picker.row.marker_separator = "|".into();
            picker.row.detail_separator = " :: ".into();
        });
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
