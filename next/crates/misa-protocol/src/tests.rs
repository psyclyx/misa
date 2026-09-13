use std::sync::{Arc, Mutex};
use std::collections::BTreeMap;
use misa_proto::{ClientInfo, ClientMsg, SessionMsg, SessionInfo, Node, Query, Fault, PROTOCOL_VERSION, SubId};
use misa_proto::wire::{Intent, RequestContext, SessionEvent, Download};
use misa_proto::view::Choice;
use misa_proto::sync::{Version, ViewSync, Stream, Change, ViewOp, ClientView};
use misa_value::Value;
use tokio::sync::{broadcast, watch};
use crate::{Session, Server, Reading, Emission};

struct Fake {
    current: Mutex<(Version, Node, Vec<Change>)>,
    clients: Mutex<BTreeMap<u64, ClientInfo>>,
    requests: Mutex<Vec<RequestContext>>,
    revision: watch::Sender<u64>,
    events: broadcast::Sender<Emission>,
    streams: Mutex<Vec<Stream>>,
}
impl Fake {
    fn new() -> Arc<Self> { Arc::new(Self {
        current: Mutex::new((Version {epoch: "one".into(), rev: 0}, Node::section("session").id("session"), vec![])),
        clients: Mutex::new(BTreeMap::new()), requests: Mutex::new(vec![]),
        revision: watch::channel(0).0, events: broadcast::channel(16).0,
        streams: Mutex::new(vec![]),
    }) }
    fn append(&self, id: &str) {
        let mut current = self.current.lock().unwrap();
        let from = current.0.clone();
        current.0.rev += 1;
        let version = current.0.clone();
        let node = Node::section("item").id(id);
        current.1.children.push(node.clone());
        current.2.push(Change { from, version, ops: vec![ViewOp::Insert {parent: "session".into(), before: None, node}] });
        self.revision.send_replace(current.0.rev);
    }
}
impl Session for Fake {
    fn read(&self, query: &Query) -> Result<Reading, Fault> {
        if query.id == "known" { Ok(Reading::Data(Value::Int(42))) } else { Err(Fault::protocol("missing query")) }
    }
    fn rev(&self) -> u64 { self.current.lock().unwrap().0.rev }
    fn info(&self) -> SessionInfo { SessionInfo {
        id: "fake".into(), title: "Fake".into(), conversation: None, created_ms: 0,
        policy: vec![], queries: vec![misa_proto::VIEW_QUERY.into()], commands: vec![], sources: vec![],
    } }
    fn intent_from(&self, _: Intent, context: Option<RequestContext>) -> Vec<Fault> {
        self.requests.lock().unwrap().push(context.unwrap()); vec![]
    }
    fn complete(&self, _: &str, _: &str, _: Option<u32>) -> Result<(Vec<Choice>, bool), Fault> { Ok((vec![], false)) }
    fn sync(&self, since: Option<&Version>) -> (ViewSync, u64) {
        let current = self.current.lock().unwrap();
        let view = match since {
            Some(since) if since.epoch == current.0.epoch && since.rev <= current.0.rev => ViewSync::Changes {
                version: current.0.clone(), changes: current.2.iter().filter(|change| change.from.rev >= since.rev).cloned().collect(), streams: vec![],
            },
            _ => ViewSync::Snapshot { version: current.0.clone(), view: current.1.clone(), streams: self.streams.lock().unwrap().clone() },
        };
        (view, 10)
    }
    fn changes(&self, since: Option<&Version>) -> ViewSync { self.sync(since).0 }
    fn streams(&self) -> Vec<Stream> { vec![] }
    fn watch_rev(&self) -> watch::Receiver<u64> { self.revision.subscribe() }
    fn subscribe_events(&self) -> broadcast::Receiver<Emission> { self.events.subscribe() }
    fn attached(&self, id: u64, client: ClientInfo) { self.clients.lock().unwrap().insert(id, client); }
    fn detached(&self, id: u64) { self.clients.lock().unwrap().remove(&id); }
}
fn hello() -> ClientMsg { ClientMsg::Hello { version: PROTOCOL_VERSION, client: ClientInfo::new("fake-client", "1") } }
fn subscribe(since: Option<Version>) -> ClientMsg { ClientMsg::Subscribe {id: SubId(1), query: Query::new(misa_proto::VIEW_QUERY), since} }

#[test]
fn handshake_gates_requests_and_tracks_client_lifetime() {
    let fake = Fake::new();
    let mut server = Server::new(fake.clone());
    assert!(matches!(server.handle(subscribe(None))[0], SessionMsg::Fault {..}));
    assert!(matches!(server.handle(hello())[0], SessionMsg::Welcome {..}));
    assert_eq!(fake.clients.lock().unwrap().values().next().unwrap().version, "1");
    drop(server);
    assert!(fake.clients.lock().unwrap().is_empty());
}
#[test]
fn reconnect_replays_changes_and_foreign_epoch_gets_snapshot() {
    let fake = Fake::new();
    let mut server = Server::new(fake.clone()); server.handle(hello());
    let mut view = ClientView::default();
    for message in server.handle(subscribe(None)) { view.receive(&message).unwrap(); }
    let before = view.version().unwrap().clone();
    fake.append("a"); fake.append("b");
    for message in server.handle(subscribe(Some(before))) {
        assert!(!matches!(message, SessionMsg::View {..})); view.receive(&message).unwrap();
    }
    assert_eq!(view.canonical().unwrap(), fake.current.lock().unwrap().1);
    assert!(matches!(server.handle(subscribe(Some(Version {epoch: "other".into(), rev: 2})))[0], SessionMsg::View {..}));
}
#[test]
fn unchanged_values_are_silent_and_unknown_queries_are_correlated() {
    let fake = Fake::new(); let mut server = Server::new(fake); server.handle(hello());
    let request = |id, query| ClientMsg::Subscribe {id: SubId(id), query: Query::new(query), since: None};
    assert!(matches!(server.handle(request(4, "known"))[0], SessionMsg::Value {id: SubId(4), ..}));
    assert!(server.refresh().is_empty());
    assert!(matches!(server.handle(request(5, "missing"))[0], SessionMsg::QueryFault {id: SubId(5), ..}));
}

#[test]
fn refresh_snapshot_fallback_restores_streams_before_the_next_append() {
    use misa_proto::sync::StreamUpdate;
    let fake = Fake::new();
    let mut server = Server::new(fake.clone());
    server.handle(hello());
    let mut view = ClientView::default();
    for message in server.handle(subscribe(None)) { view.receive(&message).unwrap(); }
    fake.current.lock().unwrap().0.epoch = "replacement".into();
    *fake.streams.lock().unwrap() = vec![Stream { id: "live.text".into(), role: "message.assistant".into(), text: "é".into() }];
    let messages = server.refresh();
    assert!(matches!(messages.as_slice(), [SessionMsg::View {..}, SessionMsg::Streams {..}]));
    for message in messages { view.receive(&message).unwrap(); }
    let append = SessionMsg::Event { seq: 10, event: SessionEvent::Stream {
        update: StreamUpdate::Append {id: "live.text".into(), offset: 2, text: "!".into()},
    }};
    assert!(view.receive(&append).unwrap());
}
#[test]
fn save_answers_are_delivered_only_to_the_requesting_connection() {
    let fake = Fake::new(); let mut first = Server::new(fake.clone()); let mut second = Server::new(fake.clone());
    first.handle(hello()); second.handle(hello());
    first.handle(ClientMsg::Intent {id: 7, intent: Intent::Prompt {text: "hello".into(), attachments: vec![]}});
    let context = fake.requests.lock().unwrap()[0];
    let emission = Emission {recipient: Some(context.recipient), seq: 1,
        event: SessionEvent::DownloadReady {id: context.id, download: Download {blob: None, name: "file".into(), error: "gone".into()}}};
    assert!(second.events(&[emission.clone()]).is_empty());
    assert!(matches!(first.events(&[emission.clone()])[0], SessionMsg::Download {id: 7, ..}));
    assert!(first.events(&[emission]).is_empty());
}

#[test]
fn queued_stream_events_cannot_rewind_a_newer_current_snapshot() {
    use misa_proto::sync::StreamUpdate;
    let fake = Fake::new(); let mut server = Server::new(fake); server.handle(hello());
    let snapshot = server.handle(subscribe(None));
    assert!(snapshot.iter().any(|message| matches!(message, SessionMsg::Streams {streams} if streams.is_empty())));
    let old = Emission {recipient: None, seq: 2, event: SessionEvent::Stream {
        update: StreamUpdate::Current {stream: Stream {id: "msg.1.text".into(), role: "assistant".into(), text: "obsolete".into()}},
    }};
    assert!(server.events(&[old]).is_empty(), "queued current resurrected a finished stream");
    let notice = Emission {recipient: None, seq: 3, event: SessionEvent::Status {text: "notice".into()}};
    assert_eq!(server.events(&[notice]).len(), 1, "the stream watermark dropped an unrelated notice");
    let fresh = Emission {recipient: None, seq: 10, event: SessionEvent::Stream {
        update: StreamUpdate::Current {stream: Stream {id: "msg.2.text".into(), role: "assistant".into(), text: "fresh".into()}},
    }};
    assert_eq!(server.events(&[fresh]).len(), 1);
}
