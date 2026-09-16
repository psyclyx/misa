//! Scoped daemon protocol, with bounded tasks/queues and injected domain owners.
//! Ordered observation lanes and finite-read lanes are independent of control
//! replies. Busy observers coalesce owner changes instead of queueing snapshots.
use crate::{
    admission::Admission,
    scoped_io::{Reader, Writer},
};
use iroh::{
    endpoint::Connection,
    protocol::{AcceptError, ProtocolHandler},
};
use misa_proto::{
    Fault,
    observation::{Handle, Publication, Scope},
    schema::Limits,
    scoped::{ClientMessage, Lane, ServerMessage, VERSION},
};
use misa_protocol::{
    invocation::CallContext,
    owner::{Resolver, Router},
};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
};
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinSet,
};

const QUEUE: usize = 8;
const OBSERVATIONS: usize = misa_proto::scoped::DEFAULT_OBSERVATION_LIMIT;
const INVOCATIONS: usize = 8;
const REQUEST_BYTES: usize = 4 * 1024 * 1024;
const RESPONSE_BYTES: usize = 64 * 1024 * 1024;

struct Presence { resolver: Arc<dyn Resolver>, context: CallContext }
impl Drop for Presence {
    fn drop(&mut self) { self.resolver.disconnected(&self.context); }
}

pub struct Handler {
    pub daemon: String,
    pub scope: Scope,
    pub resolver: Arc<dyn Resolver>,
    pub admission: Arc<Admission>,
}
impl std::fmt::Debug for Handler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("scoped daemon handler")
    }
}
impl ProtocolHandler for Handler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let principal = connection.remote_id().to_string();
        if !self.admission.admits(&principal) {
            return Ok(());
        }
        let mut admission = self.admission.watch();
        let (send, recv) = connection.accept_bi().await?;
        let mut reader = Reader::new(recv, REQUEST_BYTES);
        let mut writer =
            Writer::new(send, REQUEST_BYTES).with_budget(Arc::new(Semaphore::new(8 * 1024 * 1024)));
        let hello = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            reader.next::<ClientMessage>(),
        )
        .await;
        let Ok(Ok(Some(hello @ ClientMessage::Hello { .. }))) = hello else {
            return Ok(());
        };
        if let Err(fault) = hello.validate() {
            let _ = writer.send(&ServerMessage::Fault { fault }).await;
            return Ok(());
        }
        if !self.admission.admits(&principal) {
            return Ok(());
        }
        let context = CallContext {
            principal: principal.clone(),
            connection: misa_proto::wire::RequestContext::connection(),
        };
        let ClientMessage::Hello { client, .. } = &hello else { unreachable!() };
        if let Err(fault) = self.resolver.connected(&context, client) {
            let _ = writer.send(&ServerMessage::Fault { fault }).await;
            return Ok(());
        }
        let _presence = Presence { resolver: self.resolver.clone(), context: context.clone() };
        if writer
            .send(&ServerMessage::Welcome {
                version: VERSION,
                daemon: self.daemon.clone(),
                scope: self.scope.clone(),
            })
            .await
            .is_err()
        {
            return Ok(());
        }
        let mut router = Router::new(
            context,
            self.resolver.clone(),
            OBSERVATIONS,
            INVOCATIONS,
            Limits {
                bytes: REQUEST_BYTES,
                ..Limits::default()
            },
            Limits {
                bytes: REQUEST_BYTES,
                nodes: 1_000_000,
                ..Limits::default()
            },
        );
        let (input_tx, mut input) = mpsc::channel(QUEUE);
        let (output, mut output_rx) = mpsc::channel::<ServerMessage>(1);
        let mut io = JoinSet::new();
        io.spawn(async move {
            loop {
                let message = match reader.next::<ClientMessage>().await {
                    Ok(Some(message)) => message,
                    _ => return,
                };
                if input_tx.send(message).await.is_err() {
                    return;
                }
            }
        });
        io.spawn(async move {
            while let Some(message) = output_rx.recv().await {
                if writer.send(&message).await.is_err() {
                    return;
                }
            }
        });
        let data_budget = Arc::new(Semaphore::new(128 * 1024 * 1024));
        let (ready_tx, mut ready) = mpsc::channel::<(Handle, bool)>(OBSERVATIONS * 2);
        let mut lanes = BTreeMap::<u64, OutputLane>::new();
        let mut watch_tasks = JoinSet::new();
        let mut lane_tasks = JoinSet::<(Handle, Option<String>)>::new();
        let mut read_tasks = JoinSet::<(u64, Result<(), String>)>::new();
        let mut calls = JoinSet::new();
        let mut pending = VecDeque::new();
        loop {
            if !self.admission.admits(&principal) {
                break;
            }
            tokio::select! {
                _ = admission.changed() => { if !self.admission.admits(&principal) { break; } }
                _ = io.join_next() => break,
                _ = watch_tasks.join_next(), if !watch_tasks.is_empty() => {}
                completed = lane_tasks.join_next(), if !lane_tasks.is_empty() && pending.len() < QUEUE => {
                    if let Some(Ok((handle, error))) = completed {
                        if lanes.get(&handle.id).is_some_and(|lane| lane.handle == handle) {
                            lanes.remove(&handle.id);
                            router.cancel(handle);
                            if error.is_some() { pending.push_back(ServerMessage::Publication { publication: Publication::Closed { handle, reason: Fault::new("lane_failed", "Observation output lane failed") } }); }
                        }
                    }
                }
                completed = read_tasks.join_next(), if !read_tasks.is_empty() && pending.len() < QUEUE => {
                    if let Some(Ok((id, Err(_)))) = completed { pending.push_back(ServerMessage::ReadReply { reply: misa_proto::scoped::ReadReply { id, outcome: misa_proto::scoped::ReadOutcome::Rejected { fault: Fault::new("lane_failed", "Finite read output lane failed") } } }); }
                }
                permit = output.reserve(), if !pending.is_empty() => {
                    let Ok(permit) = permit else { break };
                    permit.send(pending.pop_front().expect("pending output"));
                }
                result = calls.join_next(), if !calls.is_empty() && pending.len() < QUEUE => {
                    if let Some(Ok(reply)) = result { pending.push_back(ServerMessage::Reply { reply }); }
                }
                message = input.recv(), if pending.len() < QUEUE && calls.len() < INVOCATIONS && watch_tasks.len() < OBSERVATIONS * 2 && lane_tasks.len() < OBSERVATIONS * 2 => {
                    let Some(message) = message else { break };
                    if let Err(fault) = message.validate() { pending.push_back(ServerMessage::Fault { fault }); continue; }
                    match message {
                        ClientMessage::Hello { .. } => pending.push_back(ServerMessage::Fault { fault: Fault::protocol("Daemon greeting was already exchanged") }),
                        ClientMessage::Read { read } => {
                            if read_tasks.len() >= INVOCATIONS {
                                pending.push_back(ServerMessage::ReadReply { reply: misa_proto::scoped::ReadReply { id: read.id, outcome: misa_proto::scoped::ReadOutcome::Rejected { fault: Fault::new("busy", "Too many finite read lanes") } } });
                            } else {
                                let reply = router.read(read);
                                let connection = connection.clone(); let budget = data_budget.clone();
                                read_tasks.spawn(async move {
                                    let id = reply.id;
                                    let result = async {
                                        let send = connection.open_uni().await.map_err(|e| e.to_string())?;
                                        let mut writer = Writer::new(send, RESPONSE_BYTES).with_budget(budget);
                                        writer.send(&Lane::Read { id }).await?;
                                        writer.send(&ServerMessage::ReadReply { reply }).await?;
                                        writer.finish()
                                    }.await;
                                    (id, result)
                                });
                            }
                        }
                        ClientMessage::Invoke { invocation } => { calls.spawn(router.invoke(invocation)); }
                        ClientMessage::CancelObservation { handle } => {
                            if router.cancel(handle) { lanes.remove(&handle.id); }
                        }
                        ClientMessage::Observe { handle, selection, resume } => {
                            let publication = match router.observe(handle, selection, resume) {
                                Ok(publication) => publication,
                                Err(reason) => Publication::Closed { handle, reason },
                            };
                            if lanes.get(&handle.id).is_some_and(|old| router.watch(old.handle).is_none()) { lanes.remove(&handle.id); }
                            if let Some(mut changed) = router.watch(handle) {
                                let notices = ready_tx.clone();
                                let watch = watch_tasks.spawn(async move {
                                    loop {
                                        if changed.changed().await.is_err() { let _ = notices.send((handle, false)).await; return; }
                                        if notices.send((handle, false)).await.is_err() { return; }
                                    }
                                });
                                let (send, receive) = mpsc::channel(1);
                                let connection = connection.clone(); let budget = data_budget.clone(); let notices = ready_tx.clone();
                                let task = lane_tasks.spawn(async move {
                                    let result = write_observation(connection, handle, publication, receive, notices, budget).await;
                                    (handle, result.err())
                                });
                                lanes.insert(handle.id, OutputLane { handle, send, busy: true, dirty: false, task, watch });
                            } else { pending.push_back(ServerMessage::Publication { publication }); }
                        }
                    }
                }
                notice = ready.recv() => {
                    if let Some((handle, writable)) = notice {
                        if let Some(lane) = lanes.get_mut(&handle.id).filter(|lane| lane.handle == handle) {
                            if writable { lane.busy = false; } else { lane.dirty = true; }
                            if !lane.busy && lane.dirty {
                                lane.dirty = false;
                                if let Some(publication) = router.poll(handle) {
                                    lane.busy = true;
                                    if lane.send.try_send(publication).is_err() {
                                        router.cancel(handle); lanes.remove(&handle.id);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        connection.close(0u32.into(), b"scoped connection closed");
        Ok(())
    }
}

struct OutputLane {
    handle: Handle,
    send: mpsc::Sender<Publication>,
    busy: bool,
    dirty: bool,
    task: tokio::task::AbortHandle,
    watch: tokio::task::AbortHandle,
}
impl Drop for OutputLane {
    fn drop(&mut self) {
        self.task.abort();
        self.watch.abort();
    }
}
async fn write_observation(
    connection: Connection,
    handle: Handle,
    initial: Publication,
    mut updates: mpsc::Receiver<Publication>,
    ready: mpsc::Sender<(Handle, bool)>,
    budget: Arc<Semaphore>,
) -> Result<(), String> {
    let send = connection
        .open_uni()
        .await
        .map_err(|error| error.to_string())?;
    let mut writer = Writer::new(send, RESPONSE_BYTES).with_budget(budget);
    writer.send(&Lane::Observation { handle }).await?;
    let mut next = initial;
    loop {
        let closed = matches!(next, Publication::Closed { .. });
        writer
            .send(&ServerMessage::Publication { publication: next })
            .await?;
        if closed {
            return writer.finish();
        }
        ready
            .send((handle, true))
            .await
            .map_err(|_| "Connection closed")?;
        next = updates.recv().await.ok_or("Observation cancelled")?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{
        ClientInfo, Query,
        invocation::{Invocation, Outcome},
        observation::{Encoding, Member, ScopeId, Selection},
        scoped::{Read, ReadOutcome},
    };
    use misa_protocol::owner::Owner;
    use misa_value::Value;
    struct Registry(Vec<Arc<misa_session::Runtime>>);
    impl Resolver for Registry {
        fn resolve(&self, context: &CallContext, scope: &Scope) -> Result<Arc<dyn Owner>, Fault> {
            assert!(!context.principal.is_empty());
            self.0
                .iter()
                .find(|runtime| runtime.scope() == *scope)
                .cloned()
                .map(|owner| owner as Arc<dyn Owner>)
                .ok_or_else(|| Fault::new("scope_unavailable", "Scope unavailable"))
        }
    }
    fn runtime(id: &str) -> Arc<misa_session::Runtime> {
        misa_session::Runtime::start(
            id,
            id,
            None,
            Arc::new(misa_kernel::LocalKernel::new(
                misa_kernel::ScriptedProvider::new([misa_kernel::Turn::say("done")]),
            )),
            "scripted",
            "scripted-1",
            Value::Null,
        )
    }
    fn selection(runtime: &misa_session::Runtime) -> Selection {
        Selection {
            scope: runtime.scope(),
            members: BTreeMap::from([(
                "summary".into(),
                Member {
                    query: Query::new("session.summary"),
                    contract: "session.summary@1".into(),
                    encoding: Encoding::Value,
                    optional: false,
                },
            )]),
        }
    }
    async fn next(client: &mut crate::scoped_client::Client) -> ServerMessage {
        match tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
            .await
            .unwrap()
            .unwrap()
        {
            crate::scoped_client::Event::Message(frame) => frame.message,
            crate::scoped_client::Event::LaneClosed { reason, .. } => {
                panic!("unexpected lane failure: {reason}")
            }
        }
    }
    #[tokio::test]
    async fn real_connection_reads_observes_and_invokes_multiple_session_scopes() {
        let server = crate::iroh::bind(None, false).await.unwrap();
        let endpoint = crate::iroh::bind(None, false).await.unwrap();
        let first = runtime("one");
        let second = runtime("two");
        let daemon_scope = Scope {
            id: ScopeId::Daemon,
            incarnation: "daemon-run".into(),
        };
        let router = iroh::protocol::Router::builder(server.clone())
            .accept(
                misa_proto::scoped::ALPN,
                Handler {
                    daemon: server.id().to_string(),
                    scope: daemon_scope.clone(),
                    resolver: Arc::new(Registry(vec![first.clone(), second.clone()])),
                    admission: Arc::new(Admission::open()),
                },
            )
            .spawn();
        let address = crate::iroh::address_of(&crate::iroh::node_of(&server)).unwrap();
        let mut client =
            crate::scoped_client::Client::connect(&endpoint, address, ClientInfo::new("test", "1"))
                .await
                .unwrap();
        assert_eq!(client.welcome.scope, daemon_scope);
        client
            .try_send(ClientMessage::Read {
                read: Read {
                    id: 1,
                    selection: selection(&first),
                },
            })
            .unwrap();
        assert!(matches!(
            next(&mut client).await,
            ServerMessage::ReadReply {
                reply: misa_proto::scoped::ReadReply {
                    id: 1,
                    outcome: ReadOutcome::Snapshot { .. }
                }
            }
        ));
        for (id, runtime) in [(2, &first), (3, &second)] {
            client
                .try_send(ClientMessage::Observe {
                    handle: Handle { id, generation: 1 },
                    selection: selection(runtime),
                    resume: None,
                })
                .unwrap();
            assert!(
                matches!(next(&mut client).await, ServerMessage::Publication { publication: Publication::Snapshot { handle, .. } } if handle.id == id)
            );
        }
        client
            .try_send(ClientMessage::Invoke {
                invocation: Invocation {
                    id: 4,
                    scope: first.scope(),
                    command: "session.prompt".into(),
                    input: Value::map([("text", Value::str("hello"))]),
                },
            })
            .unwrap();
        let mut replied = false;
        let mut changed = false;
        for _ in 0..12 {
            match next(&mut client).await {
                ServerMessage::Reply { reply } if reply.id == 4 => {
                    assert!(matches!(
                        reply.outcome,
                        Outcome::Completed { .. } | Outcome::Accepted { .. }
                    ));
                    replied = true;
                }
                ServerMessage::Publication { publication } if publication.handle().id == 2 => {
                    changed = true
                }
                _ => {}
            }
            if replied && changed {
                break;
            }
        }
        assert!(replied && changed);
        drop(client);
        router.shutdown().await.unwrap();
        endpoint.close().await;
    }
    #[tokio::test]
    async fn revocation_wakes_an_idle_scoped_connection() {
        let server = crate::iroh::bind(None, false).await.unwrap();
        let endpoint = crate::iroh::bind(None, false).await.unwrap();
        let admission = Arc::new(Admission::paired(crate::admission::Paired::in_memory()));
        admission
            .admit_local(&endpoint.id().to_string(), 0)
            .unwrap();
        let router = iroh::protocol::Router::builder(server.clone())
            .accept(
                misa_proto::scoped::ALPN,
                Handler {
                    daemon: server.id().to_string(),
                    scope: Scope {
                        id: ScopeId::Daemon,
                        incarnation: "run".into(),
                    },
                    resolver: Arc::new(Registry(vec![])),
                    admission: admission.clone(),
                },
            )
            .spawn();
        let address = crate::iroh::address_of(&crate::iroh::node_of(&server)).unwrap();
        let mut client =
            crate::scoped_client::Client::connect(&endpoint, address, ClientInfo::new("test", "1"))
                .await
                .unwrap();
        assert!(admission.revoke(&endpoint.id().to_string()).unwrap());
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(5), client.next())
                .await
                .unwrap()
                .is_err()
        );
        drop(client);
        router.shutdown().await.unwrap();
        endpoint.close().await;
    }
    #[tokio::test]
    async fn unread_large_observation_does_not_block_commands_or_another_observer() {
        use misa_proto::invocation::Command;
        use misa_proto::observation::{Content, Resume, Snapshot};
        use misa_protocol::invocation::{CommandOwner, Execution};
        struct Mixed(Arc<misa_session::Runtime>);
        impl CommandOwner for Mixed {
            fn scope(&self) -> Scope {
                self.0.scope()
            }
            fn command(&self, context: &CallContext, id: &str) -> Option<Command> {
                self.0.command(context, id)
            }
            fn execute<'a>(
                &'a self,
                context: &'a CallContext,
                invocation: Invocation,
            ) -> Execution<'a> {
                self.0.execute(context, invocation)
            }
        }
        struct Dormant(
            tokio::sync::watch::Sender<u64>,
            tokio::sync::watch::Receiver<u64>,
        );
        impl misa_protocol::owner::Observation for Dormant {
            fn changed(&mut self) -> &mut tokio::sync::watch::Receiver<u64> {
                &mut self.1
            }
            fn poll(&mut self) -> Option<Publication> {
                let _ = self.0.receiver_count();
                None
            }
        }
        impl Owner for Mixed {
            fn read(
                &self,
                context: &CallContext,
                selection: &Selection,
            ) -> Result<Snapshot, Fault> {
                Owner::read(self.0.as_ref(), context, selection)
            }
            fn observe(
                self: Arc<Self>,
                context: CallContext,
                handle: Handle,
                selection: Selection,
                resume: Option<Resume>,
            ) -> Result<(Box<dyn misa_protocol::owner::Observation>, Publication), Fault>
            {
                if selection.members.contains_key("large") {
                    let (tx, rx) = tokio::sync::watch::channel(0);
                    Ok((
                        Box::new(Dormant(tx, rx)),
                        Publication::Snapshot {
                            handle,
                            snapshot: Snapshot {
                                position: 0,
                                members: BTreeMap::from([(
                                    "large".into(),
                                    Content::Value(Value::str("x".repeat(2 * 1024 * 1024))),
                                )]),
                            },
                        },
                    ))
                } else {
                    Owner::observe(self.0.clone(), context, handle, selection, resume)
                }
            }
        }
        struct RegistryOne(Arc<Mixed>);
        impl Resolver for RegistryOne {
            fn resolve(&self, _: &CallContext, scope: &Scope) -> Result<Arc<dyn Owner>, Fault> {
                if *scope == self.0.scope() {
                    Ok(self.0.clone())
                } else {
                    Err(Fault::query("missing scope"))
                }
            }
        }
        let server = crate::iroh::bind(None, false).await.unwrap();
        // A 4 KiB stream window proves the unread 2 MiB lane cannot finish;
        // a larger connection window leaves capacity for independent streams.
        let endpoint = iroh::Endpoint::builder(iroh::endpoint::presets::N0)
            .relay_mode(iroh::RelayMode::Disabled)
            .transport_config(
                iroh::endpoint::QuicTransportConfig::builder()
                    .stream_receive_window(4096u32.into())
                    .receive_window((1024 * 1024u32).into())
                    .build(),
            )
            .bind()
            .await
            .unwrap();
        let runtime = runtime("fair");
        let router = iroh::protocol::Router::builder(server.clone())
            .accept(
                misa_proto::scoped::ALPN,
                Handler {
                    daemon: server.id().to_string(),
                    scope: Scope {
                        id: ScopeId::Daemon,
                        incarnation: "run".into(),
                    },
                    resolver: Arc::new(RegistryOne(Arc::new(Mixed(runtime.clone())))),
                    admission: Arc::new(Admission::open()),
                },
            )
            .spawn();
        let address = crate::iroh::address_of(&crate::iroh::node_of(&server)).unwrap();
        let connection = endpoint
            .connect(address, misa_proto::scoped::ALPN)
            .await
            .unwrap();
        let (send, recv) = connection.open_bi().await.unwrap();
        let mut control = Writer::new(send, REQUEST_BYTES);
        let mut replies = Reader::new(recv, RESPONSE_BYTES);
        control
            .send(&ClientMessage::Hello {
                version: VERSION,
                client: ClientInfo::new("test", "1"),
            })
            .await
            .unwrap();
        assert!(matches!(
            replies.next::<ServerMessage>().await.unwrap(),
            Some(ServerMessage::Welcome { .. })
        ));
        let large = Selection {
            scope: runtime.scope(),
            members: BTreeMap::from([(
                "large".into(),
                Member {
                    query: Query::new("large"),
                    contract: "large@1".into(),
                    encoding: Encoding::Value,
                    optional: false,
                },
            )]),
        };
        control
            .send(&ClientMessage::Observe {
                handle: Handle {
                    id: 1,
                    generation: 1,
                },
                selection: large,
                resume: None,
            })
            .await
            .unwrap();
        let recv = tokio::time::timeout(std::time::Duration::from_secs(5), connection.accept_uni())
            .await
            .unwrap()
            .unwrap();
        let mut blocked = Reader::new(recv, RESPONSE_BYTES);
        assert_eq!(
            blocked.next::<Lane>().await.unwrap(),
            Some(Lane::Observation {
                handle: Handle {
                    id: 1,
                    generation: 1
                }
            })
        );
        // Keep `blocked` alive without reading its publication.
        control
            .send(&ClientMessage::Observe {
                handle: Handle {
                    id: 2,
                    generation: 1,
                },
                selection: selection(&runtime),
                resume: None,
            })
            .await
            .unwrap();
        let recv = tokio::time::timeout(std::time::Duration::from_secs(5), connection.accept_uni())
            .await
            .unwrap()
            .unwrap();
        let mut small = Reader::new(recv, RESPONSE_BYTES);
        assert_eq!(
            small.next::<Lane>().await.unwrap(),
            Some(Lane::Observation {
                handle: Handle {
                    id: 2,
                    generation: 1
                }
            })
        );
        assert!(matches!(
            small.next::<ServerMessage>().await.unwrap(),
            Some(ServerMessage::Publication {
                publication: Publication::Snapshot { .. }
            })
        ));
        control
            .send(&ClientMessage::Invoke {
                invocation: Invocation {
                    id: 3,
                    scope: runtime.scope(),
                    command: "session.prompt".into(),
                    input: Value::map([("text", Value::str("hello"))]),
                },
            })
            .await
            .unwrap();
        let reply = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            replies.next::<ServerMessage>(),
        )
        .await
        .unwrap()
        .unwrap()
        .unwrap();
        assert!(matches!(reply, ServerMessage::Reply { reply } if reply.id == 3));
        assert!(matches!(
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                small.next::<ServerMessage>()
            )
            .await
            .unwrap()
            .unwrap(),
            Some(ServerMessage::Publication { .. })
        ));
        drop(blocked);
        connection.close(0u32.into(), b"done");
        router.shutdown().await.unwrap();
        endpoint.close().await;
    }
}
