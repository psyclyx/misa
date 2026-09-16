//! Incremental document consumption for surfaces. This tracks local delivery,
//! not a second protocol cursor. A slow surface explicitly rebuilds once from
//! the current replica; contiguous delivery retains original ordered operations.
use std::sync::Arc;

use misa_proto::{Fault, observation::Document};
use misa_protocol::observation::{Applied, MemberChange, MemberState, Replica, Status};

use crate::{
    ObservationId,
    driver::{Notice, Observation},
};

pub enum Update {
    Reset(Document),
    Changed {
        member: String,
        applied: Arc<Applied>,
    },
    Unavailable(Fault),
    Status(Status),
}

/// Explicit full materialization for exports and cold rendering. Interactive
/// retained renderers consume `Update` instead of copying history per append.
pub fn rendered(document: &misa_protocol::observation::DocumentState) -> misa_proto::Node {
    use misa_proto::{
        Node,
        view::{Span, State},
    };
    let mut tree = document.tree().snapshot();
    let mut overlay = Node::section("streams").id("streams");
    for stream in document.live().values() {
        let owner = stream
            .id
            .rsplit_once('.')
            .map_or(stream.id.as_str(), |(owner, _)| owner);
        if stream.text.is_empty() || document.tree().contains(owner) {
            continue;
        }
        overlay.children.push(
            Node::text(&stream.role, [Span::plain(&stream.text)])
                .id(&stream.id)
                .state(State::Streaming),
        );
    }
    if !overlay.children.is_empty() {
        if let Some(transcript) = tree
            .children
            .iter_mut()
            .find(|node| node.id == "transcript")
        {
            transcript.children.push(overlay);
        } else {
            tree.children.push(overlay);
        }
    }
    tree
}

pub struct Reader {
    member: String,
    observation: Option<ObservationId>,
    daemon: Option<String>,
    sequence: Option<u64>,
    baseline: bool,
}

/// Capture all local document readers at one owner publication boundary.
pub fn capture_many<'a>(
    observation: &Observation,
    readers: impl IntoIterator<Item = (&'a str, &'a mut Reader)>,
) -> Vec<(String, Update)> {
    let daemon = observation.daemon_identity();
    let mut readers = readers.into_iter().collect::<Vec<_>>();
    for (_, reader) in &mut readers {
        if reader.observation != Some(observation.id()) || reader.daemon.as_ref() != Some(&daemon) {
            reader.observation = Some(observation.id());
            reader.daemon = Some(daemon.clone());
            reader.invalidate();
        }
    }
    observation
        .inspect(|replica, notice| {
            readers
                .into_iter()
                .filter_map(|(id, reader)| {
                    reader
                        .read(replica, notice)
                        .map(|update| (id.to_string(), update))
                })
                .collect()
        })
        .unwrap_or_default()
}

impl Reader {
    /// Discard only this surface's delivery baseline after local decode/apply
    /// failure. The next capture rebuilds from the authoritative replica; no
    /// observation cursor or server state is changed.
    pub fn invalidate(&mut self) {
        self.sequence = None;
        self.baseline = false;
    }

    pub fn new(member: impl Into<String>) -> Self {
        Self {
            member: member.into(),
            observation: None,
            daemon: None,
            sequence: None,
            baseline: false,
        }
    }

    pub fn capture(&mut self, observation: &Observation) -> Option<Update> {
        let daemon = observation.daemon_identity();
        if self.observation != Some(observation.id()) || self.daemon.as_ref() != Some(&daemon) {
            self.observation = Some(observation.id());
            self.daemon = Some(daemon);
            self.invalidate();
        }
        observation
            .inspect(|replica, notice| self.read(replica, notice))
            .flatten()
    }

    fn read(&mut self, replica: &Replica, notice: Option<&Notice>) -> Option<Update> {
        let notice = notice?;
        if self.sequence == Some(notice.sequence) {
            return None;
        }
        let contiguous =
            self.sequence.and_then(|value| value.checked_add(1)) == Some(notice.sequence);
        self.sequence = Some(notice.sequence);
        if !matches!(replica.status(), Status::Current) {
            return Some(Update::Status(replica.status().clone()));
        }
        let members = replica
            .current()
            .expect("current replica has its selected members");
        let member = members.get(&self.member)?;
        if let MemberState::Unavailable(fault) = member {
            self.baseline = false;
            return Some(Update::Unavailable(fault.clone()));
        }
        let MemberState::Document(document) = member else {
            self.baseline = false;
            return Some(Update::Unavailable(Fault::query(
                "Selected member is not a document",
            )));
        };
        if contiguous && self.baseline {
            if let Applied::Changed(changes) = notice.applied.as_ref() {
                match changes.get(&self.member) {
                    Some(MemberChange::Document { .. }) => {
                        return Some(Update::Changed {
                            member: self.member.clone(),
                            applied: notice.applied.clone(),
                        });
                    }
                    None => return None,
                    Some(MemberChange::Replace) => {}
                }
            }
        }
        self.baseline = true;
        Some(Update::Reset(document.snapshot()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{
        Node, Query,
        observation::{
            Content, Delta, Encoding, Handle, Member, Publication, Scope, ScopeId, Selection,
            Snapshot,
        },
        sync::{Stream, StreamUpdate, Version},
    };
    use std::collections::BTreeMap;

    #[test]
    fn contiguous_appends_stay_incremental_and_skipped_notifications_reset() {
        let handle = Handle {
            id: 1,
            generation: 1,
        };
        let selection = Selection {
            scope: Scope {
                id: ScopeId::Session { id: "one".into() },
                incarnation: "run".into(),
            },
            members: BTreeMap::from([(
                "body".into(),
                Member {
                    query: Query::new("document"),
                    contract: "document@1".into(),
                    encoding: Encoding::Document,
                    optional: false,
                },
            )]),
        };
        let mut replica = Replica::new(handle, selection).unwrap();
        let applied = replica
            .apply(Publication::Snapshot {
                handle,
                snapshot: Snapshot {
                    position: 0,
                    members: BTreeMap::from([(
                        "body".into(),
                        Content::Document(Document {
                            version: Version {
                                epoch: "run".into(),
                                rev: 0,
                            },
                            tree: Node::section("document").id("document"),
                            streams: vec![Stream {
                                id: "text".into(),
                                role: "message".into(),
                                text: String::new(),
                            }],
                        }),
                    )]),
                },
            })
            .unwrap()
            .unwrap();
        let mut reader = Reader::new("body");
        assert!(matches!(
            reader.read(
                &replica,
                Some(&Notice {
                    sequence: 1,
                    applied: Arc::new(applied)
                })
            ),
            Some(Update::Reset(_))
        ));
        for position in 1..=3 {
            let applied = replica
                .apply(Publication::Update {
                    handle,
                    update: misa_proto::observation::Update {
                        from: position - 1,
                        position,
                        members: BTreeMap::from([(
                            "body".into(),
                            Delta::Document {
                                changes: vec![],
                                streams: vec![StreamUpdate::Append {
                                    id: "text".into(),
                                    offset: (position as usize - 1) * 2,
                                    text: "é".into(),
                                }],
                            },
                        )]),
                    },
                })
                .unwrap()
                .unwrap();
            let notice = Notice {
                sequence: position + 1,
                applied: Arc::new(applied),
            };
            if position == 1 {
                assert!(matches!(
                    reader.read(&replica, Some(&notice)),
                    Some(Update::Changed { .. })
                ));
                assert!(reader.read(&replica, Some(&notice)).is_none());
                reader.invalidate();
                let Some(Update::Reset(document)) = reader.read(&replica, Some(&notice)) else {
                    panic!("local apply failure must rebaseline even without a new publication")
                };
                assert_eq!(document.streams[0].text, "é");
            } else if position == 3 {
                let Some(Update::Reset(document)) = reader.read(&replica, Some(&notice)) else {
                    panic!("missed local delivery requires a coherent reset")
                };
                assert_eq!(document.streams[0].text, "ééé");
            }
        }
        reader.invalidate();
        replica.mark_disconnected();
        assert!(matches!(
            reader.read(
                &replica,
                Some(&Notice {
                    sequence: 5,
                    applied: Arc::new(Applied::Stale)
                })
            ),
            Some(Update::Status(_))
        ));
        let applied = replica
            .apply(Publication::Update {
                handle,
                update: misa_proto::observation::Update {
                    from: 3,
                    position: 4,
                    members: BTreeMap::new(),
                },
            })
            .unwrap()
            .unwrap();
        assert!(
            matches!(
                reader.read(
                    &replica,
                    Some(&Notice {
                        sequence: 6,
                        applied: Arc::new(applied)
                    })
                ),
                Some(Update::Reset(_))
            ),
            "a status is not a render baseline"
        );
        let MemberState::Document(document) = &replica.current().unwrap()["body"] else {
            panic!("document")
        };
        let exported = rendered(document);
        assert_eq!(exported.children[0].id, "streams");
        assert_eq!(exported.children[0].children[0].id, "text");
        let misa_proto::view::Kind::Text { spans } = &exported.children[0].children[0].kind else {
            panic!("live text")
        };
        assert_eq!(spans[0].text, "ééé");
        assert!(
            document.tree().snapshot().children.is_empty(),
            "export must not put live text in canonical state"
        );
    }
}
