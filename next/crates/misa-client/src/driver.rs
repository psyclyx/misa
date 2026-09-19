//! Shared asynchronous connection owner. Slow surfaces coalesce notifications;
//! they never hold back replica application or directed request results.
use crate::{Connection, Limits, ObservationId, Outgoing, ReadValue, Settled};
use iroh::{Endpoint, EndpointAddr};
use misa_proto::{
    ClientInfo, Fault,
    invocation::{Command as Definition, Reply},
    observation::{Scope, Selection},
    scoped::ServerMessage,
};
use misa_protocol::observation::{Applied, Checkpoint, Replica};
use misa_transport::scoped_client::{Client as Wire, Event, Welcome};
use misa_value::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::sync::{Notify, mpsc, oneshot, watch};

/// The latest invalidation and local delivery sequence. Incremental renderers
/// apply it only when contiguous with their last sequence; otherwise rebuild
/// from the coherent replica supplied in the same inspect call.
#[derive(Clone)]
pub struct Notice {
    pub sequence: u64,
    pub applied: Arc<Applied>,
}
struct State {
    core: Connection,
    notices: BTreeMap<ObservationId, Notice>,
}
struct Inner {
    status: watch::Receiver<Status>,
    state: Arc<Mutex<State>>,
    commands: mpsc::UnboundedSender<Request>,
    cancellations: mpsc::Sender<ObservationId>,
    sweep: Arc<Notify>,
    stop: Arc<AtomicBool>,
    task: tokio::task::AbortHandle,
}
#[derive(Clone, Debug)]
pub enum Phase {
    Online,
    Reconnecting { attempt: u32 },
    Offline,
    Disconnected,
}
#[derive(Clone, Debug)]
pub struct Status {
    pub phase: Phase,
    pub welcome: Welcome,
    pub generation: u64,
    pub last_error: Option<String>,
}
#[derive(Clone)]
struct Dial {
    endpoint: Endpoint,
    address: EndpointAddr,
    info: ClientInfo,
}
impl Drop for Inner {
    fn drop(&mut self) {
        self.task.abort();
    }
}
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}
pub struct Observation {
    id: ObservationId,
    client: Client,
    changes: Option<watch::Receiver<u64>>,
}
impl Observation {
    pub fn id(&self) -> ObservationId {
        self.id
    }
    pub fn daemon_identity(&self) -> String {
        self.client.welcome().daemon
    }
    pub fn watch(&self) -> watch::Receiver<u64> {
        self.changes.as_ref().expect("live lease").clone()
    }
    pub async fn changed(&mut self) -> Result<(), Fault> {
        self.changes
            .as_mut()
            .expect("live lease")
            .changed()
            .await
            .map_err(|_| Fault::new("closed", "Observation driver closed"))
    }
    /// Borrow current state briefly. Never block or perform IO in this closure.
    /// Notice and replica are read under the same publication boundary.
    pub fn inspect<R>(&self, inspect: impl FnOnce(&Replica, Option<&Notice>) -> R) -> Option<R> {
        let state = self
            .client
            .inner
            .state
            .lock()
            .expect("client state poisoned");
        Some(inspect(
            state.core.replica(self.id)?,
            state.notices.get(&self.id),
        ))
    }
}
impl Drop for Observation {
    fn drop(&mut self) {
        self.changes.take();
        let _ = self.client.inner.cancellations.try_send(self.id);
        // If the bounded cancellation queue is full, scanning closed receivers
        // still releases every dropped lease. There is no unbounded side queue.
        self.client.inner.sweep.notify_one();
    }
}
struct Lease {
    id: ObservationId,
    changes: watch::Receiver<u64>,
}
enum Request {
    RefreshAddress(EndpointAddr),
    Observe {
        selection: Selection,
        checkpoint: Option<Checkpoint>,
        reply: oneshot::Sender<Result<Lease, Fault>>,
    },
    Invoke {
        scope: Scope,
        definition: Definition,
        input: Value,
        deadline: Instant,
        reply: oneshot::Sender<Result<Reply, Fault>>,
    },
    Read {
        selection: Selection,
        deadline: Instant,
        reply: oneshot::Sender<Result<ReadValue, Fault>>,
    },
}
type Calls = BTreeMap<u64, oneshot::Sender<Result<Reply, Fault>>>;
type Reads = BTreeMap<u64, oneshot::Sender<Result<ReadValue, Fault>>>;

impl Client {
    /// Refresh routing hints for the same authenticated peer without replacing leases.
    pub async fn refresh_address(&self, address: EndpointAddr) -> Result<(), Fault> {
        if address.id.to_string() != self.welcome().daemon {
            return Err(Fault::new(
                "identity",
                "Routing hints name a different daemon",
            ));
        }
        self.inner
            .commands
            .send(Request::RefreshAddress(address))
            .map_err(|_| stopped())
    }

    pub async fn connect(
        endpoint: &Endpoint,
        address: EndpointAddr,
        info: ClientInfo,
        limits: Limits,
    ) -> Result<Self, Fault> {
        let dial = Dial {
            endpoint: endpoint.clone(),
            address: address.clone(),
            info: info.clone(),
        };
        let wire = Wire::connect(endpoint, address, info)
            .await
            .map_err(|reason| Fault::new("connect", reason))?;
        let welcome = wire.welcome.clone();
        let capacity = limits.observations.max(1);
        let state = Arc::new(Mutex::new(State {
            core: Connection::new(limits),
            notices: BTreeMap::new(),
        }));
        // Requests are already bounded by their own deadlines and by the daemon's
        // validated protocol/resource budgets. A second fixed queue here turns an
        // ordinary burst of independent reads into a user-visible "queue full"
        // failure, even though the driver could simply retain the request until it
        // reaches the wire.
        let (commands, receiver) = mpsc::unbounded_channel();
        let (cancellations, cancelled) = mpsc::channel(capacity);
        let sweep = Arc::new(Notify::new());
        let stop = Arc::new(AtomicBool::new(false));
        let (status_tx, status) = watch::channel(Status {
            phase: Phase::Online,
            welcome,
            generation: 1,
            last_error: None,
        });
        let task = tokio::spawn(run(
            wire,
            state.clone(),
            receiver,
            cancelled,
            sweep.clone(),
            dial,
            status_tx,
            stop.clone(),
        ))
        .abort_handle();
        Ok(Self {
            inner: Arc::new(Inner {
                status,
                state,
                commands,
                cancellations,
                sweep,
                stop,
                task,
            }),
        })
    }
    pub fn welcome(&self) -> Welcome {
        self.inner.status.borrow().welcome.clone()
    }
    pub fn status(&self) -> watch::Receiver<Status> {
        self.inner.status.clone()
    }
    pub async fn disconnect(&self) -> Result<(), Fault> {
        self.inner.stop.store(true, Ordering::Release);
        self.inner.sweep.notify_one();
        let mut status = self.status();
        while !matches!(status.borrow().phase, Phase::Disconnected) {
            status.changed().await.map_err(|_| stopped())?;
        }
        Ok(())
    }
    pub fn online(&self) -> bool {
        self.inner
            .state
            .lock()
            .expect("client state poisoned")
            .core
            .online()
    }
    pub async fn observe(
        &self,
        selection: Selection,
        checkpoint: Option<Checkpoint>,
    ) -> Result<Observation, Fault> {
        let (reply, result) = oneshot::channel();
        self.submit(Request::Observe {
            selection,
            checkpoint,
            reply,
        })
        .await?;
        let lease = result.await.map_err(|_| stopped())??;
        Ok(Observation {
            id: lease.id,
            changes: Some(lease.changes),
            client: self.clone(),
        })
    }
    pub async fn invoke(
        &self,
        scope: Scope,
        definition: Definition,
        input: Value,
        timeout: Duration,
    ) -> Result<Reply, Fault> {
        let (reply, result) = oneshot::channel();
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| Fault::protocol("Invalid invocation timeout"))?;
        self.submit(Request::Invoke {
            scope,
            definition,
            input,
            deadline,
            reply,
        })
        .await?;
        result.await.map_err(|_| stopped())?
    }
    pub async fn read(&self, selection: Selection, timeout: Duration) -> Result<ReadValue, Fault> {
        let (reply, result) = oneshot::channel();
        let deadline = Instant::now()
            .checked_add(timeout)
            .ok_or_else(|| Fault::protocol("Invalid read timeout"))?;
        self.submit(Request::Read {
            selection,
            deadline,
            reply,
        })
        .await?;
        result.await.map_err(|_| stopped())?
    }
    async fn submit(&self, request: Request) -> Result<(), Fault> {
        self.inner.commands.send(request).map_err(|_| stopped())
    }
}
fn stopped() -> Fault {
    Fault::new("closed", "Client connection owner stopped")
}
fn queued(wire: &Option<Wire>, request: Outgoing) -> Result<(), Fault> {
    wire.as_ref()
        .ok_or_else(stopped)?
        .try_send(request.into())
        .map_err(|reason| Fault::new("queue", reason))
}
fn settle(settled: Settled, calls: &mut Calls, reads: &mut Reads) {
    for reply in settled.invocations {
        if let Some(target) = calls.remove(&reply.id) {
            let _ = target.send(Ok(reply));
        }
    }
    for reply in settled.reads {
        if let Some(target) = reads.remove(&reply.id) {
            let _ = target.send(reply.result);
        }
    }
}
fn notice(
    state: &mut State,
    watches: &BTreeMap<ObservationId, watch::Sender<u64>>,
    id: ObservationId,
    applied: Applied,
) {
    let sequence = state
        .notices
        .get(&id)
        .map_or(1, |notice| notice.sequence + 1);
    state.notices.insert(
        id,
        Notice {
            sequence,
            applied: Arc::new(applied),
        },
    );
    if let Some(watch) = watches.get(&id) {
        watch.send_replace(sequence);
    }
}
fn disconnect(
    state: &mut State,
    watches: &BTreeMap<ObservationId, watch::Sender<u64>>,
    calls: &mut Calls,
    reads: &mut Reads,
) {
    let generation = state.core.generation();
    settle(state.core.disconnected(generation), calls, reads);
    for id in watches.keys() {
        notice(state, watches, *id, Applied::Stale);
    }
}
async fn run(
    wire: Wire,
    state: Arc<Mutex<State>>,
    mut commands: mpsc::UnboundedReceiver<Request>,
    mut cancelled: mpsc::Receiver<ObservationId>,
    sweep: Arc<Notify>,
    mut dial: Dial,
    status: watch::Sender<Status>,
    stop: Arc<AtomicBool>,
) {
    let mut wire = Some(wire);
    let mut watches = BTreeMap::new();
    let mut calls = Calls::new();
    let mut reads = Reads::new();
    let mut attempts = 0u32;
    let mut enabled = true;
    let mut reconnect = tokio::task::JoinSet::new();
    // At most one resume/repair and one cancellation per retained observation.
    // New observations cannot overtake this ordered, observation-bounded backlog.
    let mut interests = VecDeque::<Outgoing>::new();
    loop {
        if stop.load(Ordering::Acquire) && !matches!(status.borrow().phase, Phase::Disconnected) {
            enabled = false;
            reconnect.abort_all();
            let mut state = state.lock().expect("client state poisoned");
            disconnect(&mut state, &watches, &mut calls, &mut reads);
            wire = None;
            status.send_modify(|state| state.phase = Phase::Disconnected);
        }
        // Readers dropped during cancelled request futures also release interest.
        let abandoned: Vec<_> = watches
            .iter()
            .filter(|(_, sender): &(_, &watch::Sender<u64>)| sender.receiver_count() == 0)
            .map(|(id, _)| *id)
            .collect();
        for id in abandoned {
            watches.remove(&id);
            let mut state = state.lock().expect("client state poisoned");
            if let Some(request) = state.core.cancel(id) {
                if wire.is_some() {
                    interests.push_back(request);
                }
            }
            state.notices.remove(&id);
        }
        if wire.is_none() {
            interests.clear();
        }
        if wire.is_none() && enabled && reconnect.is_empty() && attempts < 6 {
            attempts += 1;
            status.send_modify(|state| {
                state.phase = Phase::Reconnecting { attempt: attempts };
            });
            let dial = dial.clone();
            let attempt = attempts;
            reconnect.spawn(async move {
                tokio::time::sleep(Duration::from_millis((100u64 << (attempt - 1)).min(2000)))
                    .await;
                Wire::connect(&dial.endpoint, dial.address, dial.info).await
            });
        }
        let deadline = state.lock().expect("client state poisoned").core.deadline();
        let capacity = wire
            .as_ref()
            .filter(|_| !interests.is_empty())
            .map(Wire::reserve);
        tokio::select! {
            ready = async { capacity.expect("guarded").await }, if !interests.is_empty() && wire.is_some() => {
                match ready {
                    Ok(permit) => {
                        let message = interests.pop_front().expect("pending interest").into();
                        permit.send(message);
                    },
                    Err(reason) => {
                        let mut state = state.lock().expect("client state poisoned");
                        disconnect(&mut state, &watches, &mut calls, &mut reads); wire = None;
                        status.send_modify(|state| state.last_error = Some(reason));
                    }
                }
            },
            result = reconnect.join_next(), if !reconnect.is_empty() => {
                if !enabled { continue; }
                match result.expect("guarded") {
                    Ok(Ok(fresh)) => {
                        let mut state = state.lock().expect("client state poisoned");
                        match state.core.reconnected() {
                            Ok(requests) => {
                                let welcome = fresh.welcome.clone(); wire = Some(fresh);
                                interests = requests.into();
                                attempts = 0;
                                status.send_replace(Status { phase: Phase::Online, welcome, generation: state.core.generation(), last_error: None });
                            },
                            Err(fault) => { enabled = false; status.send_modify(|state| { state.phase = Phase::Offline; state.last_error = Some(fault.message.clone()); }); }
                        }
                    },
                    failed => {
                        let reason = match failed { Ok(Err(reason)) => reason, Err(error) => error.to_string(), _ => unreachable!() };
                        status.send_modify(|state| { state.last_error = Some(reason.clone()); if attempts >= 6 { state.phase = Phase::Offline; } });
                    },
                }
            },
            _ = sweep.notified() => {},
            id = cancelled.recv() => if let Some(id) = id {
                watches.remove(&id);
                let mut state = state.lock().expect("client state poisoned");
                if let Some(request) = state.core.cancel(id) {
                    if wire.is_some() { interests.push_back(request); }
                }
                state.notices.remove(&id);
            },
            _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline.unwrap_or_else(|| Instant::now()+Duration::from_secs(3600)))), if deadline.is_some() => {
                let settled = state.lock().expect("client state poisoned").core.expire(Instant::now());
                settle(settled, &mut calls, &mut reads);
            },
            request = commands.recv() => {
                let Some(request) = request else { return; };
                let mut state = state.lock().expect("client state poisoned");
                match request {
                    Request::RefreshAddress(address) => {
                        dial.address = address;
                        if wire.is_none() && enabled {
                            reconnect.abort_all();
                            reconnect = tokio::task::JoinSet::new();
                            attempts = 0;
                        }
                    }
                    Request::Observe { selection, checkpoint, reply } => {
                        if reply.is_closed() { continue; }
                        if !interests.is_empty() { let _ = reply.send(Err(Fault::new("busy", "Restoring observation interests"))); continue; }
                        let result = state.core.observe(selection, checkpoint).and_then(|(id, request)| {
                            if let Err(error) = queued(&wire, request) { state.core.cancel(id); return Err(error); }
                            let (sender, changes) = watch::channel(0);
                            watches.insert(id, sender);
                            Ok(Lease { id, changes })
                        });
                        let _ = reply.send(result);
                    }
                    Request::Invoke { scope, definition, input, deadline, reply } => {
                        if reply.is_closed() { continue; }
                        match state.core.invoke(scope, &definition, input, deadline) {
                            Ok(request) => {
                                let Outgoing::Invoke(ref call) = request else { unreachable!() }; let id = call.id;
                                if let Err(error) = queued(&wire, request) { state.core.abandon(id); let _ = reply.send(Err(error)); }
                                else { calls.insert(id, reply); }
                            },
                            Err(error) => { let _ = reply.send(Err(error)); }
                        }
                    }
                    Request::Read { selection, deadline, reply } => {
                        if reply.is_closed() { continue; }
                        match state.core.read(selection, deadline) {
                            Ok(request) => {
                                let Outgoing::Read(ref read) = request else { unreachable!() }; let id = read.id;
                                if let Err(error) = queued(&wire, request) { state.core.abandon(id); let _ = reply.send(Err(error)); }
                                else { reads.insert(id, reply); }
                            },
                            Err(error) => { let _ = reply.send(Err(error)); }
                        }
                    }
                }
            },
            incoming = async { wire.as_mut().expect("guarded").next().await }, if wire.is_some() => {
                let mut state = state.lock().expect("client state poisoned");
                let generation = state.core.generation();
                let _permit;
                let incoming = match incoming {
                    Ok(Event::Message(frame)) => { _permit = frame.permit; Ok(frame.message) },
                    Ok(Event::LaneClosed { lane, reason }) => {
                        match lane {
                            misa_proto::scoped::Lane::Observation { handle } => {
                                if let Some(received) = state.core.lane_closed(generation, handle, reason) {
                                    if let Some(applied) = received.applied { notice(&mut state, &watches, received.observation, applied); }
                                    if let Some(request) = received.recovery { interests.push_back(request); }
                                }
                            },
                            misa_proto::scoped::Lane::Read { id } => {
                                let reply = misa_proto::scoped::ReadReply { id, outcome: misa_proto::scoped::ReadOutcome::Rejected { fault: Fault::new("read_lane", reason) } };
                                if let Some(reply) = state.core.read_reply(generation, reply) { if let Some(target) = reads.remove(&id) { let _ = target.send(reply.result); } }
                            },
                        }
                        continue;
                    },
                    Err(reason) => { _permit = None; Err(reason) },
                };
                let failure_reason = match &incoming { Err(reason) => Some(reason.clone()), Ok(ServerMessage::Fault { fault }) => Some(fault.message.clone()), _ => None };
                let failed = match incoming {
                    Ok(ServerMessage::Publication { publication }) => {
                        if let Some(received) = state.core.receive(generation, publication) {
                            if let Some(applied) = received.applied { notice(&mut state, &watches, received.observation, applied); }
                            if received.fault.is_some() { notice(&mut state, &watches, received.observation, Applied::Stale); }
                            if let Some(request) = received.recovery { interests.push_back(request); }
                            false
                        } else { false }
                    },
                    Ok(ServerMessage::Reply { reply }) => {
                        if let Some(reply) = state.core.reply(generation, reply) { if let Some(target) = calls.remove(&reply.id) { let _ = target.send(Ok(reply)); } } false
                    },
                    Ok(ServerMessage::ReadReply { reply }) => {
                        if let Some(reply) = state.core.read_reply(generation, reply) { if let Some(target) = reads.remove(&reply.id) { let _ = target.send(reply.result); } } false
                    },
                    _ => true,
                };
                if failed {
                    disconnect(&mut state, &watches, &mut calls, &mut reads); wire = None;
                    status.send_modify(|state| state.last_error = failure_reason.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{
        Query,
        invocation::{Invocation, Outcome},
        observation::{Content, Encoding, Handle, Member, Publication, Resume, ScopeId, Snapshot},
        schema::Schema,
        scoped,
    };
    use misa_protocol::{
        invocation::{CallContext, CommandOwner, Execution},
        owner::{Observation as OwnerObservation, Owner, Resolver},
    };
    use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

    struct Counter {
        scope: Scope,
        count: AtomicU64,
        revision: watch::Sender<u64>,
        active: Arc<AtomicUsize>,
        hold_result: AtomicBool,
    }
    impl Counter {
        fn new() -> Arc<Self> {
            let (revision, _) = watch::channel(0);
            Arc::new(Self {
                scope: Scope {
                    id: ScopeId::Daemon,
                    incarnation: "test-owner".into(),
                },
                count: AtomicU64::new(0),
                revision,
                active: Arc::new(AtomicUsize::new(0)),
                hold_result: AtomicBool::new(false),
            })
        }
        fn selection(&self) -> Selection {
            Selection {
                scope: self.scope.clone(),
                members: BTreeMap::from([(
                    "count".into(),
                    Member {
                        query: Query::new("counter"),
                        contract: "counter/1".into(),
                        encoding: Encoding::Value,
                        optional: false,
                    },
                )]),
            }
        }
        fn snapshot(&self) -> Snapshot {
            let position = self.count.load(Ordering::SeqCst);
            Snapshot {
                position,
                members: BTreeMap::from([(
                    "count".into(),
                    Content::Value(Value::Int(position as i64)),
                )]),
            }
        }
    }
    impl CommandOwner for Counter {
        fn scope(&self) -> Scope {
            self.scope.clone()
        }
        fn command(&self, _: &CallContext, id: &str) -> Option<Definition> {
            (id == "increment").then(|| Definition {
                preparation: Default::default(),
                id: id.into(),
                input: Schema::Int,
                result: Schema::Int,
            })
        }
        fn execute<'a>(&'a self, _: &'a CallContext, _: Invocation) -> Execution<'a> {
            Box::pin(async move {
                let value = self.count.fetch_add(1, Ordering::SeqCst) + 1;
                self.revision.send_replace(value);
                if self.hold_result.load(Ordering::SeqCst) {
                    std::future::pending::<()>().await;
                }
                Outcome::Completed {
                    value: Value::Int(value as i64),
                }
            })
        }
    }
    struct Observing {
        owner: Arc<Counter>,
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
            Some(Publication::Snapshot {
                handle: self.handle,
                snapshot: self.owner.snapshot(),
            })
        }
    }
    impl Owner for Counter {
        fn read(&self, _: &CallContext, selection: &Selection) -> Result<Snapshot, Fault> {
            if selection != &self.selection() {
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
            let changed = self.revision.subscribe();
            let snapshot = self.read(&context, &selection)?;
            self.active.fetch_add(1, Ordering::SeqCst);
            Ok((
                Box::new(Observing {
                    owner: self,
                    handle,
                    changed,
                }),
                Publication::Snapshot { handle, snapshot },
            ))
        }
    }
    struct Registry(Arc<Counter>);
    impl Resolver for Registry {
        fn resolve(&self, _: &CallContext, scope: &Scope) -> Result<Arc<dyn Owner>, Fault> {
            if scope != &self.0.scope {
                return Err(Fault::query("Unknown scope"));
            }
            Ok(self.0.clone())
        }
    }
    async fn fixture(
        wrong_identity: bool,
    ) -> (Endpoint, Endpoint, iroh::protocol::Router, Arc<Counter>) {
        let server = misa_transport::iroh::bind(None, false).await.unwrap();
        let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
        let counter = Counter::new();
        let handler = misa_transport::scoped_server::Handler::new(
            if wrong_identity {
                endpoint.id().to_string()
            } else {
                server.id().to_string()
            },
            counter.scope.clone(),
            Arc::new(Registry(counter.clone())),
            Arc::new(misa_transport::admission::Admission::open()),
        );
        let router = iroh::protocol::Router::builder(server.clone())
            .accept(scoped::ALPN, handler)
            .spawn();
        (server, endpoint, router, counter)
    }
    async fn address(endpoint: &Endpoint) -> EndpointAddr {
        misa_transport::iroh::address_of(&misa_transport::iroh::node_of(endpoint)).unwrap()
    }
    struct ReconnectingServer {
        first: misa_transport::scoped_server::Handler,
        second: misa_transport::scoped_server::Handler,
        accepted: AtomicUsize,
        cut: Arc<Notify>,
    }
    impl std::fmt::Debug for ReconnectingServer {
        fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            out.write_str("reconnect fixture")
        }
    }
    impl iroh::protocol::ProtocolHandler for ReconnectingServer {
        async fn accept(
            &self,
            connection: iroh::endpoint::Connection,
        ) -> Result<(), iroh::protocol::AcceptError> {
            if self.accepted.fetch_add(1, Ordering::SeqCst) == 0 {
                let socket = connection.clone();
                let cut = self.cut.clone();
                let task = tokio::spawn(async move {
                    cut.notified().await;
                    socket.close(1u32.into(), b"test link loss");
                });
                let result = self.first.accept(connection).await;
                task.abort();
                result
            } else {
                self.second.accept(connection).await
            }
        }
    }
    #[tokio::test]
    async fn reconnect_drains_more_interests_than_the_wire_queue_without_replaying_cancelled_leases()
     {
        let server = misa_transport::iroh::bind(None, false).await.unwrap();
        let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
        let owner = Counter::new();
        let handler = || {
            misa_transport::scoped_server::Handler::new(
                server.id().to_string(),
                owner.scope.clone(),
                Arc::new(Registry(owner.clone())),
                Arc::new(misa_transport::admission::Admission::open()),
            )
        };
        let cut = Arc::new(Notify::new());
        let router = iroh::protocol::Router::builder(server.clone())
            .accept(
                scoped::ALPN,
                ReconnectingServer {
                    first: handler(),
                    second: handler(),
                    accepted: AtomicUsize::new(0),
                    cut: cut.clone(),
                },
            )
            .spawn();
        let client = Client::connect(
            &endpoint,
            address(&server).await,
            ClientInfo::new("many-interests", "1"),
            Limits::default(),
        )
        .await
        .unwrap();
        let mut observations = Vec::new();
        for _ in 0..112 {
            let mut observation = client.observe(owner.selection(), None).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), observation.changed())
                .await
                .unwrap()
                .unwrap();
            assert!(
                observation
                    .inspect(|replica, _| replica.current().is_some())
                    .unwrap()
            );
            observations.push(observation);
        }
        let mut status = client.status();
        cut.notify_one();
        tokio::time::timeout(Duration::from_secs(8), async {
            while status.borrow().generation < 2 {
                status.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        observations.truncate(96);
        // Reads are still serviced (or immediately refused for queue pressure)
        // while restores and cancellations compete for control writer capacity.
        let _ = tokio::time::timeout(
            Duration::from_secs(1),
            client.read(owner.selection(), Duration::from_secs(3)),
        )
        .await
        .expect("Resume pumping must not block directed requests");
        tokio::time::timeout(Duration::from_secs(8), async {
            loop {
                if observations.iter().all(|observation| {
                    observation
                        .inspect(|replica, _| replica.current().is_some())
                        .unwrap()
                }) && owner.active.load(Ordering::SeqCst) == 96
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap_or_else(|error| {
            panic!(
                "{error}: active={}, current={}, status={:?}",
                owner.active.load(Ordering::SeqCst),
                observations
                    .iter()
                    .filter(|observation| observation
                        .inspect(|replica, _| replica.current().is_some())
                        .unwrap())
                    .count(),
                client.status().borrow().clone()
            )
        });
        assert_eq!(
            client.status().borrow().generation,
            2,
            "One reconnect must restore all interests without another socket churn"
        );
        let read = client
            .read(owner.selection(), Duration::from_secs(3))
            .await
            .unwrap();
        assert_eq!(read.position(), 0);
        client.disconnect().await.unwrap();
        assert!(matches!(
            client.status().borrow().phase,
            Phase::Disconnected
        ));
        router.shutdown().await.unwrap();
        endpoint.close().await;
    }

    #[tokio::test]
    async fn automatic_reconnect_retains_interest_and_reports_new_owner_incarnations() {
        for new_incarnation in [false, true] {
            let server = misa_transport::iroh::bind(None, false).await.unwrap();
            let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
            let first = Counter::new();
            let second = if new_incarnation {
                let mut next = Counter::new();
                Arc::get_mut(&mut next).unwrap().scope.incarnation = "owner-restarted".into();
                next
            } else {
                first.clone()
            };
            let handler = |owner: Arc<Counter>| {
                misa_transport::scoped_server::Handler::new(
                    server.id().to_string(),
                    owner.scope.clone(),
                    Arc::new(Registry(owner)),
                    Arc::new(misa_transport::admission::Admission::open()),
                )
            };
            let cut = Arc::new(Notify::new());
            let router = iroh::protocol::Router::builder(server.clone())
                .accept(
                    scoped::ALPN,
                    ReconnectingServer {
                        first: handler(first.clone()),
                        second: handler(second.clone()),
                        accepted: AtomicUsize::new(0),
                        cut: cut.clone(),
                    },
                )
                .spawn();
            let client = Client::connect(
                &endpoint,
                address(&server).await,
                ClientInfo::new("reconnect", "1"),
                Limits::default(),
            )
            .await
            .unwrap();
            let mut observer = client.observe(first.selection(), None).await.unwrap();
            tokio::time::timeout(Duration::from_secs(5), observer.changed())
                .await
                .unwrap()
                .unwrap();
            let definition = Definition {
                preparation: Default::default(),
                id: "increment".into(),
                input: Schema::Int,
                result: Schema::Int,
            };
            first.hold_result.store(true, Ordering::SeqCst);
            let invoking_client = client.clone();
            let invoking_scope = first.scope.clone();
            let pending = tokio::spawn(async move {
                invoking_client
                    .invoke(
                        invoking_scope,
                        definition,
                        Value::Int(1),
                        Duration::from_secs(5),
                    )
                    .await
                    .unwrap()
            });
            tokio::time::timeout(Duration::from_secs(5), async {
                while first.count.load(Ordering::SeqCst) == 0 {
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .unwrap();
            let mut status = client.status();
            cut.notify_one();
            let outcome = tokio::time::timeout(Duration::from_secs(5), pending)
                .await
                .unwrap()
                .unwrap();
            assert!(matches!(outcome.outcome, Outcome::Indeterminate { .. }));
            tokio::time::timeout(Duration::from_secs(8), async {
                while status.borrow().generation < 2
                    || !matches!(status.borrow().phase, Phase::Online)
                {
                    status.changed().await.unwrap();
                }
            })
            .await
            .unwrap();
            assert_eq!(client.welcome().scope, second.scope);
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let ready = observer
                        .inspect(|replica, _| {
                            if new_incarnation {
                                matches!(
                                    replica.status(),
                                    misa_protocol::observation::Status::Closed(_)
                                )
                            } else {
                                replica.current().is_some() && replica.position() == Some(1)
                            }
                        })
                        .unwrap();
                    if ready {
                        break;
                    }
                    observer.changed().await.unwrap();
                }
            })
            .await
            .unwrap();
            assert_eq!(first.count.load(Ordering::SeqCst), 1);
            let expected = if new_incarnation { 0 } else { 1 };
            assert_eq!(
                client
                    .read(second.selection(), Duration::from_secs(5))
                    .await
                    .unwrap()
                    .position(),
                expected
            );
            client.disconnect().await.unwrap();
            assert!(matches!(
                client.status().borrow().phase,
                Phase::Disconnected
            ));
            tokio::time::sleep(Duration::from_millis(250)).await;
            assert!(!client.online());
            drop(observer);
            drop(client);
            router.shutdown().await.unwrap();
            endpoint.close().await;
            server.close().await;
        }
    }
    #[tokio::test]
    async fn slow_observer_does_not_block_directed_results_or_replica_application() {
        let (server, endpoint, router, counter) = fixture(false).await;
        let client = Client::connect(
            &endpoint,
            address(&server).await,
            ClientInfo::new("test", "1"),
            Limits::default(),
        )
        .await
        .unwrap();
        assert_eq!(client.welcome().daemon, server.id().to_string());
        let mut observer = client.observe(counter.selection(), None).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), observer.changed())
            .await
            .unwrap()
            .unwrap();
        let context = CallContext {
            principal: "test".into(),
            connection: 1,
        };
        let definition = counter.command(&context, "increment").unwrap();
        // Intentionally do not poll the observation notification for 100 updates.
        for expected in 1..=100 {
            let reply = client
                .invoke(
                    counter.scope.clone(),
                    definition.clone(),
                    Value::Int(1),
                    Duration::from_secs(5),
                )
                .await
                .unwrap();
            assert!(
                matches!(reply.outcome, Outcome::Completed { value: Value::Int(value) } if value == expected)
            );
        }
        let result = client
            .read(counter.selection(), Duration::from_secs(5))
            .await
            .unwrap();
        assert_eq!(result.position(), 100);
        tokio::time::timeout(Duration::from_secs(5), async {
            while observer.inspect(|replica, _| replica.position()).flatten() != Some(100) {
                observer.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert!(
            observer
                .inspect(|_, notice| notice.unwrap().sequence)
                .unwrap()
                > 1
        );
        assert_eq!(counter.active.load(Ordering::SeqCst), 1);
        drop(observer);
        tokio::time::timeout(Duration::from_secs(5), async {
            while counter.active.load(Ordering::SeqCst) != 0 {
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            client
                .read(counter.selection(), Duration::from_secs(5))
                .await
                .is_ok()
        );
        drop(client);
        router.shutdown().await.unwrap();
        endpoint.close().await;
        server.close().await;
    }
    #[tokio::test]
    async fn greeting_cannot_substitute_another_daemon_identity() {
        let (server, endpoint, router, _) = fixture(true).await;
        let result = Client::connect(
            &endpoint,
            address(&server).await,
            ClientInfo::new("test", "1"),
            Limits::default(),
        )
        .await;
        assert!(result.is_err());
        router.shutdown().await.unwrap();
        endpoint.close().await;
        server.close().await;
    }
}
