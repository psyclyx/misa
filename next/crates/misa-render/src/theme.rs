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
        Style {
            fg: color,
            ..Style::PLAIN
        }
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
            fg: if other.fg == Color::Default {
                self.fg
            } else {
                other.fg
            },
            bg: if other.bg == Color::Default {
                self.bg
            } else {
                other.bg
            },
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
        set("header.title", Style::fg(hex(0x81bdb5)).bold());
        set("header.detail", Style::fg(hex(0x84919c)).dim());
        set("session.header", Style::fg(hex(0x84919c)));
        set("session.title", Style::fg(hex(0x81bdb5)).bold());
        // Message bodies use the text surface; only their rails carry the role
        // colour. This is the reference's distinction between readable prose
        // and the small marker that identifies its speaker.
        set("message.user", Style::PLAIN);
        set("surface.user", Style::PLAIN.on(hex(0x1d2824)));
        set("message.user.rail", Style::fg(hex(0x93b99a)));
        set("message.assistant", Style::PLAIN);
        set("surface.assistant", Style::PLAIN.on(hex(0x1e252f)));
        set("message.assistant.rail", Style::fg(hex(0x8dafd2)));
        set("message.thinking", Style::PLAIN.dim());
        set("surface.thinking", Style::PLAIN.on(hex(0x282330)));
        set("message.thinking.rail", Style::fg(hex(0xb8a1c9)));
        // A model reasoning aloud: present, readable, and clearly not the answer.
        set("message.assistant.thinking", Style::PLAIN.dim());
        set("message.assistant.thinking.rail", Style::fg(hex(0xb8a1c9)));
        // What the session told the model on its own: a background command finishing is the
        // only thing that speaks this way, and it should read as a footnote rather than prose.
        set("message.system", Style::fg(hex(0x84919c)).dim());
        set("user", Style::PLAIN);
        set("text", Style::PLAIN);
        set("dim", Style::PLAIN.dim());
        set("tool.call", Style::PLAIN);
        set("surface.tool", Style::PLAIN.on(hex(0x2b2820)));
        set("tool.call.rail", Style::fg(hex(0xc9b07f)));
        set("tool.result", Style::PLAIN);
        set("tool.result.rail", Style::fg(hex(0xc9b07f)));
        set("notice", Style::fg(hex(0x84919c)));
        set("error", Style::fg(hex(0xde9397)));
        set("surface.error", Style::PLAIN.on(hex(0x322329)));
        set("error.rail", Style::fg(hex(0xde9397)));
        set("status", Style::fg(hex(0x84919c)).dim());
        set("composer", Style::PLAIN);
        set("mode.insert", Style::fg(hex(0x81bdb5)).bold());
        set("mode.normal", Style::fg(hex(0x81bdb5)).bold());
        set("mode.visual", Style::fg(hex(0x81bdb5)).bold());
        set("palette.title", Style::fg(hex(0x81bdb5)).bold());
        set(
            "palette.item.selected",
            Style::PLAIN.on(hex(0x3b5260)).bold(),
        );
        set("palette.item", Style::PLAIN);
        set("palette.hint", Style::PLAIN.dim());
        // Names used by the old component vocabulary. Keeping these as theme
        // roles lets the terminal renderer use the same semantic styles as a
        // windowed client without copying a second palette into the picker.
        set("choice.prompt", Style::fg(hex(0x81bdb5)));
        set("choice.query", Style::PLAIN);
        set("choice.row", Style::PLAIN);
        set("choice.row.active", Style::fg(hex(0x81bdb5)));
        set("choice.row.selected", Style::PLAIN.on(hex(0x3b5260)).bold());
        set("choice.hint", Style::PLAIN.dim());
        set("choice.view", Style::PLAIN.bold());
        set("choice.view.active", Style::fg(hex(0x81bdb5)).bold());
        set("choice.empty", Style::PLAIN.dim());
        set("label", Style::PLAIN.dim());
        set("value", Style::PLAIN);
        set("keybinding", Style::fg(hex(0x81bdb5)).dim());
        set("pending", Style::fg(hex(0x81bdb5)).dim());
        set("plain", Style::PLAIN);
        set("value.context", Style::fg(hex(0x9dbfb5)));
        set("value.money", Style::fg(hex(0xd0bb86)));
        set("turn", Style::fg(hex(0x81bdb5)));
        set("indicator", Style::fg(hex(0x84919c)));
        set("indicator.activity", Style::fg(hex(0x81bdb5)));
        set("indicator.model", Style::fg(hex(0x8dafd2)));
        set("indicator.context", Style::fg(hex(0x9dbfb5)));
        set("status.separator", Style::fg(hex(0x84919c)));
        set("message.group.footer", Style::PLAIN.dim());
        set("dialog", Style::fg(hex(0x84919c)));
        set("dialog.label", Style::fg(hex(0x84919c)).dim());
        set("dialog.message", Style::PLAIN);
        set("dialog.hint", Style::PLAIN.dim());
        set("dialog.value", Style::PLAIN);
        set("dialog.title", Style::fg(hex(0x81bdb5)).bold());
        set("surface.dialog", Style::PLAIN.on(hex(0x242b33)));
        set("surface.code", Style::PLAIN.on(hex(0x151b23)));
        // A reader's selection. A background rather than a foreground, because it has
        // to sit over whatever the role underneath already decided.
        set("selection", Style::PLAIN.on(hex(0x3b5260)));
        // A diff. Named per line rather than once, because what changed is the whole point
        // of looking at one: an added line, a removed one, the hunk header, and the file
        // headers that say what the hunks are hunks of. A body is a diff because the
        // session said so — a role that ends in `.diff`, or a fence that said `diff` — so
        // nothing here has to guess.
        set("diff", Style::fg(hex(0x84919c)));
        set("diff.add", Style::fg(hex(0x97c49e)));
        set("diff.remove", Style::fg(hex(0xde9397)));
        set("diff.hunk", Style::fg(hex(0x86bfc4)).dim());
        set("diff.header", Style::fg(hex(0x84919c)).bold());
        set("diff.meta", Style::fg(hex(0x84919c)).dim());
        set("value.money", Style::fg(hex(0xd0bb86)));
        set("value.count", Style::fg(hex(0x84919c)));

        let mut tokens = BTreeMap::new();
        let mut token = |name: &str, style: Style| {
            tokens.insert(name.to_string(), style);
        };
        token("comment", Style::fg(hex(0x84919c)).italic());
        token("string", Style::fg(hex(0xa7c799)));
        token("number", Style::fg(hex(0xc2a2c9)));
        token("keyword", Style::fg(hex(0x86bfc4)));
        token("type", Style::fg(hex(0xd0bb86)));
        token("function", Style::fg(hex(0x96b5da)));
        token("constant", Style::fg(hex(0xb4a4da)));
        token("variable", Style::fg(hex(0x84919c)));
        token("property", Style::fg(hex(0x9dbfb5)));
        token("tag", Style::fg(hex(0xde9397)));
        token("attribute", Style::fg(hex(0xd0bb86)));
        token("operator", Style::fg(hex(0x86bfc4)));
        token("punctuation", Style::fg(hex(0x84919c)));
        token("escape", Style::fg(hex(0xd5b783)));
        token("embedded", Style::fg(hex(0x84919c)));

        let mut states = BTreeMap::new();
        states.insert(State::Pending, Style::fg(hex(0x81bdb5)).dim());
        states.insert(State::Streaming, Style::fg(hex(0x97c49e)));
        states.insert(State::Done, Style::fg(hex(0x84919c)));
        states.insert(State::Failed, Style::fg(hex(0xde9397)));
        states.insert(State::Cancelled, Style::fg(hex(0x84919c)).dim());

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

    /// The same semantic accents, with darker foregrounds for light surfaces.
    pub fn light() -> Theme {
        let mut theme = Self::dark();
        theme.name = "light".into();
        for style in theme
            .roles
            .values_mut()
            .chain(theme.tokens.values_mut())
            .chain(theme.states.values_mut())
            .chain(std::iter::once(&mut theme.default))
        {
            style.fg = match style.fg {
                Color::Default => Color::Rgb(28, 32, 40),
                Color::Rgb(r, g, b) => Color::Rgb(r / 2, g / 2, b / 2),
                value => value,
            };
        }
        // The light reference is not merely dark colours with their foregrounds
        // dimmed: its message/dialog/code surfaces are deliberately paper-like.
        for (role, color) in [
            ("surface.user", (238, 245, 240)),
            ("surface.assistant", (238, 242, 248)),
            ("surface.thinking", (244, 239, 247)),
            ("surface.tool", (247, 243, 233)),
            ("surface.error", (250, 238, 240)),
            ("surface.dialog", (238, 242, 244)),
            ("surface.code", (226, 232, 239)),
            ("selection", (205, 223, 227)),
        ] {
            if let Some(style) = theme.roles.get_mut(role) {
                style.bg = Color::Rgb(color.0, color.1, color.2);
            }
        }
        theme
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

    /// Whether this theme names a role *itself*, rather than inheriting one.
    ///
    /// [`Theme::role`] answers "what does this look like", which is the right question
    /// almost everywhere. This is for the one place where the answer is not enough: a
    /// renderer that wants to fall back to a *generic* role when a theme has said
    /// nothing about a specific one, rather than to whatever the inherited prefix
    /// happens to be.
    pub fn names(&self, role: &str) -> bool {
        self.roles.contains_key(role)
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

    /// Resolve the named full-row surface for a semantic role.
    ///
    /// Surfaces are deliberately resolved independently from foreground roles:
    /// markdown/code spans and rails may vary without changing the continuous
    /// background of the message that owns them.
    pub fn surface(&self, role: &str) -> Option<Style> {
        let name = if role == "code" || role.ends_with(".code") {
            "surface.code"
        } else if role.starts_with("message.user") {
            "surface.user"
        } else if role.starts_with("message.assistant.thinking")
            || role.starts_with("message.thinking")
        {
            "surface.thinking"
        } else if role.starts_with("message.assistant") {
            "surface.assistant"
        } else if role.starts_with("tool.") {
            "surface.tool"
        } else if role.starts_with("error") {
            "surface.error"
        } else if role.starts_with("dialog") {
            "surface.dialog"
        } else {
            return None;
        };
        let style = self.role(name);
        (style.bg != Color::Default).then_some(Style::PLAIN.on(style.bg))
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
        assert_eq!(theme.role("message.user"), Style::PLAIN);
        assert_ne!(theme.surface("message.user"), None);
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
        assert_eq!(
            theme.role("message.handoff"),
            Style::fg(hex(0xaa00aa)).bold()
        );
        // And a role under it still finds it.
        assert_eq!(
            theme.role("message.handoff.detail"),
            Style::fg(hex(0xaa00aa)).bold()
        );
    }
}
