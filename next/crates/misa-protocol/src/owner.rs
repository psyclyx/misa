//! Authenticated scoped routing, independent of sockets and application kernels.
use crate::invocation::{CallContext, CommandOwner, Dispatcher};
use misa_proto::{
    Fault,
    invocation::{Invocation, Outcome, Reply},
    observation::{Handle, Publication, Resume, Scope, Selection, Snapshot},
    scoped::{Read, ReadOutcome, ReadReply},
};
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};
use tokio::sync::watch;

/// Construct snapshot and watch registration under one owner publication boundary.
pub trait Owner: CommandOwner {
    fn read(&self, context: &CallContext, selection: &Selection) -> Result<Snapshot, Fault>;
    fn observe(
        self: Arc<Self>,
        context: CallContext,
        handle: Handle,
        selection: Selection,
        resume: Option<Resume>,
    ) -> Result<(Box<dyn Observation>, Publication), Fault>;
}
/// An observation retains its owner's authority context. Revalidate export access
/// when polling; revocation must close/reset rather than replay restricted state.
pub trait Observation: Send {
    fn changed(&mut self) -> &mut watch::Receiver<u64>;
    fn poll(&mut self) -> Option<Publication>;
}
/// Resolve under trusted principal authority. Existence errors must not leak
/// inaccessible scopes. Owners additionally check current incarnation on execution.
pub trait Resolver: Send + Sync {
    fn resolve(&self, context: &CallContext, scope: &Scope) -> Result<Arc<dyn Owner>, Fault>;
    /// An authenticated daemon connection, independent of any selected scope.
    /// Client names are display metadata, never authority.
    fn connected(&self, _context: &CallContext, _client: &misa_proto::ClientInfo) -> Result<(), Fault> { Ok(()) }
    fn disconnected(&self, _context: &CallContext) {}
}
struct Active {
    handle: Handle,
    selection: Selection,
    observation: Box<dyn Observation>,
}

pub struct Router {
    context: CallContext,
    resolver: Arc<dyn Resolver>,
    dispatcher: Arc<Dispatcher>,
    capacity: usize,
    highest_id: Option<u64>,
    active: BTreeMap<u64, Active>,
}
impl Router {
    pub fn new(
        context: CallContext,
        resolver: Arc<dyn Resolver>,
        capacity: usize,
        invocation_capacity: usize,
        input_limits: misa_proto::schema::Limits,
        result_limits: misa_proto::schema::Limits,
    ) -> Self {
        Self {
            dispatcher: Arc::new(Dispatcher::new(
                context.clone(),
                invocation_capacity,
                input_limits,
                result_limits,
            )),
            context,
            resolver,
            capacity,
            highest_id: None,
            active: BTreeMap::new(),
        }
    }
    pub fn active(&self) -> usize {
        self.active.len()
    }
    fn resolve(&self, scope: &Scope) -> Result<Arc<dyn Owner>, Fault> {
        scope.validate()?;
        let owner = self.resolver.resolve(&self.context, scope)?;
        if owner.scope() != *scope {
            return Err(Fault::new(
                "stale_scope",
                "Scope incarnation is no longer current",
            ));
        }
        Ok(owner)
    }
    /// Finite reads allocate no retained observation state. The owner evaluates
    /// the complete selection coherently; external IO belongs to commands.
    pub fn read(&self, request: Read) -> ReadReply {
        let outcome = match request
            .selection
            .validate()
            .and_then(|_| self.resolve(&request.selection.scope))
            .and_then(|owner| owner.read(&self.context, &request.selection))
        {
            Ok(snapshot) => ReadOutcome::Snapshot { snapshot },
            Err(fault) => ReadOutcome::Rejected { fault },
        };
        ReadReply {
            id: request.id,
            outcome,
        }
    }
    /// New IDs increase monotonically on each connection; this rejects cancelled
    /// handle reuse with constant tombstone space. Re-observing the exact active
    /// selection requires a newer generation so queued old replies cannot rewind it.
    pub fn observe(
        &mut self,
        handle: Handle,
        selection: Selection,
        resume: Option<Resume>,
    ) -> Result<Publication, Fault> {
        misa_proto::scoped::ClientMessage::Observe {
            handle,
            selection: selection.clone(),
            resume: resume.clone(),
        }
        .validate()?;
        if let Some(active) = self.active.get(&handle.id) {
            if handle.generation <= active.handle.generation || active.selection != selection {
                return Err(Fault::protocol(
                    "Active observation identity cannot be reassigned",
                ));
            }
        } else {
            if self.highest_id.is_some_and(|id| handle.id <= id) {
                return Err(Fault::protocol("Observation identity was already used"));
            }
            if self.active.len() >= self.capacity {
                return Err(Fault::new("busy", "Too many active observations"));
            }
            // A refused initial observation consumes its ID too: late traffic
            // must never acquire a new meaning under the same identity.
            self.highest_id = Some(handle.id);
        }
        // A valid replacement retires the old source even if reopening fails.
        // The caller turns an open refusal into Closed for the requested handle;
        // recoverable query faults come from a live observation instead.
        self.active.remove(&handle.id);
        let owner = self.resolve(&selection.scope)?;
        let (observation, initial) =
            owner.observe(self.context.clone(), handle, selection.clone(), resume)?;
        if initial.handle() != handle {
            return Err(Fault::protocol("Owner answered another observation handle"));
        }
        if matches!(initial, Publication::Closed { .. }) {
            self.active.remove(&handle.id);
        } else {
            self.active.insert(
                handle.id,
                Active {
                    handle,
                    selection,
                    observation,
                },
            );
        }
        Ok(initial)
    }
    pub fn cancel(&mut self, handle: Handle) -> bool {
        if self
            .active
            .get(&handle.id)
            .is_some_and(|active| active.handle == handle)
        {
            self.active.remove(&handle.id);
            true
        } else {
            false
        }
    }
    /// Drivers can wait on this cheap cloned watch handle without borrowing the
    /// router, then poll that observation. One pending wakeup coalesces revisions.
    pub fn watch(&mut self, handle: Handle) -> Option<watch::Receiver<u64>> {
        let active = self.active.get_mut(&handle.id)?;
        (active.handle == handle).then(|| active.observation.changed().clone())
    }
    pub fn poll(&mut self, handle: Handle) -> Option<Publication> {
        let active = self.active.get_mut(&handle.id)?;
        if active.handle != handle {
            return None;
        }
        // Opening authority is not a permanent grant. Recheck before reading
        // a queued owner publication, so revocation cannot leak buffered state.
        let resolution = self
            .resolver
            .resolve(&self.context, &active.selection.scope)
            .and_then(|owner| {
                if owner.scope() == active.selection.scope {
                    Ok(())
                } else {
                    Err(Fault::new(
                        "stale_scope",
                        "Scope incarnation is no longer current",
                    ))
                }
            });
        if let Err(reason) = resolution {
            self.active.remove(&handle.id);
            return Some(Publication::Closed { handle, reason });
        }
        let publication = active.observation.poll()?;
        let publication = if publication.handle() != handle {
            Publication::Closed {
                handle,
                reason: Fault::protocol("Owner answered another observation handle"),
            }
        } else {
            publication
        };
        if matches!(publication, Publication::Closed { .. }) {
            self.active.remove(&handle.id);
        }
        Some(publication)
    }
    /// Owned future deliberately holds no router borrow: the socket driver can
    /// keep receiving observations/cancellations while execution is pending.
    pub fn invoke(&self, invocation: Invocation) -> Pin<Box<dyn Future<Output = Reply> + Send>> {
        let resolver = self.resolver.clone();
        let context = self.context.clone();
        let dispatcher = self.dispatcher.clone();
        Box::pin(async move {
            match resolver.resolve(&context, &invocation.scope) {
                Ok(owner) => dispatcher.dispatch(owner.as_ref(), invocation).await,
                Err(fault) => Reply {
                    id: invocation.id,
                    outcome: Outcome::Rejected { fault },
                },
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::invocation::Execution;
    use misa_proto::{
        Query,
        invocation::Command,
        observation::{Content, Encoding, Member, ScopeId},
        schema::{Limits, Schema},
    };
    use misa_value::Value;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    struct Fake {
        revision: watch::Sender<u64>,
        drops: Arc<AtomicUsize>,
        denied: Mutex<bool>,
    }
    impl CommandOwner for Fake {
        fn scope(&self) -> Scope {
            scope()
        }
        fn command(&self, context: &CallContext, _: &str) -> Option<Command> {
            assert_eq!(context.principal, "trusted");
            Some(Command {
                preparation: Default::default(), id: "run".into(),
                input: Schema::Bool,
                result: Schema::Bool,
            })
        }
        fn execute<'a>(&'a self, _: &'a CallContext, _: Invocation) -> Execution<'a> {
            Box::pin(async {
                Outcome::Completed {
                    value: Value::Bool(true),
                }
            })
        }
    }
    impl Owner for Fake {
        fn read(&self, _: &CallContext, _: &Selection) -> Result<Snapshot, Fault> {
            Ok(snapshot(*self.revision.borrow()))
        }
        fn observe(
            self: Arc<Self>,
            _: CallContext,
            handle: Handle,
            _: Selection,
            _: Option<Resume>,
        ) -> Result<(Box<dyn Observation>, Publication), Fault> {
            let observation = Box::new(Watch {
                receiver: self.revision.subscribe(),
                owner: self.clone(),
                handle,
            });
            Ok((
                observation,
                Publication::Snapshot {
                    handle,
                    snapshot: snapshot(*self.revision.borrow()),
                },
            ))
        }
    }
    struct Watch {
        receiver: watch::Receiver<u64>,
        owner: Arc<Fake>,
        handle: Handle,
    }
    impl Drop for Watch {
        fn drop(&mut self) {
            self.owner.drops.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl Observation for Watch {
        fn changed(&mut self) -> &mut watch::Receiver<u64> {
            &mut self.receiver
        }
        fn poll(&mut self) -> Option<Publication> {
            if *self.owner.denied.lock().unwrap() {
                return Some(Publication::Closed {
                    handle: self.handle,
                    reason: Fault::new("denied", "Access revoked"),
                });
            }
            if !self.receiver.has_changed().unwrap_or(true) {
                return None;
            }
            Some(Publication::Snapshot {
                handle: self.handle,
                snapshot: snapshot(*self.receiver.borrow_and_update()),
            })
        }
    }
    struct Registry(Arc<Fake>);
    impl Resolver for Registry {
        fn resolve(&self, context: &CallContext, _: &Scope) -> Result<Arc<dyn Owner>, Fault> {
            assert_eq!(context.principal, "trusted");
            if *self.0.denied.lock().unwrap() {
                return Err(Fault::new("denied", "Scope unavailable"));
            }
            Ok(self.0.clone())
        }
    }
    fn scope() -> Scope {
        Scope {
            id: ScopeId::Daemon,
            incarnation: "current".into(),
        }
    }
    fn selection() -> Selection {
        Selection {
            scope: scope(),
            members: BTreeMap::from([(
                "n".into(),
                Member {
                    query: Query::new("n"),
                    contract: "int.v1".into(),
                    encoding: Encoding::Value,
                    optional: false,
                },
            )]),
        }
    }
    fn snapshot(position: u64) -> Snapshot {
        Snapshot {
            position,
            members: BTreeMap::from([("n".into(), Content::Value(Value::Int(position as i64)))]),
        }
    }
    fn setup(capacity: usize) -> (Router, Arc<Fake>) {
        let owner = Arc::new(Fake {
            revision: watch::channel(0).0,
            drops: Arc::new(AtomicUsize::new(0)),
            denied: Mutex::new(false),
        });
        (
            Router::new(
                CallContext {
                    principal: "trusted".into(),
                    connection: 1,
                },
                Arc::new(Registry(owner.clone())),
                capacity,
                2,
                Limits::default(),
                Limits::default(),
            ),
            owner,
        )
    }
    fn handle(id: u64) -> Handle {
        Handle { id, generation: 1 }
    }
    #[test]
    fn finite_reads_leave_no_subscription_and_reject_old_incarnation() {
        let (router, _) = setup(1);
        assert!(matches!(
            router
                .read(Read {
                    id: 1,
                    selection: selection()
                })
                .outcome,
            ReadOutcome::Snapshot { .. }
        ));
        assert_eq!(router.active(), 0);
        let mut old = selection();
        old.scope.incarnation = "old".into();
        assert!(matches!(
            router
                .read(Read {
                    id: 2,
                    selection: old
                })
                .outcome,
            ReadOutcome::Rejected { .. }
        ));
    }
    #[test]
    fn capacity_recovery_and_generation_safe_cancellation_release_owner() {
        let (mut router, owner) = setup(1);
        router.observe(handle(1), selection(), None).unwrap();
        assert!(router.observe(handle(2), selection(), None).is_err());
        let recovered = Handle {
            id: 1,
            generation: 2,
        };
        assert!(router.observe(handle(1), selection(), None).is_err());
        router.observe(recovered, selection(), None).unwrap();
        assert!(router.poll(handle(1)).is_none());
        assert_eq!(owner.drops.load(Ordering::SeqCst), 1);
        assert!(!router.cancel(Handle {
            id: 1,
            generation: 0
        }));
        assert!(!router.cancel(handle(1)));
        assert!(router.cancel(recovered));
        assert_eq!(owner.drops.load(Ordering::SeqCst), 2);
        assert!(router.observe(handle(1), selection(), None).is_err());
        router.observe(handle(2), selection(), None).unwrap();
    }
    #[tokio::test]
    async fn changes_and_revocation_are_scoped_and_invocation_has_no_router_borrow() {
        let (mut router, owner) = setup(2);
        router.observe(handle(1), selection(), None).unwrap();
        let mut watch = router.watch(handle(1)).unwrap();
        let call = router.invoke(Invocation {
            id: 3,
            scope: scope(),
            command: "run".into(),
            input: Value::Bool(true),
        });
        router.observe(handle(2), selection(), None).unwrap();
        assert!(matches!(call.await.outcome, Outcome::Completed { .. }));
        owner.revision.send_replace(2);
        watch.changed().await.unwrap();
        assert!(matches!(
            router.poll(handle(1)),
            Some(Publication::Snapshot {
                snapshot: Snapshot { position: 2, .. },
                ..
            })
        ));
        *owner.denied.lock().unwrap() = true;
        assert!(matches!(
            router.poll(handle(1)),
            Some(Publication::Closed { .. })
        ));
        assert_eq!(router.active(), 1);
        assert!(router.poll(handle(1)).is_none());
    }
    #[test]
    fn denied_replacement_retires_the_previous_registration() {
        let (mut router, owner) = setup(1);
        router.observe(handle(1), selection(), None).unwrap();
        *owner.denied.lock().unwrap() = true;
        assert!(
            router
                .observe(
                    Handle {
                        id: 1,
                        generation: 2
                    },
                    selection(),
                    None
                )
                .is_err()
        );
        assert_eq!(router.active(), 0);
        assert_eq!(owner.drops.load(Ordering::SeqCst), 1);
        assert!(router.poll(handle(1)).is_none());
    }
}
