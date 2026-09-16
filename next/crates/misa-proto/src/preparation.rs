//! Local input preparation refers to exported sources, reads and commands.
use serde::{Deserialize, Serialize};
use crate::{observation::Member, wire::{Arg, SourceKind}};

pub const SOURCES: &str = "completion.catalog";
pub const SEARCH: &str = "completion.search";
pub const SHORTCUTS: &str = "commands.shortcuts";
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Candidates { pub items: Vec<crate::view::Choice>, pub truncated: bool }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub label: String,
    pub kind: SourceKind,
    /// Resident members use no arguments; finite members use source, prefix, limit.
    pub member: Member,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Shortcut {
    pub id: String,
    pub label: String,
    pub description: String,
    pub args: Vec<Arg>,
    pub target: Target,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Target {
    Command { command: String },
    Read { member: Member },
}
