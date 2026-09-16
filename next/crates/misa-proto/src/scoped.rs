//! Proposed major-version envelopes. The legacy session-attached protocol stays
//! on its existing version until all consumers cut over together.
use crate::{
    ClientInfo, Fault,
    invocation::{Invocation, Reply},
    observation::{Handle, Publication, Resume, Scope, ScopeId, Selection, Snapshot},
};
use serde::{Deserialize, Serialize};

pub const VERSION: u16 = 3;
/// Default live observations per relationship, including bootstrap queries.
/// Owners may refuse earlier under policy/resource pressure; refusal is scoped.
pub const DEFAULT_OBSERVATION_LIMIT: usize = 128;
pub const ALPN: &[u8] = b"/misa/scoped/3";

/// Server-created ordered data lanes. These carry existing publications/results;
/// they do not introduce another application lifecycle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "lane", rename_all = "snake_case")]
pub enum Lane {
    Observation { handle: Handle },
    Read { id: u64 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Read {
    pub id: u64,
    pub selection: Selection,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReadReply {
    pub id: u64,
    pub outcome: ReadOutcome,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum ReadOutcome {
    Snapshot { snapshot: Snapshot },
    Rejected { fault: Fault },
}

/// Admission/pairing precedes this authenticated application protocol. A greeting
/// identifies a daemon; it does not select a session or enumerate its resources.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello {
        version: u16,
        client: ClientInfo,
    },
    Read {
        read: Read,
    },
    Observe {
        handle: Handle,
        selection: Selection,
        resume: Option<Resume>,
    },
    CancelObservation {
        handle: Handle,
    },
    Invoke {
        invocation: Invocation,
    },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum ServerMessage {
    Welcome {
        version: u16,
        daemon: String,
        scope: Scope,
    },
    ReadReply {
        reply: ReadReply,
    },
    Publication {
        publication: Publication,
    },
    Reply {
        reply: Reply,
    },
    /// Connection/protocol fault. Query faults and invocation results retain their
    /// own handles/identities in the corresponding envelopes.
    Fault {
        fault: Fault,
    },
}

pub fn validate_version(version: u16) -> Result<(), Fault> {
    if version != VERSION {
        return Err(Fault::new(
            "protocol_version",
            "Scoped protocol major version does not match",
        ));
    }
    Ok(())
}
impl ClientMessage {
    /// Structural checks only; routing must enforce greeting order and authority.
    pub fn validate(&self) -> Result<(), Fault> {
        match self {
            Self::Hello { version, client } => {
                validate_version(*version)?;
                if client.name.is_empty() {
                    return Err(Fault::protocol("Client greeting needs a name"));
                }
            }
            Self::Read { read } => read.selection.validate()?,
            Self::Observe {
                selection, resume, ..
            } => {
                selection.validate()?;
                if let Some(resume) = resume {
                    if resume.selection != *selection {
                        return Err(Fault::protocol("Resume belongs to another selection"));
                    }
                    for (name, version) in &resume.documents {
                        if version.epoch.is_empty()
                            || !selection.members.get(name).is_some_and(|member| {
                                member.encoding == crate::observation::Encoding::Document
                            })
                        {
                            return Err(Fault::protocol("Resume names an invalid document member"));
                        }
                    }
                }
            }
            Self::Invoke { invocation } => {
                invocation.scope.validate()?;
                if invocation.command.is_empty() {
                    return Err(Fault::protocol("Invocation needs a command"));
                }
            }
            Self::CancelObservation { .. } => {}
        }
        Ok(())
    }
}
impl ServerMessage {
    /// Check the greeting against the transport-authenticated daemon identity.
    /// Version negotiation never silently downgrades to attached-session semantics.
    pub fn validate_welcome(&self, authenticated_daemon: &str) -> Result<&Scope, Fault> {
        let Self::Welcome {
            version,
            daemon,
            scope,
        } = self
        else {
            return Err(Fault::protocol("Expected daemon greeting"));
        };
        validate_version(*version)?;
        if daemon.is_empty() || daemon != authenticated_daemon {
            return Err(Fault::protocol(
                "Daemon greeting identity differs from authenticated peer",
            ));
        }
        scope.validate()?;
        if scope.id != ScopeId::Daemon {
            return Err(Fault::protocol("Greeting must identify the daemon scope"));
        }
        Ok(scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Query,
        invocation::Outcome,
        observation::{Content, Encoding, Member},
    };
    use misa_value::Value;
    use std::collections::BTreeMap;
    fn scope() -> Scope {
        Scope {
            id: ScopeId::Daemon,
            incarnation: "run".into(),
        }
    }
    fn selection() -> Selection {
        Selection {
            scope: scope(),
            members: BTreeMap::from([(
                "sessions".into(),
                Member {
                    query: Query::new("sessions"),
                    contract: "sessions.v1".into(),
                    encoding: Encoding::Value,
                    optional: false,
                },
            )]),
        }
    }
    fn roundtrip<T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(
        value: T,
    ) {
        let wire = crate::chunk::encode(&value).unwrap();
        let mut decoder = crate::chunk::Decoder::new();
        for bytes in wire.chunks(7) {
            decoder.push(bytes).unwrap();
        }
        let decoded: T = crate::chunk::decode(&decoder.next().unwrap().unwrap()).unwrap();
        assert_eq!(value, decoded);
    }
    #[test]
    fn envelopes_roundtrip_without_an_attached_session() {
        let handle = Handle {
            id: 4,
            generation: 2,
        };
        let snapshot = Snapshot {
            position: 7,
            members: BTreeMap::from([("sessions".into(), Content::Value(Value::list([])))]),
        };
        for message in [
            ClientMessage::Hello {
                version: VERSION,
                client: ClientInfo::new("test", "1"),
            },
            ClientMessage::Read {
                read: Read {
                    id: 1,
                    selection: selection(),
                },
            },
            ClientMessage::Observe {
                handle,
                selection: selection(),
                resume: None,
            },
            ClientMessage::CancelObservation { handle },
            ClientMessage::Invoke {
                invocation: Invocation {
                    id: 2,
                    scope: scope(),
                    command: "session.create".into(),
                    input: Value::map([]),
                },
            },
        ] {
            message.validate().unwrap();
            roundtrip(message);
        }
        for message in [
            ServerMessage::Welcome {
                version: VERSION,
                daemon: "peer".into(),
                scope: scope(),
            },
            ServerMessage::ReadReply {
                reply: ReadReply {
                    id: 1,
                    outcome: ReadOutcome::Snapshot {
                        snapshot: snapshot.clone(),
                    },
                },
            },
            ServerMessage::Publication {
                publication: Publication::Snapshot { handle, snapshot },
            },
            ServerMessage::Reply {
                reply: Reply {
                    id: 2,
                    outcome: Outcome::Completed {
                        value: Value::Bool(true),
                    },
                },
            },
            ServerMessage::Fault {
                fault: Fault::protocol("bad message"),
            },
        ] {
            roundtrip(message);
        }
    }
    #[test]
    fn incompatible_or_misattributed_greetings_are_rejected() {
        for version in [0, crate::PROTOCOL_VERSION, VERSION + 1] {
            assert!(
                ClientMessage::Hello {
                    version,
                    client: ClientInfo::new("test", "1")
                }
                .validate()
                .is_err()
            );
            assert!(
                ServerMessage::Welcome {
                    version,
                    daemon: "peer".into(),
                    scope: scope()
                }
                .validate_welcome("peer")
                .is_err()
            );
        }
        let greeting = ServerMessage::Welcome {
            version: VERSION,
            daemon: "peer".into(),
            scope: scope(),
        };
        assert_eq!(greeting.validate_welcome("peer").unwrap(), &scope());
        assert!(greeting.validate_welcome("other").is_err());
        assert!(
            ServerMessage::Welcome {
                version: VERSION,
                daemon: "peer".into(),
                scope: Scope {
                    id: ScopeId::Session { id: "demo".into() },
                    incarnation: "run".into()
                }
            }
            .validate_welcome("peer")
            .is_err()
        );
    }
    #[test]
    fn resume_must_match_selection_and_document_contracts() {
        let mut resume = Resume {
            selection: selection(),
            publication: Some(3),
            documents: BTreeMap::new(),
        };
        resume.selection.scope.incarnation = "old".into();
        let handle = Handle {
            id: 1,
            generation: 1,
        };
        assert!(
            ClientMessage::Observe {
                handle,
                selection: selection(),
                resume: Some(resume)
            }
            .validate()
            .is_err()
        );
        let resume = Resume {
            selection: selection(),
            publication: Some(3),
            documents: BTreeMap::from([(
                "sessions".into(),
                crate::sync::Version {
                    epoch: "run".into(),
                    rev: 1,
                },
            )]),
        };
        assert!(
            ClientMessage::Observe {
                handle,
                selection: selection(),
                resume: Some(resume)
            }
            .validate()
            .is_err()
        );
    }
}
