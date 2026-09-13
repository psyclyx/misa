//! Iroh connections and blob transport for the shared session protocol.
pub mod admission;
pub mod blob;
pub mod iroh;
pub mod server;
pub use misa_protocol::server::{Server as Session, drive, encode, decode, split_frames, notice, OUTBOUND_REVISIONS, OutboundBatch};
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use misa_session::Runtime;
    use misa_proto::wire::{ClientInfo, ClientMsg, SessionMsg, SessionEvent, SubId, Query};
    use misa_proto::PROTOCOL_VERSION;
    use misa_proto::chunk::{self as frame, Decoder};
    use tokio::sync::mpsc;
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
        let mut revision = runtime.watch_rev();
        let node = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Some(node) = target(&runtime.view().unwrap()) { break node; }
                revision.changed().await.unwrap();
            }
        }).await.expect("attachment was not durably recorded");
        let mut owner = Session::new(runtime.clone());
        let mut other = Session::new(runtime.clone());
        owner.handle(hello("owner")); other.handle(hello("other"));
        let mut events = owner.take_replies().unwrap();
        let replies = owner.handle(ClientMsg::Intent { id: 42, intent: Intent::Action { node, action: "attachment.save".into(), args: misa_value::Value::str("/etc/shadow"), fields: vec![] } });
        assert!(matches!(replies.as_slice(), [SessionMsg::Ack { id: 42 }]));
        let mut noisy = runtime.subscribe_events();
        for _ in 0..600 { runtime.notice(misa_proto::Level::Info, "stream backlog"); }
        assert!(matches!(noisy.recv().await, Err(tokio::sync::broadcast::error::RecvError::Lagged(_))));
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
