//! The server endpoint of the protocol over injected session state.
use std::collections::BTreeMap;
use std::sync::Arc;

use misa_proto::chunk::{self as frame, Decoder};
use misa_proto::wire::{
    ClientInfo, ClientMsg, Level, Query, SessionEvent, SessionMsg, SubId,
};
use misa_proto::{Fault, PROTOCOL_VERSION};
use crate::Reading;
use crate::Session as Backend;
use tokio::sync::{broadcast, mpsc, watch};

/// What one attached client remembers.
///
/// Per connection, because two clients may ask different questions and one may be
/// attached from a browser while the other is a terminal of a very different width.
pub struct Server {
    recipient: u64,
    replies: Option<mpsc::Receiver<crate::Emission>>,
    runtime: Arc<dyn Backend>,
    client: Option<ClientInfo>,
    subscriptions: BTreeMap<SubId, Subscription>,
    last_seq: u64,
    stream_cursor: u64,
    welcomed: bool,
}

struct Subscription {
    query: Query,
    current: Option<Reading>,
    version: Option<misa_proto::sync::Version>,
}

impl Server {
    pub fn new(runtime: Arc<dyn Backend>) -> Self {
        let recipient = misa_proto::wire::RequestContext::connection();
        let replies = Some(runtime.subscribe_replies(recipient));
        Server {
            recipient,
            replies,
            runtime,
            client: None,
            subscriptions: BTreeMap::new(),
            last_seq: 0,
            stream_cursor: 0,
            welcomed: false,
        }
    }

    pub fn client(&self) -> Option<&ClientInfo> {
        self.client.as_ref()
    }

    pub fn take_replies(&mut self) -> Option<mpsc::Receiver<crate::Emission>> { self.replies.take() }

    fn directed(&self, emission: crate::Emission) -> Option<SessionMsg> {
        if emission.recipient != Some(self.recipient) { return None; }
        match emission.event {
            SessionEvent::DownloadReady {id, download} => Some(SessionMsg::Download {id, download}),
            _ => None,
        }
    }

    /// Handle one client message. Anything arriving before `Hello` is refused.
    pub fn handle(&mut self, message: ClientMsg) -> Vec<SessionMsg> {
        if !self.welcomed && !matches!(message, ClientMsg::Hello { .. }) {
            return vec![SessionMsg::Fault {
                id: None,
                fault: Fault::protocol(format!("a `{}` arrived before `hello`", message.name())),
            }];
        }
        match message {
            ClientMsg::Pair { .. } => vec![SessionMsg::Fault {
                id: None,
                fault: Fault::protocol("this connection is already paired"),
            }],
            ClientMsg::Hello { version, client } => {
                if version != PROTOCOL_VERSION {
                    return vec![SessionMsg::Fault {
                        id: None,
                        fault: Fault::protocol(format!(
                            "this session speaks protocol {PROTOCOL_VERSION}, not {version}"
                        )),
                    }];
                }
                self.runtime.attached(self.recipient, client.clone());
                self.client = Some(client);
                self.welcomed = true;
                vec![SessionMsg::Welcome {
                    version: PROTOCOL_VERSION,
                    session: self.runtime.info(),
                }]
            }
            ClientMsg::Subscribe { id, query, since } => {
                if query.id == misa_proto::VIEW_QUERY {
                    let (sync, cursor) = self.runtime.sync(since.as_ref());
                    self.stream_cursor = cursor;
                    let (version, replies) = sync_answer(id, sync, true);
                    self.subscriptions.insert(id, Subscription { query, current: None, version: Some(version) });
                    replies
                } else {
                    match self.runtime.read(&query) {
                        Ok(reading) => {
                            let message = answer(id, self.runtime.rev(), reading.clone());
                            self.subscriptions.insert(id, Subscription { query, current: Some(reading), version: None });
                            message.into_iter().collect()
                        }
                        Err(fault) => {
                            self.subscriptions.insert(id, Subscription { query, current: None, version: None });
                            vec![SessionMsg::QueryFault { id, fault }]
                        }
                    }
                }
            }
            ClientMsg::Unsubscribe { id } => {
                self.subscriptions.remove(&id);
                Vec::new()
            }
            // A completion request is a request with a reply, so it is answered here
            // and correlated by the intent's own id.
            ClientMsg::Intent { id, intent: misa_proto::wire::Intent::Complete { source, prefix, limit } } => {
                match self.runtime.complete(&source, &prefix, limit) {
                    Ok((candidates, truncated)) => vec![SessionMsg::Completion {
                        id,
                        source,
                        candidates,
                        truncated,
                    }],
                    Err(fault) => vec![SessionMsg::Fault { id: Some(id), fault }],
                }
            }
            ClientMsg::Intent { id, intent } => {
                let faults = self.runtime.intent_from(intent, Some(misa_proto::wire::RequestContext { recipient: self.recipient, id }));
                let mut out: Vec<SessionMsg> = faults
                    .into_iter()
                    .map(|fault| SessionMsg::Fault { id: Some(id), fault })
                    .collect();
                if out.is_empty() {
                    out.push(SessionMsg::Ack { id });
                }
                out
            }
            // A session is named by the transport before its state machine starts,
            // so by the time a message reaches here there is nothing left to choose.
            ClientMsg::Attach { .. } => vec![SessionMsg::Fault {
                id: None,
                fault: Fault::protocol("this connection is already attached to a session"),
            }],
        }
    }

    /// Re-read every subscription, and report only what changed.
    ///
    /// Called when the runtime's revision changes. A client that has asked for 40
    /// messages gets a tree when the 41st arrives and nothing when a notice is
    /// added elsewhere in the database.
    pub fn refresh(&mut self) -> Vec<SessionMsg> {
        let revision = self.runtime.rev();
        let mut out = Vec::new();
        for (id, subscription) in self.subscriptions.iter_mut() {
            if subscription.query.id == misa_proto::VIEW_QUERY {
                let changes = self.runtime.changes(subscription.version.as_ref());
                let snapshot = matches!(changes, misa_proto::sync::ViewSync::Snapshot { .. });
                let sync = if snapshot {
                    let (sync, cursor) = self.runtime.sync(None);
                    self.stream_cursor = cursor;
                    sync
                } else { changes };
                let (version, replies) = sync_answer(*id, sync, snapshot);
                subscription.version = Some(version);
                out.extend(replies);
                continue;
            }
            match self.runtime.read(&subscription.query) {
                Ok(reading) => {
                    let changed = match (&subscription.current, &reading) {
                        (Some(previous), now) => !same(previous, now),
                        (None, _) => true,
                    };
                    subscription.current = Some(reading.clone());
                    if changed
                        && let Some(message) = answer(*id, revision, reading)
                    {
                        out.push(message);
                    }
                }
                Err(fault) => {
                    out.push(SessionMsg::QueryFault { id: *id, fault });
                }
            }
        }
        out
    }

    fn resync(&mut self) -> Vec<SessionMsg> {
        let mut replies = Vec::new();
        for (id, subscription) in &mut self.subscriptions {
            if subscription.query.id == misa_proto::VIEW_QUERY {
                let (sync, cursor) = self.runtime.sync(None);
                let (version, messages) = sync_answer(*id, sync, true);
                self.stream_cursor = cursor;
                subscription.version = Some(version);
                replies.extend(messages);
            } else {
                match self.runtime.read(&subscription.query) {
                    Ok(reading) => {
                        subscription.current = Some(reading.clone());
                        replies.extend(answer(*id, self.runtime.rev(), reading));
                    }
                    Err(fault) => replies.push(SessionMsg::QueryFault {id: *id, fault}),
                }
            }
        }
        replies
    }

    /// The next ephemeral events, as wire messages. Anything already seen is
    /// The next ephemeral events, as wire messages.
    ///
    /// A client is only sent an event it has not seen, so two clients sharing one
    /// broadcast do not receive each other's repetition, and a re-delivered event
    /// is dropped rather than applied twice.
    pub fn events(&mut self, emissions: &[crate::Emission]) -> Vec<SessionMsg> {
        let mut out = Vec::new();
        for emission in emissions {
            if emission.seq < self.last_seq {
                continue;
            }
            self.last_seq = emission.seq + 1;
            if matches!(emission.event, SessionEvent::Stream { .. }) && emission.seq < self.stream_cursor { continue; }
            if emission.recipient.is_some_and(|recipient| recipient != self.recipient) { continue; }
            match &emission.event {
                SessionEvent::DownloadReady { id, download } if emission.recipient.is_some() =>
                    out.push(SessionMsg::Download { id: *id, download: download.clone() }),
                SessionEvent::DownloadReady { .. } => {},
                _ => out.push(SessionMsg::Event { seq: emission.seq, event: emission.event.clone() }),
            }
        }
        out
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.runtime.detached(self.recipient);
    }
}

fn sync_answer(id: SubId, sync: misa_proto::sync::ViewSync, include_streams: bool) -> (misa_proto::sync::Version, Vec<SessionMsg>) {
    use misa_proto::sync::ViewSync;
    let (version, mut replies, streams) = match sync {
        ViewSync::Snapshot { version, view, streams } => (version.clone(), vec![SessionMsg::View { id, version, view }], streams),
        ViewSync::Changes { version, changes, streams } => {
            let replies = if changes.is_empty() { vec![] } else { vec![SessionMsg::Changes { id, changes }] };
            (version, replies, streams)
        }
    };
    if include_streams { replies.push(SessionMsg::Streams { streams }); }
    (version, replies)
}

fn answer(id: SubId, revision: u64, reading: Reading) -> Option<SessionMsg> {
    match reading {
        Reading::View(_) => unreachable!("view subscriptions use the revision protocol"),
        Reading::Data(value) => Some(SessionMsg::Value { id, rev: revision, value }),
    }
}

fn same(a: &Reading, b: &Reading) -> bool {
    match (a, b) {
        (Reading::Data(a), Reading::Data(b)) => a.same(b),
        (Reading::View(a), Reading::View(b)) => a == b,
        _ => false,
    }
}

/// Encode a message for a stream.
pub fn encode(message: &SessionMsg) -> Result<Vec<u8>, Fault> {
    frame::encode(message).map_err(|err| Fault::protocol(err.to_string()))
}

/// Decode one client message from a frame's payload.
pub fn decode(payload: &[u8]) -> Result<ClientMsg, Fault> {
    frame::decode(payload).map_err(|err| Fault::protocol(err.to_string()))
}

/// Drive one connection's session state machine over channels.
///
/// The transport calls this with whatever it has: a QUIC stream pair, a pair of
/// in-memory channels, or a test. Everything the protocol does happens here, which
/// is why the interesting cases are testable without a network.
pub const OUTBOUND_REVISIONS: usize = 64;
pub type OutboundBatch = Vec<SessionMsg>;

/// Each queue slot carries at most one canonical revision, regardless of catch-up size.
fn revision_batches(messages: Vec<SessionMsg>) -> Vec<OutboundBatch> {
    let mut batches = Vec::new();
    for message in messages {
        match message {
            SessionMsg::Changes { id, changes } => {
                for change in changes {
                    batches.push(vec![SessionMsg::Changes { id, changes: vec![change] }]);
                }
            }
            message => batches.push(vec![message]),
        }
    }
    batches
}

pub async fn drive(
    mut session: Server,
    mut inbound: mpsc::Receiver<ClientMsg>,
    outbound: mpsc::Sender<OutboundBatch>,
    mut revision: watch::Receiver<u64>,
    mut events: broadcast::Receiver<crate::Emission>,
) {
    let mut replies = session.take_replies().expect("a connection owns its reply receiver");
    let mut behind = false;
    loop {
        tokio::select! {
            reply = replies.recv(), if !replies.is_closed() || !replies.is_empty() => {
                if let Some(reply) = reply.and_then(|reply| session.directed(reply)) {
                    if outbound.send(vec![reply]).await.is_err() { return; }
                }
            }
            permit = outbound.reserve(), if behind => {
                let Ok(permit) = permit else { return };
                permit.send(session.resync());
                behind = false;
            }
            message = inbound.recv() => match message {
                Some(message) => {
                    let replies = session.handle(message);
                    for batch in revision_batches(replies) {
                        if outbound.send(batch).await.is_err() { return; }
                    }
                }
                None => return,
            },
            changed = revision.changed() => {
                if changed.is_err() { return; }
                if !behind {
                    let replies = session.refresh();
                    for batch in revision_batches(replies) {
                        match outbound.try_send(batch) {
                            Ok(()) => {},
                            Err(mpsc::error::TrySendError::Full(_)) => { behind = true; break; },
                            Err(mpsc::error::TrySendError::Closed(_)) => return,
                        }
                    }
                }
            }
            emission = events.recv() => match emission {
                Ok(emission) if emission.recipient.is_some() => {
                    let replies = session.events(&[emission]);
                    if !replies.is_empty() && outbound.send(replies).await.is_err() { return; }
                }
                Ok(emission) if !behind => {
                    let replies = session.events(&[emission]);
                    if !replies.is_empty() {
                        match outbound.try_send(replies) {
                            Ok(()) => {},
                            Err(mpsc::error::TrySendError::Full(_)) => behind = true,
                            Err(mpsc::error::TrySendError::Closed(_)) => return,
                        }
                    }
                }
                Ok(_) => {},
                Err(broadcast::error::RecvError::Lagged(_)) => behind = true,
                Err(broadcast::error::RecvError::Closed) => return,
            },
        }
    }
}

/// Frame bytes into client messages.
pub fn split_frames(decoder: &mut Decoder, bytes: &[u8]) -> Vec<Result<ClientMsg, Fault>> {
    if let Err(err) = decoder.push(bytes) {
        return vec![Err(Fault::protocol(err.to_string()))];
    }
    let mut out = Vec::new();
    while let Some(frame) = decoder.next() {
        out.push(match frame {
            Ok(payload) => decode(&payload),
            Err(err) => Err(Fault::protocol(err.to_string())),
        });
    }
    out
}

/// A notice a bare transport needs to send without a session, as a message.
pub fn notice(level: Level, text: impl Into<String>) -> SessionMsg {
    SessionMsg::Event { seq: 0, event: SessionEvent::Notice { level, text: text.into() } }
}

/// What a client sends to reach a session: its address and the session id.
pub use misa_proto::Ticket;
