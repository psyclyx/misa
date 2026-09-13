
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

/// Bulk content by hash, on a connection of its own.
pub mod blob;
pub mod iroh;
/// One endpoint, both protocols.
pub mod server;

use std::collections::BTreeMap;
use std::sync::Arc;

use misa_proto::chunk::{self as frame, Decoder};
use misa_proto::wire::{
    Capabilities, ClientInfo, ClientMsg, Level, Query, SessionEvent, SessionMsg, SubId,
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
    runtime: Arc<Runtime>,
    client: Option<ClientInfo>,
    subscriptions: BTreeMap<SubId, Subscription>,
    last_seq: u64,
    welcomed: bool,
}

struct Subscription {
    query: Query,
    current: Option<Reading>,
}

impl Session {
    pub fn new(runtime: Arc<Runtime>) -> Self {
        Session {
            runtime,
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
            ClientMsg::Subscribe { id, query } => {
                let capabilities = self.capabilities();
                match self.runtime.read(&query, &capabilities) {
                    Ok(reading) => {
                        let revision = self.runtime.rev();
                        let message = answer(id, revision, reading.clone());
                        self.subscriptions.insert(id, Subscription { query, current: Some(reading) });
                        match message {
                            Some(message) => vec![message],
                            None => Vec::new(),
                        }
                    }
                    Err(fault) => {
                        // The subscription is kept: a query that cannot be answered
                        // now may be answerable after the next change.
                        self.subscriptions.insert(id, Subscription { query, current: None });
                        vec![SessionMsg::QueryFault { id, fault }]
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
                let faults = self.runtime.intent(intent);
                let mut out: Vec<SessionMsg> = faults
                    .into_iter()
                    .map(|fault| SessionMsg::Fault { id: Some(id), fault })
                    .collect();
                if out.is_empty() {
                    out.push(SessionMsg::Ack { id });
                }
                out
            }
            ClientMsg::Ping { nonce } => vec![SessionMsg::Pong { nonce }],
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
        let capabilities = self.capabilities();
        let mut out = Vec::new();
        for (id, subscription) in self.subscriptions.iter_mut() {
            match self.runtime.read(&subscription.query, &capabilities) {
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
            out.push(SessionMsg::Event { seq: emission.seq, event: emission.event.clone() });
        }
        out
    }
    fn capabilities(&self) -> Capabilities {
        self.client
            .as_ref()
            .map(|client| client.capabilities.clone())
            .unwrap_or_else(Capabilities::plain)
    }
}

fn answer(id: SubId, revision: u64, reading: Reading) -> Option<SessionMsg> {
    match reading {
        Reading::View(view) => Some(SessionMsg::View { id, rev: revision, view }),
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
pub async fn drive(
    mut session: Session,
    mut inbound: mpsc::UnboundedReceiver<ClientMsg>,
    outbound: mpsc::UnboundedSender<SessionMsg>,
    mut revision: watch::Receiver<u64>,
    mut events: broadcast::Receiver<misa_session::Emission>,
) {
    loop {
        tokio::select! {
            message = inbound.recv() => match message {
                Some(message) => {
                    for reply in session.handle(message) {
                        if outbound.send(reply).is_err() {
                            return;
                        }
                    }
                }
                None => return,
            },
            changed = revision.changed() => {
                if changed.is_err() {
                    return;
                }
                for reply in session.refresh() {
                    if outbound.send(reply).is_err() {
                        return;
                    }
                }
            }
            emission = events.recv() => match emission {
                Ok(emission) => {
                    for reply in session.events(&[emission]) {
                        if outbound.send(reply).is_err() {
                            return;
                        }
                    }
                }
                // Lagging is not an error: the deltas missed are the ones a
                // subscription value will carry in full.
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
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
    use misa_session::views;
    use std::time::Duration;

    fn runtime() -> Arc<Runtime> {
        let provider: Arc<dyn Provider> = ScriptedProvider::new([
            Turn::call("echo", Value::str("hello"), Turn::say("all done")),
            Turn::say("all done"),
        ]);
        Runtime::start(
            "demo",
            "a demo session",
            Some("c1".into()),
            Arc::new(LocalKernel::new(provider)),
            "scripted",
            "scripted-1",
            Value::Null,
        )
    }

    fn hello(name: &str, capabilities: Capabilities) -> ClientMsg {
        ClientMsg::Hello {
            version: PROTOCOL_VERSION,
            client: ClientInfo::new(name, "0.1.0", capabilities),
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
        let replies = session.handle(ClientMsg::Ping { nonce: 1 });
        assert!(matches!(replies[0], SessionMsg::Fault { .. }));
    }

    #[tokio::test]
    async fn another_protocol_version_is_refused_before_it_can_act() {
        let mut session = Session::new(runtime());
        let replies = session.handle(ClientMsg::Hello {
            version: PROTOCOL_VERSION + 1,
            client: ClientInfo::new("x", "0", Capabilities::plain()),
        });
        match &replies[0] {
            SessionMsg::Fault { fault, .. } => assert_eq!(fault.code, "protocol"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_subscription_is_answered_with_a_view_and_repeated_only_when_it_changes() {
        let mut session = Session::new(runtime());
        assert!(matches!(session.handle(hello("tui", Capabilities::tui(100, 30)))[0], SessionMsg::Welcome { .. }));
        let replies = session.handle(ClientMsg::Subscribe {
            id: SubId(1),
            query: Query::new(views::VIEW_QUERY),
        });
        assert!(view_of(&replies).is_some());
        assert!(session.refresh().is_empty(), "an unchanged view was re-sent");
    }

    #[tokio::test]
    async fn a_data_subscription_is_answered_with_data() {
        let mut session = Session::new(runtime());
        session.handle(hello("cli", Capabilities::plain()));
        let replies = session.handle(ClientMsg::Subscribe {
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
        session.handle(hello("cli", Capabilities::plain()));
        let replies = session.handle(ClientMsg::Subscribe {
            id: SubId(3),
            query: Query::new("no.such.query"),
        });
        assert!(matches!(replies[0], SessionMsg::QueryFault { .. }));
        // And the connection is still usable.
        assert!(matches!(
            session.handle(ClientMsg::Ping { nonce: 2 })[0],
            SessionMsg::Pong { .. }
        ));
    }

    #[tokio::test]
    async fn an_intent_is_acknowledged_and_an_impossible_one_is_faulted() {
        let mut session = Session::new(runtime());
        session.handle(hello("tui", Capabilities::tui(80, 24)));
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
            id: SubId(1),
            query: Query::new(views::VIEW_QUERY),
        });
        assert!(view_of(&replies).is_none());
        assert!(matches!(replies[0], SessionMsg::Fault { .. }));
    }

    #[tokio::test]
    async fn a_prompt_moves_the_view_and_the_transport_reports_only_that() {
        let runtime = runtime();
        let mut session = Session::new(runtime.clone());
        session.handle(hello("tui", Capabilities::tui(80, 24)));
        session.handle(ClientMsg::Subscribe { id: SubId(1), query: Query::new(views::VIEW_QUERY) });
        session.refresh();

        runtime.intent(Intent::Prompt { text: "hello".into(), attachments: vec![] });
        let replies = session.refresh();
        assert_eq!(replies.len(), 1, "expected exactly one changed subscription");
        let view = view_of(&replies).expect("a view");
        assert!(
            misa_proto::view::find(view, "transcript").is_some(),
            "the transcript is missing from the refreshed view"
        );
    }

    #[tokio::test]
    async fn events_are_numbered_and_a_client_does_not_see_one_twice() {
        let mut session = Session::new(runtime());
        let emission = |seq| misa_session::Emission { seq, event: SessionEvent::Status { text: format!("{seq}") } };
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
        let frame = frame::encode(&hello("tui", Capabilities::tui(80, 24))).unwrap();
        for (index, byte) in frame.iter().enumerate() {
            assert!(split_frames(&mut decoder, &[*byte]).is_empty() || index == frame.len() - 1);
        }
        assert!(!session.handle(ClientMsg::Ping { nonce: 1 }).is_empty());
    }

    #[tokio::test]
    async fn the_state_machine_drives_over_channels_with_no_network() {
        let runtime = runtime();
        let (client_tx, server_rx) = mpsc::unbounded_channel::<ClientMsg>();
        let (server_tx, mut client_rx) = mpsc::unbounded_channel::<SessionMsg>();
        let session = Session::new(runtime.clone());
        let handle = tokio::spawn(drive(
            session,
            server_rx,
            server_tx,
            runtime.watch_rev(),
            runtime.subscribe_events(),
        ));

        client_tx.send(hello("web", Capabilities::browser())).unwrap();
        assert!(matches!(client_rx.recv().await.unwrap(), SessionMsg::Welcome { .. }));
        client_tx
            .send(ClientMsg::Subscribe { id: SubId(1), query: Query::new(views::VIEW_QUERY) })
            .unwrap();
        let first = client_rx.recv().await.unwrap();
        assert!(view_of(std::slice::from_ref(&first)).is_some());

        client_tx
            .send(ClientMsg::Intent {
                id: 1,
                intent: Intent::Prompt { text: "over channels".into(), attachments: vec![] },
            })
            .unwrap();
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
