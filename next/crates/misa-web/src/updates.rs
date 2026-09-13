//! Browser changes are rendered once per changed subtree. Snapshots are materialized only
//! for a new subscriber, a lagged subscriber, or a protocol reset.
use axum::response::sse::{Event, KeepAlive, Sse};
use misa_proto::{
    SessionEvent, SessionMsg,
    sync::{ClientView, ViewOp},
};
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;

#[derive(Clone, Debug)]
struct Message {
    name: Option<&'static str>,
    data: String,
}
impl Message {
    fn event(&self, lead: &str) -> Event {
        match self.name {
            Some(name) => Event::default().event(name).data(&self.data),
            None => Event::default().data(format!("{lead}{}", self.data)),
        }
    }
}
#[derive(Clone, Debug)]
struct Batch {
    sequence: u64,
    messages: Vec<Message>,
}
#[derive(Default)]
struct State {
    view: ClientView,
    html: Option<String>,
    sequence: u64,
}
#[derive(Clone)]
pub struct Region {
    state: Arc<Mutex<State>>,
    updates: broadcast::Sender<Batch>,
}
impl Default for Region {
    fn default() -> Self {
        Self::new()
    }
}
impl Region {
    pub fn new() -> Self {
        let (updates, _) = broadcast::channel(64);
        Self {
            state: Arc::new(Mutex::new(State::default())),
            updates,
        }
    }
    /// Compatibility for static regions and fixtures.
    pub fn set(&self, html: String) {
        let mut state = self.state.lock().unwrap();
        state.view = ClientView::default();
        state.html = Some(html.clone());
        self.publish(
            &mut state,
            vec![
                Message {
                    name: None,
                    data: html,
                },
                Message {
                    name: Some("streams"),
                    data: "[]".into(),
                },
            ],
        );
    }
    pub fn get(&self) -> Arc<String> {
        Arc::new(html(&self.state.lock().unwrap()))
    }
    pub fn receive(&self, message: &SessionMsg) -> Result<bool, String> {
        let mut state = self.state.lock().unwrap();
        if !state.view.receive(message)? {
            return Ok(false);
        }
        state.html = None;
        let messages = match message {
            SessionMsg::View { view, .. } => vec![
                Message {
                    name: None,
                    data: crate::render_main(view),
                },
                Message {
                    name: Some("streams"),
                    data: "[]".into(),
                },
            ],
            SessionMsg::Changes { changes, .. } => {
                let ops: Vec<_> = changes.iter().flat_map(|change| &change.ops).map(|op| match op {
                    ViewOp::Insert { parent, before, node } => serde_json::json!({"op":"insert","parent":parent,"before":before,"html":crate::render_main(node)}),
                    ViewOp::Remove { id } => serde_json::json!({"op":"remove","id":id}),
                    ViewOp::Replace { id, node } => serde_json::json!({"op":"replace","id":id,"html":crate::render_main(node)}),
                }).collect();
                vec![Message {
                    name: Some("changes"),
                    data: serde_json::to_string(&ops).unwrap(),
                }]
            }
            SessionMsg::Streams { streams } => vec![Message {
                name: Some("streams"),
                data: serde_json::to_string(streams).unwrap(),
            }],
            SessionMsg::Event {
                event: SessionEvent::Stream { update },
                ..
            } => vec![Message {
                name: Some("stream"),
                data: serde_json::to_string(update).unwrap(),
            }],
            _ => vec![],
        };
        self.publish(&mut state, messages);
        Ok(true)
    }
    fn publish(&self, state: &mut State, messages: Vec<Message>) {
        state.sequence += 1;
        let _ = self.updates.send(Batch {
            sequence: state.sequence,
            messages,
        });
    }
    fn snapshot(&self) -> Batch {
        let state = self.state.lock().unwrap();
        Batch {
            sequence: state.sequence,
            messages: vec![
                Message {
                    name: None,
                    data: html(&state),
                },
                Message {
                    name: Some("streams"),
                    data: serde_json::to_string(&state.view.streams()).unwrap(),
                },
            ],
        }
    }
    fn subscribe(&self) -> Subscription {
        // Register before capturing state; sequence filtering removes the overlap.
        let receiver = self.updates.subscribe();
        let snapshot = self.snapshot();
        Subscription {
            region: self.clone(),
            receiver,
            sequence: snapshot.sequence,
            pending: snapshot.messages.into(),
        }
    }
    pub fn events(
        &self,
        lead: String,
    ) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>> + use<>>
    {
        let stream = futures::stream::unfold(
            (self.subscribe(), lead),
            |(mut subscription, lead)| async move {
                subscription
                    .next()
                    .await
                    .map(|message| (Ok(message.event(&lead)), (subscription, lead)))
            },
        );
        Sse::new(stream).keep_alive(KeepAlive::default())
    }
}
fn html(state: &State) -> String {
    state.html.clone().unwrap_or_else(|| {
        state
            .view
            .canonical()
            .map(|view| crate::render_main(&view))
            .unwrap_or_default()
    })
}
struct Subscription {
    region: Region,
    receiver: broadcast::Receiver<Batch>,
    sequence: u64,
    pending: VecDeque<Message>,
}
impl Subscription {
    async fn next(&mut self) -> Option<Message> {
        loop {
            if let Some(message) = self.pending.pop_front() {
                return Some(message);
            }
            let batch = match self.receiver.recv().await {
                Ok(batch) if batch.sequence <= self.sequence => continue,
                Ok(batch) => batch,
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    self.receiver = self.region.updates.subscribe();
                    self.region.snapshot()
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            };
            self.sequence = batch.sequence;
            self.pending = batch.messages.into();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{
        SubId,
        sync::{Change, Stream, StreamUpdate, Version},
        view::{Node, Span},
    };
    fn version(rev: u64) -> Version {
        Version {
            epoch: "test".into(),
            rev,
        }
    }
    fn snapshot(region: &Region, count: usize) {
        let tree = Node::section("root")
            .id("root")
            .children((0..count).map(|index| {
                Node::text("text", [Span::plain("old transcript ".repeat(80))])
                    .id(format!("old-{index}"))
            }));
        region
            .receive(&SessionMsg::View {
                id: SubId(1),
                version: version(0),
                view: tree,
            })
            .unwrap();
    }
    fn append(region: &Region, rev: u64) {
        region
            .receive(&SessionMsg::Changes {
                id: SubId(1),
                changes: vec![Change {
                    from: version(rev - 1),
                    version: version(rev),
                    ops: vec![ViewOp::Insert {
                        parent: "root".into(),
                        before: None,
                        node: Node::text("text", [Span::plain("new text")])
                            .id(format!("new-{rev}")),
                    }],
                }],
            })
            .unwrap();
    }
    #[tokio::test]
    async fn append_bytes_do_not_grow_with_the_existing_transcript() {
        let mut sizes = vec![];
        for count in [1, 1000] {
            let region = Region::new();
            snapshot(&region, count);
            let mut subscriber = region.subscribe();
            subscriber.next().await.unwrap();
            subscriber.next().await.unwrap();
            append(&region, 1);
            let message = subscriber.next().await.unwrap();
            assert_eq!(message.name, Some("changes"));
            assert!(!message.data.contains("old transcript"));
            sizes.push(message.data.len());
        }
        assert_eq!(sizes[0], sizes[1]);
        assert!(sizes[0] < 200);
    }
    #[tokio::test]
    async fn lag_recovers_canonical_html_and_separate_current_streams() {
        let region = Region::new();
        snapshot(&region, 1);
        let mut subscriber = region.subscribe();
        subscriber.next().await.unwrap();
        subscriber.next().await.unwrap();
        for rev in 1..=70 {
            append(&region, rev);
        }
        region
            .receive(&SessionMsg::Streams {
                streams: vec![Stream {
                    id: "live.text".into(),
                    role: "message.assistant".into(),
                    text: "live-only".into(),
                }],
            })
            .unwrap();
        let recovered = subscriber.next().await.unwrap();
        assert_eq!(recovered.name, None);
        assert!(recovered.data.contains("new-70"));
        assert!(!recovered.data.contains("live-only"));
        let streams = subscriber.next().await.unwrap();
        assert_eq!(streams.name, Some("streams"));
        assert!(streams.data.contains("live-only"));
        append(&region, 71);
        let next = subscriber.next().await.unwrap();
        assert_eq!(next.name, Some("changes"));
        assert!(next.data.contains("new-71"));
    }
    #[tokio::test]
    async fn stream_append_sends_only_the_offset_and_new_bytes() {
        let region = Region::new();
        snapshot(&region, 10);
        region
            .receive(&SessionMsg::Streams {
                streams: vec![Stream {
                    id: "live.text".into(),
                    role: "text".into(),
                    text: "a".repeat(10000),
                }],
            })
            .unwrap();
        let mut subscriber = region.subscribe();
        subscriber.next().await.unwrap();
        subscriber.next().await.unwrap();
        region
            .receive(&SessionMsg::Event {
                seq: 1,
                event: SessionEvent::Stream {
                    update: StreamUpdate::Append {
                        id: "live.text".into(),
                        offset: 10000,
                        text: "λ".into(),
                    },
                },
            })
            .unwrap();
        let event = subscriber.next().await.unwrap();
        assert_eq!(event.name, Some("stream"));
        assert!(event.data.len() < 100);
        assert!(event.data.contains("10000"));
    }
    #[tokio::test]
    async fn a_subscriber_cannot_miss_an_update_after_its_snapshot() {
        let region = Region::new();
        snapshot(&region, 0);
        let mut subscriber = region.subscribe();
        append(&region, 1);
        assert!(!subscriber.next().await.unwrap().data.contains("new-1"));
        assert_eq!(subscriber.next().await.unwrap().name, Some("streams"));
        assert!(subscriber.next().await.unwrap().data.contains("new-1"));
    }
    #[tokio::test]
    async fn replacements_and_removals_render_only_their_targets() {
        let region = Region::new();
        snapshot(&region, 2);
        let mut subscriber = region.subscribe();
        subscriber.next().await.unwrap();
        subscriber.next().await.unwrap();
        region
            .receive(&SessionMsg::Changes {
                id: SubId(1),
                changes: vec![Change {
                    from: version(0),
                    version: version(1),
                    ops: vec![
                        ViewOp::Replace {
                            id: "old-0".into(),
                            node: Node::text("text", [Span::plain("<replacement>")]).id("old-0"),
                        },
                        ViewOp::Remove { id: "old-1".into() },
                    ],
                }],
            })
            .unwrap();
        let event = subscriber.next().await.unwrap();
        let ops: serde_json::Value = serde_json::from_str(&event.data).unwrap();
        assert_eq!(ops[0]["op"], "replace");
        assert!(
            ops[0]["html"]
                .as_str()
                .unwrap()
                .contains("&lt;replacement&gt;")
        );
        assert_eq!(ops[1], serde_json::json!({"op":"remove","id":"old-1"}));
        let snapshot = region.get();
        assert!(!snapshot.contains("old-1"));
        assert!(snapshot.contains("&lt;replacement&gt;"));
    }
}
