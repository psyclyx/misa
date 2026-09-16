//! Bounded wakeups and deferred captures avoid snapshots queued behind a paused UI.
use misa_client::{document, driver::Observation};
use misa_protocol::observation::{Applied, MemberChange};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
type Pending = Box<dyn FnOnce() -> Value + Send>;
pub struct Mailbox {
    queue: Mutex<VecDeque<Pending>>,
    space: tokio::sync::Notify,
    wake: Arc<dyn Fn(Value) + Send + Sync>,
}
impl Mailbox {
    pub fn new(wake: Arc<dyn Fn(Value) + Send + Sync>) -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            space: tokio::sync::Notify::new(),
            wake,
        }
    }
    pub async fn push(&self, pending: Pending) {
        let mut pending = Some(pending);
        loop {
            let wait = self.space.notified();
            tokio::pin!(wait);
            wait.as_mut().enable();
            let inserted = {
                let mut queue = self.queue.lock().unwrap();
                if queue.len() < 32 {
                    let wake = queue.is_empty();
                    queue.push_back(pending.take().unwrap());
                    Some(wake)
                } else {
                    None
                }
            };
            if let Some(wake) = inserted {
                if wake {
                    (self.wake)(json!({"kind":"wake"}));
                }
                return;
            }
            wait.await;
        }
    }
    pub async fn event(&self, event: Value) {
        self.push(Box::new(move || event)).await;
    }
    pub fn startup_fault(&self, message: &str) {
        (self.wake)(json!({"kind":"fault","message":message}));
    }
    pub fn next(&self) -> Option<Value> {
        let pending = self.queue.lock().unwrap().pop_front()?;
        self.space.notify_one();
        Some(pending())
    }
}
pub struct Documents {
    pub observation: Observation,
    instance: String,
    readers: Mutex<Vec<(String, document::Reader)>>,
    queued: AtomicBool,
    generation: u64,
    render: AtomicU64,
}
impl Documents {
    pub fn new(
        observation: Observation,
        instance: String,
        generation: u64,
        members: Vec<(String, String)>,
    ) -> Self {
        Self {
            observation,
            instance,
            generation,
            render: AtomicU64::new(0),
            readers: Mutex::new(
                members
                    .into_iter()
                    .map(|(slot, member)| (slot, document::Reader::new(member)))
                    .collect(),
            ),
            queued: AtomicBool::new(false),
        }
    }
    pub async fn notify(self: &Arc<Self>, mailbox: &Mailbox) {
        if self.queued.swap(true, Ordering::AcqRel) {
            return;
        }
        let feed = self.clone();
        mailbox.push(Box::new(move || feed.capture())).await;
    }
    /// Rebuild only the local render cache. The shared replica and its resume
    /// checkpoint remain authoritative and unchanged.
    pub fn invalidate(&self) -> Result<(), String> {
        let mut readers = self.readers.lock().unwrap();
        self.render
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |generation| {
                generation.checked_add(1)
            })
            .map_err(|_| "Render generations exhausted")?;
        for (_, reader) in readers.iter_mut() {
            reader.invalidate();
        }
        Ok(())
    }
    fn capture(&self) -> Value {
        self.queued.store(false, Ordering::Release);
        let mut readers = self.readers.lock().unwrap();
        let render = self.render.load(Ordering::Acquire);
        let updates = document::capture_many(
            &self.observation,
            readers
                .iter_mut()
                .map(|(slot, reader)| (slot.as_str(), reader)),
        )
        .into_iter()
        .map(|(slot, update)| (slot, encode(update)))
        .collect::<serde_json::Map<_, _>>();
        json!({"kind":"transaction","instance":self.instance,"composition":self.generation,"render":render,"documents":updates})
    }
}
fn encode(update: document::Update) -> Value {
    match update {
        document::Update::Reset(document) => {
            json!({"mode":"reset","tree":document.tree,"streams":document.streams})
        }
        document::Update::Changed { member, applied } => {
            let Applied::Changed(changes) = &*applied else {
                unreachable!()
            };
            let Some(MemberChange::Document {
                tree,
                live,
                reset_live,
            }) = changes.get(&member)
            else {
                unreachable!()
            };
            json!({"mode":"change","ops":tree,"live":live,"reset_live":reset_live})
        }
        document::Update::Unavailable(fault) => {
            json!({"mode":"unavailable","message":fault.message})
        }
        document::Update::Status(status) => json!({"mode":"status","status":format!("{status:?}")}),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn a_stalled_surface_bounds_queue_and_defers_materialization() {
        let mailbox = Arc::new(Mailbox::new(Arc::new(|_| {})));
        let captured = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        for _ in 0..32 {
            let captured = captured.clone();
            mailbox
                .push(Box::new(move || {
                    captured.fetch_add(1, Ordering::Relaxed);
                    Value::Null
                }))
                .await;
        }
        assert_eq!(captured.load(Ordering::Relaxed), 0);
        let full = mailbox.clone();
        let extra = tokio::spawn(async move {
            full.event(Value::Null).await;
        });
        tokio::task::yield_now().await;
        assert!(!extra.is_finished());
        assert_eq!(mailbox.next(), Some(Value::Null));
        extra.await.unwrap();
        assert_eq!(mailbox.queue.lock().unwrap().len(), 32);
        assert_eq!(captured.load(Ordering::Relaxed), 1);
    }
}
