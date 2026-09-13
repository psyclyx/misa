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
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::picker::Frecency;

/// A client's memory between runs.
///
/// Every field is optional in the document and defaults to "nothing remembered", which is
/// what makes an older or newer file readable: a field this version does not know is dropped,
/// and one it wants and does not find is a default. Both are the right answer for state that
/// nobody has to be told about.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// The theme, by the name a person would say: `dark`, `plain`. A name rather than a
    /// frontend's type, because this crate is below every frontend and a pixel frontend's
    /// theme is not a terminal's.
    pub theme: String,
    /// The nodes somebody opened, by the id the session gave them. `*` means all of them,
    /// which is what a key that toggles "every tool call" writes.
    pub opened: Vec<String>,
    /// What was in the composer, unsent.
    pub draft: String,
    /// How often each choice was accepted, by the value the picker offered.
    pub frecency: BTreeMap<String, i64>,
}

impl Prefs {
    /// The document a client reads and writes unless it was told otherwise.
    ///
    /// The state directory, because this is state and not configuration: nobody edits it, and
    /// losing it costs nothing. `MISA_PREFS` points it elsewhere, which is what a test and a
    /// person running two clients side by side both want.
    pub fn default_path() -> PathBuf {
        if let Ok(explicit) = std::env::var("MISA_PREFS")
            && !explicit.is_empty()
        {
            return PathBuf::from(explicit);
        }
        let state = std::env::var("XDG_STATE_HOME")
            .ok()
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var("HOME").ok().map(|home| PathBuf::from(home).join(".local/state")))
            .unwrap_or_else(|| PathBuf::from("."));
        state.join("misa/client.json")
    }

    /// What a client knew last time, or nothing at all.
    ///
    /// Never fails: a missing file, a file somebody edited by hand into nonsense, and a file
    /// from a version that wrote a different shape are all "nothing remembered", which is a
    /// state the client already handles because it is where every client starts.
    pub fn load(path: &Path) -> Prefs {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Prefs::default();
        };
        serde_json::from_str(&text).unwrap_or_default()
    }

    /// Write what this client knows.
    ///
    /// Through a temporary file and a rename, so a client that is killed while writing leaves
    /// one of the two documents whole rather than half of the new one — a preferences file is
    /// not worth a lock, and it is worth not being corrupt.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|err| format!("could not make {}: {err}", parent.display()))?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|err| err.to_string())?;
        let temporary = path.with_extension("writing");
        std::fs::write(&temporary, text).map_err(|err| format!("could not write {}: {err}", temporary.display()))?;
        std::fs::rename(&temporary, path).map_err(|err| format!("could not write {}: {err}", path.display()))
    }

    /// Whether a node is one somebody opened.
    pub fn is_open(&self, id: &str) -> bool {
        self.opened.iter().any(|opened| opened == id || opened == "*")
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

    fn path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("misa-prefs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("nested/client.json")
    }

    #[test]
    fn what_a_client_remembers_is_what_it_reads_back() {
        let path = path("round-trip");
        let mut prefs = Prefs::default();
        prefs.theme = "plain".into();
        prefs.draft = "half a question".into();
        prefs.toggle("call.1");
        prefs.remembered("scripted-1");
        prefs.remembered("scripted-1");
        prefs.remembered("/model");
        prefs.save(&path).expect("a save");

        let read = Prefs::load(&path);
        assert_eq!(read.theme, "plain");
        assert_eq!(read.draft, "half a question");
        assert!(read.is_open("call.1"));
        assert!(!read.is_open("call.2"));
        // The counts arrive in the shape a picker wants them, which is the whole point of
        // remembering them.
        assert_eq!(read.frecency().score("scripted-1"), 2);
        assert!(read.frecency().score("scripted-1") > read.frecency().score("/model"));
        let _ = std::fs::remove_dir_all(path.parent().expect("a parent"));
    }

    #[test]
    fn nothing_remembered_is_a_default_and_not_a_failure() {
        // A missing file, and a file somebody edited into nonsense: both are "nothing
        // remembered", because the client already knows how to start there.
        let missing = path("missing");
        assert_eq!(Prefs::load(&missing), Prefs::default());
        assert_eq!(Prefs::load(Path::new("/nonexistent/directory/client.json")), Prefs::default());

        let broken = path("broken");
        std::fs::create_dir_all(broken.parent().expect("a parent")).expect("a directory");
        std::fs::write(&broken, "{ this is not json").expect("a write");
        assert_eq!(Prefs::load(&broken), Prefs::default());
        let _ = std::fs::remove_dir_all(broken.parent().expect("a parent"));
    }

    #[test]
    fn a_document_that_does_not_mention_a_field_still_reads() {
        // The shape has to survive a version that knew less (or more): a field this one wants
        // and cannot find is a default, which is why every field carries `serde(default)`.
        let path = path("older");
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a directory");
        std::fs::write(&path, r#"{"theme": "dark"}"#).expect("a write");
        let prefs = Prefs::load(&path);
        assert_eq!(prefs.theme, "dark");
        assert_eq!(prefs.draft, "");
        assert!(prefs.opened.is_empty());
        let _ = std::fs::remove_dir_all(path.parent().expect("a parent"));
    }

    #[test]
    fn a_save_that_cannot_be_made_says_why() {
        // The one failure worth telling somebody about: a state directory that is not writable.
        // The path is a file, so the directory above it cannot be created.
        let path = path("unwritable");
        std::fs::create_dir_all(&path).expect("a directory where a file should be");
        let error = Prefs::default().save(&path).unwrap_err();
        assert!(error.contains("could not"), "{error}");
        let _ = std::fs::remove_dir_all(&path);
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
