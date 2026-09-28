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
use serde::{Deserialize, Serialize};

use misa_style::{Color, Style, StylePatch};

/// Client-owned attribute patches over the selected theme.
///
/// This is the rewrite's form of the previous system's `config.styles`/`config.palette`:
/// presentation overrides that never reach a session. A name that the theme does
/// not define is applied anyway, so a client can prepare a role a plugin has not
/// emitted yet; a name that is never used costs nothing.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ThemeOverrides {
    /// Named-colour patches, applied by rebuilding the theme before the role
    /// patches below, so `accent` recolours every role that references it.
    pub palette: BTreeMap<String, Color>,
    pub roles: BTreeMap<String, StylePatch>,
    pub tokens: BTreeMap<String, StylePatch>,
    pub states: BTreeMap<String, StylePatch>,
}

impl ThemeOverrides {
    pub fn is_empty(&self) -> bool {
        self.palette.is_empty()
            && self.roles.is_empty()
            && self.tokens.is_empty()
            && self.states.is_empty()
    }
}

/// The global role a GitHub-alert quote resolves through, when `role` names one.
///
/// The parser prefixes every role it emits with the containing message's role
/// (`message.assistant.markdown.alert.note`), and dotted lookup walks *prefixes*,
/// so a theme's `markdown.alert.note` is never reached on its own. A renderer
/// extracts the suffix here and folds it in explicitly, the same way it does for
/// a heading level's `markdown.heading.<level>`.
pub fn alert_role(role: &str) -> Option<&str> {
    let index = role.find("markdown.alert.")?;
    Some(&role[index..])
}

/// A state's name, which is how an override reaches one.
fn state_from_name(name: &str) -> Option<State> {
    match name {
        "pending" => Some(State::Pending),
        "streaming" => Some(State::Streaming),
        "done" => Some(State::Done),
        "failed" => Some(State::Failed),
        "cancelled" => Some(State::Cancelled),
        _ => None,
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
    /// The named colours this theme was built from, so a palette override can
    /// rebuild it without every role restating its own colour.
    palette: Palette,
}

/// The named colours a theme is built from.
///
/// This is the rewrite's form of the previous system's `theme.palette`: a role
/// references a name, and overriding the name recolours every role that uses it.
/// The names are semantic rather than literal, so a theme author says `accent`
/// rather than a hex value a second time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Palette {
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    pub user: Color,
    pub assistant: Color,
    pub thinking: Color,
    pub tool: Color,
    pub error: Color,
    pub success: Color,
    pub property: Color,
    #[serde(rename = "type")]
    pub type_color: Color,
    pub keyword: Color,
    pub string: Color,
    pub number: Color,
    pub function: Color,
    pub constant: Color,
    pub escape: Color,
    pub selection: Color,
    pub user_surface: Color,
    pub assistant_surface: Color,
    pub thinking_surface: Color,
    pub tool_surface: Color,
    pub error_surface: Color,
    pub dialog_surface: Color,
    pub code_surface: Color,
}

impl Palette {
    pub fn dark() -> Palette {
        Palette {
            text: Color::Default,
            muted: hex(0x84919c),
            accent: hex(0x81bdb5),
            user: hex(0x93b99a),
            assistant: hex(0x8dafd2),
            thinking: hex(0xb8a1c9),
            tool: hex(0xc9b07f),
            error: hex(0xde9397),
            success: hex(0x97c49e),
            property: hex(0x9dbfb5),
            type_color: hex(0xd0bb86),
            keyword: hex(0x86bfc4),
            string: hex(0xa7c799),
            number: hex(0xc2a2c9),
            function: hex(0x96b5da),
            constant: hex(0xb4a4da),
            escape: hex(0xd5b783),
            selection: hex(0x3b5260),
            user_surface: hex(0x25322c),
            assistant_surface: hex(0x262e39),
            thinking_surface: hex(0x312b3c),
            tool_surface: hex(0x343026),
            error_surface: hex(0x3d2c33),
            dialog_surface: hex(0x2d343e),
            code_surface: hex(0x1d252e),
        }
    }

    /// The previous system's light reference, as an explicit palette rather than
    /// dark colours with their foregrounds dimmed.
    pub fn light() -> Palette {
        Palette {
            text: Color::Default,
            muted: hex(0x68757f),
            accent: hex(0x267c83),
            user: hex(0x448565),
            assistant: hex(0x527caf),
            thinking: hex(0x8967a3),
            tool: hex(0x9a7638),
            error: hex(0xb24e59),
            success: hex(0x347552),
            property: hex(0x428486),
            type_color: hex(0x936e31),
            keyword: hex(0x287d89),
            string: hex(0x407b55),
            number: hex(0x986386),
            function: hex(0x427ab0),
            constant: hex(0x8866a4),
            escape: hex(0x9c752f),
            selection: hex(0xcddfe3),
            user_surface: hex(0xe5efe8),
            assistant_surface: hex(0xe5ecf4),
            thinking_surface: hex(0xebe1f1),
            tool_surface: hex(0xeee5d2),
            error_surface: hex(0xf2dfe4),
            dialog_surface: hex(0xe4e9ed),
            code_surface: hex(0xd9e1ea),
        }
    }

    /// Patch a named colour. An unknown name is ignored, so a palette written
    /// for a later theme still loads.
    pub fn set(&mut self, name: &str, color: Color) {
        let slot = match name {
            "text" => &mut self.text,
            "muted" => &mut self.muted,
            "accent" => &mut self.accent,
            "user" => &mut self.user,
            "assistant" => &mut self.assistant,
            "thinking" => &mut self.thinking,
            "tool" => &mut self.tool,
            "error" => &mut self.error,
            "success" => &mut self.success,
            "property" => &mut self.property,
            "type" => &mut self.type_color,
            "keyword" => &mut self.keyword,
            "string" => &mut self.string,
            "number" => &mut self.number,
            "function" => &mut self.function,
            "constant" => &mut self.constant,
            "escape" => &mut self.escape,
            "selection" => &mut self.selection,
            "user_surface" => &mut self.user_surface,
            "assistant_surface" => &mut self.assistant_surface,
            "thinking_surface" => &mut self.thinking_surface,
            "tool_surface" => &mut self.tool_surface,
            "error_surface" => &mut self.error_surface,
            "dialog_surface" => &mut self.dialog_surface,
            "code_surface" => &mut self.code_surface,
            _ => return,
        };
        *slot = color;
    }
}

impl Default for Palette {
    fn default() -> Self {
        Palette::dark()
    }
}

impl Theme {
    /// The shipped theme: terminal-default body text, a few muted accents.
    ///
    /// Deliberately small. A theme that names every role it might meet is a theme
    /// that has to change when a plugin appears; this one names the roles the
    /// shipped session emits and lets everything else fall back through its
    /// prefix.
    pub fn dark() -> Theme {
        let mut theme = Theme::from_palette(Palette::dark());
        theme.name = "dark".into();
        theme
    }

    /// Build a theme from a palette. Every role references a named colour, so a
    /// palette patch recolours a whole family at once.
    pub fn from_palette(palette: Palette) -> Theme {
        let mut roles = BTreeMap::new();
        let mut set = |role: &str, style: Style| {
            roles.insert(role.to_string(), style);
        };
        set("", Style::PLAIN);
        set("header.title", Style::fg(palette.accent).bold());
        set("header.detail", Style::fg(palette.muted).dim());
        set("session.header", Style::fg(palette.muted));
        set("session.title", Style::fg(palette.accent).bold());
        // Message bodies use the text surface; only their rails carry the role
        // colour. This is the reference's distinction between readable prose
        // and the small marker that identifies its speaker.
        set("message.user", Style::PLAIN);
        set("surface.user", Style::PLAIN.on(palette.user_surface));
        set("message.user.rail", Style::fg(palette.user));
        set("message.assistant", Style::PLAIN);
        set(
            "surface.assistant",
            Style::PLAIN.on(palette.assistant_surface),
        );
        set("message.assistant.rail", Style::fg(palette.assistant));
        set("message.thinking", Style::PLAIN.dim());
        set(
            "surface.thinking",
            Style::PLAIN.on(palette.thinking_surface),
        );
        set("message.thinking.rail", Style::fg(palette.thinking));
        // A model reasoning aloud: present, readable, and clearly not the answer.
        set("message.assistant.thinking", Style::PLAIN.dim());
        set(
            "message.assistant.thinking.rail",
            Style::fg(palette.thinking),
        );
        // What the session told the model on its own: a background command finishing is the
        // only thing that speaks this way, and it should read as a footnote rather than prose.
        set("message.system", Style::fg(palette.muted).dim());
        set("user", Style::PLAIN);
        set("text", Style::PLAIN);
        set("dim", Style::PLAIN.dim());
        set("tool.call", Style::PLAIN);
        set("surface.tool", Style::PLAIN.on(palette.tool_surface));
        set("tool.call.rail", Style::fg(palette.tool));
        set("tool.result", Style::PLAIN);
        set("tool.result.rail", Style::fg(palette.tool));
        set("notice", Style::fg(palette.muted));
        set("error", Style::fg(palette.error));
        set("surface.error", Style::PLAIN.on(palette.error_surface));
        set("error.rail", Style::fg(palette.error));
        set("status", Style::fg(palette.muted).dim());
        set("composer", Style::PLAIN);
        set("mode.insert", Style::fg(palette.accent).bold());
        set("mode.normal", Style::fg(palette.accent).bold());
        set("mode.visual", Style::fg(palette.accent).bold());
        set("palette.title", Style::fg(palette.accent).bold());
        set(
            "palette.item.selected",
            Style::PLAIN.on(palette.selection).bold(),
        );
        set("palette.item", Style::PLAIN);
        set("palette.hint", Style::PLAIN.dim());
        // Names used by the old component vocabulary. Keeping these as theme
        // roles lets the terminal renderer use the same semantic styles as a
        // windowed client without copying a second palette into the picker.
        set("choice.prompt", Style::fg(palette.accent));
        set("choice.query", Style::PLAIN);
        set("choice.row", Style::PLAIN);
        set("choice.row.active", Style::fg(palette.accent));
        set(
            "choice.row.selected",
            Style::PLAIN.on(palette.selection).bold(),
        );
        set("choice.hint", Style::PLAIN.dim());
        set("choice.view", Style::PLAIN.bold());
        set("choice.view.active", Style::fg(palette.accent).bold());
        set("choice.empty", Style::PLAIN.dim());
        set("label", Style::PLAIN.dim());
        set("value", Style::PLAIN);
        set("keybinding", Style::fg(palette.accent).dim());
        set("pending", Style::fg(palette.accent).dim());
        set("plain", Style::PLAIN);
        set("value.context", Style::fg(palette.property));
        set("value.money", Style::fg(palette.type_color));
        set("turn", Style::fg(palette.accent));
        set("indicator", Style::fg(palette.muted));
        set("indicator.activity", Style::fg(palette.accent));
        set("indicator.model", Style::fg(palette.assistant));
        set("indicator.context", Style::fg(palette.property));
        set("status.separator", Style::fg(palette.muted));
        set("message.group.footer", Style::PLAIN.dim());
        set("dialog", Style::fg(palette.muted));
        set("dialog.label", Style::fg(palette.muted).dim());
        set("dialog.message", Style::PLAIN);
        set("dialog.hint", Style::PLAIN.dim());
        set("dialog.value", Style::PLAIN);
        set("dialog.title", Style::fg(palette.accent).bold());
        set("surface.dialog", Style::PLAIN.on(palette.dialog_surface));
        set("surface.code", Style::PLAIN.on(palette.code_surface));
        // A reader's selection. A background rather than a foreground, because it has
        // to sit over whatever the role underneath already decided.
        set("selection", Style::PLAIN.on(palette.selection));
        // A diff. Named per line rather than once, because what changed is the whole point
        // of looking at one: an added line, a removed one, the hunk header, and the file
        // headers that say what the hunks are hunks of. A body is a diff because the
        // session said so — a role that ends in `.diff`, or a fence that said `diff` — so
        // nothing here has to guess.
        set("diff", Style::fg(palette.muted));
        set("diff.added", Style::fg(palette.success));
        set("diff.removed", Style::fg(palette.error));
        set("diff.hunk", Style::fg(palette.keyword).dim());
        set("diff.header", Style::fg(palette.muted).bold());
        set("diff.meta", Style::fg(palette.muted).dim());
        set("value.money", Style::fg(palette.type_color));
        set("value.count", Style::fg(palette.muted));
        // Global semantic modifiers. The role vocabulary is a tree, so a renderer
        // overlays one of these on whatever role a node already resolved to: that is
        // how emphasis and links stay themable without every node inventing a
        // compound role of its own.
        set("bold", Style::PLAIN.bold());
        set("italic", Style::PLAIN.italic());
        set("underline", Style::PLAIN.underline());
        set(
            "strikethrough",
            Style {
                strikethrough: true,
                ..Style::PLAIN
            },
        );
        set("code", Style::fg(palette.property));
        set("link", Style::fg(palette.accent).underline());
        // Inline semantic modifiers the parser emits as span kinds. Highlight is a
        // background so it sits over whatever foreground the containing role chose;
        // a cell grid cannot raise or lower a run, so sub/superscript stay distinct
        // by dimming the lowered one rather than by geometry.
        set("highlight", Style::PLAIN.on(palette.selection));
        set("subscript", Style::PLAIN.dim());
        set("superscript", Style::PLAIN);
        set("quote", Style::PLAIN.dim());
        // A GitHub alert is still a quote; its role names the kind, and these globals
        // are what a renderer folds in for the marker and the body.
        set("markdown.alert.note", Style::fg(palette.accent));
        set("markdown.alert.tip", Style::fg(palette.success));
        set("markdown.alert.important", Style::fg(palette.thinking));
        set("markdown.alert.warning", Style::fg(palette.tool));
        set("markdown.alert.caution", Style::fg(palette.error));
        // Markdown vocabulary. The parser emits prefixed roles
        // (`message.assistant.markdown.heading`), so a theme names these globals
        // once and every message's markdown follows; naming the fully qualified
        // role instead overrides one place without touching the rest.
        set("markdown.heading.1", Style::PLAIN.bold().underline());
        set("markdown.heading.2", Style::PLAIN.bold());
        set("markdown.heading.3", Style::PLAIN.bold().italic());
        set("markdown.heading.4", Style::PLAIN.italic());
        set("markdown.heading.5", Style::PLAIN.underline());
        set("markdown.heading.6", Style::PLAIN.dim());
        set("markdown.list.marker", Style::PLAIN.bold());
        set("markdown.rule", Style::PLAIN.dim());
        set("markdown.table.border", Style::PLAIN.dim());
        set("markdown.table.header", Style::PLAIN.bold());
        set("markdown.table.cell", Style::PLAIN);
        set("markdown.table.rule", Style::fg(palette.muted).dim());
        set("markdown.definition.term", Style::PLAIN.bold());
        set("markdown.definition.marker", Style::PLAIN.dim());
        set("markdown.footnote", Style::fg(palette.muted).dim());
        set("markdown.footnote.marker", Style::fg(palette.accent));
        set("markdown.details.summary", Style::fg(palette.muted).dim());
        set("markdown.code.label", Style::PLAIN.bold());
        set("markdown.code.border", Style::PLAIN.dim());
        set("choice.preview", Style::PLAIN.dim());

        let mut tokens = BTreeMap::new();
        let mut token = |name: &str, style: Style| {
            tokens.insert(name.to_string(), style);
        };
        token("comment", Style::fg(palette.muted).italic());
        token("string", Style::fg(palette.string));
        token("number", Style::fg(palette.number));
        token("keyword", Style::fg(palette.keyword));
        token("type", Style::fg(palette.type_color));
        token("function", Style::fg(palette.function));
        token("constant", Style::fg(palette.constant));
        token("variable", Style::fg(palette.muted));
        token("property", Style::fg(palette.property));
        token("tag", Style::fg(palette.error));
        token("attribute", Style::fg(palette.type_color));
        token("operator", Style::fg(palette.keyword));
        token("punctuation", Style::fg(palette.muted));
        token("escape", Style::fg(palette.escape));
        token("embedded", Style::fg(palette.muted));

        let mut states = BTreeMap::new();
        states.insert(State::Pending, Style::fg(palette.accent).dim());
        states.insert(State::Streaming, Style::fg(palette.success));
        states.insert(State::Done, Style::fg(palette.muted));
        states.insert(State::Failed, Style::fg(palette.error));
        states.insert(State::Cancelled, Style::fg(palette.muted).dim());

        Theme {
            name: "custom".into(),
            roles,
            tokens,
            states,
            default: Style::PLAIN,
            palette,
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
            palette: Palette::dark(),
        }
    }

    /// The previous system's light reference, with explicit light colours rather
    /// than dark ones with their foregrounds dimmed.
    pub fn light() -> Theme {
        let mut theme = Self::from_palette(Palette::light());
        theme.name = "light".into();
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

    /// Apply client-owned attribute patches, returning a new theme.
    ///
    /// This is the previous system's `styles.normalize` override half, adapted to
    /// the role model: a patch is applied to the exact name it addresses, and a
    /// missing name is created from the default rather than refused. The base theme
    /// is untouched, so a client may hold several overlays over one palette.
    pub fn overlay(mut self, overrides: &ThemeOverrides) -> Theme {
        // A palette patch rebuilds every role from the named colours, then the
        // explicit role patches below win over what the rebuild produced. Plain has
        // no roles to rebuild; a palette patch there is a no-op.
        if !overrides.palette.is_empty() && !self.roles.is_empty() {
            let mut palette = self.palette;
            for (name, color) in &overrides.palette {
                palette.set(name, *color);
            }
            let name = self.name.clone();
            self = Theme::from_palette(palette);
            self.name = name;
        }
        for (role, patch) in &overrides.roles {
            let base = self.roles.get(role).copied().unwrap_or(self.default);
            self.roles.insert(role.clone(), patch.apply(base));
        }
        for (name, patch) in &overrides.tokens {
            let base = self.tokens.get(name).copied().unwrap_or(self.default);
            self.tokens.insert(name.clone(), patch.apply(base));
        }
        for (name, patch) in &overrides.states {
            if let Some(state) = state_from_name(name) {
                let base = self.states.get(&state).copied().unwrap_or(self.default);
                self.states.insert(state, patch.apply(base));
            }
        }
        self
    }

    /// The ordered merge the previous system called `compose`: later names win.
    ///
    /// The renderer already overlays one node-specific role on one global role;
    /// this is for a component that wants to say a whole chain, such as the tool
    /// role and a markdown heading level at once.
    pub fn compose(&self, names: &[&str]) -> Style {
        names
            .iter()
            .fold(Style::PLAIN, |style, name| style.over(self.role(name)))
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
    ///
    /// Both the bare capture name (`keyword`) and the previous system's namespaced
    /// spelling (`syntax.keyword`) resolve, so a theme written for either vocabulary
    /// colours the same capture.
    pub fn token(&self, name: &str) -> Style {
        if let Some(style) = self.tokens.get(name) {
            return *style;
        }
        if let Some(rest) = name.strip_prefix("syntax.") {
            if let Some(style) = self.tokens.get(rest) {
                return *style;
            }
        } else {
            let prefixed = format!("syntax.{name}");
            if let Some(style) = self.tokens.get(&prefixed) {
                return *style;
            }
        }
        self.default
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
    fn a_palette_override_recolours_every_role_that_uses_it() {
        let overrides = ThemeOverrides {
            palette: BTreeMap::from([("accent".to_string(), Color::Rgb(1, 2, 3))]),
            ..ThemeOverrides::default()
        };
        let theme = Theme::dark().overlay(&overrides);
        assert_eq!(theme.role("header.title").fg, Color::Rgb(1, 2, 3));
        assert_eq!(theme.role("choice.prompt").fg, Color::Rgb(1, 2, 3));
        // A role-specific patch still wins over the palette rebuild.
        let overrides = ThemeOverrides {
            palette: BTreeMap::from([("accent".to_string(), Color::Rgb(1, 2, 3))]),
            roles: BTreeMap::from([(
                "choice.prompt".to_string(),
                StylePatch {
                    fg: Some(Color::Indexed(7)),
                    ..StylePatch::default()
                },
            )]),
            ..ThemeOverrides::default()
        };
        let theme = Theme::dark().overlay(&overrides);
        assert_eq!(theme.role("choice.prompt").fg, Color::Indexed(7));
        assert_eq!(theme.role("header.title").fg, Color::Rgb(1, 2, 3));
        // The palette survives onto the result, so a second overlay can build on it.
        assert_eq!(theme.palette.accent, Color::Rgb(1, 2, 3));
    }

    #[test]
    fn the_light_palette_is_explicit_rather_than_a_dimmed_dark() {
        let light = Theme::light();
        assert_eq!(light.role("error").fg, Color::Rgb(0xb2, 0x4e, 0x59));
        assert_eq!(light.role("header.title").fg, Color::Rgb(0x26, 0x7c, 0x83));
        assert_eq!(
            light.surface("message.user").unwrap().bg,
            Color::Rgb(0xe5, 0xef, 0xe8)
        );
    }

    #[test]
    fn table_roles_exist_in_both_palettes() {
        for theme in [Theme::dark(), Theme::light()] {
            assert!(theme.names("markdown.table.header"));
            assert!(theme.names("markdown.table.cell"));
            assert!(theme.names("markdown.table.rule"));
        }
        // The rule is drawn from the palette's muted colour, so the light
        // palette gives it a light-palette value rather than the dark one.
        assert_ne!(
            Theme::light().role("markdown.table.rule"),
            Theme::dark().role("markdown.table.rule")
        );
    }

    #[test]
    fn new_markdown_roles_exist_in_both_palettes() {
        for theme in [Theme::dark(), Theme::light()] {
            assert!(theme.names("markdown.definition.term"));
            assert!(theme.names("markdown.definition.marker"));
            assert!(theme.names("markdown.footnote"));
            assert!(theme.names("markdown.footnote.marker"));
            assert!(theme.names("markdown.details.summary"));
        }
        // The reference marker takes the palette's accent, so the light
        // palette gives it a light-palette value rather than the dark one.
        assert_ne!(
            Theme::light().role("markdown.footnote.marker"),
            Theme::dark().role("markdown.footnote.marker")
        );
        assert_ne!(
            Theme::light().role("markdown.footnote"),
            Theme::dark().role("markdown.footnote")
        );
    }

    #[test]
    fn an_override_patches_attributes_and_can_clear_them() {
        let base = Theme::dark().with_role("error", Style::rgb(1, 2, 3).bold());
        let overrides = ThemeOverrides {
            roles: BTreeMap::from([(
                "error".to_string(),
                StylePatch {
                    fg: Some(Color::Indexed(9)),
                    bold: Some(false),
                    ..StylePatch::default()
                },
            )]),
            ..ThemeOverrides::default()
        };
        let theme = base.overlay(&overrides);
        assert_eq!(theme.role("error").fg, Color::Indexed(9));
        assert!(
            !theme.role("error").bold,
            "an override can clear an attribute"
        );
        // The base theme is untouched, so one overlay does not leak into another.
        assert_eq!(theme.role("message.user"), Style::PLAIN);
    }

    #[test]
    fn an_override_can_create_a_role_and_an_unknown_state_is_ignored() {
        let overrides = ThemeOverrides {
            roles: BTreeMap::from([(
                "plugin.handoff".to_string(),
                StylePatch {
                    italic: Some(true),
                    ..StylePatch::default()
                },
            )]),
            states: BTreeMap::from([(
                "nonsense".to_string(),
                StylePatch {
                    bold: Some(true),
                    ..StylePatch::default()
                },
            )]),
            ..ThemeOverrides::default()
        };
        let theme = Theme::dark().overlay(&overrides);
        assert!(theme.role("plugin.handoff").italic);
        assert_eq!(
            theme.state(Some(State::Failed)),
            Theme::dark().state(Some(State::Failed))
        );
    }

    #[test]
    fn compose_merges_named_roles_in_order() {
        let theme = Theme::dark();
        let composed = theme.compose(&["message.assistant", "markdown.heading.2"]);
        assert!(composed.bold, "the heading level's bold survived the merge");
        assert!(theme.compose(&[]).eq(&Style::PLAIN));
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
