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
    pub components: misa_render::components::Settings,
    /// The theme, by the name a person would say: `dark`, `plain`. A name rather than a
    /// frontend's type, because this crate is below every frontend and a pixel frontend's
    /// theme is not a terminal's.
    pub theme: String,
    /// The nodes somebody opened, by the id the session gave them. `*` means all of them,
    /// which is what a key that toggles "every tool call" writes.
    pub opened: Vec<String>,
    /// What was in the composer, unsent.
    pub draft: String,
    /// Drafts are qualified by daemon identity and exact session incarnation.
    pub drafts: BTreeMap<String,String>,
    /// How often each choice was accepted, by the value the picker offered.
    pub frecency: BTreeMap<String, i64>,
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
