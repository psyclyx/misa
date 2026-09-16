//! One replica owner for every surface. Application is synchronous and complete
//! before callers receive an update notification. A bad batch invalidates the
//! accumulator; partially applied members are never exposed as a current result.
use std::collections::BTreeMap;

use misa_proto::{
    Fault,
    observation::*,
    sync::{Change, IndexedTree, Stream, StreamUpdate, Version},
};
use misa_value::Value;

/// The decoded operations are retained, in order, only after the complete
/// transaction succeeds. Render caches can apply insert/remove sequences without
/// reconstructing vanished intermediate nodes or copying the existing document.
#[derive(Clone, Debug, PartialEq)]
pub enum MemberChange {
    Replace,
    Document {
        tree: Vec<misa_proto::sync::ViewOp>,
        live: Vec<StreamUpdate>,
        reset_live: bool,
    },
}
#[derive(Clone, Debug, PartialEq)]
pub enum Applied {
    Reset,
    Changed(BTreeMap<String, MemberChange>),
    Stale,
    Closed,
}

#[derive(Debug)]
pub struct DocumentState {
    version: Version,
    tree: IndexedTree,
    streams: BTreeMap<String, Stream>,
}

impl DocumentState {
    fn new(document: Document) -> Result<Self, String> {
        if document.version.epoch.is_empty() {
            return Err("Document needs an incarnation".into());
        }
        misa_proto::view::validate(&document.tree).map_err(|fault| fault.to_string())?;
        let mut tree = document.tree;
        misa_proto::sync::address(&mut tree);
        // Generated identities can collide with explicit identities.
        misa_proto::view::validate(&tree).map_err(|fault| fault.to_string())?;
        Ok(Self {
            version: document.version,
            tree: IndexedTree::new(tree),
            streams: streams(document.streams)?,
        })
    }

    pub fn version(&self) -> &Version {
        &self.version
    }
    pub fn tree(&self) -> &IndexedTree {
        &self.tree
    }
    pub fn live(&self) -> &BTreeMap<String, Stream> {
        &self.streams
    }

    /// Materialization is explicit. Applying appends does not copy the tree.
    pub fn snapshot(&self) -> Document {
        Document {
            version: self.version.clone(),
            tree: self.tree.snapshot(),
            streams: self.streams.values().cloned().collect(),
        }
    }

    fn changes(&mut self, changes: Vec<Change>) -> Result<Vec<misa_proto::sync::ViewOp>, String> {
        let mut edits = Vec::new();
        for change in changes {
            if change.from != self.version
                || change.version.epoch != self.version.epoch
                || change.version.rev <= self.version.rev
            {
                return Err("Document revision gap".into());
            }
            for op in change.ops {
                self.tree.apply(&op)?;
                edits.push(op);
            }
            self.version = change.version;
        }
        Ok(edits)
    }

    fn append(&mut self, updates: Vec<StreamUpdate>) -> Result<Vec<StreamUpdate>, String> {
        for update in &updates {
            match update {
                StreamUpdate::Current { stream } => {
                    validate_stream(stream)?;
                    self.streams.insert(stream.id.clone(), stream.clone());
                }
                StreamUpdate::Append { id, offset, text } => {
                    let stream = self
                        .streams
                        .get_mut(id)
                        .ok_or("Text append needs a current stream")?;
                    if stream.text.len() != *offset {
                        return Err("Stream byte offset gap".into());
                    }
                    stream.text.push_str(text);
                }
                StreamUpdate::End { id } => {
                    if self.streams.remove(id).is_none() {
                        return Err("Unknown stream ended".into());
                    }
                }
            }
        }
        Ok(updates)
    }
}

fn validate_stream(stream: &Stream) -> Result<(), String> {
    if stream.id.is_empty() || stream.role.is_empty() {
        return Err("A live stream needs an identity and role".into());
    }
    Ok(())
}

fn streams(values: Vec<Stream>) -> Result<BTreeMap<String, Stream>, String> {
    let mut out = BTreeMap::new();
    for stream in values {
        validate_stream(&stream)?;
        if out.insert(stream.id.clone(), stream).is_some() {
            return Err("Duplicate live stream identity".into());
        }
    }
    Ok(out)
}

#[derive(Debug)]
pub enum MemberState {
    Value(Value),
    Document(DocumentState),
    Unavailable(Fault),
}

impl MemberState {
    fn new(definition: &Member, content: Content) -> Result<Self, String> {
        match (definition.encoding, content) {
            (Encoding::Value, Content::Value(value)) => Ok(Self::Value(value)),
            (Encoding::Document, Content::Document(document)) => {
                Ok(Self::Document(DocumentState::new(document)?))
            }
            (_, Content::Unavailable(fault)) if definition.optional => Ok(Self::Unavailable(fault)),
            _ => Err("Member content does not match its selected contract".into()),
        }
    }

    fn content(&self) -> Content {
        match self {
            Self::Value(value) => Content::Value(value.clone()),
            Self::Document(document) => Content::Document(document.snapshot()),
            Self::Unavailable(fault) => Content::Unavailable(fault.clone()),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Awaiting,
    Current,
    /// The last complete result is still available, explicitly stale.
    Stale(Fault),
    /// Invalid updates leave no readable partial replica.
    Recovering(String),
    Closed(Fault),
}

/// Checkpoints save the selected contracts together with exactly the represented
/// state. Omitting transient text clears publication resumption, while retaining
/// canonical versions for independent canonical replay.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Checkpoint {
    pub resume: Resume,
    pub members: BTreeMap<String, Content>,
}

pub struct Replica {
    handle: Handle,
    selection: Selection,
    position: Option<u64>,
    needs_recovery: bool,
    members: BTreeMap<String, MemberState>,
    status: Status,
}

impl Replica {
    pub fn new(handle: Handle, selection: Selection) -> Result<Self, Fault> {
        selection.validate()?;
        Ok(Self {
            handle,
            selection,
            position: None,
            needs_recovery: false,
            members: BTreeMap::new(),
            status: Status::Awaiting,
        })
    }
    /// Reconnect retains the exact replica and its cursor, but does not claim it
    /// is fresh until the new observation answers. No snapshot copy is required.
    pub fn rebind(&mut self, handle: Handle) -> Result<(), String> {
        if matches!(self.status, Status::Closed(_)) {
            return Err("Closed observations cannot be rebound".into());
        }
        if handle == self.handle {
            return Err("Rebinding requires a fresh handle".into());
        }
        self.handle = handle;
        self.mark_disconnected();
        Ok(())
    }
    /// Transport loss differs from a required-query fault: the complete replica
    /// remains eligible for publication replay on reconnect.
    pub fn mark_disconnected(&mut self) {
        if !matches!(self.status, Status::Closed(_) | Status::Recovering(_)) {
            self.status = Status::Stale(Fault::new(
                "observation.disconnected",
                "Connection is unavailable",
            ));
        }
    }
    pub fn selection(&self) -> &Selection {
        &self.selection
    }
    pub fn status(&self) -> &Status {
        &self.status
    }
    pub fn position(&self) -> Option<u64> {
        self.position
    }
    pub fn current(&self) -> Option<&BTreeMap<String, MemberState>> {
        matches!(self.status, Status::Current).then_some(&self.members)
    }
    pub fn last_good(&self) -> Option<&BTreeMap<String, MemberState>> {
        (matches!(self.status, Status::Current | Status::Stale(_))
            && self.members.len() == self.selection.members.len())
        .then_some(&self.members)
    }

    pub fn resume(&self) -> Resume {
        Resume {
            selection: self.selection.clone(),
            publication: if self.needs_recovery {
                None
            } else {
                self.position
            },
            documents: self
                .members
                .iter()
                .filter_map(|(name, member)| match member {
                    MemberState::Document(document) => {
                        Some((name.clone(), document.version.clone()))
                    }
                    _ => None,
                })
                .collect(),
        }
    }

    pub fn checkpoint(&self, include_live: bool) -> Option<Checkpoint> {
        self.last_good()?;
        let mut resume = self.resume();
        let members = self
            .members
            .iter()
            .map(|(name, member)| {
                let content = match member {
                    MemberState::Document(document) if !include_live => {
                        resume.publication = None;
                        Content::Document(Document {
                            version: document.version.clone(),
                            tree: document.tree.snapshot(),
                            streams: vec![],
                        })
                    }
                    member => member.content(),
                };
                (name.clone(), content)
            })
            .collect();
        Some(Checkpoint { resume, members })
    }

    pub fn restore(
        handle: Handle,
        expected: Selection,
        checkpoint: Checkpoint,
    ) -> Result<Self, String> {
        if checkpoint.resume.selection != expected {
            return Err("Checkpoint belongs to another selection or owner incarnation".into());
        }
        let mut replica = Self::new(handle, expected).map_err(|fault| fault.message)?;
        replica.replace_members(checkpoint.members)?;
        if replica.resume().documents != checkpoint.resume.documents {
            return Err("Checkpoint document versions do not match saved content".into());
        }
        replica.position = checkpoint.resume.publication;
        replica.needs_recovery = replica.position.is_none();
        replica.status = Status::Stale(Fault::new(
            "observation.restored",
            "Restored state needs synchronization",
        ));
        Ok(replica)
    }

    /// The caller notifies consumers only after Ok(Some(applied)). Old generations are
    /// harmless; a closed handle cannot be resurrected by queued snapshots.
    pub fn apply(&mut self, publication: Publication) -> Result<Option<Applied>, String> {
        if publication.handle() != self.handle || matches!(self.status, Status::Closed(_)) {
            return Ok(None);
        }
        let result = self.apply_inner(publication);
        if let Err(error) = &result {
            self.members.clear();
            self.position = None;
            self.status = Status::Recovering(error.clone());
        }
        result
    }

    fn apply_inner(&mut self, publication: Publication) -> Result<Option<Applied>, String> {
        let applied = match publication {
            Publication::Snapshot { snapshot, .. } => {
                if self
                    .position
                    .is_some_and(|position| position > snapshot.position)
                {
                    return Err("Snapshot would rewind the observation".into());
                }
                self.replace_members(snapshot.members)?;
                self.position = Some(snapshot.position);
                self.needs_recovery = false;
                self.status = Status::Current;
                Applied::Reset
            }
            Publication::Update { update, .. } => {
                if self.position != Some(update.from) || update.position <= update.from {
                    return Err("Observation publication gap".into());
                }
                // An incomplete saved replica cannot accept live appends before recovery.
                if self.needs_recovery || !matches!(self.status, Status::Current | Status::Stale(_))
                {
                    return Err("Update needs a complete replica".into());
                }
                let mut changed = BTreeMap::new();
                for (name, delta) in update.members {
                    let definition = self
                        .selection
                        .members
                        .get(&name)
                        .ok_or("Update names an unselected member")?;
                    match delta {
                        Delta::Replace { content } => {
                            self.members
                                .insert(name.clone(), MemberState::new(definition, content)?);
                            changed.insert(name, MemberChange::Replace);
                        }
                        Delta::Document { changes, streams } => {
                            let Some(MemberState::Document(document)) = self.members.get_mut(&name)
                            else {
                                return Err("Document delta needs a document member".into());
                            };
                            let tree = document.changes(changes)?;
                            let live = document.append(streams)?;
                            changed.insert(
                                name,
                                MemberChange::Document {
                                    tree,
                                    live,
                                    reset_live: false,
                                },
                            );
                        }
                    }
                }
                self.position = Some(update.position);
                self.needs_recovery = false;
                self.status = Status::Current;
                Applied::Changed(changed)
            }
            Publication::Recovered { recovered, .. } => {
                if self
                    .position
                    .is_some_and(|position| position > recovered.position)
                {
                    return Err("Recovery would rewind the observation".into());
                }
                self.check_member_names(recovered.members.keys())?;
                let mut changed = BTreeMap::new();
                for (name, recovery) in recovered.members {
                    match recovery {
                        Recovery::Replace { content } => {
                            self.members.insert(
                                name.clone(),
                                MemberState::new(&self.selection.members[&name], content)?,
                            );
                            changed.insert(name, MemberChange::Replace);
                        }
                        Recovery::Document {
                            from,
                            changes,
                            streams,
                        } => {
                            let Some(MemberState::Document(document)) = self.members.get_mut(&name)
                            else {
                                return Err("Canonical recovery needs a retained document".into());
                            };
                            if document.version != from {
                                return Err(
                                    "Canonical recovery base differs from retained document".into(),
                                );
                            }
                            let tree = document.changes(changes)?;
                            document.streams = self::streams(streams)?;
                            changed.insert(
                                name,
                                MemberChange::Document {
                                    tree,
                                    live: Vec::new(),
                                    reset_live: true,
                                },
                            );
                        }
                    }
                }
                self.position = Some(recovered.position);
                self.needs_recovery = false;
                self.status = Status::Current;
                Applied::Changed(changed)
            }
            Publication::Fault { fault, .. } => {
                // A failed coherent selection must be re-established as a whole.
                // Retain canonical bases, but never resume sparse updates from it.
                self.needs_recovery = true;
                self.status = Status::Stale(fault);
                Applied::Stale
            }
            Publication::Closed { reason, .. } => {
                self.members.clear();
                self.position = None;
                self.status = Status::Closed(reason);
                Applied::Closed
            }
        };
        Ok(Some(applied))
    }

    fn check_member_names<'a>(
        &self,
        names: impl Iterator<Item = &'a String>,
    ) -> Result<(), String> {
        if !names.eq(self.selection.members.keys()) {
            return Err("Snapshot/recovery must answer exactly the selected members".into());
        }
        Ok(())
    }

    fn replace_members(&mut self, contents: BTreeMap<String, Content>) -> Result<(), String> {
        self.check_member_names(contents.keys())?;
        let members = contents
            .into_iter()
            .map(|(name, content)| {
                MemberState::new(&self.selection.members[&name], content)
                    .map(|member| (name, member))
            })
            .collect::<Result<_, _>>()?;
        self.members = members;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{Node, Query, sync::ViewOp};

    const HANDLE: Handle = Handle {
        id: 1,
        generation: 1,
    };
    fn version(rev: u64) -> Version {
        Version {
            epoch: "session-incarnation".into(),
            rev,
        }
    }
    fn selection() -> Selection {
        Selection {
            scope: Scope {
                id: ScopeId::Session { id: "demo".into() },
                incarnation: "session-incarnation".into(),
            },
            members: BTreeMap::from([
                (
                    "a-usage".into(),
                    Member {
                        query: Query::new("usage"),
                        contract: "usage.v1".into(),
                        encoding: Encoding::Value,
                        optional: false,
                    },
                ),
                (
                    "conversation".into(),
                    Member {
                        query: Query::new("conversation"),
                        contract: "document.v1".into(),
                        encoding: Encoding::Document,
                        optional: false,
                    },
                ),
            ]),
        }
    }
    fn snapshot() -> Publication {
        Publication::Snapshot {
            handle: HANDLE,
            snapshot: Snapshot {
                position: 3,
                members: BTreeMap::from([
                    ("a-usage".into(), Content::Value(Value::Int(0))),
                    (
                        "conversation".into(),
                        Content::Document(Document {
                            version: version(1),
                            tree: Node::section("conversation").id("root"),
                            streams: vec![Stream {
                                id: "attempt.1".into(),
                                role: "message.assistant".into(),
                                text: "é".into(),
                            }],
                        }),
                    ),
                ]),
            },
        }
    }
    fn replica() -> Replica {
        let mut replica = Replica::new(HANDLE, selection()).unwrap();
        replica.apply(snapshot()).unwrap();
        replica
    }
    fn doc(replica: &Replica) -> &DocumentState {
        let MemberState::Document(document) = &replica.current().unwrap()["conversation"] else {
            panic!("document member")
        };
        document
    }
    fn finish() -> Publication {
        Publication::Update {
            handle: HANDLE,
            update: Update {
                from: 3,
                position: 4,
                members: BTreeMap::from([
                    (
                        "a-usage".into(),
                        Delta::Replace {
                            content: Content::Value(Value::Int(2)),
                        },
                    ),
                    (
                        "conversation".into(),
                        Delta::Document {
                            changes: vec![Change {
                                from: version(1),
                                version: version(2),
                                ops: vec![ViewOp::Insert {
                                    parent: "root".into(),
                                    before: None,
                                    node: Node::text(
                                        "message.assistant",
                                        [misa_proto::view::Span::plain("é")],
                                    )
                                    .id("message.1"),
                                }],
                            }],
                            streams: vec![StreamUpdate::End {
                                id: "attempt.1".into(),
                            }],
                        },
                    ),
                ]),
            },
        }
    }

    #[test]
    fn settlement_is_one_update_including_usage_content_and_live_retirement() {
        let mut replica = replica();
        assert!(replica.apply(finish()).unwrap().is_some());
        assert!(matches!(
            replica.current().unwrap()["a-usage"],
            MemberState::Value(Value::Int(2))
        ));
        assert!(doc(&replica).tree().contains("message.1"));
        assert!(doc(&replica).live().is_empty());
        assert_eq!(replica.position(), Some(4));
    }

    #[test]
    fn a_bad_later_member_never_exposes_the_already_applied_first_member() {
        let mut replica = replica();
        let Publication::Update { mut update, .. } = finish() else {
            unreachable!()
        };
        update.members.insert(
            "conversation".into(),
            Delta::Document {
                changes: vec![],
                streams: vec![StreamUpdate::Append {
                    id: "attempt.1".into(),
                    offset: 1,
                    text: "🙂".into(),
                }],
            },
        );
        assert!(
            replica
                .apply(Publication::Update {
                    handle: HANDLE,
                    update
                })
                .is_err()
        );
        assert!(replica.current().is_none());
        assert!(replica.last_good().is_none());
        assert!(replica.checkpoint(true).is_none());
        assert_eq!(replica.position(), None);
        assert!(replica.apply(snapshot()).unwrap().is_some());
        assert!(matches!(
            replica.current().unwrap()["a-usage"],
            MemberState::Value(Value::Int(0))
        ));
    }

    #[test]
    fn old_handles_and_closed_observations_cannot_be_resurrected() {
        let mut replica = replica();
        let mut message = finish();
        if let Publication::Update { handle, .. } = &mut message {
            handle.generation = 0;
        }
        assert!(replica.apply(message).unwrap().is_none());
        assert_eq!(replica.position(), Some(3));
        replica
            .apply(Publication::Closed {
                handle: HANDLE,
                reason: Fault::new("scope.gone", "Session closed"),
            })
            .unwrap();
        assert!(replica.apply(snapshot()).unwrap().is_none());
        assert!(replica.current().is_none());
    }

    #[test]
    fn canonical_checkpoints_recover_live_state_before_accepting_appends() {
        let replica = replica();
        let checkpoint = replica.checkpoint(false).unwrap();
        assert_eq!(checkpoint.resume.publication, None);
        assert_eq!(checkpoint.resume.documents["conversation"], version(1));
        let mut restored = Replica::restore(HANDLE, selection(), checkpoint).unwrap();
        assert!(restored.current().is_none());
        restored
            .apply(Publication::Recovered {
                handle: HANDLE,
                recovered: Recovered {
                    position: 3000,
                    members: BTreeMap::from([
                        (
                            "a-usage".into(),
                            Recovery::Replace {
                                content: Content::Value(Value::Int(0)),
                            },
                        ),
                        (
                            "conversation".into(),
                            Recovery::Document {
                                from: version(1),
                                changes: vec![],
                                streams: vec![Stream {
                                    id: "attempt.1".into(),
                                    role: "message.assistant".into(),
                                    text: "é🙂".into(),
                                }],
                            },
                        ),
                    ]),
                },
            })
            .unwrap();
        restored
            .apply(Publication::Update {
                handle: HANDLE,
                update: Update {
                    from: 3000,
                    position: 3001,
                    members: BTreeMap::from([(
                        "conversation".into(),
                        Delta::Document {
                            changes: vec![],
                            streams: vec![StreamUpdate::Append {
                                id: "attempt.1".into(),
                                offset: 6,
                                text: "!".into(),
                            }],
                        },
                    )]),
                },
            })
            .unwrap();
        assert_eq!(doc(&restored).live()["attempt.1"].text, "é🙂!");
        assert_eq!(doc(&restored).version(), &version(1));
    }

    #[test]
    fn checkpoints_bind_scope_incarnation_and_query_arguments() {
        let replica = replica();
        for variant in 0..3 {
            let mut expected = selection();
            match variant {
                0 => expected.scope.incarnation = "new-session".into(),
                1 => expected
                    .members
                    .get_mut("a-usage")
                    .unwrap()
                    .query
                    .args
                    .push(Value::Int(3)),
                _ => expected.members.get_mut("a-usage").unwrap().contract = "usage.v2".into(),
            }
            assert!(
                Replica::restore(HANDLE, expected, replica.checkpoint(false).unwrap()).is_err()
            );
        }
    }

    #[test]
    fn required_faults_and_missing_members_cannot_be_published_as_success() {
        let mut replica = replica();
        let Publication::Snapshot { mut snapshot, .. } = snapshot() else {
            unreachable!()
        };
        snapshot.members.insert(
            "a-usage".into(),
            Content::Unavailable(Fault::query("provider unavailable")),
        );
        assert!(
            replica
                .apply(Publication::Snapshot {
                    handle: HANDLE,
                    snapshot
                })
                .is_err()
        );
        assert!(replica.current().is_none());
        let mut replica = Replica::new(HANDLE, selection()).unwrap();
        replica
            .apply(Publication::Fault {
                handle: HANDLE,
                fault: Fault::query("not ready"),
            })
            .unwrap();
        assert!(replica.checkpoint(true).is_none());
    }
    #[test]
    fn incremental_notifications_retain_only_changed_operations() {
        let mut replica = replica();
        let Some(Applied::Changed(changed)) = replica.apply(finish()).unwrap() else {
            panic!()
        };
        assert_eq!(changed["a-usage"], MemberChange::Replace);
        assert_eq!(
            changed["conversation"],
            MemberChange::Document {
                tree: vec![ViewOp::Insert {
                    node: Node::text("message.assistant", [misa_proto::view::Span::plain("é")])
                        .id("message.1"),
                    parent: "root".into(),
                    before: None
                }],
                live: vec![StreamUpdate::End {
                    id: "attempt.1".into()
                }],
                reset_live: false,
            }
        );
    }
    #[test]
    fn invalid_late_tree_operation_discards_entire_transaction() {
        let mut replica = replica();
        let Publication::Update { mut update, .. } = finish() else {
            panic!()
        };
        let Delta::Document { changes, .. } = update.members.get_mut("conversation").unwrap()
        else {
            panic!()
        };
        changes[0].ops.push(ViewOp::Remove {
            id: "missing".into(),
        });
        assert!(
            replica
                .apply(Publication::Update {
                    handle: HANDLE,
                    update
                })
                .is_err()
        );
        assert!(replica.last_good().is_none());
        assert!(replica.resume().publication.is_none());
    }
    #[test]
    fn incomplete_checkpoint_cannot_consume_sparse_updates() {
        let checkpoint = replica().checkpoint(false).unwrap();
        let mut restored = Replica::restore(HANDLE, selection(), checkpoint).unwrap();
        assert!(restored.apply(finish()).is_err());
        assert!(restored.current().is_none());
    }
    #[test]
    fn complete_checkpoint_and_rebinding_preserve_replay_without_cloning() {
        let checkpoint = replica().checkpoint(true).unwrap();
        let mut restored = Replica::restore(HANDLE, selection(), checkpoint).unwrap();
        let new_handle = Handle {
            id: 1,
            generation: 2,
        };
        restored.rebind(new_handle).unwrap();
        assert_eq!(restored.resume().publication, Some(3));
        assert!(restored.apply(finish()).unwrap().is_none());
        let Publication::Update { update, .. } = finish() else {
            panic!()
        };
        restored
            .apply(Publication::Update {
                handle: new_handle,
                update,
            })
            .unwrap();
        assert_eq!(restored.position(), Some(4));
        assert!(doc(&restored).live().is_empty());
        restored
            .apply(Publication::Closed {
                handle: new_handle,
                reason: Fault::new("gone", "gone"),
            })
            .unwrap();
        assert!(restored.rebind(HANDLE).is_err());
    }
    #[test]
    fn query_fault_requires_complete_recovery_and_cannot_rewind() {
        let mut state = replica();
        state
            .apply(Publication::Fault {
                handle: HANDLE,
                fault: Fault::query("temporarily unavailable"),
            })
            .unwrap();
        assert!(state.last_good().is_some());
        assert_eq!(state.resume().publication, None);
        assert!(state.apply(finish()).is_err());
        let mut state = replica();
        state.apply(finish()).unwrap();
        state
            .apply(Publication::Fault {
                handle: HANDLE,
                fault: Fault::query("temporarily unavailable"),
            })
            .unwrap();
        assert!(state.apply(snapshot()).is_err());
        assert!(state.current().is_none());
        let mut state = replica();
        state
            .apply(Publication::Fault {
                handle: HANDLE,
                fault: Fault::query("temporarily unavailable"),
            })
            .unwrap();
        assert_eq!(state.apply(snapshot()).unwrap(), Some(Applied::Reset));
        assert!(state.current().is_some());
    }
    #[test]
    fn notification_preserves_an_insert_removed_within_the_same_transaction() {
        let mut replica = replica();
        let operations = vec![
            ViewOp::Insert {
                parent: "root".into(),
                before: None,
                node: Node::section("temporary").id("temporary"),
            },
            ViewOp::Remove {
                id: "temporary".into(),
            },
        ];
        let publication = Publication::Update {
            handle: HANDLE,
            update: Update {
                from: 3,
                position: 4,
                members: BTreeMap::from([(
                    "conversation".into(),
                    Delta::Document {
                        changes: vec![Change {
                            from: version(1),
                            version: version(2),
                            ops: operations.clone(),
                        }],
                        streams: vec![StreamUpdate::Append {
                            id: "attempt.1".into(),
                            offset: 2,
                            text: "🙂".into(),
                        }],
                    },
                )]),
            },
        };
        let Some(Applied::Changed(changed)) = replica.apply(publication).unwrap() else {
            panic!()
        };
        let MemberChange::Document { tree, live, .. } = &changed["conversation"] else {
            panic!()
        };
        assert_eq!(tree, &operations);
        assert_eq!(
            live,
            &[StreamUpdate::Append {
                id: "attempt.1".into(),
                offset: 2,
                text: "🙂".into()
            }]
        );
        assert!(!doc(&replica).tree().contains("temporary"));
        let mut cache = IndexedTree::new(Node::section("conversation").id("root"));
        for op in tree {
            cache.apply(op).unwrap();
        }
        assert_eq!(cache.snapshot(), doc(&replica).tree().snapshot());
    }
}
