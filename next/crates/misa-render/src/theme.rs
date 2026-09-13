//! The client's theme: the only place a colour is decided.
//!
//! A theme is a map from a *role* — the same open, dotted name a view node carries
//! — to a style. Lookup walks the role's dotted prefixes, so a theme may name
//! `message.assistant.tool` or `message.assistant` or `message` and get the
//! resolution it asked for. That is what lets a plugin introduce
//! `message.handoff` and still be legible in a theme that has never heard of it:
//! the nearest named ancestor wins, and the default is always available.
//!
//! Nothing in a theme can reach a session. This is presentation, it lives on the
//! client, and a session that wanted to influence it would need a new field in a
//! message type that deliberately has none.

use std::collections::BTreeMap;

use misa_proto::view::State;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    /// Whatever the medium's default is.
    Default,
    /// A terminal palette entry, for a theme that wants to follow the terminal.
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style::PLAIN
    }
}

impl Style {
    pub const PLAIN: Style = Style {
        fg: Color::Default,
        bg: Color::Default,
        bold: false,
        dim: false,
        italic: false,
        underline: false,
        strikethrough: false,
    };

    pub fn fg(color: Color) -> Style {
        Style { fg: color, ..Style::PLAIN }
    }

    pub fn rgb(r: u8, g: u8, b: u8) -> Style {
        Style::fg(Color::Rgb(r, g, b))
    }

    pub fn bold(mut self) -> Style {
        self.bold = true;
        self
    }

    pub fn dim(mut self) -> Style {
        self.dim = true;
        self
    }

    pub fn italic(mut self) -> Style {
        self.italic = true;
        self
    }

    pub fn underline(mut self) -> Style {
        self.underline = true;
        self
    }

    pub fn on(mut self, color: Color) -> Style {
        self.bg = color;
        self
    }

    /// Merge: the other style's non-default fields win.
    pub fn over(self, other: Style) -> Style {
        Style {
            fg: if other.fg == Color::Default { self.fg } else { other.fg },
            bg: if other.bg == Color::Default { self.bg } else { other.bg },
            bold: self.bold || other.bold,
            dim: self.dim || other.dim,
            italic: self.italic || other.italic,
            underline: self.underline || other.underline,
            strikethrough: self.strikethrough || other.strikethrough,
        }
    }
}

fn hex(value: u32) -> Color {
    Color::Rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}

/// A role-addressed set of styles.
#[derive(Clone, Debug)]
pub struct Theme {
    pub name: String,
    roles: BTreeMap<String, Style>,
    tokens: BTreeMap<String, Style>,
    states: BTreeMap<State, Style>,
    default: Style,
}

impl Theme {
    /// The shipped theme: terminal-default body text, a few muted accents.
    ///
    /// Deliberately small. A theme that names every role it might meet is a theme
    /// that has to change when a plugin appears; this one names the roles the
    /// shipped session emits and lets everything else fall back through its
    /// prefix.
    pub fn dark() -> Theme {
        let mut roles = BTreeMap::new();
        let mut set = |role: &str, style: Style| {
            roles.insert(role.to_string(), style);
        };
        set("", Style::PLAIN);
        set("session.header", Style::fg(hex(0x8a8f98)));
        set("session.title", Style::fg(hex(0xe6e8ea)).bold());
        set("message.user", Style::fg(hex(0xd7dbe0)));
        set("message.user.rail", Style::fg(hex(0x6f7bd6)));
        set("message.assistant", Style::fg(hex(0xe9ebee)));
        set("message.assistant.rail", Style::fg(hex(0x64b5a0)));
        set("message.thinking", Style::fg(hex(0x9aa2ad)).italic());
        set("message.thinking.rail", Style::fg(hex(0x5b6270)));
        // A model reasoning aloud: present, readable, and clearly not the answer.
        set("message.assistant.thinking", Style::fg(hex(0x8a919c)).italic());
        set("message.assistant.thinking.rail", Style::fg(hex(0x4d5462)));
        // What the session told the model on its own: a background command finishing is the
        // only thing that speaks this way, and it should read as a footnote rather than prose.
        set("message.system", Style::fg(hex(0x8a8f98)).dim());
        set("tool.call", Style::fg(hex(0xd3d7dc)));
        set("tool.call.rail", Style::fg(hex(0xc9a227)));
        set("tool.result", Style::fg(hex(0xb9bec6)));
        set("tool.result.rail", Style::fg(hex(0x8a8f98)));
        set("notice", Style::fg(hex(0x8a8f98)));
        set("error", Style::fg(hex(0xe06c75)));
        set("error.rail", Style::fg(hex(0xe06c75)));
        set("status", Style::fg(hex(0x7f8690)).dim());
        set("composer", Style::fg(hex(0xe6e8ea)));
        set("dialog", Style::fg(hex(0xe6e8ea)));
        set("dialog.title", Style::fg(hex(0xe6e8ea)).bold());
        set("value.money", Style::fg(hex(0xc9a227)));
        set("value.count", Style::fg(hex(0x8a8f98)));

        let mut tokens = BTreeMap::new();
        let mut token = |name: &str, style: Style| {
            tokens.insert(name.to_string(), style);
        };
        token("comment", Style::fg(hex(0x6a7178)).italic());
        token("string", Style::fg(hex(0x98c379)));
        token("number", Style::fg(hex(0xd19a66)));
        token("keyword", Style::fg(hex(0xc678dd)));
        token("type", Style::fg(hex(0xe5c07b)));
        token("function", Style::fg(hex(0x61afef)));
        token("constant", Style::fg(hex(0xd19a66)));
        token("variable", Style::fg(hex(0xe9ebee)));
        token("property", Style::fg(hex(0x9ecbff)));
        token("tag", Style::fg(hex(0xe06c75)));
        token("attribute", Style::fg(hex(0xd19a66)));
        token("operator", Style::fg(hex(0x56b6c2)));
        token("punctuation", Style::fg(hex(0x8a8f98)));
        token("escape", Style::fg(hex(0x56b6c2)));
        token("embedded", Style::fg(hex(0xe9ebee)));

        let mut states = BTreeMap::new();
        states.insert(State::Pending, Style::fg(hex(0x8a8f98)).dim());
        states.insert(State::Streaming, Style::fg(hex(0x64b5a0)));
        states.insert(State::Done, Style::fg(hex(0x5b6270)));
        states.insert(State::Failed, Style::fg(hex(0xe06c75)));
        states.insert(State::Cancelled, Style::fg(hex(0x8a8f98)).dim());

        Theme {
            name: "dark".into(),
            roles,
            tokens,
            states,
            default: Style::PLAIN,
        }
    }

    /// No colour at all. For a pipe, a log, or a test that should not depend on
    /// the theme to read its own output.
    pub fn plain() -> Theme {
        Theme {
            name: "plain".into(),
            roles: BTreeMap::new(),
            tokens: BTreeMap::new(),
            states: BTreeMap::new(),
            default: Style::PLAIN,
        }
    }

    pub fn with_role(mut self, role: &str, style: Style) -> Theme {
        self.roles.insert(role.to_string(), style);
        self
    }

    pub fn with_token(mut self, name: &str, style: Style) -> Theme {
        self.tokens.insert(name.to_string(), style);
        self
    }

    /// Resolve a role, longest matching dotted prefix first.
    pub fn role(&self, role: &str) -> Style {
        let mut candidate = role;
        loop {
            if let Some(style) = self.roles.get(candidate) {
                return *style;
            }
            match candidate.rfind('.') {
                Some(index) => candidate = &candidate[..index],
                None => return self.roles.get("").copied().unwrap_or(self.default),
            }
        }
    }

    /// Resolve a syntax capture name. An unknown name is plain text, because a
    /// grammar may know more than a theme does.
    pub fn token(&self, name: &str) -> Style {
        self.tokens.get(name).copied().unwrap_or(self.default)
    }

    /// The style for a node's state marker. `None` when the state has no marker.
    pub fn state(&self, state: Option<State>) -> Option<Style> {
        state.and_then(|state| self.states.get(&state).copied())
    }

    /// The single character a state is announced with.
    ///
    /// A glyph rather than a word, so a one-line status does not jump in width
    /// when a tool finishes. A client without Unicode simply does not draw it.
    pub fn state_mark(&self, state: State) -> &'static str {
        match state {
            State::Pending => "…",
            State::Streaming => "▏",
            State::Done => "✓",
            State::Failed => "!",
            State::Cancelled => "×",
        }
    }

    /// The rail glyph for a role that has one, and its style.
    ///
    /// A rail is the vertical bar the previous system's transcript used to make a
    /// message's extent visible. It is presentation, entirely: a session says
    /// which role a node is, and the theme decides that a user message has a blue
    /// bar and a thinking block a grey one.
    pub fn rail(&self, role: &str) -> Option<(String, Style)> {
        let style = self.roles.get(&format!("{role}.rail")).copied()?;
        Some(("┃ ".to_string(), style))
    }

    /// Every role this theme names, for a diagnostics view.
    pub fn roles(&self) -> impl Iterator<Item = &str> {
        self.roles.keys().map(String::as_str)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::dark()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_role_resolves_to_its_nearest_named_ancestor() {
        let theme = Theme::dark();
        assert_ne!(theme.role("message.user"), Style::PLAIN);
        // A role nobody named, under one somebody did.
        let resolved = theme.role("message.assistant.handoff");
        assert_eq!(resolved, theme.role("message.assistant"));
        // A role with no named ancestor at all gets the default.
        assert_eq!(theme.role("plugin.unknown.thing"), Style::PLAIN);
    }

    #[test]
    fn an_unknown_capture_is_plain_text() {
        let theme = Theme::dark();
        assert_eq!(theme.token("keyword"), theme.token("keyword"));
        assert_ne!(theme.token("keyword"), Style::PLAIN);
        assert_eq!(theme.token("whatever-a-new-grammar-says"), Style::PLAIN);
    }

    #[test]
    fn the_plain_theme_decides_nothing() {
        let theme = Theme::plain();
        assert_eq!(theme.role("message.user"), Style::PLAIN);
        assert_eq!(theme.token("keyword"), Style::PLAIN);
        assert_eq!(theme.state(Some(State::Failed)), None);
    }

    #[test]
    fn a_state_has_a_marker_and_a_style() {
        let theme = Theme::dark();
        assert_eq!(theme.state_mark(State::Failed), "!");
        assert!(theme.state(Some(State::Failed)).is_some());
        assert_eq!(theme.state(None), None);
    }

    #[test]
    fn a_rail_is_optional_and_only_exists_where_a_theme_names_one() {
        let theme = Theme::dark();
        assert!(theme.rail("message.user").is_some());
        assert!(theme.rail("plugin.unknown").is_none());
    }

    #[test]
    fn merging_lets_a_state_tint_a_role_without_replacing_it() {
        let base = Style::rgb(1, 2, 3);
        let tint = Style::PLAIN.on(Color::Rgb(9, 9, 9)).bold();
        let merged = base.over(tint);
        assert_eq!(merged.fg, Color::Rgb(1, 2, 3));
        assert_eq!(merged.bg, Color::Rgb(9, 9, 9));
        assert!(merged.bold);
    }

    #[test]
    fn a_theme_can_be_extended_for_a_plugin_role() {
        let theme = Theme::dark().with_role("message.handoff", Style::fg(hex(0xaa00aa)).bold());
        assert_eq!(theme.role("message.handoff"), Style::fg(hex(0xaa00aa)).bold());
        // And a role under it still finds it.
        assert_eq!(theme.role("message.handoff.detail"), Style::fg(hex(0xaa00aa)).bold());
    }
}

