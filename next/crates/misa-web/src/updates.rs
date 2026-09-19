//! Browser changes are rendered once per changed subtree. Snapshots are materialized only
//! for a new subscriber, a lagged subscriber, or a protocol reset.
use axum::response::sse::{Event, KeepAlive, Sse};
use misa_proto::sync::{IndexedTree, Stream, StreamUpdate, ViewOp};
use std::{
    collections::{BTreeMap, VecDeque},
    sync::{Arc, Mutex},
};
use tokio::sync::broadcast;

#[derive(Clone, Debug)]
struct Message {
    name: Option<&'static str>,
    data: String,
}
#[derive(Clone, Debug)]
struct Batch {
    sequence: u64,
    messages: Vec<Message>,
}
#[derive(Default)]
struct State {
    closed: bool,
    activity: String,
    documents: BTreeMap<String, State>,
    valid: bool,
    notice: Option<String>,
    tree: Option<IndexedTree>,
    streams: BTreeMap<String, Stream>,
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
    pub(crate) fn activity_html(&self) -> String {
        self.state.lock().unwrap().activity.clone()
    }
    pub(crate) fn activity(&self, html: String) {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return;
        }
        if state.activity == html {
            return;
        }
        state.activity = html.clone();
        self.publish(
            &mut state,
            vec![Message {
                name: Some("activity"),
                data: html,
            }],
        );
    }
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
        if state.closed {
            return;
        }
        state.documents.clear();
        state.tree = None;
        state.streams.clear();
        state.html = Some(html.clone());
        state.valid = true;
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
    pub fn observed(&self, update: &misa_client::document::Update) -> Result<bool, String> {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return Err("Presentation is closed".into());
        }
        let messages = apply_document(&mut state, update, "")?;
        if messages.is_empty() {
            return Ok(false);
        }
        self.publish(&mut state, messages);
        Ok(true)
    }

    /// All readers must be captured from one replica publication before this
    /// call. No selected member is published until every cache update succeeds.
    pub fn observed_documents(
        &self,
        updates: Vec<(String, misa_client::document::Update)>,
    ) -> Result<bool, String> {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return Err("Presentation is closed".into());
        }
        if !state.valid
            && !state.documents.is_empty()
            && !state.documents.keys().all(|id| {
                updates.iter().any(|(member, update)| {
                    member == id && matches!(update, misa_client::document::Update::Reset(_))
                })
            })
        {
            return Err("Selected document caches require a complete replacement".into());
        }
        let mut documents = Vec::new();
        state.valid = false;
        for (id, update) in updates {
            let prefix = document_prefix(&id);
            let document = state.documents.entry(id.clone()).or_default();
            let messages = apply_document(document, &update, &prefix)?;
            if !messages.is_empty() {
                documents.push(document_message(&id, &prefix, messages));
            }
        }
        state.valid = true;
        if documents.is_empty() {
            return Ok(false);
        }
        self.publish(
            &mut state,
            vec![Message {
                name: Some("documents"),
                data: serde_json::to_string(&documents).unwrap(),
            }],
        );
        Ok(true)
    }

    /// Build the replacement off to the side. Existing content remains usable
    /// if any member fails validation or rendering.
    pub fn replace_documents(
        &self,
        updates: Vec<(String, misa_client::document::Update)>,
    ) -> Result<(), String> {
        if updates.is_empty()
            || updates
                .iter()
                .any(|(_, update)| !matches!(update, misa_client::document::Update::Reset(_)))
        {
            return Err("Replacement selection is not a complete current snapshot".into());
        }
        let candidate = Self::new();
        candidate.observed_documents(updates)?;
        let snapshot = candidate.snapshot();
        let mut replacement = std::mem::take(&mut *candidate.state.lock().unwrap());
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return Err("Presentation is closed".into());
        }
        replacement.sequence = state.sequence;
        replacement.activity = state.activity.clone();
        *state = replacement;
        self.publish(&mut state, snapshot.messages);
        Ok(())
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
        if state.closed {
            return Batch {
                sequence: state.sequence,
                messages: vec![Message {
                    name: Some("closed"),
                    data: "Presentation closed; session work continues".into(),
                }],
            };
        }
        if !state.documents.is_empty() && state.valid {
            return Batch {
                sequence: state.sequence,
                messages: vec![
                    Message {
                        name: Some("selection"),
                        data: serde_json::to_string(&state.documents.keys().collect::<Vec<_>>())
                            .unwrap(),
                    },
                    Message {
                        name: Some("documents"),
                        data: serde_json::to_string(
                            &state
                                .documents
                                .iter()
                                .map(|(id, document)| {
                                    let prefix = document_prefix(id);
                                    document_message(
                                        id,
                                        &prefix,
                                        document_snapshot(document, &prefix),
                                    )
                                })
                                .collect::<Vec<_>>(),
                        )
                        .unwrap(),
                    },
                ]
                .into_iter()
                .chain((!state.activity.is_empty()).then(|| Message {
                    name: Some("activity"),
                    data: state.activity.clone(),
                }))
                .collect(),
            };
        }
        let mut batch = Batch {
            sequence: state.sequence,
            messages: vec![
                Message {
                    name: None,
                    data: html(&state),
                },
                Message {
                    name: Some("streams"),
                    data: if state.valid {
                        serde_json::to_string(&state.streams.values().collect::<Vec<_>>()).unwrap()
                    } else {
                        "[]".into()
                    },
                },
            ],
        };
        if let Some(notice) = &state.notice {
            batch.messages.push(Message {
                name: Some("status"),
                data: notice.clone(),
            });
        }
        if !state.activity.is_empty() {
            batch.messages.push(Message {
                name: Some("activity"),
                data: state.activity.clone(),
            });
        }
        batch
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
            ended: false,
        }
    }
    pub fn events(
        &self,
        lead: String,
        lease: Arc<dyn Send + Sync>,
    ) -> Sse<impl tokio_stream::Stream<Item = Result<Event, std::convert::Infallible>> + use<>>
    {
        let stream = futures::stream::unfold(
            (self.subscribe(), lead, lease),
            |(mut subscription, lead, lease)| async move {
                subscription
                    .next_batch()
                    .await
                    .map(|messages| {
                        let values: Vec<_> = messages.into_iter().map(|message| {
                            serde_json::json!({
                                "kind": message.name.unwrap_or("snapshot"),
                                "data": if message.name.is_none() { format!("{lead}{}", message.data) } else { message.data },
                            })
                        }).collect();
                        (Ok(Event::default().event("transaction").data(serde_json::to_string(&values).unwrap())), (subscription, lead, lease))
                    })
            },
        );
        Sse::new(stream).keep_alive(KeepAlive::default())
    }
    pub(crate) fn close(&self) {
        let mut state = self.state.lock().unwrap();
        if state.closed {
            return;
        }
        state.closed = true;
        self.publish(
            &mut state,
            vec![Message {
                name: Some("closed"),
                data: "Presentation closed; session work continues".into(),
            }],
        );
    }
}
fn apply_document(
    state: &mut State,
    update: &misa_client::document::Update,
    prefix: &str,
) -> Result<Vec<Message>, String> {
    use misa_client::document::Update;
    use misa_protocol::observation::{Applied, MemberChange};
    let mut messages = Vec::new();
    match update {
        Update::Reset(document) => {
            state.valid = false;
            state.tree = Some(IndexedTree::new(document.tree.clone()));
            state.streams = document
                .streams
                .iter()
                .map(|stream| (stream.id.clone(), stream.clone()))
                .collect();
            state.html = None;
            messages.push(Message {
                name: None,
                data: crate::render_scoped(&document.tree, prefix),
            });
            messages.push(Message {
                name: Some("streams"),
                data: serde_json::to_string(&document.streams).unwrap(),
            });
        }
        Update::Changed { member, applied } => {
            let Applied::Changed(members) = applied.as_ref() else {
                return Ok(vec![]);
            };
            let Some(MemberChange::Document {
                tree,
                live,
                reset_live,
            }) = members.get(member)
            else {
                return Ok(vec![]);
            };
            state.valid = false;
            if !tree.is_empty() {
                let current = state
                    .tree
                    .as_mut()
                    .ok_or("Document cache needs a snapshot")?;
                let mut ops = Vec::new();
                for op in tree {
                    current.apply(op)?;
                    ops.push(match op {
                            ViewOp::Insert { parent, before, node } => serde_json::json!({"op":"insert","parent":format!("{prefix}{parent}"),"before":before.as_ref().map(|id|format!("{prefix}{id}")),"html":crate::render_scoped(node, prefix)}),
                            ViewOp::Remove { id } => serde_json::json!({"op":"remove","id":format!("{prefix}{id}")}),
                            ViewOp::Replace { id, node } => serde_json::json!({"op":"replace","id":format!("{prefix}{id}"),"html":crate::render_scoped(node, prefix)}),
                        });
                }
                state.html = None;
                messages.push(Message {
                    name: Some("changes"),
                    data: serde_json::to_string(&ops).unwrap(),
                });
            }
            if *reset_live {
                state.streams.clear();
                messages.push(Message {
                    name: Some("streams"),
                    data: "[]".into(),
                });
            }
            for update in live {
                match update {
                    StreamUpdate::Current { stream } => {
                        state.streams.insert(stream.id.clone(), stream.clone());
                    }
                    StreamUpdate::Append { id, offset, text } => {
                        let stream = state
                            .streams
                            .get_mut(id)
                            .ok_or("Live cache needs a snapshot")?;
                        if stream.text.len() != *offset {
                            return Err("Live cache has an invalid append offset".into());
                        }
                        stream.text.push_str(text);
                    }
                    StreamUpdate::End { id } => {
                        state.streams.remove(id);
                    }
                }
                messages.push(Message {
                    name: Some("stream"),
                    data: serde_json::to_string(update).unwrap(),
                });
            }
        }
        Update::Unavailable(fault) => {
            state.notice = Some(fault.message.clone());
            return Ok(vec![Message {
                name: Some("status"),
                data: fault.message.clone(),
            }]);
        }
        Update::Status(status) => {
            use misa_protocol::observation::Status;
            let text = match status {
                Status::Awaiting => "Loading session…".into(),
                Status::Current => String::new(),
                Status::Recovering(_) => "Refreshing session…".into(),
                Status::Stale(fault) | Status::Closed(fault) => fault.message.clone(),
            };
            state.notice = (!text.is_empty()).then(|| text.clone());
            return Ok(vec![Message {
                name: Some("status"),
                data: text,
            }]);
        }
    }
    state.valid = true;
    state.notice = None;
    Ok(messages)
}

fn html(state: &State) -> String {
    if !state.valid {
        return "<p data-session-loading role=\"status\">Loading session…</p>".into();
    }
    if !state.documents.is_empty() {
        return state
            .documents
            .iter()
            .map(|(id, document)| {
                format!(
                    "<section data-presentation=\"{}\">{}</section>",
                    crate::escape(id),
                    document_html(document, &document_prefix(id))
                )
            })
            .collect();
    }
    state.html.clone().unwrap_or_else(|| {
        state
            .tree
            .as_ref()
            .map(IndexedTree::snapshot)
            .map(|view| crate::render_main(&view))
            .unwrap_or_default()
    })
}

fn document_prefix(id: &str) -> String {
    // Conversation keeps the existing composer/transcript DOM affordances.
    if id == "conversation" {
        return String::new();
    }
    format!(
        "document-{}:",
        id.as_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}
fn document_html(state: &State, prefix: &str) -> String {
    if !state.valid {
        return "<p role=\"status\">Loading presentation…</p>".into();
    }
    state
        .tree
        .as_ref()
        .map(|tree| crate::render_scoped(&tree.snapshot(), prefix))
        .unwrap_or_default()
}
fn document_snapshot(state: &State, prefix: &str) -> Vec<Message> {
    let mut messages = vec![
        Message {
            name: None,
            data: document_html(state, prefix),
        },
        Message {
            name: Some("streams"),
            data: serde_json::to_string(&state.streams.values().collect::<Vec<_>>()).unwrap(),
        },
    ];
    if let Some(notice) = &state.notice {
        messages.push(Message {
            name: Some("status"),
            data: notice.clone(),
        });
    }
    messages
}
fn document_message(id: &str, prefix: &str, messages: Vec<Message>) -> serde_json::Value {
    serde_json::json!({"id":id,"prefix":prefix,"messages":messages.into_iter().map(|message| serde_json::json!({"kind":message.name.unwrap_or("snapshot"),"data":message.data})).collect::<Vec<_>>()})
}
struct Subscription {
    ended: bool,
    region: Region,
    receiver: broadcast::Receiver<Batch>,
    sequence: u64,
    pending: VecDeque<Message>,
}
impl Subscription {
    async fn next_batch(&mut self) -> Option<Vec<Message>> {
        let first = self.next().await?;
        Some(
            std::iter::once(first)
                .chain(self.pending.drain(..))
                .collect(),
        )
    }
    async fn next(&mut self) -> Option<Message> {
        loop {
            if self.ended {
                return None;
            }
            if let Some(message) = self.pending.pop_front() {
                if message.name == Some("closed") {
                    self.ended = true;
                }
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
        sync::{Stream, StreamUpdate, Version},
        view::{Node, Span},
    };
    #[tokio::test]
    async fn closing_presentation_ends_existing_and_future_subscriptions() {
        let region = Region::new();
        let mut existing = region.subscribe();
        existing.next_batch().await.unwrap();
        region.close();
        region.activity("late work summary".into());
        let batch = existing.next_batch().await.unwrap();
        assert_eq!(batch.len(), 1);
        assert_eq!(batch[0].name, Some("closed"));
        assert!(existing.next_batch().await.is_none());
        let mut late = region.subscribe();
        assert_eq!(late.next_batch().await.unwrap()[0].name, Some("closed"));
        assert!(late.next_batch().await.is_none());
    }
    #[tokio::test]
    async fn selected_documents_publish_together_and_failure_hides_partial_cache() {
        use misa_client::document::Update;
        let region = Region::new();
        let reset = |text: &str| {
            Update::Reset(misa_proto::observation::Document {
                tree: Node::text("value", [Span::plain(text)]).id("same"),
                version: version(0),
                streams: vec![],
            })
        };
        region
            .observed_documents(vec![
                ("conversation".into(), reset("first")),
                ("status".into(), reset("second")),
            ])
            .unwrap();
        let mut subscriber = region.subscribe();
        let batch = subscriber.next_batch().await.unwrap();
        let documents: serde_json::Value = serde_json::from_str(
            &batch
                .iter()
                .find(|message| message.name == Some("documents"))
                .unwrap()
                .data,
        )
        .unwrap();
        assert_eq!(documents.as_array().unwrap().len(), 2);
        assert!(region.get().contains("id=\"same\""));
        assert!(region.get().contains("id=\"document-737461747573:same\""));
        let invalid = Update::Changed {
            member: "status".into(),
            applied: Arc::new(misa_protocol::observation::Applied::Changed(
                BTreeMap::from([(
                    "status".into(),
                    misa_protocol::observation::MemberChange::Document {
                        tree: vec![ViewOp::Remove {
                            id: "missing".into(),
                        }],
                        live: vec![],
                        reset_live: false,
                    },
                )]),
            )),
        };
        assert!(
            region
                .observed_documents(vec![
                    ("conversation".into(), reset("unpublished")),
                    ("status".into(), invalid)
                ])
                .is_err()
        );
        assert!(!region.get().contains("unpublished"));
        assert!(
            subscriber.receiver.try_recv().is_err(),
            "failed composition must publish nothing"
        );
        assert!(
            region
                .observed_documents(vec![("status".into(), reset("partial repair"))])
                .is_err()
        );
        assert!(!region.get().contains("unpublished"));
        region
            .observed_documents(vec![
                ("conversation".into(), reset("recovered")),
                ("status".into(), reset("coherent")),
            ])
            .unwrap();
        assert!(region.get().contains("recovered") && region.get().contains("coherent"));
    }

    #[tokio::test]
    async fn sse_delivery_preserves_complete_publication_batches() {
        let region = Region::new();
        region.set("first".into());
        let mut subscriber = region.subscribe();
        region.set("second".into());
        let first = subscriber.next_batch().await.unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].data, "first");
        assert_eq!(first[1].name, Some("streams"));
        let second = subscriber.next_batch().await.unwrap();
        assert_eq!(second.len(), 2);
        assert_eq!(second[0].data, "second");
        assert_eq!(second[1].data, "[]");
    }
    fn version(rev: u64) -> Version {
        Version {
            epoch: "test".into(),
            rev,
        }
    }
    fn changes(region: &Region, tree: Vec<ViewOp>, live: Vec<StreamUpdate>, reset_live: bool) {
        region
            .observed(&misa_client::document::Update::Changed {
                member: "body".into(),
                applied: Arc::new(misa_protocol::observation::Applied::Changed(
                    BTreeMap::from([(
                        "body".into(),
                        misa_protocol::observation::MemberChange::Document {
                            tree,
                            live,
                            reset_live,
                        },
                    )]),
                )),
            })
            .unwrap();
    }
    fn streams(region: &Region, streams: Vec<Stream>) {
        changes(
            region,
            vec![],
            streams
                .into_iter()
                .map(|stream| StreamUpdate::Current { stream })
                .collect(),
            true,
        );
    }
    fn snapshot(region: &Region, count: usize) {
        let tree = Node::section("root")
            .id("root")
            .children((0..count).map(|index| {
                Node::text("text", [Span::plain("old transcript ".repeat(80))])
                    .id(format!("old-{index}"))
            }));
        region
            .observed(&misa_client::document::Update::Reset(
                misa_proto::observation::Document {
                    version: version(0),
                    tree,
                    streams: vec![],
                },
            ))
            .unwrap();
    }
    fn append(region: &Region, rev: u64) {
        changes(
            region,
            vec![ViewOp::Insert {
                parent: "root".into(),
                before: None,
                node: Node::text("text", [Span::plain("new text")]).id(format!("new-{rev}")),
            }],
            vec![],
            false,
        );
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
        streams(
            &region,
            vec![Stream {
                id: "live.text".into(),
                role: "message.assistant".into(),
                text: "live-only".into(),
            }],
        );
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
        streams(
            &region,
            vec![Stream {
                id: "live.text".into(),
                role: "text".into(),
                text: "a".repeat(10000),
            }],
        );
        let mut subscriber = region.subscribe();
        subscriber.next().await.unwrap();
        subscriber.next().await.unwrap();
        changes(
            &region,
            vec![],
            vec![StreamUpdate::Append {
                id: "live.text".into(),
                offset: 10000,
                text: "λ".into(),
            }],
            false,
        );
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
        changes(
            &region,
            vec![
                ViewOp::Replace {
                    id: "old-0".into(),
                    node: Node::text("text", [Span::plain("<replacement>")]).id("old-0"),
                },
                ViewOp::Remove { id: "old-1".into() },
            ],
            vec![],
            false,
        );
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
