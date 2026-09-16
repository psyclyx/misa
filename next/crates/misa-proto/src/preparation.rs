//! Local input preparation refers to exported sources, reads and commands.
use serde::{Deserialize, Serialize};
use crate::observation::Member;

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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Arg {
    pub name: String,
    pub label: String,
    /// A command cannot run without it. This is what lets a client narrow a
    /// command into its arguments rather than sending it and being refused.
    #[serde(default)]
    pub required: bool,
    /// A completion source's id, when the argument has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

impl Arg {
    pub fn new(name: impl Into<String>, label: impl Into<String>) -> Arg {
        Arg {
            name: name.into(),
            label: label.into(),
            required: false,
            source: None,
            placeholder: None,
        }
    }

    pub fn required(mut self) -> Arg {
        self.required = true;
        self
    }

    pub fn from(mut self, source: impl Into<String>) -> Arg {
        self.source = Some(source.into());
        self
    }
}


#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Small and slow-changing: a client subscribes to it and filters locally, so
    /// nothing is asked for while someone is typing.
    Resident,
    /// Large, dynamic, or a capability: a client asks, naming a prefix.
    OnDemand,
}


use misa_value::Value;
/// One candidate: the same shape a resident source's items decode to.
///
/// Reusing [`Choice`](crate::view::Choice) means a picker built for one path works
/// unchanged for the other, which is what keeps the two paths indistinguishable to
/// a client once the candidates are in hand.
pub fn candidates(value: &Value) -> Vec<crate::view::Choice> {
    let mut out = Vec::new();
    let Some(items) = value.as_list() else {
        return out;
    };
    for item in items {
        let Some(value) = item.get("value").and_then(Value::as_str) else {
            continue;
        };
        out.push(crate::view::Choice {
            value: value.to_string(),
            label: item
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or(value)
                .to_string(),
            detail: item.get("detail").and_then(Value::as_str).map(str::to_string),
        });
    }
    out
}

/// How many candidates a source will answer with at most.
pub const DEFAULT_CANDIDATES: u32 = 50;
