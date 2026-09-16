//! Bounded invocation dispatch over an injected, authenticated owner interface.
//! No reply replay cache: request correlation is not execution deduplication.
use misa_proto::{
    Fault,
    invocation::{Command, Invocation, Outcome, Reply},
    observation::Scope,
    schema::Limits,
};
use std::{collections::HashSet, future::Future, pin::Pin, sync::Mutex};

/// Supplied by authenticated host routing, never deserialized from an invocation.
#[derive(Clone, Debug)]
pub struct CallContext {
    pub principal: String,
    pub connection: u64,
}
impl CallContext {
    /// Host-assigned process-local identity. Never decode this from a peer.
    pub fn next_connection_id() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
}

pub type Execution<'a> = Pin<Box<dyn Future<Output = Outcome> + Send + 'a>>;

/// Bind this interface to authenticated caller authority outside wire decoding.
/// Registry lookup and schema checking are preflight checks. `execute` must
/// recheck scope incarnation, authorization and resource preconditions atomically
/// with the owner transition. A client action binding conveys no authority.
pub trait CommandOwner: Send + Sync {
    fn scope(&self) -> Scope;
    fn command(&self, context: &CallContext, id: &str) -> Option<Command>;
    fn execute<'a>(&'a self, context: &'a CallContext, invocation: Invocation) -> Execution<'a>;
}

/// One authenticated connection's bounded in-flight calls. The connection driver
/// must finish dispatch independently of frontend waits. Dropping a dispatch
/// future releases bookkeeping, not domain work already accepted by the owner.
pub struct Dispatcher {
    context: CallContext,
    capacity: usize,
    input_limits: Limits,
    result_limits: Limits,
    pending: Mutex<HashSet<u64>>,
}
impl Dispatcher {
    pub fn new(
        context: CallContext,
        capacity: usize,
        input_limits: Limits,
        result_limits: Limits,
    ) -> Self {
        Self {
            context,
            capacity,
            input_limits,
            result_limits,
            pending: Mutex::new(HashSet::new()),
        }
    }
    pub fn pending(&self) -> usize {
        self.pending
            .lock()
            .expect("invocation bookkeeping poisoned")
            .len()
    }
    fn reserve(&self, id: u64) -> Result<Reservation<'_>, Fault> {
        let mut pending = self
            .pending
            .lock()
            .expect("invocation bookkeeping poisoned");
        if pending.contains(&id) {
            return Err(Fault::new(
                "duplicate_request",
                "Request is already in flight",
            ));
        }
        if pending.len() >= self.capacity {
            return Err(Fault::new("busy", "Too many pending invocations"));
        }
        pending.insert(id);
        Ok(Reservation {
            dispatcher: self,
            id,
        })
    }
    pub async fn dispatch(&self, owner: &dyn CommandOwner, invocation: Invocation) -> Reply {
        let id = invocation.id;
        let rejected = |fault| Reply {
            id,
            outcome: Outcome::Rejected { fault },
        };
        let _reservation = match self.reserve(id) {
            Ok(held) => held,
            Err(fault) => return rejected(fault),
        };
        if invocation.scope != owner.scope() {
            return rejected(Fault::new(
                "stale_scope",
                "Invocation scope is no longer current",
            ));
        }
        let Some(command) = owner.command(&self.context, &invocation.command) else {
            return rejected(Fault::unsupported(
                "Command is not exported for this caller",
            ));
        };
        if let Err(fault) = invocation.validate(&command, self.input_limits) {
            return rejected(fault);
        }
        let outcome = owner.execute(&self.context, invocation).await;
        let valid = match &outcome {
            Outcome::Completed { value } => command
                .result
                .validate_with(value, self.result_limits)
                .is_ok(),
            Outcome::Accepted { operation } => {
                !operation.id.is_empty() && operation.scope.validate().is_ok()
            }
            Outcome::Rejected { .. } | Outcome::Indeterminate { .. } => true,
        };
        // Do not echo malformed owner results: they may contain credential values.
        if !valid {
            return Reply {
                id,
                outcome: Outcome::Indeterminate {
                    fault: Fault::new(
                        "invalid_result",
                        "Owner result is invalid; execution may have occurred",
                    ),
                },
            };
        }
        Reply { id, outcome }
    }
}
struct Reservation<'a> {
    dispatcher: &'a Dispatcher,
    id: u64,
}
impl Drop for Reservation<'_> {
    fn drop(&mut self) {
        self.dispatcher
            .pending
            .lock()
            .expect("invocation bookkeeping poisoned")
            .remove(&self.id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{observation::ScopeId, schema::Schema};
    use misa_value::Value;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Owner {
        calls: AtomicUsize,
        gate: Option<tokio::sync::Notify>,
        result: Outcome,
    }
    impl Owner {
        fn new() -> Self {
            Self {
                calls: AtomicUsize::new(0),
                gate: None,
                result: Outcome::Completed {
                    value: Value::Bool(true),
                },
            }
        }
    }
    fn scope() -> Scope {
        Scope {
            id: ScopeId::Session {
                id: "same-label".into(),
            },
            incarnation: "current".into(),
        }
    }
    fn call(id: u64) -> Invocation {
        Invocation {
            id,
            scope: scope(),
            command: "run".into(),
            input: Value::Bool(true),
        }
    }
    fn dispatcher(capacity: usize) -> Dispatcher {
        Dispatcher::new(
            CallContext {
                principal: "authenticated".into(),
                connection: 7,
            },
            capacity,
            Limits::default(),
            Limits::default(),
        )
    }
    impl CommandOwner for Owner {
        fn scope(&self) -> Scope {
            scope()
        }
        fn command(&self, context: &CallContext, id: &str) -> Option<Command> {
            assert_eq!(context.principal, "authenticated");
            (id == "run").then(|| Command {
                id: id.into(),
                input: Schema::Bool,
                result: Schema::Bool,
            })
        }
        fn execute<'a>(&'a self, context: &'a CallContext, _: Invocation) -> Execution<'a> {
            assert_eq!(context.connection, 7);
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                if let Some(gate) = &self.gate {
                    gate.notified().await;
                }
                self.result.clone()
            })
        }
    }
    fn code(reply: Reply) -> String {
        match reply.outcome {
            Outcome::Rejected { fault } => fault.code,
            _ => panic!("expected rejection"),
        }
    }
    #[tokio::test]
    async fn stale_scope_and_invalid_inputs_never_reach_handler() {
        let owner = Owner::new();
        let dispatcher = dispatcher(2);
        let mut stale = call(1);
        stale.scope.incarnation = "old".into();
        assert_eq!(
            code(dispatcher.dispatch(&owner, stale).await),
            "stale_scope"
        );
        let mut invalid = call(2);
        invalid.input = Value::str("credential");
        assert_eq!(code(dispatcher.dispatch(&owner, invalid).await), "protocol");
        let mut unknown = call(3);
        unknown.command = "private".into();
        assert_eq!(
            code(dispatcher.dispatch(&owner, unknown).await),
            "unsupported"
        );
        assert_eq!(owner.calls.load(Ordering::SeqCst), 0);
        assert_eq!(dispatcher.pending(), 0);
    }
    #[tokio::test]
    async fn capacity_duplicates_and_cancelled_wait_release_bookkeeping() {
        let owner = Owner {
            gate: Some(tokio::sync::Notify::new()),
            ..Owner::new()
        };
        let dispatcher = dispatcher(1);
        let mut waiting = Box::pin(dispatcher.dispatch(&owner, call(1)));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(1), waiting.as_mut())
                .await
                .is_err()
        );
        assert_eq!(dispatcher.pending(), 1);
        assert_eq!(
            code(dispatcher.dispatch(&owner, call(1)).await),
            "duplicate_request"
        );
        assert_eq!(code(dispatcher.dispatch(&owner, call(2)).await), "busy");
        drop(waiting);
        assert_eq!(dispatcher.pending(), 0);
        // Protocol cancellation cannot undo execution already begun.
        assert_eq!(owner.calls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn completed_results_are_checked_and_not_cached_or_replayed() {
        let dispatcher = dispatcher(1);
        let owner = Owner::new();
        for id in [1, 2] {
            assert!(matches!(
                dispatcher.dispatch(&owner, call(id)).await.outcome,
                Outcome::Completed { .. }
            ));
        }
        assert_eq!(owner.calls.load(Ordering::SeqCst), 2);
        assert_eq!(dispatcher.pending(), 0);
        let invalid = Owner {
            result: Outcome::Completed {
                value: Value::str("credential"),
            },
            ..Owner::new()
        };
        let reply = dispatcher.dispatch(&invalid, call(3)).await;
        let Outcome::Indeterminate { fault } = reply.outcome else {
            panic!()
        };
        assert_eq!(fault.code, "invalid_result");
        assert_eq!(invalid.calls.load(Ordering::SeqCst), 1);
        assert!(!fault.message.contains("credential"));
    }
    #[tokio::test]
    async fn accepted_work_returns_reference_without_waiting_for_completion() {
        let owner = Owner {
            result: Outcome::Accepted {
                operation: misa_proto::invocation::OperationRef {
                    scope: Scope {
                        id: ScopeId::Daemon,
                        incarnation: "daemon-run".into(),
                    },
                    id: "op-1".into(),
                },
            },
            ..Owner::new()
        };
        assert!(matches!(
            dispatcher(1).dispatch(&owner, call(1)).await.outcome,
            Outcome::Accepted { .. }
        ));
    }
}
