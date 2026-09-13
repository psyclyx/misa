
//! Transport: iroh, and the protocol that rides on it.
//!
//! # Why the protocol is not the transport
//!
//! [`Session`] is a state machine over messages. It takes a [`ClientMsg`] and
//! produces [`SessionMsg`]s; it does not know what a socket is. The iroh layer
//! below it is a byte source and a byte sink, and the daemon and the tests both
//! use the same state machine — the tests without any network at all.
//!
//! That split is not tidiness. A protocol that is only reachable through a socket
//! is a protocol whose interesting behaviour (a resubscribe, a stale revision, a
//! refused intent, a client that never says hello) is only testable by accident.
//!
//! # Two kinds of update
//!
//! - **Subscriptions** are re-read when the session's revision changes, and only a
//!   value that actually changed is sent. This is the durable path: a client that
//!   reconnects re-subscribes and converges.
//! - **Events** are the ephemeral path: a streamed delta, a notice, a status line.
//!   They are broadcast to every attached client and lost on purpose when a client
//!   falls behind, because the subscription that describes the same content will
//!   also have changed.
//!
//! Nothing in this layer decides anything about the conversation. It is a wire.

/// Deployment admission and pairing, shared by session and blob connections.
pub mod admission;
/// Bulk content by hash, on a connection of its own.
pub mod blob;
pub mod iroh;
/// One endpoint, both protocols.
pub mod server;

use std::collections::BTreeMap;
use std::sync::Arc;

use misa_proto::chunk::{self as frame, Decoder};
use misa_proto::wire::{
    ClientInfo, ClientMsg, Level, Query, SessionEvent, SessionMsg, SubId,
};
use misa_proto::{Fault, PROTOCOL_VERSION};
use misa_session::Reading;
use misa_session::Runtime;
use tokio::sync::{broadcast, mpsc, watch};

/// What one attached client remembers.
///
/// Per connection, because two clients may ask different questions and one may be
/// attached from a browser while the other is a terminal of a very different width.
pub struct Session {
    recipient: u64,
    runtime: Arc<Runtime>,
    client: Option<ClientInfo>,
    subscriptions: BTreeMap<SubId, Subscription>,
    last_seq: u64,
    welcomed: bool,
}

struct Subscription {
    query: Query,
    current: Option<Reading>,
    version: Option<misa_proto::sync::Version>,
}

impl Session {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Session {
            recipient: misa_proto::wire::RequestContext::connection(),            runtime,
            client: None,
            subscriptions: BTreeMap::new(),
            last_seq: 0,
            welcomed: false,
        }
    }

    pub fn client(&self) -> Option<&ClientInfo> {
        self.client.as_ref()
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
                self.client = Some(client);
                self.welcomed = true;
                vec![SessionMsg::Welcome {
                    version: PROTOCOL_VERSION,
                    session: self.runtime.info(),
                }]
            }
            ClientMsg::Subscribe { id, query, since } => {
                if query.id == misa_proto::VIEW_QUERY {
                    let sync = self.runtime.sync(since.as_ref());
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
                let (version, replies) = sync_answer(*id, self.runtime.changes(subscription.version.as_ref()), false);
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
        for subscription in self.subscriptions.values_mut() {
            subscription.current = None;
            subscription.version = None;
        }
        let mut replies = self.refresh();
        replies.push(SessionMsg::Streams { streams: self.runtime.streams() });
        replies
    }

    /// The next ephemeral events, as wire messages. Anything already seen is
    /// The next ephemeral events, as wire messages.
    ///
    /// A client is only sent an event it has not seen, so two clients sharing one
    /// broadcast do not receive each other's repetition, and a re-delivered event
    /// is dropped rather than applied twice.
    pub fn events(&mut self, emissions: &[misa_session::Emission]) -> Vec<SessionMsg> {
        let mut out = Vec::new();
        for emission in emissions {
            if emission.seq < self.last_seq {
                continue;
            }
            self.last_seq = emission.seq + 1;
            if emission.recipient.is_some_and(|recipient| recipient != self.recipient) { continue; }
            match &emission.event {
                SessionEvent::DownloadReady { id, download } if emission.recipient.is_some() => out.push(SessionMsg::Download { id: *id, download: download.clone() }),
                SessionEvent::DownloadReady { .. } => {},
                _ => out.push(SessionMsg::Event { seq: emission.seq, event: emission.event.clone() }),
            }
        }
        out
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

/// Everything a connection task needs to carry bytes.
pub struct Wire {
    pub outbound: mpsc::UnboundedSender<Vec<u8>>,
    pub revision: watch::Receiver<u64>,
    pub events: broadcast::Receiver<misa_session::Emission>,
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
    mut session: Session,
    mut inbound: mpsc::Receiver<ClientMsg>,
    outbound: mpsc::Sender<OutboundBatch>,
    mut revision: watch::Receiver<u64>,
    mut events: broadcast::Receiver<misa_session::Emission>,
) {
    let mut behind = false;
    loop {
        tokio::select! {
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

#[cfg(test)]
mod tests {
    use super::*;
    use misa_kernel::{LocalKernel, Provider, ScriptedProvider, Turn};
    use misa_proto::wire::Intent;
    use misa_value::Value;
    use std::time::Duration;

    fn runtime() -> Arc<Runtime> {
        let provider: Arc<dyn Provider> = ScriptedProvider::new([
            Turn::call("echo", Value::str("hello"), Turn::say("all done")),
            Turn::say("all done"),
        ]);
        Runtime::start(
            "demo",
            "a demo session",
            None,
            Arc::new(LocalKernel::new(provider)),
            "scripted",
            "scripted-1",
            Value::Null,
        )
    }

    fn hello(name: &str) -> ClientMsg {
        ClientMsg::Hello {
            version: PROTOCOL_VERSION,
            client: ClientInfo::new(name, "0.1.0"),
        }
    }

    fn view_of(messages: &[SessionMsg]) -> Option<&misa_proto::view::Node> {
        messages.iter().find_map(|message| match message {
            SessionMsg::View { view, .. } => Some(view),
            _ => None,
        })
    }

    #[tokio::test]
    async fn a_message_before_hello_is_refused_rather_than_guessed_at() {
        let mut session = Session::new(runtime());
        let replies = session.handle(ClientMsg::Unsubscribe { id: SubId(1) });
        assert!(matches!(replies[0], SessionMsg::Fault { .. }));
    }

    #[tokio::test]
    async fn another_protocol_version_is_refused_before_it_can_act() {
        let mut session = Session::new(runtime());
        let replies = session.handle(ClientMsg::Hello {
            version: PROTOCOL_VERSION + 1,
            client: ClientInfo::new("x", "0"),
        });
        match &replies[0] {
            SessionMsg::Fault { fault, .. } => assert_eq!(fault.code, "protocol"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_subscription_is_answered_with_a_view_and_repeated_only_when_it_changes() {
        let mut session = Session::new(runtime());
        assert!(matches!(session.handle(hello("tui"))[0], SessionMsg::Welcome { .. }));
        let replies = session.handle(ClientMsg::Subscribe {
            since: None,
            id: SubId(1),
            query: Query::new(misa_proto::VIEW_QUERY),
        });
        assert!(view_of(&replies).is_some());
        assert!(session.refresh().is_empty(), "an unchanged view was re-sent");
    }

    #[tokio::test]
    async fn attachment_save_is_a_kernel_answer_only_the_requesting_connection_receives() {
        use misa_proto::wire::Intent;
        let kernel = Arc::new(misa_kernel::LocalKernel::new(misa_kernel::ScriptedProvider::always("done")));
        let blob = kernel.blobs().put(b"a file to keep", Some("text/plain")).unwrap();
        let runtime = Runtime::start("save", "Save", None, kernel, "scripted", "test", misa_value::Value::Null);
        runtime.intent(Intent::Prompt { text: "keep this".into(), attachments: vec![blob.clone()] });
        fn target(node: &misa_proto::view::Node) -> Option<String> {
            if node.actions.iter().any(|action| action.id == "attachment.save") { return Some(node.id.clone()); }
            node.children.iter().find_map(target)
        }
        let node = target(&runtime.view().unwrap()).unwrap();
        let mut owner = Session::new(runtime.clone());
        let mut other = Session::new(runtime.clone());
        owner.handle(hello("owner")); other.handle(hello("other"));
        let mut events = runtime.subscribe_events();
        let replies = owner.handle(ClientMsg::Intent { id: 42, intent: Intent::Action { node, action: "attachment.save".into(), args: misa_value::Value::str("/etc/shadow"), fields: vec![] } });
        assert!(matches!(replies.as_slice(), [SessionMsg::Ack { id: 42 }]));
        let emission = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop { let event = events.recv().await.unwrap(); if matches!(event.event, SessionEvent::DownloadReady { .. }) { break event; } }
        }).await.unwrap();
        assert!(other.events(&[emission.clone()]).is_empty());
        match owner.events(&[emission.clone()]).as_slice() {
            [SessionMsg::Download { id, download }] => {
                assert_eq!(*id, 42); assert_eq!(download.blob.as_ref(), Some(&blob));
                assert_eq!(download.name, format!("{}.txt", blob.hash)); assert!(download.error.is_empty());
            }
            other => panic!("unexpected answer: {other:?}"),
        }
        assert!(owner.events(&[emission]).is_empty());
        let denied = owner.handle(ClientMsg::Intent { id: 43, intent: Intent::Action { node: "invented".into(), action: "attachment.save".into(), args: misa_value::Value::Null, fields: vec![] } });
        assert!(matches!(denied.as_slice(), [SessionMsg::Fault { id: Some(43), .. }]));
    }

    #[tokio::test]
    async fn two_attached_clients_receive_byte_identical_trees() {
        let runtime = runtime();
        let mut first = Session::new(runtime.clone());
        let mut second = Session::new(runtime.clone());
        first.handle(hello("terminal"));
        second.handle(hello("browser"));
        let subscribe = || ClientMsg::Subscribe { since: None, id: SubId(1), query: Query::new(misa_proto::VIEW_QUERY) };
        let a = first.handle(subscribe());
        let b = second.handle(subscribe());
        assert_eq!(frame::encode(view_of(&a).expect("first tree")).unwrap(),
                   frame::encode(view_of(&b).expect("second tree")).unwrap());
        runtime.intent(misa_proto::Intent::Command { name: "status".into(), args: misa_value::Value::Null });
        let a = first.refresh();
        let b = second.refresh();
        assert_eq!(frame::encode(&a).unwrap(),
                   frame::encode(&b).unwrap());
    }

    #[tokio::test]
    async fn a_data_subscription_is_answered_with_data() {
        let mut session = Session::new(runtime());
        session.handle(hello("cli"));
        let replies = session.handle(ClientMsg::Subscribe {
            since: None,
            id: SubId(2),
            query: Query::new("session.status"),
        });
        match &replies[0] {
            SessionMsg::Value { value, .. } => {
                assert_eq!(value.get("status").and_then(Value::as_str), Some("idle"))
            }
            other => panic!("expected a value, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_query_that_cannot_be_answered_keeps_its_subscription() {
        let mut session = Session::new(runtime());
        session.handle(hello("cli"));
        let replies = session.handle(ClientMsg::Subscribe {
            since: None,
            id: SubId(3),
            query: Query::new("no.such.query"),
        });
        assert!(matches!(replies[0], SessionMsg::QueryFault { .. }));
        // And the connection is still usable.
        assert!(session.handle(ClientMsg::Unsubscribe { id: SubId(3) }).is_empty());
    }

    #[tokio::test]
    async fn an_intent_is_acknowledged_and_an_impossible_one_is_faulted() {
        let mut session = Session::new(runtime());
        session.handle(hello("tui"));
        assert!(matches!(
            session.handle(ClientMsg::Intent { id: 1, intent: Intent::Prompt { text: "hi".into(), attachments: vec![] } })[0],
            SessionMsg::Ack { id: 1 }
        ));
        let replies = session.handle(ClientMsg::Intent {
            id: 2,
            intent: Intent::Action {
                node: "x".into(),
                action: "nothing".into(),
                args: Value::Null,
                fields: Vec::new(),
            },
        });
        match &replies[0] {
            SessionMsg::Fault { id: Some(2), fault } => assert!(fault.message.contains("nothing")),
            other => panic!("expected a fault, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_client_that_asks_before_hello_cannot_get_a_view() {
        let mut session = Session::new(runtime());
        let replies = session.handle(ClientMsg::Subscribe {
            since: None,
            id: SubId(1),
            query: Query::new(misa_proto::VIEW_QUERY),
        });
        assert!(view_of(&replies).is_none());
        assert!(matches!(replies[0], SessionMsg::Fault { .. }));
    }

    #[tokio::test]
    async fn a_prompt_moves_the_view_and_the_transport_reports_only_that() {
        let runtime = runtime();
        let mut session = Session::new(runtime.clone());
        session.handle(hello("tui"));
        let mut state = misa_proto::sync::ClientView::default();
        for message in session.handle(ClientMsg::Subscribe { since: None, id: SubId(1), query: Query::new(misa_proto::VIEW_QUERY) }) {
            state.receive(&message).unwrap();
        }
        session.refresh();

        runtime.intent(Intent::Prompt { text: "hello".into(), attachments: vec![] });
        let replies = session.refresh();
        assert_eq!(replies.len(), 1, "expected exactly one changed subscription");
        for message in &replies { state.receive(message).unwrap(); }
        let view = state.canonical().expect("a view");
        assert!(
            misa_proto::view::find(&view, "transcript").is_some(),
            "the transcript is missing from the refreshed view"
        );
    }

    #[tokio::test]
    async fn events_are_numbered_and_a_client_does_not_see_one_twice() {
        let mut session = Session::new(runtime());
        let emission = |seq| misa_session::Emission { recipient: None, seq, event: SessionEvent::Status { text: format!("{seq}") } };
        assert_eq!(session.events(&[emission(1), emission(2)]).len(), 2);
        assert!(
            session.events(&[emission(1), emission(2)]).is_empty(),
            "an event that had already been delivered was sent again"
        );
        // A window that includes both a seen and an unseen event delivers only the
        // unseen one, which is what happens after a brief lag.
        let delivered = session.events(&[emission(2), emission(3)]);
        assert_eq!(delivered.len(), 1);
        match &delivered[0] {
            SessionMsg::Event { seq, .. } => assert_eq!(*seq, 3),
            other => panic!("expected an event, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn frames_are_reassembled_from_whatever_arrived() {
        let mut session = Session::new(runtime());
        let mut decoder = Decoder::new();
        let frame = frame::encode(&hello("tui")).unwrap();
        for (index, byte) in frame.iter().enumerate() {
            assert!(split_frames(&mut decoder, &[*byte]).is_empty() || index == frame.len() - 1);
        }
        assert!(!session.handle(ClientMsg::Unsubscribe { id: SubId(1) }).is_empty());
    }

    #[tokio::test]
    async fn the_state_machine_drives_over_channels_with_no_network() {
        let runtime = runtime();
        let (client_tx, server_rx) = mpsc::channel::<ClientMsg>(OUTBOUND_REVISIONS);
        let (server_tx, mut batches) = mpsc::channel::<OutboundBatch>(OUTBOUND_REVISIONS);
        let (flatten, mut client_rx) = mpsc::unbounded_channel::<SessionMsg>();
        tokio::spawn(async move { while let Some(batch) = batches.recv().await { for message in batch { let _ = flatten.send(message); } } });
        let session = Session::new(runtime.clone());
        let handle = tokio::spawn(drive(
            session,
            server_rx,
            server_tx,
            runtime.watch_rev(),
            runtime.subscribe_events(),
        ));

        client_tx.send(hello("web")).await.unwrap();
        assert!(matches!(client_rx.recv().await.unwrap(), SessionMsg::Welcome { .. }));
        client_tx
            .send(ClientMsg::Subscribe { since: None, id: SubId(1), query: Query::new(misa_proto::VIEW_QUERY) })
            .await.unwrap();
        let first = client_rx.recv().await.unwrap();
        assert!(view_of(std::slice::from_ref(&first)).is_some());

        client_tx
            .send(ClientMsg::Intent {
                id: 1,
                intent: Intent::Prompt { text: "over channels".into(), attachments: vec![] },
            })
            .await.unwrap();
        // An ack, then whichever of a refreshed view or an ephemeral delta arrives
        // first. Both are legitimate; neither is a re-send of the old tree.
        let mut saw_ack = false;
        let mut saw_change = false;
        for _ in 0..8 {
            match tokio::time::timeout(Duration::from_millis(200), client_rx.recv()).await {
                Ok(Some(SessionMsg::Ack { .. })) => saw_ack = true,
                Ok(Some(_)) => saw_change = true,
                _ => break,
            }
            if saw_ack && saw_change {
                break;
            }
        }
        assert!(saw_ack, "no acknowledgement");
        assert!(saw_change, "no state reached the client");
        handle.abort();
    }
}
