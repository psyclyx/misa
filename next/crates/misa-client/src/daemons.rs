//! Shared daemon relationships. Selecting a session is local surface state and
//! does not change these connections or their background directory observations.
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use iroh::{Endpoint, EndpointAddr};
use misa_proto::{ClientInfo, Fault, Pairing, directory::Entry};
use misa_protocol::observation::{MemberState, Status};
use tokio::sync::{Notify, OnceCell, watch};

use crate::driver::{Client, Observation};

#[derive(Clone)]
pub struct DirectorySnapshot {
    pub scope: misa_proto::observation::Scope,
    pub status: Status,
    pub publication: Option<u64>,
    pub sessions: Vec<Entry>,
}

pub struct Daemon {
    identity: String,
    pub client: Client,
    pub blobs: Arc<crate::transfers::Transfers>,
    directory: Arc<std::sync::Mutex<DirectoryState>>,
    changes: watch::Receiver<u64>,
    monitor: tokio::task::AbortHandle,
}

enum DirectoryState {
    Observed(Observation),
    Refreshing(DirectorySnapshot),
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.monitor.abort();
        self.blobs.close();
    }
}

impl Daemon {
    pub fn identity(&self) -> &str {
        &self.identity
    }
    pub fn watch(&self) -> watch::Receiver<u64> {
        self.changes.clone()
    }

    pub fn sessions(&self) -> Result<DirectorySnapshot, Fault> {
        let directory = self.directory.lock().expect("directory lock poisoned");
        let mut snapshot = match &*directory {
            DirectoryState::Observed(observation) => Self::snapshot(observation)?,
            DirectoryState::Refreshing(snapshot) => snapshot.clone(),
        };
        if snapshot.scope != self.client.welcome().scope {
            snapshot.status =
                Status::Stale(Fault::query("Daemon restarted; refreshing its directory"));
        }
        Ok(snapshot)
    }

    fn snapshot(observation: &Observation) -> Result<DirectorySnapshot, Fault> {
        observation
            .inspect(|replica, _| {
                let sessions = match replica
                    .last_good()
                    .and_then(|members| members.get("sessions"))
                {
                    Some(MemberState::Value(value)) => misa_proto::directory::entries(value)?,
                    Some(_) => {
                        return Err(Fault::query("Daemon directory has an invalid result type"));
                    }
                    None => vec![],
                };
                Ok(DirectorySnapshot {
                    scope: replica.selection().scope.clone(),
                    status: replica.status().clone(),
                    publication: replica.position(),
                    sessions,
                })
            })
            .unwrap_or_else(|| {
                Err(Fault::new(
                    "closed",
                    "Daemon directory observation is closed",
                ))
            })
    }

    fn new(
        identity: String,
        client: Client,
        directory: Observation,
        blobs: Arc<misa_transport::blob::Store>,
    ) -> Arc<Self> {
        let blobs = crate::transfers::Transfers::new(blobs);
        let transfers = blobs.clone();
        let mut scope = directory
            .inspect(|replica, _| replica.selection().scope.clone())
            .expect("new directory observation");
        let mut updates = directory.watch();
        let directory = Arc::new(std::sync::Mutex::new(DirectoryState::Observed(directory)));
        let (changes, receiver) = watch::channel(0u64);
        let observed = directory.clone();
        let connection = client.clone();
        let mut status = connection.status();
        let monitor = tokio::spawn(async move {
            let mut refreshing = false;
            loop {
                tokio::select! {
                    result = updates.changed(), if !refreshing => if result.is_err() { break; },
                    result = status.changed() => if result.is_err() { break; },
                    _ = tokio::time::sleep(Duration::from_millis(250)),
                        if matches!(status.borrow().phase, crate::driver::Phase::Online) && status.borrow().welcome.scope != scope => {},
                }
                let current = status.borrow_and_update().clone();
                if matches!(current.phase, crate::driver::Phase::Online) && current.welcome.scope != scope {
                    {
                        let mut directory = observed.lock().expect("directory lock poisoned");
                        if let DirectoryState::Observed(observation) = &*directory {
                            let mut snapshot = Self::snapshot(observation).unwrap_or_else(|fault| DirectorySnapshot { scope: scope.clone(), status: Status::Stale(fault), publication: None, sessions: vec![] });
                            snapshot.status = Status::Stale(Fault::query("Daemon restarted; refreshing its directory"));
                            *directory = DirectoryState::Refreshing(snapshot);
                        }
                    }
                    refreshing = true;
                    match connection.observe(misa_proto::directory::selection(current.welcome.scope.clone()), None).await {
                        Ok(replacement) => {
                            updates = replacement.watch();
                            *observed.lock().expect("directory lock poisoned") = DirectoryState::Observed(replacement);
                            scope = current.welcome.scope;
                            refreshing = false;
                        }
                        Err(_) => { /* Retry transient admission/queue pressure while this owner remains current. */ }
                    }
                }
                changes.send_modify(|sequence| *sequence = sequence.wrapping_add(1));
                if matches!(current.phase, crate::driver::Phase::Disconnected) { transfers.close(); break; }
            }
        }).abort_handle();
        Arc::new(Self {
            identity,
            client,
            blobs,
            directory,
            changes: receiver,
            monitor,
        })
    }
}

/// A single endpoint supplies the client's persistent identity to every daemon.
/// Concurrent connects to one peer share one attempt; failed attempts release
/// capacity and can be retried explicitly. The default relationship cap is 32.
pub struct Daemons {
    endpoint: Endpoint,
    info: ClientInfo,
    capacity: usize,
    connected: std::sync::Mutex<BTreeMap<String, Arc<Connecting>>>,
}

#[derive(Default)]
struct Connecting {
    result: OnceCell<Result<Arc<Daemon>, Fault>>,
    cancelled: AtomicBool,
    stop: Notify,
    waiters: AtomicUsize,
}

/// A cancelled final connect future must release its reserved relationship slot.
/// Other callers waiting on the same OnceCell can continue its initializer.
struct Waiting<'a> {
    registry: &'a Daemons,
    identity: &'a str,
    pending: &'a Arc<Connecting>,
}
impl Drop for Waiting<'_> {
    fn drop(&mut self) {
        let mut entries = self
            .registry
            .connected
            .lock()
            .expect("daemon registry lock poisoned");
        if self.pending.waiters.fetch_sub(1, Ordering::AcqRel) == 1
            && self.pending.result.get().is_none()
            && entries
                .get(self.identity)
                .is_some_and(|entry| Arc::ptr_eq(entry, self.pending))
        {
            entries.remove(self.identity);
        }
    }
}

impl Daemons {
    pub fn new(endpoint: Endpoint, info: ClientInfo) -> Self {
        Self {
            endpoint,
            info,
            capacity: 32,
            connected: std::sync::Mutex::new(BTreeMap::new()),
        }
    }

    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }

    pub async fn connect(&self, address: EndpointAddr) -> Result<Arc<Daemon>, Fault> {
        let identity = address.id.to_string();
        let pending = {
            let mut connected = self
                .connected
                .lock()
                .expect("daemon registry lock poisoned");
            let pending = if let Some(existing) = connected.get(&identity) {
                existing.clone()
            } else {
                if connected.len() >= self.capacity {
                    return Err(Fault::new("busy", "Client daemon capacity reached"));
                }
                let pending = Arc::new(Connecting::default());
                connected.insert(identity.clone(), pending.clone());
                pending
            };
            pending.waiters.fetch_add(1, Ordering::Relaxed);
            pending
        };
        let _waiting = Waiting {
            registry: self,
            identity: &identity,
            pending: &pending,
        };
        let result = pending.result.get_or_init(|| async {
            if pending.cancelled.load(Ordering::Acquire) { return Err(Fault::new("closed", "Daemon connection was cancelled")); }
            let client = tokio::select! {
                _ = pending.stop.notified() => return Err(Fault::new("closed", "Daemon connection was cancelled")),
                result = tokio::time::timeout(Duration::from_secs(10), Client::connect(&self.endpoint, address.clone(), self.info.clone(), crate::Limits::default())) =>
                    result.map_err(|_| Fault::new("timeout", "Connecting to daemon timed out"))??,
            };
            let directory = client.observe(misa_proto::directory::selection(client.welcome().scope.clone()), None).await?;
            Ok(Daemon::new(identity.clone(), client, directory, misa_transport::blob::Store::new(self.endpoint.clone(), address.clone())))
        }).await.clone();
        if pending.cancelled.load(Ordering::Acquire) {
            if let Ok(daemon) = result {
                let _ = daemon.client.disconnect().await;
            }
            return Err(Fault::new("closed", "Daemon connection was cancelled"));
        }
        if result.is_err() {
            let mut connected = self
                .connected
                .lock()
                .expect("daemon registry lock poisoned");
            if connected
                .get(&identity)
                .is_some_and(|entry| Arc::ptr_eq(entry, &pending))
            {
                connected.remove(&identity);
            }
        }
        result
    }

    /// Pairing authorizes this client's key; the ticket's session name remains a
    /// selection hint for the calling surface, never a connection attachment.
    pub async fn connect_target(
        &self,
        target: &str,
    ) -> Result<(Arc<Daemon>, Option<String>), Fault> {
        let (node, code, hint) = if target.starts_with("misa:") || target.starts_with("misa-pair:")
        {
            let (ticket, code) = Pairing::given(target).map_err(Fault::protocol)?;
            let hint = (!ticket.session.is_empty()).then_some(ticket.session);
            (ticket.node, code, hint)
        } else {
            (target.to_owned(), None, None)
        };
        let address = misa_transport::iroh::address_of(&node).map_err(Fault::protocol)?;
        #[cfg(unix)]
        let local = misa_transport::local::pair(&node, &self.endpoint.id().to_string())
            .await
            .map_err(Fault::protocol)?;
        #[cfg(not(unix))]
        let local = false;
        if let Some(code) = code.filter(|_| !local) {
            // Admission is a separate exchange from
            // scoped application traffic; no attached session is opened here.
            misa_transport::pairing::pair(
                &self.endpoint,
                address.clone(),
                &code,
                &self.info.name,
            )
            .await
            .map_err(Fault::protocol)?;
        }
        Ok((self.connect(address).await?, hint))
    }

    pub async fn connected(&self) -> Vec<Arc<Daemon>> {
        self.connected
            .lock()
            .expect("daemon registry lock poisoned")
            .values()
            .filter_map(|pending| {
                pending
                    .result
                    .get()
                    .and_then(|result| result.as_ref().ok())
                    .cloned()
            })
            .collect()
    }

    /// Forget a relationship and explicitly stop its connection, even when local
    /// presentation instances still retain a handle to its stale replica.
    pub async fn disconnect(&self, identity: &str) -> bool {
        let pending = self
            .connected
            .lock()
            .expect("daemon registry lock poisoned")
            .remove(identity);
        let Some(pending) = pending else {
            return false;
        };
        pending.cancelled.store(true, Ordering::Release);
        pending.stop.notify_one();
        if let Some(Ok(daemon)) = pending.result.get() {
            daemon.blobs.close();
            let _ = daemon.client.disconnect().await;
        }
        true
    }

    #[cfg(unix)]
    pub async fn discover_local(&self) -> Result<Vec<Result<Arc<Daemon>, Fault>>, Fault> {
        let targets = misa_transport::local::discover()
            .await
            .map_err(Fault::protocol)?;
        let mut results = Vec::with_capacity(targets.len().min(self.capacity));
        for target in targets.into_iter().take(self.capacity) {
            results.push(self.connect_target(&target).await.map(|(daemon, _)| daemon));
        }
        Ok(results)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{
        invocation::{Command as Definition, Invocation, Outcome},
        observation::{Content, Handle, Publication, Resume, Scope, ScopeId, Selection, Snapshot},
        scoped,
    };
    use misa_protocol::{
        invocation::{CallContext, CommandOwner, Execution},
        owner::{Observation as OwnerObservation, Owner, Resolver},
    };
    use misa_value::Value;

    struct Directory {
        scope: Scope,
        revisions: watch::Sender<u64>,
    }
    impl Directory {
        fn new(incarnation: &str) -> Arc<Self> {
            Arc::new(Self {
                scope: Scope {
                    id: ScopeId::Daemon,
                    incarnation: incarnation.into(),
                },
                revisions: watch::channel(0).0,
            })
        }
        fn snapshot(&self) -> Snapshot {
            Snapshot {
                position: 0,
                members: BTreeMap::from([(
                    "sessions".into(),
                    Content::Value(Value::list([Value::map([
                        ("id", Value::str("same-label")),
                        ("incarnation", Value::str(&self.scope.incarnation)),
                        ("title", Value::str("same-label")),
                        ("availability", Value::str("current")),
                        ("source_position".into(), Value::Int(0)),
                        ("summary".into(), Value::Null),
                    ])])),
                )]),
            }
        }
    }
    impl CommandOwner for Directory {
        fn scope(&self) -> Scope {
            self.scope.clone()
        }
        fn command(&self, _: &CallContext, _: &str) -> Option<Definition> {
            None
        }
        fn execute<'a>(&'a self, _: &'a CallContext, _: Invocation) -> Execution<'a> {
            Box::pin(async {
                Outcome::Rejected {
                    fault: Fault::query("No commands"),
                }
            })
        }
    }
    struct Idle {
        changed: watch::Receiver<u64>,
        _owner: Arc<Directory>,
    }
    impl OwnerObservation for Idle {
        fn changed(&mut self) -> &mut watch::Receiver<u64> {
            &mut self.changed
        }
        fn poll(&mut self) -> Option<Publication> {
            None
        }
    }
    impl Owner for Directory {
        fn read(&self, _: &CallContext, selection: &Selection) -> Result<Snapshot, Fault> {
            if selection != &misa_proto::directory::selection(self.scope.clone()) {
                return Err(Fault::query("Unknown selection"));
            }
            Ok(self.snapshot())
        }
        fn observe(
            self: Arc<Self>,
            context: CallContext,
            handle: Handle,
            selection: Selection,
            _: Option<Resume>,
        ) -> Result<(Box<dyn OwnerObservation>, Publication), Fault> {
            let snapshot = self.read(&context, &selection)?;
            Ok((
                Box::new(Idle {
                    changed: self.revisions.subscribe(),
                    _owner: self,
                }),
                Publication::Snapshot { handle, snapshot },
            ))
        }
    }
    struct Resolve(Arc<Directory>);
    impl Resolver for Resolve {
        fn resolve(&self, _: &CallContext, scope: &Scope) -> Result<Arc<dyn Owner>, Fault> {
            if scope != &self.0.scope {
                return Err(Fault::query("Owner incarnation ended"));
            }
            Ok(self.0.clone())
        }
    }
    fn handler(server: &Endpoint, owner: Arc<Directory>) -> misa_transport::scoped_server::Handler {
        misa_transport::scoped_server::Handler {
            daemon: server.id().to_string(),
            scope: owner.scope.clone(),
            resolver: Arc::new(Resolve(owner)),
            admission: Arc::new(misa_transport::admission::Admission::open()),
        }
    }
    fn address(endpoint: &Endpoint) -> EndpointAddr {
        misa_transport::iroh::address_of(&misa_transport::iroh::node_of(endpoint)).unwrap()
    }
    async fn current(daemon: &Daemon, incarnation: &str) {
        let mut changes = daemon.watch();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let snapshot = daemon.sessions().unwrap();
                if matches!(snapshot.status, Status::Current)
                    && snapshot.scope.incarnation == incarnation
                    && !snapshot.sessions.is_empty()
                {
                    break;
                }
                changes.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }
    #[tokio::test]
    async fn multiple_daemons_keep_same_named_sessions_distinct_and_deduplicate_peers() {
        let a = misa_transport::iroh::bind(None, false).await.unwrap();
        let b = misa_transport::iroh::bind(None, false).await.unwrap();
        let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
        let ar = iroh::protocol::Router::builder(a.clone())
            .accept(scoped::ALPN, handler(&a, Directory::new("a")))
            .spawn();
        let br = iroh::protocol::Router::builder(b.clone())
            .accept(scoped::ALPN, handler(&b, Directory::new("b")))
            .spawn();
        let registry = Daemons::new(endpoint.clone(), ClientInfo::new("registry-test", "1"));
        let (first, duplicate, second) = tokio::join!(
            registry.connect(address(&a)),
            registry.connect(address(&a)),
            registry.connect(address(&b))
        );
        let (first, duplicate, second) = (first.unwrap(), duplicate.unwrap(), second.unwrap());
        assert!(Arc::ptr_eq(&first, &duplicate));
        current(&first, "a").await;
        current(&second, "b").await;
        let one = first.sessions().unwrap().sessions.remove(0);
        let two = second.sessions().unwrap().sessions.remove(0);
        assert_eq!(one.title, two.title);
        assert_ne!(one.scope(), two.scope());
        assert_ne!(first.identity(), second.identity());
        assert_eq!(registry.connected().await.len(), 2);
        assert!(registry.disconnect(first.identity()).await);
        assert!(!first.client.online());
        assert!(second.client.online());
        registry.disconnect(second.identity()).await;
        ar.shutdown().await.unwrap();
        br.shutdown().await.unwrap();
        endpoint.close().await;
    }
    struct Restart {
        first: misa_transport::scoped_server::Handler,
        second: misa_transport::scoped_server::Handler,
        accepts: AtomicUsize,
        cut: Arc<Notify>,
    }
    impl std::fmt::Debug for Restart {
        fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            out.write_str("restart fixture")
        }
    }
    impl iroh::protocol::ProtocolHandler for Restart {
        async fn accept(
            &self,
            connection: iroh::endpoint::Connection,
        ) -> Result<(), iroh::protocol::AcceptError> {
            if self.accepts.fetch_add(1, Ordering::SeqCst) == 0 {
                let socket = connection.clone();
                let cut = self.cut.clone();
                let stop = tokio::spawn(async move {
                    cut.notified().await;
                    socket.close(1u32.into(), b"restart");
                });
                let result = self.first.accept(connection).await;
                stop.abort();
                result
            } else {
                self.second.accept(connection).await
            }
        }
    }
    #[tokio::test]
    async fn restart_replaces_only_bootstrap_and_preserves_directory_watch() {
        let server = misa_transport::iroh::bind(None, false).await.unwrap();
        let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
        let cut = Arc::new(Notify::new());
        let router = iroh::protocol::Router::builder(server.clone())
            .accept(
                scoped::ALPN,
                Restart {
                    first: handler(&server, Directory::new("first")),
                    second: handler(&server, Directory::new("second")),
                    accepts: AtomicUsize::new(0),
                    cut: cut.clone(),
                },
            )
            .spawn();
        let registry = Daemons::new(endpoint.clone(), ClientInfo::new("registry-test", "1"));
        let daemon = registry.connect(address(&server)).await.unwrap();
        current(&daemon, "first").await;
        let original = daemon.sessions().unwrap().scope;
        let mut independent = daemon
            .client
            .observe(misa_proto::directory::selection(original), None)
            .await
            .unwrap();
        independent.changed().await.unwrap();
        let mut stable_watch = daemon.watch();
        stable_watch.borrow_and_update();
        cut.notify_one();
        current(&daemon, "second").await;
        assert!(stable_watch.has_changed().unwrap());
        assert_eq!(daemon.sessions().unwrap().sessions[0].incarnation, "second");
        tokio::time::timeout(Duration::from_secs(5), async {
            while !independent
                .inspect(|replica, _| matches!(replica.status(), Status::Closed(_)))
                .unwrap()
            {
                independent.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert!(Arc::ptr_eq(
            &daemon,
            &registry.connect(address(&server)).await.unwrap()
        ));
        registry.disconnect(daemon.identity()).await;
        router.shutdown().await.unwrap();
        endpoint.close().await;
    }
    #[tokio::test]
    async fn cancelled_final_connect_releases_capacity() {
        let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
        let unavailable = misa_transport::iroh::bind(None, false).await.unwrap();
        let mut registry = Daemons::new(endpoint.clone(), ClientInfo::new("registry-test", "1"));
        registry.capacity = 1;
        let registry = Arc::new(registry);
        let caller = registry.clone();
        let target = address(&unavailable);
        let pending = tokio::spawn(async move { caller.connect(target).await });
        tokio::time::timeout(Duration::from_secs(2), async {
            while registry.connected.lock().unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        pending.abort();
        let _ = pending.await;
        assert!(registry.connected.lock().unwrap().is_empty());
        endpoint.close().await;
        unavailable.close().await;
    }
}
