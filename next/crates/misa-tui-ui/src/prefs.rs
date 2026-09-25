//! What a client remembers between runs.
//!
//! A client owns presentation state — the theme somebody chose, which nodes they opened, the
//! draft in the composer, which commands and models they reach for — and none of it is a
//! session's business. Until this file existed, it was also the only state in the system that
//! a restart forgot: a session keeps its facts in the log, and a client kept its taste in
//! memory.
//!
//! # Why this is safe to lose, and what follows from that
//!
//! Everything here is presentation, so a client that has none of it is still *correct*: it
//! opens with the default theme, an empty drawer, and no draft. That property is what makes
//! this file tolerable to corrupt — [`Prefs::load`] returns the defaults for a missing,
//! unreadable, or unparseable document and says nothing, because a client that refuses to
//! start over its own preferences is a client that turned a taste into a requirement.
//!
//! A *save* is different: it can fail for a reason worth telling somebody (a read-only state
//! directory), so it returns the reason rather than swallowing it.
//!
//! # What is deliberately not here
//!
//! Nothing a session would have to trust, and nothing the agent decides. A draft is what
//! somebody typed and has not sent; a frecency count is which of two equal matches they
//! picked last time. A client that lost the whole document would ask the same questions and
//! send the same intents.
//!
//! The prompt history is deliberately *not* here, and neither is anything else somebody
//! typed. A prompt is a question that may contain a password somebody pasted into it, and a
//! transcript of every question on this machine is a log — which is the session's, which is
//! journalled where it belongs, and which nobody agreed to keep twice.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use misa_kit::picker::Frecency;

/// Terminal bindings are named by semantic action. The translator consumes
/// this table, and the action palette displays the same table, so changing a
/// binding cannot leave a decorative key reference behind.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeymapSettings {
    pub bindings: BTreeMap<String, Vec<String>>,
}

impl Default for KeymapSettings {
    fn default() -> Self {
        let mut bindings = BTreeMap::new();
        for (id, keys) in [
            ("history.search", vec!["ctrl+r"]),
            ("picker.previous_view", vec!["alt+p"]),
            ("history.previous", vec!["ctrl+p"]),
            ("history.next", vec!["ctrl+n", "alt+n"]),
            ("app.quit", vec!["ctrl+q"]),
            ("input.interrupt", vec!["ctrl+c"]),
            ("transcript.verbose", vec!["alt+t"]),
            ("effort.cycle", vec!["alt+f"]),
            ("transcript.up", vec!["pageup", "alt+k"]),
            ("transcript.down", vec!["pagedown", "alt+j"]),
            ("model.open", vec!["alt+m"]),
            ("commands.open", vec!["alt+/"]),
            ("picker.favorite", vec!["alt+v"]),
            ("picker.slot.1", vec!["alt+1"]),
            ("picker.slot.2", vec!["alt+2"]),
            ("picker.slot.3", vec!["alt+3"]),
            ("picker.slot.4", vec!["alt+4"]),
            ("picker.slot.5", vec!["alt+5"]),
            ("picker.slot.6", vec!["alt+6"]),
            ("picker.slot.7", vec!["alt+7"]),
            ("picker.slot.8", vec!["alt+8"]),
            ("picker.slot.9", vec!["alt+9"]),
            ("queue.edit", vec!["alt+e"]),
            ("selection.open", vec!["alt+s"]),
            ("input.eof", vec!["ctrl+d"]),
            ("input.interrupt_submit", vec!["alt+enter"]),
            ("input.newline", vec!["shift+enter"]),
            ("transcript.top", vec!["ctrl+home"]),
            ("transcript.bottom", vec!["ctrl+end"]),
            ("actions.open", vec!["f1"]),
        ] {
            bindings.insert(id.into(), keys.into_iter().map(str::to_string).collect());
        }
        Self { bindings }
    }
}

impl KeymapSettings {
    pub fn keys(&self, id: &str) -> String {
        self.bindings
            .get(id)
            .map(|keys| keys.join(" · "))
            .unwrap_or_default()
    }

    pub fn matches(
        &self,
        id: &str,
        code: crossterm::event::KeyCode,
        modifiers: crossterm::event::KeyModifiers,
    ) -> bool {
        self.bindings
            .get(id)
            .is_some_and(|keys| keys.iter().any(|key| key_matches(key, code, modifiers)))
    }
}

fn key_matches(
    binding: &str,
    code: crossterm::event::KeyCode,
    modifiers: crossterm::event::KeyModifiers,
) -> bool {
    use crossterm::event::{KeyCode, KeyModifiers};
    let mut expected = KeyModifiers::empty();
    let mut base = "";
    for part in binding.split('+') {
        match part {
            "ctrl" => expected |= KeyModifiers::CONTROL,
            "alt" => expected |= KeyModifiers::ALT,
            "shift" => expected |= KeyModifiers::SHIFT,
            value => base = value,
        }
    }
    if modifiers != expected {
        return false;
    }
    match (base, code) {
        ("enter", KeyCode::Enter)
        | ("escape", KeyCode::Esc)
        | ("backspace", KeyCode::Backspace)
        | ("delete", KeyCode::Delete)
        | ("tab", KeyCode::Tab)
        | ("pageup", KeyCode::PageUp)
        | ("pagedown", KeyCode::PageDown)
        | ("home", KeyCode::Home)
        | ("end", KeyCode::End)
        | ("f1", KeyCode::F(1)) => true,
        (value, KeyCode::Char(character)) => value == character.to_string(),
        _ => false,
    }
}

/// The terminal's choice widget is a composition of actions, not a string
/// renderer with a second set of keys hidden in it. Keeping this data in the
/// client preferences gives the stock terminal the reference defaults while
/// leaving another frontend or a user configuration free to select its own
/// vocabulary and geometry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PickerHint {
    pub id: String,
    pub key: String,
    pub label: String,
    #[serde(default)]
    pub only_with_views: bool,
    #[serde(default)]
    pub inline: bool,
    #[serde(default = "default_true")]
    pub overlay: bool,
}

fn default_true() -> bool {
    true
}

/// The row grammar is presentation, not picker state. A terminal can use a marker, a
/// browser can use a selected class, and a compact client can remove the detail separator
/// without teaching the picker about either surface.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PickerRowSettings {
    pub selected_marker: String,
    pub marker: String,
    pub marker_separator: String,
    pub detail_separator: String,
    pub inline_prefix: String,
}

/// Dialog interaction is a client composition. The request model supplies
/// semantic actions; this config supplies the terminal vocabulary used to
/// expose them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DialogSettings {
    pub action_separator: String,
    pub action_keys: BTreeMap<String, String>,
}

impl Default for DialogSettings {
    fn default() -> Self {
        Self {
            // Button references use the same three-space separation as the reference
            // keybinding renderer; dots belong to facts/details, not affordance groups.
            action_separator: "   ".into(),
            action_keys: BTreeMap::from([
                ("approve".into(), "y".into()),
                ("deny".into(), "n".into()),
                ("cancel".into(), "ctrl+c".into()),
                ("submit".into(), "enter".into()),
                ("resolve".into(), "enter".into()),
                // Panels and reports use the same action vocabulary as request dialogs. These
                // are defaults, not branches in either renderer, so a client can change the
                // terminal key without leaving a decorative hint behind.
                ("panel.close".into(), "escape".into()),
                ("panel.submit".into(), "enter".into()),
            ]),
        }
    }
}

impl DialogSettings {
    /// Return the configured key for a semantic dialog action. User settings may be a partial
    /// map, so built-in actions retain their defaults unless explicitly overridden; renderers
    /// and input routing therefore cannot disagree merely because an older prefs file omitted a
    /// newly introduced action.
    pub fn key(&self, id: &str) -> Option<&str> {
        self.action_keys
            .get(id)
            .map(String::as_str)
            .or_else(|| default_action_key(id))
    }

    pub fn matches(&self, id: &str, key: &crate::Key) -> bool {
        let Some(binding) = self.key(id) else {
            return false;
        };
        match (binding, key) {
            ("escape", crate::Key::Escape)
            | ("enter", crate::Key::Submit)
            | ("ctrl+c", crate::Key::Interrupt)
            | ("alt+enter", crate::Key::InterruptSubmit)
            | ("backspace", crate::Key::Backspace)
            | ("delete", crate::Key::Delete)
            | ("tab", crate::Key::Tab) => true,
            (binding, crate::Key::Char(character)) => binding == character.to_string(),
            _ => false,
        }
    }
}

fn default_action_key(id: &str) -> Option<&'static str> {
    match id {
        "approve" => Some("y"),
        "deny" => Some("n"),
        "cancel" => Some("ctrl+c"),
        "submit" | "resolve" => Some("enter"),
        "panel.close" => Some("escape"),
        "panel.submit" => Some("enter"),
        _ => None,
    }
}

impl Default for PickerRowSettings {
    fn default() -> Self {
        Self {
            selected_marker: ">".into(),
            marker: " ".into(),
            marker_separator: " ".into(),
            detail_separator: " — ".into(),
            inline_prefix: "  ".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PickerSettings {
    pub padding: usize,
    /// Maximum selected-choice preview lines. The session supplies the facts;
    /// this is only the terminal's geometry budget.
    pub preview_lines: usize,
    pub preview: PickerPreviewSettings,
    pub preferred_height: usize,
    pub min_height: usize,
    pub max_height: usize,
    pub minimum_panel_width: usize,
    pub gap: usize,
    pub hint_separator: String,
    pub empty_label: String,
    pub more_label: String,
    pub overflow_format: String,
    pub view_titles: BTreeMap<String, String>,
    /// The views a source offers in an overlay. This is presentation policy:
    /// the picker knows how to switch views, but not which source deserves
    /// Browse/Favorites versus All.
    pub view_sets: BTreeMap<String, Vec<String>>,
    /// Inline completion may have a different purpose from an explicitly
    /// opened palette. In particular, `/` completes the command vocabulary;
    /// it is not the command-management palette.
    pub inline_view_sets: BTreeMap<String, Vec<String>>,
    pub hints: Vec<PickerHint>,
    #[serde(default)]
    pub row: PickerRowSettings,
}

/// Vocabulary for projecting model facts into a selected-choice preview. The facts are shared;
/// these labels are terminal composition data and can be replaced without changing the picker
/// or the session catalogue.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PickerPreviewSettings {
    pub context_label: String,
    pub effort_label: String,
    pub price_label: String,
    pub cache_label: String,
    pub request_label: String,
    pub unavailable_label: String,
    pub estimate_note: String,
    pub peak_label: String,
}

impl Default for PickerPreviewSettings {
    fn default() -> Self {
        Self {
            context_label: "Context".into(),
            effort_label: "Effort".into(),
            price_label: "USD per 1M tokens".into(),
            cache_label: "Cache".into(),
            request_label: "Per request".into(),
            unavailable_label: "Cost: unavailable".into(),
            estimate_note: "Estimates; reported usage cost takes precedence".into(),
            peak_label: "Peak".into(),
        }
    }
}

impl Default for PickerSettings {
    fn default() -> Self {
        Self {
            padding: 2,
            preview_lines: 5,
            preview: PickerPreviewSettings::default(),
            preferred_height: 14,
            min_height: 4,
            max_height: 18,
            minimum_panel_width: 28,
            gap: 2,
            hint_separator: "   ".into(),
            empty_label: "no matches".into(),
            more_label: "more".into(),
            overflow_format: "{start}–{last} / {total}".into(),
            view_titles: BTreeMap::from([
                ("browse".into(), "Browse".into()),
                ("all".into(), "All".into()),
                ("favorites".into(), "Favorites".into()),
                ("recent".into(), "Recent".into()),
            ]),
            view_sets: BTreeMap::from([
                ("commands".into(), vec!["browse".into(), "favorites".into()]),
                ("models".into(), vec!["browse".into(), "favorites".into()]),
                ("actions".into(), vec!["browse".into(), "favorites".into()]),
            ]),
            inline_view_sets: BTreeMap::from([("commands".into(), vec!["all".into()])]),
            hints: vec![
                ("complete", "tab", "complete", false, true),
                ("previous", "up", "previous", false, false),
                ("next", "down", "next", false, false),
                ("accept", "enter", "accept", false, false),
                ("cancel", "escape", "cancel", false, false),
                ("cycle", "right", "cycle views", true, true),
                ("replace_view", "alt+/", "replace view", true, false),
                ("favorite", "alt+v", "favorite", false, true),
            ]
            .into_iter()
            .map(|(id, key, label, only_with_views, inline)| PickerHint {
                id: id.into(),
                key: key.into(),
                label: label.into(),
                only_with_views,
                inline,
                overlay: true,
            })
            .collect(),
            row: PickerRowSettings::default(),
        }
    }
}

impl PickerSettings {
    pub fn picker_views(
        &self,
        source: &str,
        placement: misa_kit::picker::PickerPlacement,
    ) -> Vec<misa_kit::picker::PickerView> {
        let configured = match placement {
            misa_kit::picker::PickerPlacement::Inline => self
                .inline_view_sets
                .get(source)
                .or_else(|| self.view_sets.get(source)),
            misa_kit::picker::PickerPlacement::Overlay => self.view_sets.get(source),
        };
        let views = configured
            .into_iter()
            .flatten()
            .filter_map(|id| match id.as_str() {
                "browse" => Some(misa_kit::picker::PickerView::Browse),
                "all" => Some(misa_kit::picker::PickerView::All),
                "favorites" => Some(misa_kit::picker::PickerView::Favorites),
                "recent" => Some(misa_kit::picker::PickerView::Recent),
                _ => None,
            })
            .collect::<Vec<_>>();
        if views.is_empty() {
            vec![misa_kit::picker::PickerView::All]
        } else {
            views
        }
    }
}

/// A frontend supplies where its memory lives: a file, browser storage, or app data.
/// The kit neither chooses a path nor performs filesystem or environment access.
pub trait Storage {
    fn read(&self) -> Result<Option<String>, String>;
    fn write(&self, text: &str) -> Result<(), String>;
}

/// A client's memory between runs.
///
/// Every field is optional in the document and defaults to "nothing remembered", which is
/// what makes an older or newer file readable: a field this version does not know is dropped,
/// and one it wants and does not find is a default. Both are the right answer for state that
/// nobody has to be told about.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub components: misa_lines::components::Settings,
    #[serde(default)]
    pub keymap: KeymapSettings,
    #[serde(default)]
    pub picker: PickerSettings,
    #[serde(default)]
    pub dialogs: DialogSettings,
    /// The theme, by the name a person would say: `dark`, `light`, `plain`. A name rather
    /// than a frontend's type, because this crate is below every frontend and a pixel
    /// frontend's theme is not a terminal's.
    pub theme: String,
    /// Client-owned attribute patches over the selected theme. The previous system
    /// kept palette and style overrides in configuration; here they are what this
    /// client remembers, so another terminal's preferences do not change this one.
    #[serde(default)]
    pub theme_overrides: misa_render::ThemeOverrides,
    /// The nodes somebody opened, by the id the session gave them. `*` means all of them,
    /// which is what a key that toggles "every tool call" writes.
    pub opened: Vec<String>,
    /// What was in the composer, unsent.
    pub draft: String,
    /// Drafts are qualified by daemon identity and exact session incarnation.
    pub drafts: BTreeMap<String, String>,
    /// How often each choice was accepted, by the value the picker offered.
    pub frecency: BTreeMap<String, i64>,
    /// Choices somebody explicitly wants near the top of a picker.
    pub favorites: std::collections::BTreeSet<String>,
}

impl Prefs {
    /// Read through the frontend's storage capability. Missing or corrupt preferences
    /// are defaults; they never prevent a client from attaching.
    pub fn load(storage: &dyn Storage) -> Prefs {
        storage
            .read()
            .ok()
            .flatten()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Encode the shared memory shape and let the frontend persist it.
    pub fn save(&self, storage: &dyn Storage) -> Result<(), String> {
        let text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        storage.write(&text)
    }

    /// Whether a node is one somebody opened.
    pub fn is_open(&self, id: &str) -> bool {
        self.opened
            .iter()
            .any(|opened| opened == id || opened == "*")
    }

    /// Whether anything is open, which is what a key that means "all of them" asks.
    pub fn any_open(&self) -> bool {
        !self.opened.is_empty()
    }

    /// Open every node, or close every one.
    ///
    /// `*` is the id that stands for all of them, and it is remembered exactly as a node id
    /// is — one piece of state, not two, because a client that had a flag *and* a set would
    /// have to decide which of them wins.
    pub fn open_all(&mut self) {
        self.opened = vec!["*".to_string()];
    }

    pub fn close_all(&mut self) {
        self.opened.clear();
    }

    /// Open or close a node, remembering it either way.
    pub fn toggle(&mut self, id: &str) {
        match self.opened.iter().position(|opened| opened == id) {
            Some(position) => {
                self.opened.remove(position);
            }
            None => self.opened.push(id.to_string()),
        }
    }

    /// Remember that a choice was taken.
    ///
    /// The same count a picker keeps in memory, kept where a restart can find it: which of two
    /// equally good matches somebody picked last time is exactly the thing worth remembering,
    /// because the next list is ranked by it.
    pub fn remembered(&mut self, value: &str) {
        *self.frecency.entry(value.to_string()).or_insert(0) += 1;
    }

    /// The counts, in the shape a picker takes them.
    pub fn frecency(&self) -> Frecency {
        Frecency::from_counts(self.frecency.clone())
    }

    pub fn favorites(&self) -> impl Iterator<Item = String> + '_ {
        self.favorites.iter().cloned()
    }

    pub fn toggle_favorite(&mut self, value: &str) -> bool {
        if self.favorites.insert(value.to_string()) {
            true
        } else {
            self.favorites.remove(value);
            false
        }
    }

    /// Take the counts from a picker that has been used.
    pub fn remember_frecency(&mut self, frecency: &Frecency) {
        self.frecency = frecency.counts().clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Memory(std::cell::RefCell<Option<String>>);
    impl Storage for Memory {
        fn read(&self) -> Result<Option<String>, String> {
            Ok(self.0.borrow().clone())
        }
        fn write(&self, text: &str) -> Result<(), String> {
            *self.0.borrow_mut() = Some(text.to_owned());
            Ok(())
        }
    }

    #[test]
    fn the_shared_memory_shape_round_trips_through_injected_storage() {
        let storage = Memory::default();
        let mut prefs = Prefs::default();
        prefs.theme = "plain".into();
        prefs.draft = "half a question".into();
        prefs.toggle("call.1");
        prefs.remembered("scripted-1");
        prefs.save(&storage).unwrap();
        assert_eq!(Prefs::load(&storage), prefs);
        assert_eq!(Prefs::load(&storage).frecency().score("scripted-1"), 1);
    }

    #[test]
    fn missing_corrupt_and_older_documents_have_defaults() {
        let storage = Memory::default();
        assert_eq!(Prefs::load(&storage), Prefs::default());
        storage.write("not json").unwrap();
        assert_eq!(Prefs::load(&storage), Prefs::default());
        storage.write(r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(Prefs::load(&storage).theme, "dark");
        assert_eq!(Prefs::load(&storage).draft, "");
    }

    #[test]
    fn read_failure_defaults_but_save_failure_is_reported() {
        struct Unavailable;
        impl Storage for Unavailable {
            fn read(&self) -> Result<Option<String>, String> {
                Err("unavailable".into())
            }
            fn write(&self, _: &str) -> Result<(), String> {
                Err("unavailable".into())
            }
        }
        assert_eq!(Prefs::load(&Unavailable), Prefs::default());
        assert_eq!(
            Prefs::default().save(&Unavailable),
            Err("unavailable".into())
        );
    }

    #[test]
    fn opening_and_closing_a_node_is_remembered_either_way() {
        let mut prefs = Prefs::default();
        assert!(!prefs.is_open("call.1"));
        prefs.toggle("call.1");
        assert!(prefs.is_open("call.1"));
        prefs.toggle("call.1");
        assert!(!prefs.is_open("call.1"));
    }

    #[test]
    fn remembering_everything_covers_a_node_nobody_named() {
        // The memory is additive, which is the client's own rule: a node is open when the
        // session said so, when somebody opened that node, or when they opened every node.
        // Closing *one* node out of "everything" is therefore not expressible — a memory that
        // had to be subtracted from is a memory the view would have to be consulted about, and
        // this document is the client's.
        let mut prefs = Prefs::default();
        assert!(!prefs.is_open("call.9"));
        prefs.toggle("*");
        assert!(prefs.is_open("call.9"));
        assert!(prefs.is_open("anything.else"));
    }
}
