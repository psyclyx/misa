//! The terminal frontend.
//!
//! It draws lines from the semantic tree, owns a theme, and sends intents. It
//! decides no agent behaviour: every key either moves the cursor, edits the line the
//! client owns, changes what the client is showing, or is turned into one of the four
//! things a client may say.
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
//! items to hold — see [`misa_client::picker`].

use std::collections::BTreeSet;
use std::time::Duration;

use misa_client::picker::{Accept, Effect as PickerEffect, Picker};
use misa_client::{editor as ed, intent as line};
use misa_proto::view::{Choice, Field, Kind, Node};
use misa_proto::wire::{Capabilities, Command, Intent, SessionInfo, Source, SourceKind};
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

/// What a keypress caused.
#[derive(Clone, Debug, PartialEq)]
pub enum KeyOut {
    /// The client handled it alone.
    Local,
    /// Something to ask a session.
    Intent(Intent),
    /// The picker needs candidates a session has and this client does not.
    Complete { source: String, prefix: String },
    /// Leave.
    Quit,
}

/// The client's whole state.
pub struct Screen {
    pub theme: Theme,
    pub editor: ed::Editor,
    /// The picker in front of the editor, when one is open.
    pub picker: Option<Picker>,
    /// Which command an accepted argument belongs to.
    pending_command: Option<String>,
    pub opened: BTreeSet<String>,
    pub notice: Option<String>,
    pub commands: Vec<Command>,
    pub sources: Vec<Source>,
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
            theme: Theme::dark(),
            editor: ed::Editor::new(),
            picker: None,
            pending_command: None,
            opened: BTreeSet::new(),
            notice: None,
            commands: Vec::new(),
            sources: Vec::new(),
            scroll: 0,
            follow: true,
            width,
            height,
        }
    }

    pub fn capabilities(&self) -> Capabilities {
        Capabilities::tui(self.width as u32, self.height as u32)
    }

    /// Take the session's declarations.
    ///
    /// Called once, from the welcome. After this the client knows what the session
    /// can do and where a value can come from, which is what makes a picker possible
    /// without asking anything.
    pub fn declare(&mut self, info: &SessionInfo) {
        self.commands = info.commands.clone();
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
        self.pending_command = Some(command.to_string());
        let accept = Accept::Argument { command: command.to_string(), argument: argument.to_string() };
        self.picker = Some(Picker::over(source, format!("/{command} {argument}"), accept));
    }

    /// Give the picker the items a source produced.
    pub fn candidates(&mut self, source: &str, items: Vec<Choice>, truncated: bool) {
        let resident = self.is_resident(source);
        if let Some(picker) = self.picker.as_mut()
            && picker.source.as_deref() == Some(source)
            && resident
        {
            picker.set_items(items, truncated);
        }
    }

    pub fn key(&mut self, key: Key) -> KeyOut {
        // A picker in front of the editor takes everything except the way out.
        if self.picker.is_some() {
            return self.picker_key(key);
        }
        match key {
            Key::Quit => KeyOut::Quit,
            Key::Escape => {
                self.editor.set_mode(ed::Mode::Normal);
                KeyOut::Local
            }
            Key::Interrupt => {
                // The draft is kept: an interrupt is about the model, not about what
                // somebody has typed.
                self.editor.interrupt();
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
            Key::Char(character) => {
                self.editor.type_char(character);
                KeyOut::Local
            }
            Key::Backspace => {
                self.editor.backspace();
                KeyOut::Local
            }
            Key::Delete => {
                self.editor.delete();
                KeyOut::Local
            }
            Key::Tab => self.complete_argument(),
            Key::Motion(motion) => {
                self.editor.move_cursor(motion);
                KeyOut::Local
            }
            Key::ScrollPage(delta) => {
                self.scroll_by(delta);
                KeyOut::Local
            }
            Key::Action(action) => self.action(action),
        }
    }

    /// Enter: submit what is there, or open the picker a declaration asks for.
    fn submit(&mut self) -> KeyOut {
        let text = self.editor.text().to_string();
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
        // `/` on an empty line is a request for the session's commands, and the
        // promise the declaration made is that they can be listed without asking.
        if self.sources.iter().any(|source| source.id == "commands") {
            let mut picker = Picker::over("commands", "Commands", Accept::Run);
            picker.set_items(self.command_candidates(), false);
            self.picker = Some(picker);
        } else {
            let mut picker = Picker::new("Commands", Accept::Run);
            picker.set_items(self.command_candidates(), false);
            self.picker = Some(picker);
        }
        KeyOut::Local
    }

    /// Open the client's own actions.
    fn open_action_palette(&mut self) -> KeyOut {
        let mut picker = Picker::new("Actions", Accept::Run);
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
            Key::Char(character) => picker.type_char(character),
            Key::Backspace => picker.backspace(),
            _ => PickerEffect::None,
        };
        self.picker_effect(effect)
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
                self.picker = None;
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
                // showing their long form is the client's to remember.
                if self.opened.is_empty() {
                    self.opened.insert("*".to_string());
                } else {
                    self.opened.clear();
                }
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
                KeyOut::Local
            }
            Action::ThemePlain => {
                self.theme = Theme::plain();
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
    /// they closed does the opposite. The session's `open` is a default and nothing
    /// more, which is why a theme change or a re-render never loses somebody's place.
    pub fn resolve(&self, node: &Node) -> Node {
        let mut node = node.clone();
        node.children = node.children.iter().map(|child| self.resolve(child)).collect();
        if let Kind::Collapsible { summary, open } = &node.kind {
            let open = *open || self.opened.contains(&node.id) || self.opened.contains("*");
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

/// A key, in the vocabulary the client cares about.
#[derive(Clone, Debug, PartialEq)]
pub enum Key {
    Char(char),
    Backspace,
    Delete,
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
    let mut body = misa_render::render(&resolved, &screen.theme, screen.width as usize);
    let chrome = if screen.picker.is_some() { 9 } else { 3 };
    let room = (screen.height as usize).saturating_sub(chrome);

    // Scrolling is presentation, so the client does it and nobody is told. Following
    // is the default and scrolling away stops it, which is what lets somebody read
    // while a model is still writing.
    if screen.follow {
        // Nothing: the tail is what matters and it is taken below.
    }
    let start = if screen.follow {
        body.len().saturating_sub(room)
    } else {
        screen.scroll.min(body.len().saturating_sub(1))
    };
    let mut lines: Vec<Line> = body.drain(..).skip(start).take(room).collect();

    if let Some(picker) = &screen.picker {
        lines.extend(picker_lines(screen, picker));
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
    lines
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
        if picker.query.is_empty() { String::new() } else { format!(" · {}", picker.query) },
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

/// What a frontend needs from a transport, so this binary can be tested and the
/// transport can be swapped.
#[async_trait::async_trait]
pub trait Session: Send {
    /// The next view, if one changed.
    async fn next(&mut self) -> Result<Option<Node>, String>;
    async fn send(&mut self, intent: Intent) -> Result<(), String>;
    /// Candidates a session holds, for a source this client asked about.
    async fn complete(&mut self, source: &str, prefix: &str) -> Result<(Vec<Choice>, bool), String>;
    fn info(&self) -> Option<SessionInfo>;
}

/// The interactive loop.
pub async fn run(session: &mut dyn Session) -> Result<(), String> {
    use crossterm::event;
    let mut screen = Screen::new(100, 40);
    if let Some(info) = session.info() {
        screen.declare(&info);
    }
    let mut stdout = std::io::stdout();
    crossterm::terminal::enable_raw_mode().map_err(|err| err.to_string())?;
    let result = loop {
        let Some(view) = session.next().await? else {
            break Ok(());
        };
        if let Err(error) = write(&mut stdout, &draw(&screen, &view)) {
            break Err(error);
        }
        if !event::poll(Duration::from_millis(1)).map_err(|err| err.to_string())? {
            continue;
        }
        let event = event::read().map_err(|err| err.to_string())?;
        let crossterm::event::Event::Key(key) = event else {
            continue;
        };
        let Some(interpreted) = translate(key.code, key.modifiers) else {
            continue;
        };
        match screen.key(interpreted) {
            KeyOut::Local => {}
            KeyOut::Quit => break Ok(()),
            KeyOut::Intent(intent) => {
                if let Err(error) = session.send(intent).await {
                    screen.notice = Some(error);
                }
            }
            KeyOut::Complete { source, prefix } => match session.complete(&source, &prefix).await {
                Ok((items, truncated)) => screen.candidates(&source, items, truncated),
                Err(error) => screen.notice = Some(error),
            },
        }
    };
    crossterm::terminal::disable_raw_mode().map_err(|err| err.to_string())?;
    result
}

fn translate(code: crossterm::event::KeyCode, modifiers: crossterm::event::KeyModifiers) -> Option<Key> {
    use crossterm::event::KeyCode;
    let control = modifiers.contains(crossterm::event::KeyModifiers::CONTROL);
    Some(match code {
        KeyCode::Char('q') if control => Key::Quit,
        KeyCode::Char('c') if control => Key::Interrupt,
        KeyCode::Char('t') if control => Key::Action(Action::ToggleDetail),
        KeyCode::Char('d') if control => Key::Delete,
        KeyCode::Char('u') if control => Key::ScrollPage(-10),
        KeyCode::Char(character) => Key::Char(character),
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
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

fn write(stdout: &mut std::io::Stdout, lines: &[Line]) -> Result<(), String> {
    use crossterm::{cursor, execute, terminal};
    use std::io::Write as _;

    execute!(stdout, terminal::Clear(terminal::ClearType::All), cursor::MoveTo(0, 0))
        .map_err(|err| err.to_string())?;
    for line in lines {
        let mut out = String::new();
        for (style, text) in &line.spans {
            out.push_str(&sgr(style));
            out.push_str(text);
            out.push_str("\u{1b}[0m");
        }
        write!(stdout, "{}{}\r\n", " ".repeat(line.indent as usize), out).map_err(|err| err.to_string())?;
    }
    stdout.flush().map_err(|err| err.to_string())
}

/// A style as an ANSI sequence. The one place a colour becomes bytes.
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

/// A session reached over iroh.
pub struct Remote {
    client: misa_net::iroh::Client,
    view: Option<Node>,
    info: Option<SessionInfo>,
}

impl Remote {
    pub async fn attach(ticket: &str) -> Result<Remote, String> {
        // A ticket, or a pairing string: whatever the daemon printed or the QR said.
        let (ticket, code) = misa_proto::Pairing::given(ticket)?;
        let endpoint = misa_net::iroh::bind_for(&ticket.node).await?;
        let address = misa_net::iroh::address_of(&ticket.node)?;
        if let Some(code) = &code {
            misa_net::iroh::Client::pair(&endpoint, address.clone(), code, "the tui").await?;
        }
        let info = misa_proto::ClientInfo::new("misa-tui", env!("CARGO_PKG_VERSION"), Capabilities::tui(100, 40));
        let mut client = misa_net::iroh::Client::connect(&endpoint, address, info, &ticket.session).await?;
        client
            .subscribe(misa_proto::SubId(1), misa_proto::Query::new(misa_session::views::VIEW_QUERY))
            .await?;
        // Every resident source the declaration offered, held once. This is the whole
        // cost of a picker that never asks again.
        client
            .subscribe(
                misa_proto::SubId(2),
                misa_proto::Query::new(misa_session::completions::CONVERSATIONS_QUERY),
            )
            .await?;
        for source in ["models", "effort", "commands"] {
            let query = match misa_session::completions::query_for(source) {
                Ok(query) => query,
                Err(fault) => return Err(fault.message),
            };
            client.subscribe(source_subscription(source), query).await?;
        }
        Ok(Remote { client, view: None, info: None })
    }
}

/// A subscription id per source, derived so two clients agree without negotiating.
fn source_subscription(source: &str) -> misa_proto::SubId {
    let hash = source.bytes().fold(0u32, |hash, byte| hash.wrapping_mul(31).wrapping_add(byte as u32));
    misa_proto::SubId(1000 + hash % 1000)
}

#[async_trait::async_trait]
impl Session for Remote {
    async fn next(&mut self) -> Result<Option<Node>, String> {
        loop {
            match self.client.next().await? {
                Some(misa_proto::SessionMsg::Welcome { session, .. }) => {
                    self.info = Some(session);
                }
                Some(misa_proto::SessionMsg::View { view, .. }) => {
                    self.view = Some(view.clone());
                    return Ok(Some(view));
                }
                Some(misa_proto::SessionMsg::Value { value, .. }) => {
                    // A resident source's items, held for the picker.
                    let _ = value;
                }
                Some(misa_proto::SessionMsg::Event { event, .. }) => {
                    if let misa_proto::SessionEvent::TextDelta { node, text } = event
                        && let Some(view) = self.view.as_mut()
                        && let Some(target) = find_mut(view, &node)
                        && let Kind::Text { spans } = &mut target.kind
                    {
                        spans.push(misa_proto::view::Span::plain(text));
                        return Ok(Some(view.clone()));
                    }
                }
                Some(misa_proto::SessionMsg::Fault { fault, .. }) => return Err(fault.message),
                Some(_) => {}
                None => return Ok(None),
            }
        }
    }

    async fn send(&mut self, intent: Intent) -> Result<(), String> {
        self.client.intent(next_intent_id(), intent).await
    }

    async fn complete(&mut self, source: &str, prefix: &str) -> Result<(Vec<Choice>, bool), String> {
        // A picker's ask is an intent with a reply, so it is sent and waited for.
        let id = next_intent_id();
        self.client
            .intent(
                id,
                Intent::Complete {
                    source: source.to_string(),
                    prefix: prefix.to_string(),
                    limit: None,
                },
            )
            .await?;
        loop {
            match self.client.next().await? {
                Some(misa_proto::SessionMsg::Completion { id: reply, candidates, truncated, .. })
                    if reply == id =>
                {
                    return Ok((candidates, truncated));
                }
                Some(misa_proto::SessionMsg::Fault { id: Some(reply), fault }) if reply == id => {
                    return Err(fault.message);
                }
                Some(_) => {}
                None => return Err("the session closed".into()),
            }
        }
    }

    fn info(&self) -> Option<SessionInfo> {
        self.info.clone()
    }
}

fn find_mut<'a>(node: &'a mut Node, id: &str) -> Option<&'a mut Node> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter_mut().find_map(|child| find_mut(child, id))
}

fn next_intent_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
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

/// A session this binary can drive with no network, for tests.
pub struct Local {
    pub runtime: std::sync::Arc<misa_session::Runtime>,
    pub seen: u64,
    pub info: Option<SessionInfo>,
}

impl Local {
    pub fn new(runtime: std::sync::Arc<misa_session::Runtime>) -> Local {
        let info = Some(runtime.info());
        Local { runtime, seen: 0, info }
    }
}

#[async_trait::async_trait]
impl Session for Local {
    async fn next(&mut self) -> Result<Option<Node>, String> {
        // A view is read whenever the revision moved, which is what a transport
        // watching a revision would do.
        if self.runtime.rev() == self.seen {
            tokio::time::sleep(Duration::from_millis(2)).await;
            return Ok(None);
        }
        self.seen = self.runtime.rev();
        match self.runtime.view(&Capabilities::tui(100, 40)) {
            Ok(view) => Ok(Some(view)),
            Err(fault) => Err(fault.message),
        }
    }

    async fn send(&mut self, intent: Intent) -> Result<(), String> {
        let faults = self.runtime.intent(intent);
        match faults.first() {
            Some(fault) => Err(fault.message.clone()),
            None => Ok(()),
        }
    }

    async fn complete(&mut self, source: &str, prefix: &str) -> Result<(Vec<Choice>, bool), String> {
        self.runtime.complete(source, prefix, None).map_err(|fault| fault.message)
    }

    fn info(&self) -> Option<SessionInfo> {
        self.info.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::wire::{Arg, SessionInfo};

    fn declaration() -> SessionInfo {
        SessionInfo {
            id: "demo".into(),
            title: "a demo".into(),
            conversation: None,
            created_ms: 0,
            policy: Vec::new(),
            queries: Vec::new(),
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
                    open: false,
                })
                .id("call.1")
                .child(Node::text("tool.result", [misa_proto::view::Span::plain("the result")])),
            )
    }

    fn text_of(screen: &Screen, view: &Node) -> String {
        misa_render::to_plain(&draw(screen, view))
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
        assert_eq!(picker.items().len(), 3);
        assert!(picker.items().iter().any(|item| item.value == "/model"));
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
}
