//! Composer declarations and completion ownership. The screen owns presentation,
//! while this state owns what a command/source means and which source reads are live.

use std::collections::{HashMap, HashSet};

use misa_kit::intent::{Command, Source};
use misa_proto::preparation::SourceKind;
use misa_proto::view::Choice;

/// Client-side catalog for the composer, independent of any session connection.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Catalog {
    pub commands: Vec<Command>,
    pub sources: Vec<Source>,
}

#[derive(Default)]
pub(crate) struct ComposerCatalog {
    commands: Vec<Command>,
    raw_commands: HashSet<String>,
    sources: Vec<Source>,
    // Successful empty responses are held too. A failed request is not.
    resident: HashMap<String, (Vec<Choice>, bool)>,
    requests: HashSet<String>,
}

impl ComposerCatalog {
    pub fn declare_with_raw(&mut self, info: &Catalog, raw: &[Command]) {
        self.resident.clear();
        self.requests.clear();
        self.commands = info.commands.clone();
        self.raw_commands.clear();
        for command in raw {
            if !self.has_command(&command.id) {
                self.raw_commands.insert(command.id.clone());
                self.commands.push(command.clone());
            }
        }
        self.sources = info.sources.clone();
    }

    pub fn commands(&self) -> &[Command] {
        &self.commands
    }

    pub fn command(&self, id: &str) -> Option<&Command> {
        self.commands.iter().find(|command| command.id == id)
    }

    pub fn has_command(&self, id: &str) -> bool {
        self.command(id).is_some()
    }

    pub fn is_raw(&self, id: &str) -> bool {
        self.raw_commands.contains(id)
    }

    pub fn argument_index(&self, command: &str, argument: &str) -> Option<usize> {
        self.command(command)?
            .args
            .iter()
            .position(|arg| arg.name == argument)
    }

    pub fn has_source(&self, id: &str) -> bool {
        self.sources.iter().any(|source| source.id == id)
    }

    pub fn is_resident(&self, id: &str) -> bool {
        self.sources
            .iter()
            .any(|source| source.id == id && source.kind == SourceKind::Resident)
    }

    pub fn held(&self, source: &str) -> Option<&(Vec<Choice>, bool)> {
        self.resident.get(source)
    }

    /// True only when a resident read must be issued. An in-flight or held
    /// (including empty) response is not a new read.
    pub fn request(&mut self, source: &str) -> bool {
        self.is_resident(source)
            && !self.resident.contains_key(source)
            && self.requests.insert(source.to_string())
    }

    /// Dynamic sources ask per query; resident sources ask only while uncached
    /// and without an outstanding request.
    pub fn should_ask(&mut self, source: &str) -> bool {
        !self.is_resident(source) || self.request(source)
    }

    /// Cache a successful resident result, returning whether it is resident.
    /// Nonresident answers belong only to the matching picker query.
    pub fn received(&mut self, source: &str, items: &[Choice], truncated: bool) -> bool {
        if !self.is_resident(source) {
            return false;
        }
        self.requests.remove(source);
        self.resident
            .insert(source.into(), (items.to_vec(), truncated));
        true
    }

    /// An asynchronous reply is relevant only while its request is outstanding.
    /// Return whether the screen may apply it to the currently visible picker.
    pub fn completed(&mut self, source: &str, items: &[Choice], truncated: bool) -> bool {
        if !self.is_resident(source) {
            return true;
        }
        if !self.requests.remove(source) {
            return false;
        }
        self.resident
            .insert(source.into(), (items.to_vec(), truncated));
        true
    }

    pub fn failed(&mut self, source: &str) {
        self.requests.remove(source);
    }

    /// The declaration carries all candidate metadata, even before a subscription.
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
}
