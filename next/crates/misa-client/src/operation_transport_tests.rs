use super::*;
use crate::Limits;
use misa_proto::{
    ClientInfo,
    invocation::{Command, Invocation, Outcome},
    observation::{Content, Handle, Publication, Resume, Scope, ScopeId, Snapshot},
    query::{Definition, ResultContract},
    schema::Schema,
};
use misa_protocol::{
    invocation::{CallContext, CommandOwner, Execution},
    owner::{Observation as OwnerObservation, Owner, Resolver},
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{Notify, watch};
#[derive(Clone)]
enum State {
    Value(Value),
    Fault,
    Closed,
}
struct TestOwner {
    scope: Scope,
    state: Mutex<State>,
    revision: watch::Sender<u64>,
    active: AtomicUsize,
}
impl TestOwner {
    fn new() -> Arc<Self> {
        let (revision, _) = watch::channel(0);
        Arc::new(Self {
            scope: Scope {
                id: ScopeId::Session {
                    id: "target".into(),
                },
                incarnation: "incarnation".into(),
            },
            state: Mutex::new(State::Value(result(false))),
            revision,
            active: AtomicUsize::new(0),
        })
    }
    fn definitions() -> Vec<Definition> {
        vec![
            misa_proto::query::catalog_definition(),
            misa_proto::directory::definition(),
            Definition {
                id: "operation.result".into(),
                arguments: vec![Schema::String],
                contract: "operation.result@1".into(),
                result: ResultContract::Data {
                    schema: Schema::Value,
                },
            },
        ]
    }
    fn set(&self, state: State) {
        *self.state.lock().unwrap() = state;
        self.revision.send_modify(|n| *n += 1);
    }
    fn snapshot(&self, selection: &Selection) -> Snapshot {
        Snapshot {
            position: *self.revision.borrow(),
            members: selection
                .members
                .iter()
                .map(|(name, member)| {
                    let value = if member.query.id == misa_proto::query::CATALOG {
                        serde_json::from_value(serde_json::to_value(Self::definitions()).unwrap())
                            .unwrap()
                    } else if member.query.id == misa_proto::directory::SESSIONS {
                        Value::list([])
                    } else {
                        match &*self.state.lock().unwrap() {
                            State::Value(value) => value.clone(),
                            _ => Value::Null,
                        }
                    };
                    (name.clone(), Content::Value(value))
                })
                .collect(),
        }
    }
}
fn result(terminal: bool) -> Value {
    Value::map([
        ("id", Value::str("accepted")),
        ("generation", Value::Int(1)),
        (
            "state",
            Value::str(if terminal { "succeeded" } else { "running" }),
        ),
        ("terminal", Value::Bool(terminal)),
        ("outputs", Value::list([])),
    ])
}
impl CommandOwner for TestOwner {
    fn scope(&self) -> Scope {
        self.scope.clone()
    }
    fn command(&self, _: &CallContext, _: &str) -> Option<Command> {
        None
    }
    fn execute<'a>(&'a self, _: &'a CallContext, _: Invocation) -> Execution<'a> {
        Box::pin(async {
            Outcome::Rejected {
                fault: Fault::unsupported("No commands"),
            }
        })
    }
}
struct Observing {
    owner: Arc<TestOwner>,
    selection: Selection,
    handle: Handle,
    changed: watch::Receiver<u64>,
}
impl Drop for Observing {
    fn drop(&mut self) {
        self.owner.active.fetch_sub(1, Ordering::SeqCst);
    }
}
impl OwnerObservation for Observing {
    fn changed(&mut self) -> &mut watch::Receiver<u64> {
        &mut self.changed
    }
    fn poll(&mut self) -> Option<Publication> {
        let state = self.owner.state.lock().unwrap().clone();
        Some(match state {
            State::Fault => Publication::Fault {
                handle: self.handle,
                fault: Fault::query("Result failed"),
            },
            State::Closed => Publication::Closed {
                handle: self.handle,
                reason: Fault::new("closed", "Owner ended"),
            },
            _ => Publication::Snapshot {
                handle: self.handle,
                snapshot: self.owner.snapshot(&self.selection),
            },
        })
    }
}
impl Owner for TestOwner {
    fn read(&self, _: &CallContext, selection: &Selection) -> Result<Snapshot, Fault> {
        Ok(self.snapshot(selection))
    }
    fn observe(
        self: Arc<Self>,
        _: CallContext,
        handle: Handle,
        selection: Selection,
        _: Option<Resume>,
    ) -> Result<(Box<dyn OwnerObservation>, Publication), Fault> {
        let changed = self.revision.subscribe();
        let snapshot = self.snapshot(&selection);
        self.active.fetch_add(1, Ordering::SeqCst);
        Ok((
            Box::new(Observing {
                owner: self,
                selection,
                handle,
                changed,
            }),
            Publication::Snapshot { handle, snapshot },
        ))
    }
}
struct Registry(Arc<TestOwner>);
impl Resolver for Registry {
    fn resolve(&self, _: &CallContext, scope: &Scope) -> Result<Arc<dyn Owner>, Fault> {
        if scope != &self.0.scope {
            return Err(Fault::query("Unknown owner"));
        }
        Ok(self.0.clone())
    }
}
struct Server {
    handler: misa_transport::scoped_server::Handler,
    cut: Arc<Notify>,
    connections: AtomicUsize,
}
impl std::fmt::Debug for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("operation fixture")
    }
}
impl iroh::protocol::ProtocolHandler for Server {
    async fn accept(
        &self,
        connection: iroh::endpoint::Connection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        let first = self.connections.fetch_add(1, Ordering::SeqCst) == 0;
        let cut = self.cut.clone();
        let socket = connection.clone();
        let killer = first.then(|| {
            tokio::spawn(async move {
                cut.notified().await;
                socket.close(1u32.into(), b"test transport loss");
            })
        });
        let result = self.handler.accept(connection).await;
        if let Some(task) = killer {
            task.abort();
        }
        result
    }
}
#[tokio::test]
async fn exact_cross_scope_interests_survive_reconnect_and_report_expiry_fault_closure() {
    let server = misa_transport::iroh::bind(None, false).await.unwrap();
    let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
    let owner = TestOwner::new();
    let cut = Arc::new(Notify::new());
    let router = iroh::protocol::Router::builder(server.clone())
        .accept(
            misa_proto::scoped::ALPN,
            Server {
                handler: misa_transport::scoped_server::Handler::new(
                    server.id().to_string(),
                    Scope {
                        id: ScopeId::Daemon,
                        incarnation: "source".into(),
                    },
                    Arc::new(Registry(owner.clone())),
                    Arc::new(misa_transport::admission::Admission::open()),
                ),
                cut: cut.clone(),
                connections: AtomicUsize::new(0),
            },
        )
        .spawn();
    let address =
        misa_transport::iroh::address_of(&misa_transport::iroh::node_of(&server)).unwrap();
    let client = Client::connect(
        &endpoint,
        address,
        ClientInfo::new("operation-test", "1"),
        Limits::default(),
    )
    .await
    .unwrap();
    // The accepting command's interface is deliberately a different scope. Watch
    // must discover the actual referenced owner instead of copying its contracts.
    let source = Interface {
        scope: Scope {
            id: ScopeId::Daemon,
            incarnation: "source".into(),
        },
        queries: BTreeMap::new(),
        commands: BTreeMap::new(),
        actions: BTreeMap::new(),
        presentations: vec![],
    };
    let reference = OperationRef {
        scope: owner.scope.clone(),
        id: "accepted".into(),
    };
    let target = Interface::load(&client, owner.scope.clone()).await.unwrap();
    let current = detail(&client, &target, "accepted").await.unwrap().unwrap();
    assert_eq!(current.operation, reference);
    assert_eq!(current.generation, 1);
    assert!(!current.terminal);
    assert_eq!(
        owner.active.load(Ordering::SeqCst),
        0,
        "finite command preparation retains no observation"
    );
    assert!(
        detail(&client, &target, "different-operation")
            .await
            .is_err(),
        "returned identity cannot replace the requested operation"
    );
    owner.set(State::Value(Value::Null));
    assert!(
        detail(&client, &target, "accepted")
            .await
            .unwrap()
            .is_none()
    );
    owner.set(State::Value(result(false)));
    let mut watch = Watch::open(&client, &source, reference.clone(), false)
        .await
        .unwrap();
    assert!(watch.poll().is_none());
    let mut changes = watch.changes();
    let generation = client.status().borrow().generation;
    cut.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            changes.changed().await.unwrap();
            if !client.online() || client.status().borrow().generation > generation {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(
        watch.poll().is_none(),
        "stale state cannot complete accepted work"
    );
    owner.set(State::Value(result(true)));
    let completion = tokio::time::timeout(std::time::Duration::from_secs(15), watch.wait())
        .await
        .unwrap();
    assert_eq!(completion.operation, reference);
    assert!(client.status().borrow().generation > generation);
    assert!(matches!(completion.outcome,Terminal::Finished{ref state,..} if state=="succeeded"));
    for state in [State::Value(Value::Null), State::Fault, State::Closed] {
        owner.set(State::Value(result(false)));
        let watch = Watch::open(&client, &source, reference.clone(), false)
            .await
            .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !watch
                .observation
                .as_ref()
                .unwrap()
                .inspect(|replica, _| replica.current().is_some())
                .unwrap_or(false)
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let expired = matches!(state, State::Value(_));
        owner.set(state);
        let completion = tokio::time::timeout(std::time::Duration::from_secs(5), watch.wait())
            .await
            .unwrap();
        assert!(if expired {
            matches!(completion.outcome, Terminal::Expired)
        } else {
            matches!(completion.outcome, Terminal::Fault(_))
        });
    }
    owner.set(State::Value(result(false)));
    let watch = Watch::open(&client, &source, reference, false)
        .await
        .unwrap();
    drop(watch);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while owner.active.load(Ordering::SeqCst) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(
        matches!(&*owner.state.lock().unwrap(),State::Value(value) if value.get("terminal")==Some(&Value::Bool(false))),
        "dropping interest never cancels domain work"
    );
    client.disconnect().await.unwrap();
    router.shutdown().await.unwrap();
    endpoint.close().await;
}
struct RestartServer {
    daemon: String,
    owners: [Arc<TestOwner>; 2],
    accepted: AtomicUsize,
    cut: Arc<Notify>,
}
impl std::fmt::Debug for RestartServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("restart fixture")
    }
}
impl iroh::protocol::ProtocolHandler for RestartServer {
    async fn accept(
        &self,
        connection: iroh::endpoint::Connection,
    ) -> Result<(), iroh::protocol::AcceptError> {
        let first = self.accepted.fetch_add(1, Ordering::SeqCst) == 0;
        let owner = self.owners[usize::from(!first)].clone();
        let killer = first.then(|| {
            let socket = connection.clone();
            let cut = self.cut.clone();
            tokio::spawn(async move {
                cut.notified().await;
                socket.close(1u32.into(), b"restart");
            })
        });
        let handler = misa_transport::scoped_server::Handler::new(
            self.daemon.clone(),
            owner.scope.clone(),
            Arc::new(Registry(owner)),
            Arc::new(misa_transport::admission::Admission::open()),
        );
        let result = handler.accept(connection).await;
        if let Some(task) = killer {
            task.abort();
        }
        result
    }
}
#[tokio::test]
async fn overview_follows_authenticated_restart_without_relabeling_old_data_current() {
    let server = misa_transport::iroh::bind(None, false).await.unwrap();
    let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
    let mut old = TestOwner::new();
    Arc::get_mut(&mut old).unwrap().scope = Scope {
        id: ScopeId::Daemon,
        incarnation: "old".into(),
    };
    let mut new = TestOwner::new();
    Arc::get_mut(&mut new).unwrap().scope = Scope {
        id: ScopeId::Daemon,
        incarnation: "new".into(),
    };
    let cut = Arc::new(Notify::new());
    let router = iroh::protocol::Router::builder(server.clone())
        .accept(
            misa_proto::scoped::ALPN,
            RestartServer {
                daemon: server.id().to_string(),
                owners: [old, new],
                accepted: AtomicUsize::new(0),
                cut: cut.clone(),
            },
        )
        .spawn();
    let address =
        misa_transport::iroh::address_of(&misa_transport::iroh::node_of(&server)).unwrap();
    let client = Client::connect(
        &endpoint,
        address,
        ClientInfo::new("overview-test", "1"),
        Limits::default(),
    )
    .await
    .unwrap();
    let mut overview = crate::overview::Overview::open(&client).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !matches!(overview.snapshot().unwrap().status, Status::Current) {
            overview.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    cut.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(15), async {
        loop {
            let snapshot = overview.snapshot().unwrap();
            if snapshot.scope != client.welcome().scope {
                assert!(!matches!(snapshot.status, Status::Current));
            }
            if snapshot.scope.incarnation == "new" && matches!(snapshot.status, Status::Current) {
                break;
            }
            overview.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    drop(overview);
    client.disconnect().await.unwrap();
    router.shutdown().await.unwrap();
    endpoint.close().await;
}
