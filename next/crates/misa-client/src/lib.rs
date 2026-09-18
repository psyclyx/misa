//! A daemon relationship's bounded coordination state. The connection driver
//! owns IO and dispatches returned requests; surfaces observe replicas and keep
//! their own selection/drafts. No method reconnects or retries an invocation.
pub mod composition;
pub mod lifecycle;
pub mod overview;
pub mod preference_store;
pub mod operation;
pub mod form;
pub mod daemons;
pub mod transfers;
pub mod document;
pub mod interface;
pub mod interaction;
pub mod request;
pub mod driver;
use std::{collections::BTreeMap, time::Instant};

use misa_proto::{
    Fault,
    invocation::{Command, Invocation, Outcome, Reply},
    observation::{Handle, Publication, Resume, Scope, Selection},
    schema::{Limits as ValueLimits, Schema},
    scoped::{Read, ReadOutcome, ReadReply},
};
use misa_protocol::observation::{Applied, Checkpoint, MemberState, Replica, Status};
use misa_value::Value;

/// Local identities survive connection generations; wire handles do not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObservationId(u64);

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub observations: usize,
    pub values: ValueLimits,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            observations: misa_proto::scoped::DEFAULT_OBSERVATION_LIMIT,
            values: ValueLimits::default(),
        }
    }
}

/// Instructions to a connection driver, not a second serialization protocol.
#[derive(Debug)]
pub enum Outgoing {
    Observe {
        handle: Handle,
        selection: Selection,
        resume: Option<Resume>,
    },
    CancelObservation {
        handle: Handle,
    },
    Invoke(Invocation),
    Read(Read),
}

impl From<Outgoing> for misa_proto::scoped::ClientMessage {
    fn from(request: Outgoing) -> Self {
        match request {
            Outgoing::Observe {
                handle,
                selection,
                resume,
            } => Self::Observe {
                handle,
                selection,
                resume,
            },
            Outgoing::CancelObservation { handle } => Self::CancelObservation { handle },
            Outgoing::Invoke(invocation) => Self::Invoke { invocation },
            Outgoing::Read(read) => Self::Read { read },
        }
    }
}

/// A finite coherent result. It has no live handle in the connection registry.
pub struct ReadValue(Replica);
impl ReadValue {
    pub fn selection(&self) -> &Selection {
        self.0.selection()
    }
    pub fn position(&self) -> u64 {
        self.0
            .position()
            .expect("finite result is a validated snapshot")
    }
    pub fn members(&self) -> &BTreeMap<String, MemberState> {
        self.0
            .current()
            .expect("finite result is a validated snapshot")
    }
}
pub struct ReadResult {
    pub id: u64,
    pub result: Result<ReadValue, Fault>,
}
#[derive(Default)]
pub struct Settled {
    pub invocations: Vec<Reply>,
    pub reads: Vec<ReadResult>,
}
impl Settled {
    pub fn is_empty(&self) -> bool {
        self.invocations.is_empty() && self.reads.is_empty()
    }
}

/// Apply invalidations only after this is returned; Replica has already
/// accepted the complete transaction. Recovery requests go back to the driver.
pub struct Received {
    pub observation: ObservationId,
    pub applied: Option<Applied>,
    pub recovery: Option<Outgoing>,
    pub fault: Option<String>,
}

struct Observed {
    wire: Handle,
    replica: Replica,
    /// At most one recovery request is issued until a valid replacement arrives.
    recovering: bool,
}
struct Pending {
    deadline: Instant,
    result: Schema,
}
struct PendingRead {
    deadline: Instant,
    selection: Selection,
}

/// One authenticated daemon identity. A caller maintains one such owner for
/// each daemon; a selected session is deliberately absent from this type.
pub struct Connection {
    limits: Limits,
    generation: u64,
    online: bool,
    next_id: u64,
    observations: BTreeMap<ObservationId, Observed>,
    calls: BTreeMap<u64, Pending>,
    reads: BTreeMap<u64, PendingRead>,
}
impl Connection {
    /// Construct only after authenticating the transport. Generations identify
    /// driver callbacks; they are not owner incarnations or authorization.
    pub fn new(limits: Limits) -> Self {
        Self {
            limits,
            generation: 1,
            online: true,
            next_id: 1,
            observations: BTreeMap::new(),
            calls: BTreeMap::new(),
            reads: BTreeMap::new(),
        }
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn online(&self) -> bool {
        self.online
    }
    pub fn observation_count(&self) -> usize {
        self.observations.len()
    }
    pub fn pending_count(&self) -> usize {
        self.calls.len() + self.reads.len()
    }
    pub fn replica(&self, id: ObservationId) -> Option<&Replica> {
        self.observations.get(&id).map(|observed| &observed.replica)
    }
    fn allocate(&mut self) -> Result<u64, Fault> {
        let id = self.next_id;
        self.next_id = id
            .checked_add(1)
            .ok_or_else(|| Fault::new("exhausted", "Client request identities exhausted"))?;
        Ok(id)
    }
    fn require_online(&self) -> Result<(), Fault> {
        if self.online {
            Ok(())
        } else {
            Err(Fault::new("offline", "Daemon is disconnected"))
        }
    }

    /// Restored checkpoints are visible as stale until the owner synchronizes
    /// them. Checkpoint validation/cursor ownership belongs solely to Replica.
    pub fn observe(
        &mut self,
        selection: Selection,
        checkpoint: Option<Checkpoint>,
    ) -> Result<(ObservationId, Outgoing), Fault> {
        self.require_online()?;
        if self.observations.len() >= self.limits.observations {
            return Err(Fault::new("busy", "Too many observations"));
        }
        selection.validate()?;
        let id = ObservationId(self.allocate()?);
        let handle = Handle {
            id: id.0,
            generation: 1,
        };
        let replica = match checkpoint {
            Some(checkpoint) => {
                Replica::restore(handle, selection.clone(), checkpoint).map_err(|_| {
                    Fault::new("checkpoint", "Checkpoint does not match this observation")
                })?
            }
            None => Replica::new(handle, selection.clone())?,
        };
        let resume = replica.last_good().map(|_| replica.resume());
        self.observations.insert(
            id,
            Observed {
                wire: handle,
                replica,
                recovering: false,
            },
        );
        Ok((
            id,
            Outgoing::Observe {
                handle,
                selection,
                resume,
            },
        ))
    }

    /// Releases interest only. It never closes the session or cancels its work.
    pub fn cancel(&mut self, id: ObservationId) -> Option<Outgoing> {
        let observed = self.observations.remove(&id)?;
        self.online.then(|| Outgoing::CancelObservation {
            handle: observed.wire,
        })
    }

    pub fn receive(&mut self, generation: u64, publication: Publication) -> Option<Received> {
        if !self.online || generation != self.generation {
            return None;
        }
        let id = ObservationId(publication.handle().id);
        let observed = self.observations.get_mut(&id)?;
        if publication.handle() != observed.wire {
            return None;
        }
        match observed.replica.apply(publication) {
            Ok(None) => None,
            Ok(Some(applied)) => {
                if matches!(applied, Applied::Reset | Applied::Closed) {
                    observed.recovering = false;
                }
                Some(Received {
                    observation: id,
                    applied: Some(applied),
                    recovery: None,
                    fault: None,
                })
            }
            Err(reason) => {
                let recovery = if observed.recovering {
                    None
                } else {
                    observed.recovering = true;
                    let Some(next) = observed.wire.generation.checked_add(1) else {
                        let _ = observed.replica.apply(Publication::Closed {
                            handle: observed.wire,
                            reason: Fault::new("exhausted", "Observation generations exhausted"),
                        });
                        return Some(Received {
                            observation: id,
                            applied: Some(Applied::Closed),
                            recovery: None,
                            fault: Some(reason),
                        });
                    };
                    observed.wire.generation = next;
                    observed
                        .replica
                        .rebind(observed.wire)
                        .expect("invalid active replica can rebind to a fresh generation");
                    Some(Outgoing::Observe {
                        handle: observed.wire,
                        selection: observed.replica.selection().clone(),
                        resume: None,
                    })
                };
                Some(Received {
                    observation: id,
                    applied: None,
                    recovery,
                    fault: Some(reason),
                })
            }
        }
    }

    /// A data lane can fail while the authenticated control relationship stays
    /// usable. Resume only this coherent replica under a new observation generation.
    pub fn lane_closed(
        &mut self,
        generation: u64,
        handle: Handle,
        reason: String,
    ) -> Option<Received> {
        if !self.online || generation != self.generation {
            return None;
        }
        let id = ObservationId(handle.id);
        let observed = self.observations.get_mut(&id)?;
        if observed.wire != handle || matches!(observed.replica.status(), Status::Closed(_)) {
            return None;
        }
        let next = handle.generation.checked_add(1)?;
        observed.wire.generation = next;
        observed.replica.rebind(observed.wire).ok()?;
        observed.recovering = true;
        let resume = observed
            .replica
            .last_good()
            .map(|_| observed.replica.resume());
        Some(Received {
            observation: id,
            applied: Some(Applied::Stale),
            fault: Some(reason),
            recovery: Some(Outgoing::Observe {
                handle: observed.wire,
                selection: observed.replica.selection().clone(),
                resume,
            }),
        })
    }

    /// Call only after authenticating the same daemon again. Rebind observation
    /// delivery to the new driver generation; never return old invocations.
    pub fn reconnected(&mut self) -> Result<Vec<Outgoing>, Fault> {
        if self.online {
            return Err(Fault::new("connected", "Daemon is already connected"));
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or_else(|| Fault::new("exhausted", "Connection generations exhausted"))?;
        let mut requests = Vec::new();
        if self.observations.values().any(|observed| {
            !matches!(observed.replica.status(), Status::Closed(_))
                && observed.wire.generation == u64::MAX
        }) {
            return Err(Fault::new("exhausted", "Observation generations exhausted"));
        }
        for (id, observed) in &mut self.observations {
            if matches!(observed.replica.status(), Status::Closed(_)) {
                continue;
            }
            let handle = Handle {
                id: id.0,
                generation: observed.wire.generation + 1,
            };
            observed
                .replica
                .rebind(handle)
                .map_err(|reason| Fault::protocol(reason))?;
            observed.wire = handle;
            let resume = observed
                .replica
                .last_good()
                .map(|_| observed.replica.resume());
            observed.recovering = false;
            requests.push(Outgoing::Observe {
                handle,
                selection: observed.replica.selection().clone(),
                resume,
            });
        }
        self.generation = generation;
        self.online = true;
        Ok(requests)
    }

    pub fn invoke(
        &mut self,
        scope: Scope,
        command: &Command,
        input: Value,
        deadline: Instant,
    ) -> Result<Outgoing, Fault> {
        self.require_online()?;
        let id = self.allocate()?;
        let invocation = Invocation {
            id,
            scope,
            command: command.id.clone(),
            input,
        };
        invocation.validate(command, self.limits.values)?;
        self.calls.insert(
            id,
            Pending {
                deadline,
                result: command.result.clone(),
            },
        );
        Ok(Outgoing::Invoke(invocation))
    }

    /// Correlation is directed and consumed once. Late/cancelled/foreign-driver
    /// results cannot complete another caller's request. Inputs are not retained.
    pub fn reply(&mut self, generation: u64, mut reply: Reply) -> Option<Reply> {
        if !self.online || generation != self.generation {
            return None;
        }
        let pending = self.calls.remove(&reply.id)?;
        let valid = match &reply.outcome {
            Outcome::Completed { value } => pending
                .result
                .validate_with(value, self.limits.values)
                .is_ok(),
            Outcome::Accepted { operation } => {
                !operation.id.is_empty() && operation.scope.validate().is_ok()
            }
            Outcome::Rejected { .. } | Outcome::Indeterminate { .. } => true,
        };
        if !valid {
            reply.outcome = unknown(
                "invalid_result",
                "Invalid result; execution may have occurred",
            );
        }
        Some(reply)
    }

    /// Stop awaiting a local result. This does not send domain cancellation.
    pub fn abandon(&mut self, id: u64) -> bool {
        self.calls.remove(&id).is_some() || self.reads.remove(&id).is_some()
    }
    pub fn deadline(&self) -> Option<Instant> {
        self.calls
            .values()
            .map(|pending| pending.deadline)
            .chain(self.reads.values().map(|pending| pending.deadline))
            .min()
    }
    pub fn expire(&mut self, now: Instant) -> Settled {
        let expired: Vec<_> = self
            .calls
            .iter()
            .filter(|(_, pending)| pending.deadline <= now)
            .map(|(id, _)| *id)
            .collect();
        let invocations = expired
            .into_iter()
            .map(|id| {
                self.calls.remove(&id);
                Reply {
                    id,
                    outcome: unknown("timeout", "Timed out; execution may have occurred"),
                }
            })
            .collect();
        let expired: Vec<_> = self
            .reads
            .iter()
            .filter(|(_, pending)| pending.deadline <= now)
            .map(|(id, _)| *id)
            .collect();
        let reads = expired
            .into_iter()
            .map(|id| {
                self.reads.remove(&id);
                ReadResult {
                    id,
                    result: Err(Fault::new("timeout", "Read timed out")),
                }
            })
            .collect();
        Settled { invocations, reads }
    }

    /// Driver loss makes replicas stale and every pending write uncertain.
    /// Observation state survives; command inputs are never queued for replay.
    pub fn disconnected(&mut self, generation: u64) -> Settled {
        if !self.online || generation != self.generation {
            return Settled::default();
        }
        self.online = false;
        for observed in self.observations.values_mut() {
            observed.replica.mark_disconnected();
        }
        let invocations = std::mem::take(&mut self.calls)
            .into_keys()
            .map(|id| Reply {
                id,
                outcome: unknown(
                    "disconnected",
                    "Connection lost; execution may have occurred",
                ),
            })
            .collect();
        let reads = std::mem::take(&mut self.reads)
            .into_keys()
            .map(|id| ReadResult {
                id,
                result: Err(Fault::new(
                    "disconnected",
                    "Connection lost before read completed",
                )),
            })
            .collect();
        Settled { invocations, reads }
    }

    /// Execute a pure selection once. The owner validates installed definitions;
    /// successful replies use the same coherent typed replica validation as observe.
    pub fn read(&mut self, selection: Selection, deadline: Instant) -> Result<Outgoing, Fault> {
        self.require_online()?;
        selection.validate()?;
        let id = self.allocate()?;
        self.reads.insert(
            id,
            PendingRead {
                deadline,
                selection: selection.clone(),
            },
        );
        Ok(Outgoing::Read(Read { id, selection }))
    }

    pub fn read_reply(&mut self, generation: u64, reply: ReadReply) -> Option<ReadResult> {
        if !self.online || generation != self.generation {
            return None;
        }
        let pending = self.reads.remove(&reply.id)?;
        let result = match reply.outcome {
            ReadOutcome::Rejected { fault } => Err(fault),
            ReadOutcome::Snapshot { snapshot } => {
                let handle = Handle {
                    id: reply.id,
                    generation,
                };
                let mut replica = Replica::new(handle, pending.selection)
                    .expect("pending selection was validated");
                match replica.apply(Publication::Snapshot { handle, snapshot }) {
                    Ok(Some(_)) => Ok(ReadValue(replica)),
                    _ => Err(Fault::new(
                        "invalid_read",
                        "Read result does not match its selected contract",
                    )),
                }
            }
        };
        Some(ReadResult {
            id: reply.id,
            result,
        })
    }
}

fn unknown(code: &str, message: &str) -> Outcome {
    Outcome::Indeterminate {
        fault: Fault::new(code, message),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{
        Query,
        observation::{Content, Delta, Encoding, Member, ScopeId, Snapshot, Update},
    };
    fn selection(session: &str) -> Selection {
        Selection {
            scope: Scope {
                id: ScopeId::Session { id: session.into() },
                incarnation: "owner-1".into(),
            },
            members: BTreeMap::from([(
                "count".into(),
                Member {
                    query: Query::new("count"),
                    contract: "count/1".into(),
                    encoding: Encoding::Value,
                    optional: false,
                },
            )]),
        }
    }
    fn handle(request: &Outgoing) -> Handle {
        let Outgoing::Observe { handle, .. } = request else {
            panic!("expected observation")
        };
        *handle
    }
    fn snapshot(handle: Handle, position: u64) -> Publication {
        Publication::Snapshot {
            handle,
            snapshot: Snapshot {
                position,
                members: BTreeMap::from([(
                    "count".into(),
                    Content::Value(Value::Int(position as i64)),
                )]),
            },
        }
    }
    fn command() -> Command {
        Command {
            preparation: Default::default(), id: "increment".into(),
            input: Schema::Int,
            result: Schema::Int,
        }
    }
    fn read_id(request: Outgoing) -> u64 {
        let Outgoing::Read(read) = request else {
            panic!("expected read")
        };
        read.id
    }
    fn read_result(id: u64, position: u64) -> ReadReply {
        let Publication::Snapshot { snapshot, .. } =
            snapshot(Handle { id, generation: 1 }, position)
        else {
            unreachable!()
        };
        ReadReply {
            id,
            outcome: ReadOutcome::Snapshot { snapshot },
        }
    }
    #[test]
    fn finite_reads_are_correlated_coherent_and_leave_no_subscription() {
        let mut connection = Connection::new(Limits::default());
        let now = Instant::now();
        let first = read_id(connection.read(selection("one"), now).unwrap());
        let second = read_id(connection.read(selection("two"), now).unwrap());
        assert_eq!(connection.observation_count(), 0);
        let result = connection.read_reply(1, read_result(second, 8)).unwrap();
        let value = result.result.ok().unwrap();
        assert_eq!(value.selection().scope, selection("two").scope);
        assert_eq!(value.position(), 8);
        assert!(matches!(
            value.members()["count"],
            MemberState::Value(Value::Int(8))
        ));
        assert_eq!(connection.pending_count(), 1);
        assert!(connection.read_reply(1, read_result(second, 8)).is_none());
        let malformed = ReadReply {
            id: first,
            outcome: ReadOutcome::Snapshot {
                snapshot: Snapshot {
                    position: 3,
                    members: BTreeMap::new(),
                },
            },
        };
        assert!(connection.read_reply(1, malformed).unwrap().result.is_err());
        assert_eq!(connection.pending_count(), 0);
        assert_eq!(connection.observation_count(), 0);
    }
    #[test]
    fn finite_reads_and_invocations_are_correlated_and_release_on_expiry_or_abandonment() {
        let mut connection = Connection::new(Limits::default());
        let now = Instant::now();
        let id = read_id(connection.read(selection("one"), now).unwrap());
        let first = call(&mut connection, now);
        let second = read_id(connection.read(selection("two"), now).unwrap());
        assert_eq!(connection.pending_count(), 3);
        let expired = connection.expire(now);
        assert_eq!(expired.invocations.len(), 1);
        assert_eq!(expired.invocations[0].id, first);
        assert_eq!(expired.reads.len(), 2);
        assert!(expired.reads.iter().all(|reply| reply.result.is_err()));
        let another = read_id(connection.read(selection("three"), now).unwrap());
        connection.abandon(another);
        assert!(connection.read_reply(1, read_result(another, 7)).is_none());
        assert_eq!(connection.pending_count(), 0);
        assert_ne!(id, second);
    }
    #[test]
    fn lost_reads_are_not_reissued_and_old_callbacks_cannot_complete_new_requests() {
        let mut connection = Connection::new(Limits::default());
        let now = Instant::now();
        let old = read_id(connection.read(selection("one"), now).unwrap());
        let lost = connection.disconnected(1);
        assert_eq!(lost.reads.len(), 1);
        assert_eq!(lost.reads[0].id, old);
        assert!(lost.reads[0].result.is_err());
        assert!(connection.reconnected().unwrap().is_empty());
        let new = read_id(connection.read(selection("one"), now).unwrap());
        assert_ne!(old, new);
        assert!(connection.read_reply(1, read_result(new, 5)).is_none());
        assert!(connection.read_reply(2, read_result(old, 5)).is_none());
        assert!(
            connection
                .read_reply(2, read_result(new, 5))
                .unwrap()
                .result
                .is_ok()
        );
    }
    fn call(connection: &mut Connection, deadline: Instant) -> u64 {
        let Outgoing::Invoke(invocation) = connection
            .invoke(selection("demo").scope, &command(), Value::Int(1), deadline)
            .unwrap()
        else {
            unreachable!()
        };
        invocation.id
    }
    #[test]
    fn independent_scopes_and_cancelled_handles_do_not_alias() {
        let mut connection = Connection::new(Limits {
            observations: 2,
            ..Limits::default()
        });
        let (a, first) = connection.observe(selection("one"), None).unwrap();
        let (b, second) = connection.observe(selection("two"), None).unwrap();
        assert!(connection.observe(selection("three"), None).is_err());
        connection.receive(1, snapshot(handle(&first), 3)).unwrap();
        connection.receive(1, snapshot(handle(&second), 9)).unwrap();
        assert_eq!(connection.replica(a).unwrap().position(), Some(3));
        assert_eq!(connection.replica(b).unwrap().position(), Some(9));
        assert!(matches!(
            connection.cancel(a),
            Some(Outgoing::CancelObservation { .. })
        ));
        let (c, third) = connection.observe(selection("one"), None).unwrap();
        assert_ne!(handle(&first), handle(&third));
        assert!(
            connection
                .receive(1, snapshot(handle(&first), 99))
                .is_none()
        );
        assert!(connection.replica(c).unwrap().current().is_none());
        assert_eq!(connection.replica(b).unwrap().position(), Some(9));
    }
    #[test]
    fn gap_requests_one_recovery_without_disturbing_other_observations() {
        let mut connection = Connection::new(Limits::default());
        let (a, first) = connection.observe(selection("one"), None).unwrap();
        let (b, second) = connection.observe(selection("two"), None).unwrap();
        connection.receive(1, snapshot(handle(&first), 3));
        connection.receive(1, snapshot(handle(&second), 9));
        let gap = || Publication::Update {
            handle: handle(&first),
            update: Update {
                from: 20,
                position: 21,
                members: BTreeMap::from([(
                    "count".into(),
                    Delta::Replace {
                        content: Content::Value(Value::Int(21)),
                    },
                )]),
            },
        };
        let received = connection.receive(1, gap()).unwrap();
        assert!(received.applied.is_none() && received.recovery.is_some());
        let replacement = handle(received.recovery.as_ref().unwrap());
        assert_eq!(replacement.generation, 2);
        assert!(connection.receive(1, gap()).is_none());
        // A queued full snapshot from the replaced server watch must not undo recovery.
        assert!(
            connection
                .receive(1, snapshot(handle(&first), 100))
                .is_none()
        );
        assert!(connection.replica(a).unwrap().current().is_none());
        assert_eq!(connection.replica(b).unwrap().position(), Some(9));
        assert!(matches!(
            connection
                .receive(1, snapshot(replacement, 22))
                .unwrap()
                .applied,
            Some(Applied::Reset)
        ));
        connection.disconnected(1);
        let reopened = connection.reconnected().unwrap();
        let newest = handle(&reopened[0]);
        assert_eq!(connection.generation(), 2);
        assert_eq!(newest.generation, 3);
        assert!(connection.receive(2, snapshot(replacement, 100)).is_none());
        assert!(connection.receive(2, snapshot(newest, 23)).is_some());
        assert_eq!(connection.replica(a).unwrap().position(), Some(23));
    }
    #[test]
    fn failed_lane_resumes_only_its_interest_with_a_fresh_generation() {
        let mut connection = Connection::new(Limits::default());
        let (a, first) = connection.observe(selection("one"), None).unwrap();
        let (b, second) = connection.observe(selection("two"), None).unwrap();
        connection.receive(1, snapshot(handle(&first), 3));
        connection.receive(1, snapshot(handle(&second), 9));
        let recovered = connection
            .lane_closed(1, handle(&first), "stream reset".into())
            .unwrap();
        let request = recovered.recovery.unwrap();
        let newer = handle(&request);
        assert_eq!(newer.generation, 2);
        assert!(matches!(
            request,
            Outgoing::Observe {
                resume: Some(Resume {
                    publication: Some(3),
                    ..
                }),
                ..
            }
        ));
        assert!(connection.online());
        assert_eq!(connection.generation(), 1);
        assert_eq!(connection.replica(b).unwrap().position(), Some(9));
        assert!(
            connection
                .receive(1, snapshot(handle(&first), 100))
                .is_none()
        );
        connection.receive(1, snapshot(newer, 4));
        assert_eq!(connection.replica(a).unwrap().position(), Some(4));
    }
    #[test]
    fn reconnect_resumes_observations_but_never_replays_writes_or_old_callbacks() {
        let mut connection = Connection::new(Limits::default());
        let (id, request) = connection.observe(selection("one"), None).unwrap();
        connection.receive(1, snapshot(handle(&request), 8));
        let pending = call(&mut connection, Instant::now());
        let lost = connection.disconnected(1);
        assert!(
            matches!(&lost.invocations[..], [Reply { id, outcome: Outcome::Indeterminate { .. } }] if *id == pending)
        );
        assert!(connection.replica(id).unwrap().current().is_none());
        assert!(connection.replica(id).unwrap().last_good().is_some());
        let requests = connection.reconnected().unwrap();
        assert_eq!(requests.len(), 1);
        let Outgoing::Observe {
            handle: new_handle,
            resume: Some(resume),
            ..
        } = &requests[0]
        else {
            panic!("expected resumption")
        };
        assert_eq!(resume.publication, Some(8));
        assert_eq!(new_handle.generation, 2);
        assert!(
            connection
                .receive(1, snapshot(handle(&request), 99))
                .is_none()
        );
        assert!(
            connection
                .receive(2, snapshot(handle(&request), 99))
                .is_none()
        );
        assert!(connection.disconnected(1).is_empty());
        assert!(connection.online());
        assert!(
            connection
                .reply(
                    2,
                    Reply {
                        id: pending,
                        outcome: Outcome::Completed {
                            value: Value::Int(2)
                        }
                    }
                )
                .is_none()
        );
        connection.receive(2, snapshot(*new_handle, 10));
        assert_eq!(connection.replica(id).unwrap().position(), Some(10));
    }
    #[test]
    fn deadlines_abandonment_and_invalid_results_are_directed_and_bounded() {
        let mut connection = Connection::new(Limits::default());
        let now = Instant::now();
        let first = call(&mut connection, now);
        assert_eq!(connection.deadline(), Some(now));
        assert!(
            matches!(&connection.expire(now).invocations[..], [Reply { id, outcome: Outcome::Indeterminate { .. } }] if *id == first)
        );
        let second = call(&mut connection, now + std::time::Duration::from_secs(1));
        assert_ne!(first, second);
        assert!(
            connection
                .reply(
                    1,
                    Reply {
                        id: first,
                        outcome: Outcome::Completed {
                            value: Value::Int(1)
                        }
                    }
                )
                .is_none()
        );
        let result = connection
            .reply(
                1,
                Reply {
                    id: second,
                    outcome: Outcome::Completed {
                        value: Value::str("never echo malformed secret"),
                    },
                },
            )
            .unwrap();
        let Outcome::Indeterminate { fault } = result.outcome else {
            panic!("invalid result is not success")
        };
        assert!(!fault.message.contains("secret"));
        let third = call(&mut connection, now);
        assert!(connection.abandon(third));
        assert!(!connection.abandon(third));
        assert_eq!(connection.pending_count(), 0);
        assert!(
            connection
                .reply(
                    1,
                    Reply {
                        id: third,
                        outcome: Outcome::Completed {
                            value: Value::Int(3)
                        }
                    }
                )
                .is_none()
        );
    }
    #[test]
    fn closed_owner_never_reopens_on_reconnect_and_checkpoints_are_selection_bound() {
        let mut connection = Connection::new(Limits::default());
        let (id, request) = connection.observe(selection("one"), None).unwrap();
        connection.receive(1, snapshot(handle(&request), 2));
        let saved = connection.replica(id).unwrap().checkpoint(false).unwrap();
        assert!(
            connection
                .observe(selection("two"), Some(saved.clone()))
                .is_err()
        );
        let (_, restored) = connection.observe(selection("one"), Some(saved)).unwrap();
        assert!(matches!(
            restored,
            Outgoing::Observe {
                resume: Some(_),
                ..
            }
        ));
        connection.receive(
            1,
            Publication::Closed {
                handle: handle(&request),
                reason: Fault::new("closed", "Owner stopped"),
            },
        );
        connection.disconnected(1);
        assert_eq!(connection.reconnected().unwrap().len(), 1);
        assert!(matches!(
            connection.replica(id).unwrap().status(),
            Status::Closed(_)
        ));
    }
}
