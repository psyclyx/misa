//! iroh: the endpoint, the server, and the client.
//!
//! This is the only file in the workspace that knows iroh exists. Everything above
//! it speaks [`ClientMsg`] and [`SessionMsg`]; everything below it is a byte
//! stream. Keeping the confinement is what let the protocol state machine be tested
//! without a socket, and it is what would let a different transport be dropped in
//! without touching a session.
//!
//! # What a connection is
//!
//! One bidirectional QUIC stream. Client messages go up it, session messages come
//! back. There is no second channel to keep in step, and no reconnection protocol:
//! a client that reconnects subscribes again and the revision it gets is the truth.
//! [`Client`] does exactly that when a connection drops, which is why nothing above
//! this file has to know that connections can drop.
//!
//! # Identity
//!
//! iroh authenticates the peer's public key, so a client is identified before it
//! says anything, and a session's ticket names the endpoint that serves it. What a
//! daemon *admits* is policy and does not live here; what lives here is that the
//! identity is available to that policy.

use std::collections::HashMap;
use std::sync::Arc;

use iroh::endpoint::{Connection, RecvStream, SendStream};
use iroh::protocol::{AcceptError, ProtocolHandler};
use iroh::{Endpoint, EndpointAddr, RelayMode, SecretKey, endpoint::presets};
use misa_proto::chunk::Decoder;
use misa_proto::wire::{ClientInfo, ClientMsg, Query, SessionMsg, SubId};
use misa_proto::{ALPN_BLOB, ALPN_SESSION, Fault, PROTOCOL_VERSION, Ticket};
use misa_session::Runtime;
use crate::admission::Admission;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use crate::{Session, encode};
/// Bind an endpoint.
///
/// `relay` off means the endpoint uses only addresses it can reach directly, which
/// is what a test wants and what a machine on one network wants. Real use leaves it
/// on: the relay is what makes two endpoints behind different routers meet.
pub async fn bind(secret: Option<SecretKey>, relay: bool) -> Result<Endpoint, String> {
    let mut builder = Endpoint::builder(presets::N0);
    if let Some(secret) = secret {
        builder = builder.secret_key(secret);
    }
    if !relay {
        // Direct addresses only. What a test wants, and what a machine on one
        // network wants; real use leaves the relay on, because that is what makes
        // two endpoints behind different routers meet.
        builder = builder.relay_mode(RelayMode::Disabled);
    }
    builder
        // Both, because one endpoint serves a session and that session's bytes: a ticket
        // names one node, and a client that can reach a session can fetch what its views
        // point at without being told a second address.
        .alpns(vec![ALPN_SESSION.to_vec(), ALPN_BLOB.to_vec()])
        .bind()
        .await
        .map_err(|err| err.to_string())
}

/// The endpoint's identity, and — when there is no relay — where it listens.
///
/// With a relay, the public key alone is enough: discovery finds the peer, so a ticket stays
/// short and does not go stale when a machine's addresses change. Without one there is
/// nothing to ask, so the ticket has to carry the addresses, and it does, as
/// `id@ip:port,ip:port`.
pub fn node_of(endpoint: &Endpoint) -> String {
    let id = endpoint.id().to_string();
    let mut sockets: Vec<String> = endpoint
        .bound_sockets()
        .iter()
        // A wildcard bind is not an address anybody can dial. It is *reachable* at loopback
        // on the same machine, which is what somebody running a daemon and a client side by
        // side means by it, so that is what the ticket says. Advertising the machine's other
        // names is what the relay is for, and with a relay this branch does not run.
        .map(|socket| match socket {
            std::net::SocketAddr::V4(v4) if v4.ip().is_unspecified() => format!("127.0.0.1:{}", v4.port()),
            std::net::SocketAddr::V6(v6) if v6.ip().is_unspecified() => String::new(),
            other => other.to_string(),
        })
        .filter(|socket| !socket.is_empty())
        .collect();
    sockets.sort();
    sockets.dedup();
    if sockets.is_empty() {
        id
    } else {
        format!("{id}@{}", sockets.join(","))
    }
}

/// Whether a ticket's node names an endpoint only this machine can reach.
///
/// A loopback address needs no help to dial, so a client that has one should not wait for a
/// relay: on a machine with no route out — which is exactly the machine that runs a daemon
/// `--no-relay` beside a client — that wait is a timeout, and a daemon a client can see would
/// be unreachable for the sake of a service nobody needed. A node with no addresses is the
/// other case: a public key alone is only dialable through discovery, which is the relay.
pub fn names_only_this_machine(node: &str) -> bool {
    let Some((_, addresses)) = node.split_once('@') else {
        return false;
    };
    let sockets: Vec<&str> = addresses.split(',').filter(|socket| !socket.is_empty()).collect();
    !sockets.is_empty()
        && sockets.iter().all(|socket| {
            socket
                .parse::<std::net::SocketAddr>()
                .map(|socket| socket.ip().is_loopback())
                .unwrap_or(false)
        })
}

/// Bind an endpoint for reaching a ticket's node, with a relay only when one is needed.
///
/// The choice a client should not have to make: an endpoint that is reachable directly is
/// reached directly, and one that is not is reached through the relay. `--no-relay` on a
/// daemon is therefore not a flag every client has to be told about.
pub async fn bind_for(node: &str) -> Result<Endpoint, String> {
    bind(None, !names_only_this_machine(node)).await
}

/// Parse a ticket's node part back into something dialable.
pub fn address_of(node: &str) -> Result<EndpointAddr, String> {
    let (id, addresses) = match node.split_once('@') {
        Some((id, addresses)) => (id, Some(addresses)),
        None => (node, None),
    };
    let id: iroh::EndpointId = id.parse().map_err(|err| format!("bad endpoint id: {err}"))?;
    let mut address = EndpointAddr::new(id);
    if let Some(addresses) = addresses {
        for socket in addresses.split(',').filter(|socket| !socket.is_empty()) {
            let socket = socket
                .parse()
                .map_err(|err| format!("`{socket}` is not an address: {err}"))?;
            address = address.with_ip_addr(socket);
        }
    }
    Ok(address)
}

/// The sessions one endpoint serves.

#[derive(Default)]
pub struct Sessions {
    open: std::sync::Mutex<HashMap<String, Arc<Runtime>>>,
}

impl Sessions {
    pub fn new() -> Arc<Sessions> {
        Arc::new(Sessions::default())
    }

    pub fn insert(&self, runtime: Arc<Runtime>) {
        self.open
            .lock()
            .expect("the session table is never poisoned")
            .insert(runtime.id().to_string(), runtime);
    }

    pub fn get(&self, id: &str) -> Option<Arc<Runtime>> {
        self.open.lock().expect("the session table is never poisoned").get(id).cloned()
    }

    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self
            .open
            .lock()
            .expect("the session table is never poisoned")
            .keys()
            .cloned()
            .collect();
        ids.sort();
        ids
    }
}

/// Serves one connection per session id.
///
/// The client names its session in the first message rather than in the ALPN,
/// because a session is a thing that can be created after an endpoint is listening
/// and an ALPN is fixed at bind time. Serving is [`crate::server::serve`], which mounts
/// this beside the blob handler; a router with only one of them is a daemon whose tickets
/// point at a node that answers for half of what a session needs.
pub struct Handler {
    pub sessions: Arc<Sessions>,
    /// Who may attach. The decision is the middle layer's; carrying it out is this layer's,
    /// which is why the admission rules are handed in rather than built here.
    ///
    /// The one thing this layer adds is the *door*: a peer that is not admitted yet may send
    /// one message — a pairing code — while a code is showing, and nothing else. What a code
    /// means, how long it lasts, and which key it approves are all the middle layer's.
    pub admission: Arc<Admission>,
}

impl std::fmt::Debug for Handler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `ProtocolHandler` requires `Debug`; a handle to a session table has nothing
        // useful to print, and printing a key would be worse than nothing.
        f.write_str("misa session handler")
    }
}

impl ProtocolHandler for Handler {
    async fn accept(&self, connection: Connection) -> Result<(), AcceptError> {
        let peer = connection.remote_id();
        let id = peer.to_string();
        let admitted = self.admission.admits(&id);
        // Checked before `hello`, so a peer that is not admitted cannot learn whether a
        // session by any name exists, whether it is busy, or what it declared. A daemon with
        // no code showing closes the connection without a byte, so a peer that is guessing
        // learns nothing at all — not even that there is something here to guess at.
        if !admitted && !self.admission.is_inviting(now_ms()) {
            warn!(peer = %peer, "refused a session connection: not admitted, and nothing is inviting");
            return Ok(());
        }
        let (mut send, recv) = connection.accept_bi().await?;
        let mut recv = recv;
        let mut decoder = Decoder::new();
        let mut pending = Vec::new();
        if !admitted {
            // The door: one code, once, and only while somebody is showing one. A refusal is
            // answered rather than swallowed, because a client that has just scanned a code
            // and typed it wrong should be told so instead of watching a connection close.
            let (attempt, paired) = match pair(&self.admission, &id, &mut decoder, &mut recv).await {
                Ok(attempt) => (attempt, true),
                Err(message) => (Paired { ok: false, message, rest: Vec::new() }, false),
            };
            let reply = SessionMsg::Paired {
                ok: attempt.ok,
                message: attempt.message.clone(),
                endpoint: id.clone(),
            };
            if write_one(&mut send, reply).await.is_err() {
                connection.close(0u32.into(), b"not paired");
                return Ok(());
            }
            if !paired {
                warn!(peer = %peer, reason = %attempt.message, "a pairing attempt was refused");
                // Closed rather than left open: a client waiting for a session that is never
                // coming should find out now, not when its own timeout expires.
                connection.close(0u32.into(), b"not paired");
                return Ok(());
            }
            pending = attempt.rest;
        }
        debug!(peer = %peer, "a client connected");
        if let Err(error) = converse(self.sessions.clone(), send, recv, decoder, pending).await {
            warn!(peer = %peer, error = %error, "a connection ended badly");
        }
        connection.closed().await;
        Ok(())
    }
}

/// What a pairing attempt produced: whether it worked, what to say, and anything the client
/// pipelined behind its code.
struct Paired {
    ok: bool,
    message: String,
    rest: Vec<ClientMsg>,
}

/// One message, written and flushed.
///
/// Only one place needs this: a client waiting to be let in is waiting for exactly one answer,
/// and the writer task that carries everything else does not exist yet at that point.
async fn write_one(send: &mut SendStream, message: SessionMsg) -> Result<(), String> {
    let frame = encode(&message).map_err(|fault| fault.message)?;
    send.write_all(&frame).await.map_err(|err| err.to_string())
}

/// The one message a stranger may send while a code is showing.
///
/// Reads until the first message arrives, because a frame may span any number of reads. What
/// it leaves behind is the decoder — which holds the partial frame that has not arrived yet —
/// and any whole message that came in the same read, because a client is allowed to write its
/// code and its hello together and neither may be lost.
async fn pair(
    admission: &Admission,
    peer: &str,
    decoder: &mut Decoder,
    recv: &mut RecvStream,
) -> Result<Paired, String> {
    let mut buffer = vec![0u8; 16 * 1024];
    loop {
        let Some(bytes) = recv.read(&mut buffer).await.map_err(|err| err.to_string())? else {
            return Err("the connection ended before a code arrived".to_string());
        };
        let mut frames = crate::split_frames(decoder, &buffer[..bytes]).into_iter();
        let Some(first) = frames.next() else {
            continue;
        };
        let rest = frames.filter_map(Result::ok).collect::<Vec<_>>();
        let Ok(message) = first else {
            return Err("the first message was not a code".to_string());
        };
        return match message {
            ClientMsg::Pair { code, label } => {
                let now = now_ms();
                match admission.accept(peer, &code, &label, now) {
                    Ok(()) => Ok(Paired {
                        ok: true,
                        message: format!("paired {}", peer),
                        rest,
                    }),
                    Err(message) => Err(message),
                }
            }
            other => Err(format!("a `{}` arrived before a code", other.name())),
        };
    }
}

/// The clock, as the transport reads it.
///
/// A code has a deadline, and the deadline is the middle layer's policy; this layer only needs
/// to know what time it is to ask.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

/// The two tasks one connection runs: a reader and a writer.
///
/// Aborted when the connection's handler goes away, which is what makes "the connection ended"
/// true. Dropping a future aborts the future, not the tasks it spawned: without this, a handler
/// that is dropped — because the router took the daemon down, say — would leave its reader and
/// writer holding a stream open for as long as the peer kept it, which is a connection outliving
/// the thing that was serving it.
struct Tasks(tokio::task::JoinHandle<()>, tokio::task::JoinHandle<()>);

impl Drop for Tasks {
    fn drop(&mut self) {
        self.0.abort();
        self.1.abort();
    }
}

async fn converse(
    sessions: Arc<Sessions>,
    send: SendStream,
    recv: RecvStream,
    decoder: Decoder,
    pending: Vec<ClientMsg>,
) -> Result<(), String> {
    let (to_wire, mut from_session) = mpsc::channel::<crate::OutboundBatch>(crate::OUTBOUND_REVISIONS);
    let (to_session, from_wire) = mpsc::channel::<ClientMsg>(crate::OUTBOUND_REVISIONS);

    let writer = tokio::spawn(async move {
        let mut send = send;
        while let Some(batch) = from_session.recv().await {
            for message in batch {
            match encode(&message) {
                Ok(frame) => {
                    if send.write_all(&frame).await.is_err() {
                        return;
                    }
                }
                Err(error) => {
                    warn!(error = %error.message, "a message could not be encoded");
                    return;
                }
            }
            }
        }
    });

    // A reader task, so the session state machine is never cancelled mid-message by
    // a `select!` that took another branch.
    let reader_to_wire = to_wire.clone();
    let reader = tokio::spawn(async move {
        let mut recv = recv;
        let mut decoder = decoder;
        let mut buffer = vec![0u8; 16 * 1024];
        // Anything that arrived in the same read as a pairing code is a message like any
        // other and goes to the session first, so the order a client wrote in is the order the
        // session sees.
        for message in pending {
            if to_session.send(message).await.is_err() {
                return;
            }
        }
        loop {
            match recv.read(&mut buffer).await {
                Ok(None) => return,
                Ok(Some(bytes)) => {
                    for message in crate::split_frames(&mut decoder, &buffer[..bytes]) {
                        match message {
                            Ok(message) => {
                                if to_session.send(message).await.is_err() {
                                    return;
                                }
                            }
                            Err(fault) => {
                                let _ = reader_to_wire.send(vec![SessionMsg::Fault { id: None, fault }]).await;
                            }
                        }
                    }
                }
                Err(_) => return,
            }
        }
    });
    let _tasks = Tasks(reader, writer);

    // A connection establishes in two steps: `hello` says who is here and what this
    // endpoint serves, `attach` chooses one session. Everything after that is the session's
    // own state machine — the same `drive` the tests run without a socket, not a second
    // copy of it. It used to be a second copy, and the copy was missing the revision watch,
    // so a client attached over a real connection was sent a view once and never again.
    let mut client: Option<ClientInfo> = None;
    let mut inbox = from_wire;
    while let Some(message) = inbox.recv().await {
        match message {
            ClientMsg::Hello { version, client: info } => {
                let ids = sessions.ids();
                client = Some(info);
                let _ = to_wire.send(vec![SessionMsg::Welcome {
                    version: version.min(PROTOCOL_VERSION),
                    session: misa_proto::wire::SessionInfo {
                        id: String::new(),
                        title: format!("{} sessions", ids.len()),
                        conversation: None,
                        created_ms: 0,
                        policy: Vec::new(),
                        queries: ids,
                        commands: Vec::new(),
                        sources: Vec::new(),
                    },
                }]).await;
            }
            ClientMsg::Attach { session: id } => {
                let Some(runtime) = sessions.get(&id) else {
                    let _ = to_wire.send(vec![SessionMsg::Fault {
                        id: None,
                        fault: Fault::protocol(format!("no session named `{id}`")),
                    }]).await;
                    continue;
                };
                let revision = runtime.watch_rev();
                let events = runtime.subscribe_events();
                let mut state = Session::new(runtime);
                // The connection's own hello is the session's hello too: the identity a
                // client declared are what its views are built for, and they are not sent
                // twice.
                if let Some(client) = client.clone() {
                    for reply in state.handle(ClientMsg::Hello { version: PROTOCOL_VERSION, client }) {
                        let _ = to_wire.send(vec![reply]).await;
                    }
                }
                crate::drive(state, inbox, to_wire.clone(), revision, events).await;
                return Ok(());
            }
            other => {
                let _ = to_wire.send(vec![SessionMsg::Fault {
                    id: None,
                    fault: Fault::protocol(format!("a `{}` arrived before `attach`", other.name())),
                }]).await;
            }
        }
    }

    Ok(())
}
/// A client of a session.
///
/// Deliberately narrow: connect, say hello, subscribe, send an intent, read. A
/// frontend that needs to do more than that is a frontend that needs a new message
/// type, which is the conversation to have rather than a way around it.
///
/// # Reconnect
///
/// A client keeps what it needs to find its session again — the endpoint, the address, the
/// name it attached to, and what it subscribed to — and re-establishes the connection when
/// that connection drops. That is not a retry protocol: a subscription's value is the whole
/// current state, so a client that re-subscribes converges on whatever happened while it was
/// away and nothing has to be replayed. An *intent* is never replayed, because it either
/// happened or it did not and only a person knows which they meant.
pub struct Client {
    /// Kept so a connection can be re-established. A client that did not keep these would
    /// have to be handed its ticket again, which is a frontend's problem becoming a person's.
    endpoint: Endpoint,
    address: EndpointAddr,
    info: ClientInfo,
    /// The session this client asked for; empty for a client that attached to nothing.
    session_id: String,
    /// What this client subscribed to, in the order it asked, so a reconnect can ask again.
    subscriptions: Vec<(SubId, Query)>,
    /// How many times this client has silently found its way back.
    reconnects: u32,
    send: SendStream,
    recv: RecvStream,
    decoder: Decoder,
    session: Option<misa_proto::wire::SessionInfo>,
}

impl Client {
    /// How many times a dropped connection is re-established before a read gives up.
    ///
    /// Bounded, because "the daemon is gone" has to be an answer rather than a loop, and
    /// small, because every attempt already costs a dial.
    const RECONNECT_TRIES: u32 = 6;

    pub async fn connect(
        endpoint: &Endpoint,
        address: EndpointAddr,
        info: ClientInfo,
        session: &str,
    ) -> Result<Client, String> {
        let (send, recv) = dial(endpoint, &address).await?;
        let mut client = Client {
            endpoint: endpoint.clone(),
            address,
            info,
            session_id: session.to_string(),
            subscriptions: Vec::new(),
            reconnects: 0,
            send,
            recv,
            decoder: Decoder::new(),
            session: None,
        };
        client.introduce().await?;
        Ok(client)
    }

    /// Hello, attach, and read past the endpoint's greeting.
    async fn introduce(&mut self) -> Result<(), String> {
        let hello = ClientMsg::Hello { version: PROTOCOL_VERSION, client: self.info.clone() };
        self.write(&hello).await?;
        if self.session_id.is_empty() {
            return Ok(());
        }
        self.write(&ClientMsg::Attach { session: self.session_id.clone() }).await?;
        self.await_attachment().await
    }

    /// A fresh connection to the same session, with the same subscriptions.
    async fn reestablish(&mut self) -> Result<(), String> {
        let (send, recv) = dial(&self.endpoint, &self.address).await?;
        self.send = send;
        self.recv = recv;
        // The old decoder holds bytes of a connection that is gone, and half a frame from a
        // closed stream is not a frame.
        self.decoder = Decoder::new();
        self.session = None;
        self.introduce().await?;
        for (id, query) in std::mem::take(&mut self.subscriptions) {
            self.write(&ClientMsg::Subscribe { id, query: query.clone(), since: None }).await?;
            self.subscriptions.push((id, query));
        }
        self.reconnects += 1;
        Ok(())
    }

    /// How many times this client has re-established its connection. For a test or a log.
    pub fn reconnects(&self) -> u32 {
        self.reconnects
    }

    /// Read until the attached session's own greeting arrives.
    ///
    /// An endpoint answers `hello` with a welcome of its own, and that welcome names no
    /// session — none has been chosen yet. A client that does know which session it wants
    /// attaches in the same breath, so this reads past that greeting and returns when the
    /// session has introduced itself, or with the refusal rather than pretending there was
    /// one. A caller that has attached never has to know the greeting exists.
    async fn await_attachment(&mut self) -> Result<(), String> {
        loop {
            // `read_next`, not `next`: this is part of establishing a connection, and asking
            // for a whole reconnect from inside one is how a future becomes infinitely deep.
            match self.read_next().await? {
                Some(SessionMsg::Welcome { session, .. }) if !session.id.is_empty() => return Ok(()),
                Some(SessionMsg::Welcome { .. }) => continue,
                // A refused pairing is an answer, not a message to skip: waiting for the
                // greeting that follows it would be waiting for something nobody sent.
                Some(SessionMsg::Paired { ok: false, message, .. }) => return Err(message),
                Some(SessionMsg::Paired { .. }) => continue,
                Some(SessionMsg::Fault { fault, .. }) => {
                    return Err(format!("{}: {}", fault.code, fault.message));
                }
                Some(_) => continue,
                None => return Err("the daemon closed the connection before answering".to_string()),
            }
        }
    }

    /// Present a pairing code, and read the daemon's answer.
    ///
    /// A connection of its own, closed as soon as it is answered, because what is being
    /// approved is the *key* iroh authenticated before a byte was sent — written down on the
    /// daemon's side by the time this returns. The client that pairs reconnects as an ordinary
    /// client, because that is what it has become.
    pub async fn pair(
        endpoint: &Endpoint,
        address: EndpointAddr,
        code: &str,
        label: &str,
    ) -> Result<String, String> {
        let connection = endpoint
            .connect(address, ALPN_SESSION)
            .await
            .map_err(|err| format!("could not reach the daemon: {err}"))?;
        let (mut send, mut recv) = connection.open_bi().await.map_err(|err| err.to_string())?;
        let frame = encode_client(&ClientMsg::Pair { code: code.to_string(), label: label.to_string() })?;
        send.write_all(&frame).await.map_err(|err| err.to_string())?;

        let mut decoder = Decoder::new();
        let mut buffer = vec![0u8; 16 * 1024];
        let answer = loop {
            let Some(bytes) = recv.read(&mut buffer).await.map_err(|err| err.to_string())? else {
                return Err("the daemon closed the connection without answering".to_string());
            };
            // The wire is read here rather than through the client-message splitter, because a
            // pairing answer is a *session* message: the same frames, read from the other end.
            if let Err(err) = decoder.push(&buffer[..bytes]) {
                return Err(err.to_string());
            }
            let mut answered = None;
            while let Some(frame) = decoder.next() {
                let payload = frame.map_err(|err| err.to_string())?;
                let message: SessionMsg = misa_proto::chunk::decode(&payload).map_err(|err| err.to_string())?;
                if let SessionMsg::Paired { ok, message, .. } = message {
                    answered = Some((ok, message));
                }
            }
            if let Some(answer) = answered {
                break answer;
            }
        };
        connection.close(0u32.into(), b"paired");
        match answer {
            (true, message) => Ok(message),
            (false, message) => Err(message),
        }
    }

    pub fn session(&self) -> Option<&misa_proto::wire::SessionInfo> {
        self.session.as_ref()
    }

    /// Ask for a subscription, and remember it so a reconnect can ask again.
    ///
    /// A subscription is the one thing here that is safe to send twice: the session answers
    /// with the current value, and the current value *is* the state.
    pub async fn subscribe_since(&mut self, id: SubId, query: Query, since: misa_proto::sync::Version) -> Result<(), String> {
        self.write(&ClientMsg::Subscribe { id, query, since: Some(since) }).await
    }

    pub async fn subscribe(&mut self, id: SubId, query: Query) -> Result<(), String> {
        self.remember(id, &query);
        match self.write(&ClientMsg::Subscribe { id, query, since: None }).await {
            Ok(()) => Ok(()),
            Err(error) => {
                // The connection may be gone. Finding a new one re-asks every subscription
                // this client has, which includes the one that just failed.
                self.restore().await.map_err(|_| error)
            }
        }
    }

    /// An intent, which is never replayed: it either happened or it did not.
    ///
    /// A write that fails may still have been received — a connection can break after the
    /// bytes left — so this reports the failure rather than sending it again. A prompt a
    /// session recorded twice is worse than a prompt somebody has to type again.
    pub async fn intent(&mut self, id: u64, intent: misa_proto::wire::Intent) -> Result<(), String> {
        match self.write(&ClientMsg::Intent { id, intent }).await {
            Ok(()) => Ok(()),
            Err(error) => {
                // For the *next* call, not this one.
                let _ = self.restore().await;
                Err(error)
            }
        }
    }

    fn remember(&mut self, id: SubId, query: &Query) {
        self.subscriptions.retain(|(seen, _)| *seen != id);
        self.subscriptions.push((id, query.clone()));
    }

    async fn write(&mut self, message: &ClientMsg) -> Result<(), String> {
        let frame = encode_client(message)?;
        self.send.write_all(&frame).await.map_err(|err| err.to_string())
    }

    /// Re-establish, with a bounded backoff, and report whether it worked.
    async fn restore(&mut self) -> Result<(), String> {
        let mut last = String::new();
        for attempt in 1..=Self::RECONNECT_TRIES {
            tokio::time::sleep(backoff(attempt)).await;
            match self.reestablish().await {
                Ok(()) => return Ok(()),
                Err(error) => last = error,
            }
        }
        Err(last)
    }

    /// The next message, or `None` when the session is gone.
    ///
    /// A dropped connection is not the end of a session: the facts live elsewhere and a
    /// subscription converges, so the way back is to re-subscribe and keep reading. What
    /// this will not do is retry for ever — a daemon that is not there has to be an answer.
    pub async fn next(&mut self) -> Result<Option<SessionMsg>, String> {
        let mut attempts = 0;
        loop {
            // What to report if this turns out to be the last try: the error that closed the
            // connection, or nothing at all for a connection that closed quietly, which a
            // daemon that is restarting cannot announce any other way.
            let last = match self.read_next().await {
                Ok(Some(message)) => return Ok(Some(message)),
                Ok(None) => None,
                Err(error) => Some(error),
            };
            if attempts >= Self::RECONNECT_TRIES {
                return match last {
                    Some(error) => Err(error),
                    None => Ok(None),
                };
            }
            attempts += 1;
            tokio::time::sleep(backoff(attempts)).await;
            // A re-establishment that fails is not remembered: the next read fails on that
            // same connection and says the same thing, and the message a person needs is the
            // one that arrives when the tries run out.
            let _ = self.reestablish().await;
        }
    }

    /// Read one message from the connection this client has now.
    async fn read_next(&mut self) -> Result<Option<SessionMsg>, String> {
        loop {
            if let Some(frame) = self.decoder.next() {
                let payload = frame.map_err(|err| err.to_string())?;
                let message: SessionMsg = misa_proto::chunk::decode(&payload).map_err(|err| err.to_string())?;
                // The endpoint's greeting names no session; the session's own does. Only the
                // latter is what a client means by "the session I am attached to".
                if let SessionMsg::Welcome { session, .. } = &message
                    && !session.id.is_empty()
                {
                    self.session = Some(session.clone());
                }
                return Ok(Some(message));
            }
            let mut buffer = vec![0u8; 16 * 1024];
            match self.recv.read(&mut buffer).await {
                Ok(None) => return Ok(None),
                Ok(Some(bytes)) => {
                    self.decoder.push(&buffer[..bytes]).map_err(|err| err.to_string())?;
                }
                Err(err) => return Err(err.to_string()),
            }
        }
    }
}

/// Dial, and open the one stream a session is.
async fn dial(endpoint: &Endpoint, address: &EndpointAddr) -> Result<(SendStream, RecvStream), String> {
    let connection = endpoint
        .connect(address.clone(), ALPN_SESSION)
        .await
        .map_err(|err| err.to_string())?;
    connection.open_bi().await.map_err(|err| err.to_string())
}

/// How long to wait before the *n*-th attempt at finding a session again.
///
/// Growing and capped: a daemon that is restarting is back in a moment, and one that is
/// gone for good should not be dialled a hundred times a second while somebody finds out.
fn backoff(attempt: u32) -> std::time::Duration {
    std::time::Duration::from_millis((20 * u64::from(attempt)).min(200))
}

fn encode_client(message: &ClientMsg) -> Result<Vec<u8>, String> {
    misa_proto::chunk::encode(message).map_err(|err| err.to_string())
}

/// A command intent, the one place a client names something outside its four
/// intents.
pub fn command(name: &str, argument: &str) -> misa_proto::wire::Intent {
    misa_proto::wire::Intent::Command {
        name: name.to_string(),
        args: misa_value::Value::str(argument),
    }
}

/// A ticket for one session on one endpoint.
pub fn ticket(endpoint: &Endpoint, session: &str) -> Ticket {
    Ticket::new(node_of(endpoint), session)
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_kernel::{LocalKernel, Provider, ScriptedProvider};
    use misa_session::Runtime;

    #[test]
    fn a_ticket_that_names_only_this_machine_is_reached_without_a_relay() {
        // The decision a client should not have to make, and the one that decides whether a
        // daemon on a machine with no route out can be reached at all.
        assert!(names_only_this_machine("abc@127.0.0.1:5000"));
        assert!(names_only_this_machine("abc@127.0.0.1:5000,127.0.0.1:5001"));
        assert!(names_only_this_machine("abc@[::1]:5000"));
        assert!(!names_only_this_machine("abc@10.0.0.4:5000"));
        assert!(!names_only_this_machine("abc@127.0.0.1:5000,10.0.0.4:5000"));
        // A key with no address is the case discovery exists for, and discovery is the relay.
        assert!(!names_only_this_machine("abc"));
        // A node part that says nothing dialable is not a local endpoint, whatever it says.
        assert!(!names_only_this_machine("abc@not-an-address"));
        assert!(!names_only_this_machine("abc@"));
    }

    fn runtime() -> Arc<Runtime> {
        let provider: Arc<dyn Provider> = ScriptedProvider::always("hello");
        Runtime::start(
            "demo",
            "a demo session",
            None,
            Arc::new(LocalKernel::new(provider)),
            "scripted",
            "scripted-1",
            misa_value::Value::Null,
        )
    }

    #[test]
    fn a_ticket_carries_an_identity_and_may_carry_where_to_find_it() {
        let key = SecretKey::generate();
        let node = key.public().to_string();
        // With a relay, the identity is enough: discovery does the rest.
        let address = address_of(&node).expect("a bare endpoint id parses");
        assert_eq!(address.id.to_string(), node);
        assert!(address.addrs.is_empty());

        // Without one, the addresses travel in the ticket, because there is nothing to ask.
        let with_addresses = format!("{node}@127.0.0.1:1234,10.0.0.5:5678");
        let address = address_of(&with_addresses).expect("an id with addresses parses");
        assert_eq!(address.id.to_string(), node);
        assert_eq!(address.addrs.len(), 2);

        assert!(address_of("not a key").is_err());
        assert!(address_of(&format!("{node}@not-an-address")).is_err());
    }

    #[test]
    fn a_ticket_names_both_an_endpoint_and_a_session() {
        let ticket = Ticket::new("abc", "demo");
        assert_eq!(ticket.to_string(), "misa:abc:demo");
        assert_eq!(ticket.to_string().parse::<Ticket>().unwrap(), ticket);
    }

    #[tokio::test]
    async fn a_connection_without_attach_cannot_reach_a_session() {
        // The state machine refuses anything before `hello`, so a client cannot send
        // an intent into a session it has not been given.
        let mut session = Session::new(runtime());
        let replies = session.handle(ClientMsg::Intent {
            id: 1,
            intent: misa_proto::wire::Intent::Cancel { target: None },
        });
        match &replies[0] {
            SessionMsg::Fault { fault, .. } => assert_eq!(fault.code, "protocol"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_command_names_what_it_wants() {
        match command("attach", "demo") {
            misa_proto::wire::Intent::Command { name, args } => {
                assert_eq!(name, "attach");
                assert_eq!(args.as_str(), Some("demo"));
            }
            other => panic!("expected a command, got {other:?}"),
        }
    }
}
