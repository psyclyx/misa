use super::*;
use misa_proto::{
    ClientInfo,
    invocation::{Command, Invocation, Outcome},
    observation::{ScopeId, Selection, Snapshot, Resume},
    schema::Schema,
};
use misa_protocol::{invocation::{CommandOwner, Execution}, owner::{Owner, Observation}};
use misa_value::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Notify;

struct DelayedOwner {
    scope: Scope,
    started: Notify,
    release: Notify,
    finished: Notify,
    completed: AtomicUsize,
}
impl CommandOwner for DelayedOwner {
    fn scope(&self) -> Scope { self.scope.clone() }
    fn command(&self, _: &CallContext, id: &str) -> Option<Command> {
        (id == "write").then(|| Command { preparation: Default::default(), id: id.into(), input: Schema::Value, result: Schema::Value })
    }
    fn execute<'a>(&'a self, _: &'a CallContext, _: Invocation) -> Execution<'a> {
        Box::pin(async move {
            self.started.notify_one();
            // Represents an admitted write whose durable acknowledgement has
            // not arrived yet. Losing its caller must not abort finalization.
            self.release.notified().await;
            self.completed.fetch_add(1, Ordering::SeqCst);
            self.finished.notify_one();
            Outcome::Completed { value: Value::Null }
        })
    }
}
impl Owner for DelayedOwner {
    fn read(&self, _: &CallContext, _: &Selection) -> Result<Snapshot, Fault> { Err(Fault::unsupported("No reads")) }
    fn observe(self: Arc<Self>, _: CallContext, _: Handle, _: Selection, _: Option<Resume>) -> Result<(Box<dyn Observation>, Publication), Fault> { Err(Fault::unsupported("No observations")) }
}
struct Routes { owner: Arc<DelayedOwner>, disconnected: Notify }
impl Resolver for Routes {
    fn resolve(&self, _: &CallContext, scope: &Scope) -> Result<Arc<dyn Owner>, Fault> {
        if scope == &self.owner.scope { Ok(self.owner.clone()) } else { Err(Fault::query("Wrong owner")) }
    }
    fn disconnected(&self, _: &CallContext) { self.disconnected.notify_one(); }
}

#[tokio::test]
async fn admitted_writes_survive_disconnection_and_router_shutdown_drains_them() {
    let server = crate::iroh::bind(None, false).await.unwrap();
    let endpoint = crate::iroh::bind(None, false).await.unwrap();
    let scope = Scope { id: ScopeId::Daemon, incarnation: "owner".into() };
    let owner = Arc::new(DelayedOwner { scope: scope.clone(), started: Notify::new(), release: Notify::new(), finished: Notify::new(), completed: AtomicUsize::new(0) });
    let routes = Arc::new(Routes { owner: owner.clone(), disconnected: Notify::new() });
    let router = iroh::protocol::Router::builder(server.clone()).accept(
        misa_proto::scoped::ALPN,
        Handler::new(server.id().to_string(), scope.clone(), routes.clone(), Arc::new(Admission::open())),
    ).spawn();
    let connect = || crate::scoped_client::Client::connect(&endpoint, server.addr(), ClientInfo::new("lifetime-test", "1"));
    let invoke = |client: &crate::scoped_client::Client, id| client.try_send(ClientMessage::Invoke { invocation: Invocation { id, scope: scope.clone(), command: "write".into(), input: Value::Null } }).unwrap();
    let client = connect().await.unwrap();
    invoke(&client, 1);
    tokio::time::timeout(std::time::Duration::from_secs(5), owner.started.notified()).await.unwrap();
    drop(client);
    tokio::time::timeout(std::time::Duration::from_secs(5), routes.disconnected.notified()).await.unwrap();
    assert_eq!(owner.completed.load(Ordering::SeqCst), 0);
    owner.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), owner.finished.notified()).await.unwrap();
    assert_eq!(owner.completed.load(Ordering::SeqCst), 1);

    let client = connect().await.unwrap();
    invoke(&client, 2);
    tokio::time::timeout(std::time::Duration::from_secs(5), owner.started.notified()).await.unwrap();
    let mut shutdown = tokio::spawn(async move { router.shutdown().await.unwrap(); });
    assert!(tokio::time::timeout(std::time::Duration::from_millis(50), &mut shutdown).await.is_err(), "shutdown must wait for admitted writes");
    owner.release.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(5), shutdown).await.unwrap().unwrap();
    assert_eq!(owner.completed.load(Ordering::SeqCst), 2);
    drop(client);
    endpoint.close().await;
}
