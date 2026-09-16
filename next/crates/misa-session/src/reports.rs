//! Retain automatic reports whose transaction is waiting for plugin persistence.
//! Journal replies bypass this queue; otherwise the decision releasing it could
//! wait behind its own blocked listener. No lock is held while handlers execute.
use std::collections::VecDeque;
use misa_proto::Fault;
use misa_reframe::Event;
use misa_value::Value;
use crate::Runtime;

const COUNT: usize = 128;
const BYTES: usize = 8 * 1024 * 1024;

#[derive(Default)]
pub(crate) struct Pending {
    events: VecDeque<(Event, usize)>,
    bytes: usize,
    draining: bool,
    wake: u64,
}
impl Pending {
    pub(crate) fn clear(&mut self) { self.events.clear(); self.bytes = 0; }
}
fn busy(faults: &[Fault]) -> bool { faults.iter().any(|fault| fault.code == "composition.busy") }
fn size(value: &Value) -> usize {
    let payload = match value {
        Value::Str(text) => text.len(), Value::Bytes(bytes) => bytes.len(),
        Value::List(items) => items.iter().fold(0usize, |sum, value| sum.saturating_add(size(value))),
        Value::Map(items) => items.iter().fold(0usize, |sum, (key, value)| sum.saturating_add(key.len()).saturating_add(size(value)).saturating_add(64)),
        _ => 0,
    };
    payload.saturating_add(std::mem::size_of::<Value>())
}
impl Runtime {
    pub fn dispatch(&self, event: Event) -> Vec<Fault> {
        let faults = self.dispatch_once(event.clone());
        let faults = if busy(&faults) && event.kind.starts_with("kernel/") {
            let bytes = size(&event.data).saturating_add(event.kind.len());
            let mut pending = self.reports.lock().expect("report queue poisoned");
            if pending.events.len() + usize::from(pending.draining) >= COUNT || pending.bytes.saturating_add(bytes) > BYTES {
                pending.events.clear(); pending.bytes = 0;
                drop(pending);
                self.notice(misa_proto::wire::Level::Error, "Session closed: deferred report capacity exhausted");
                self.shutdown();
                return vec![Fault::new("report_capacity", "Session closed rather than dropping an authoritative report")];
            }
            pending.bytes += bytes;
            pending.events.push_back((event, bytes));
            vec![]
        } else { faults };
        self.drain_reports();
        faults
    }

    fn drain_reports(&self) {
        {
            let mut pending = self.reports.lock().expect("report queue poisoned");
            pending.wake = pending.wake.wrapping_add(1);
            if pending.draining { return; }
            pending.draining = true;
        }
        loop {
            let (event, bytes, wake) = {
                let mut pending = self.reports.lock().expect("report queue poisoned");
                if self.is_closed() { pending.events.clear(); pending.bytes = 0; }
                let Some((event, bytes)) = pending.events.pop_front() else { pending.draining = false; return; };
                (event, bytes, pending.wake)
            };
            let faults = self.dispatch_once(event.clone());
            let mut pending = self.reports.lock().expect("report queue poisoned");
            if busy(&faults) {
                pending.events.push_front((event, bytes));
                // An acknowledgement may have arrived while dispatch ran. Its
                // wake must be consumed before parking, even if it found us busy.
                if pending.wake != wake { continue; }
                pending.draining = false;
                return;
            }
            pending.bytes = pending.bytes.saturating_sub(bytes);
            drop(pending);
            for fault in faults { self.notice(misa_proto::wire::Level::Error, fault.message); }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[tokio::test]
    async fn journal_acknowledgement_releases_a_blocked_automatic_report() {
        let handler = |name| Arc::new(misa_reframe::FnHandler::new(name, |tx: &mut misa_reframe::Tx<'_>, _: &Event| {
            tx.set("counter.value", Value::Int(tx.int("counter.value") + 1))
        })) as Arc<dyn misa_reframe::Handler>;
        let contribution = crate::Contribution::default()
            .with_root("counter", Value::map([("value", Value::Int(0))])).unwrap()
            .with_handler("counter/increment", 0, handler("command"))
            .with_handler("kernel/counter", 0, handler("report"));
        let runtime = Runtime::start_with("reports", "reports", None,
            Arc::new(misa_kernel::LocalKernel::new(misa_kernel::ScriptedProvider::always("unused"))),
            "scripted", "scripted-1", Value::Null, contribution);
        let value = || runtime.state.lock().unwrap().state.db().get("counter").unwrap().get("value").cloned().unwrap();
        let acknowledge = || {
            let state = runtime.state.lock().unwrap();
            let session = state.state.db().get("session").unwrap();
            Event::new("kernel/log.appended").with("conversation", session.get("conversation").unwrap().clone())
                .with("kind", Value::str(crate::contribution::PATCH_KIND))
                .with("data", session.get("plugin_write").unwrap().clone()).with("seq", Value::Int(1))
        };
        assert!(runtime.dispatch(Event::new("counter/increment")).is_empty());
        assert!(runtime.dispatch(Event::new("kernel/counter")).is_empty());
        assert_eq!(runtime.reports.lock().unwrap().events.len(), 1);
        assert_eq!(value(), Value::Int(0));
        assert!(runtime.dispatch(acknowledge()).is_empty());
        assert!(runtime.reports.lock().unwrap().events.is_empty());
        assert_eq!(value(), Value::Int(1));
        assert!(runtime.dispatch(acknowledge()).is_empty());
        assert_eq!(value(), Value::Int(2));
        assert!(runtime.dispatch(Event::new("counter/increment")).is_empty());
        for _ in 0..COUNT { assert!(runtime.dispatch(Event::new("kernel/counter")).is_empty()); }
        let overflow = runtime.dispatch(Event::new("kernel/counter"));
        assert_eq!(overflow[0].code, "report_capacity");
        assert!(runtime.is_closed());
        assert!(runtime.reports.lock().unwrap().events.is_empty());
    }
}
