//! Scoped query selections and their atomic publication contract.
//!
//! Selection is data naming installed queries. An owner publishes one coherent
//! result; the payload retains value, document and append semantics independently
//! of the lifecycle that carries it.
use std::collections::BTreeMap;

use misa_value::Value;
use serde::{Deserialize, Serialize};

use crate::{Fault, Node, Query, sync::{Change, Stream, StreamUpdate, Version}};

pub const MAX_SELECTION_MEMBERS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScopeId {
    Daemon,
    Session { id: String },
}

/// Incarnation belongs to the owner, not the socket or the human-readable label.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Scope {
    pub id: ScopeId,
    pub incarnation: String,
}

impl Scope {
    pub fn validate(&self) -> Result<(), Fault> {
        if self.incarnation.is_empty() { return Err(Fault::protocol("Scope needs an incarnation")); }
        if matches!(&self.id, ScopeId::Session { id } if id.is_empty()) {
            return Err(Fault::protocol("Session scope needs an identity"));
        }
        Ok(())
    }
}

/// The generation prevents late queued traffic from reaching a reused handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Handle {
    pub id: u64,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Encoding { Value, Document }

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub query: Query,
    /// Installed exported result contract. Its revision is part of its identity.
    pub contract: String,
    pub encoding: Encoding,
    /// Required failures fault the whole selection; optional failures are data
    /// in this member, published at the same boundary as its successful siblings.
    #[serde(default)]
    pub optional: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Selection {
    pub scope: Scope,
    pub members: BTreeMap<String, Member>,
}

impl Selection {
    pub fn validate(&self) -> Result<(), Fault> {
        self.scope.validate()?;
        if self.members.is_empty() || self.members.len() > MAX_SELECTION_MEMBERS {
            return Err(Fault::query("A selection needs between 1 and 64 members"));
        }
        for (name, member) in &self.members {
            if name.is_empty() || member.query.id.is_empty() || member.contract.is_empty() {
                return Err(Fault::query("Selection members need a name, query and result contract"));
            }
        }
        Ok(())
    }
}

/// A conversation document owns its live overlay. Consumers never have to know
/// that a second hidden query is required for correct message settlement.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Document {
    pub version: Version,
    pub tree: Node,
    pub streams: Vec<Stream>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum Content {
    Value(Value),
    Document(Document),
    Unavailable(Fault),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Delta {
    Replace { content: Content },
    /// Both lists are one application unit. In particular, durable insertion
    /// cannot become visible separately from retirement of its live text.
    Document { changes: Vec<Change>, streams: Vec<StreamUpdate> },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub position: u64,
    pub members: BTreeMap<String, Content>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Update {
    pub from: u64,
    pub position: u64,
    pub members: BTreeMap<String, Delta>,
}

/// A complete live replica can resume publication delivery. A disk checkpoint
/// that omits live text retains document versions instead. The owner may replay
/// canonical changes while supplying fresh live state; token traffic does not
/// have to occupy canonical history slots.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Resume {
    pub selection: Selection,
    pub publication: Option<u64>,
    pub documents: BTreeMap<String, Version>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Recovery {
    Replace { content: Content },
    Document { from: Version, changes: Vec<Change>, streams: Vec<Stream> },
}

/// Recovery supplies every member at one current owner publication. It can
/// replace small values while reusing a large retained canonical document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recovered {
    pub position: u64,
    pub members: BTreeMap<String, Recovery>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Publication {
    Snapshot { handle: Handle, snapshot: Snapshot },
    Update { handle: Handle, update: Update },
    Recovered { handle: Handle, recovered: Recovered },
    Fault { handle: Handle, fault: Fault },
    Closed { handle: Handle, reason: Fault },
}

impl Publication {
    pub fn handle(&self) -> Handle {
        match self {
            Self::Snapshot { handle, .. } | Self::Update { handle, .. } |
            Self::Recovered { handle, .. } | Self::Fault { handle, .. } |
            Self::Closed { handle, .. } => *handle,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_composed_publication_keeps_domain_values_and_presentation_distinct() {
        let message = Publication::Update {
            handle: Handle { id: 4, generation: 2 },
            update: Update { from: 19, position: 21, members: BTreeMap::from([
                ("usage".into(), Delta::Replace { content: Content::Value(Value::Int(12)) }),
                ("conversation".into(), Delta::Document { changes: vec![], streams: vec![
                    StreamUpdate::Append { id: "attempt.1".into(), offset: 2, text: "🙂".into() },
                ] }),
            ]) },
        };
        let bytes = crate::chunk::encode(&message).unwrap();
        let mut decoder = crate::chunk::Decoder::new();
        for bytes in bytes.chunks(3) { decoder.push(bytes).unwrap(); }
        let actual: Publication = crate::chunk::decode(&decoder.next().unwrap().unwrap()).unwrap();
        assert_eq!(actual, message);
    }
}
