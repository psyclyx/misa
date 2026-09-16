//! Instance-local observation of shared work and directed command completions.
//! The shared client owns replicas and operation state machines; this adapter
//! only retains bounded display results and reserves capacity before invocation.
use std::{collections::{BTreeMap, VecDeque}, sync::Arc};
use misa_client::{driver::Client, interface::Interface, operation::{Tracker, Watch, Completion, Terminal}};
use misa_proto::{Fault, invocation::OperationRef, observation::Selection};
use misa_protocol::observation::{MemberState, Status};
use misa_value::Value;
use tokio::sync::{mpsc, OwnedSemaphorePermit, Semaphore};
use crate::{escape, Region};

const MAX_ACTIVE: usize = 64;
const MAX_FINISHED: usize = 32;
pub(crate) struct Accepted {
    pub watch: Watch,
    pub permit: OwnedSemaphorePermit,
}
pub(crate) struct Activity {
    pub capacity: Arc<Semaphore>,
    pub accepted: mpsc::Sender<Accepted>,
    task: tokio::task::AbortHandle,
}
impl Drop for Activity { fn drop(&mut self) { self.task.abort(); } }
impl Activity { pub(crate) fn close(&self) { self.capacity.close(); self.task.abort(); } }

pub(crate) async fn start(client: &Client, interface: &Interface, region: Region) -> Result<Activity, Fault> {
    let members = ["operations.summary", "requests.summary"].into_iter()
        .map(|id| interface.query(id, vec![]).map(|member| (id.to_string(), member)))
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let mut observation = client.observe(Selection { scope: interface.scope.clone(), members }, None).await?;
    let (accepted, mut incoming) = mpsc::channel::<Accepted>(MAX_ACTIVE);
    let capacity = Arc::new(Semaphore::new(MAX_ACTIVE));
    let task = tokio::spawn(async move {
        let mut tracker = Tracker::new(MAX_ACTIVE);
        let mut permits: Vec<(OperationRef, OwnedSemaphorePermit)> = vec![];
        let mut finished = VecDeque::new();
        loop {
            for completion in tracker.drain() {
                permits.retain(|(reference, _)| reference != &completion.operation);
                if finished.len() == MAX_FINISHED { finished.pop_front(); }
                finished.push_back(completed(&completion));
            }
            let shared = observation.inspect(|replica, _| {
                match replica.status() {
                    Status::Current => {
                        let members = replica.current().expect("current replica has members");
                        let values = ["operations.summary", "requests.summary"].into_iter().filter_map(|id| match members.get(id) {
                            Some(MemberState::Value(value)) => Some(value), _ => None,
                        });
                        summaries(values)
                    }
                    Status::Closed(fault) | Status::Stale(fault) => format!("<p role=\"status\">Work status unavailable: {}</p>", escape(&fault.message)),
                    _ => "<p role=\"status\">Loading work status…</p>".into(),
                }
            }).unwrap_or_else(|| "<p role=\"status\">Work status unavailable</p>".into());
            let mut html = shared;
            if !finished.is_empty() {
                html.push_str("<details><summary>Recent command outcomes</summary>");
                for item in &finished { html.push_str(item); }
                html.push_str("</details>");
            }
            region.activity(html);
            tokio::select! {
                update = observation.changed() => { if update.is_err() { break; } }
                _ = tracker.changed() => {}
                accepted = incoming.recv() => {
                    let Some(accepted) = accepted else { break; };
                    let reference = accepted.watch.reference().clone();
                    // Every insertion owns a permit reserved before sending the
                    // invocation, so an accepted call cannot exceed the tracker.
                    match tracker.insert(accepted.watch) {
                        Ok(()) => permits.push((reference, accepted.permit)),
                        Err(fault) => {
                            if finished.len() == MAX_FINISHED { finished.pop_front(); }
                            finished.push_back(format!("<p role=\"alert\">Accepted operation {} could not be monitored: {}</p>", escape(&reference.id), escape(&fault.message)));
                        }
                    }
                }
            }
        }
    });
    Ok(Activity { capacity, accepted, task: task.abort_handle() })
}

fn summaries<'a>(values: impl Iterator<Item=&'a Value>) -> String {
    let mut rows = BTreeMap::new();
    for value in values {
        for entry in value.as_list().unwrap_or(&[]) {
            let Some(id) = entry.get("id").and_then(Value::as_str) else { continue; };
            let state = entry.get("state").and_then(Value::as_str).unwrap_or("unknown");
            if entry.get("terminal").and_then(Value::as_bool) == Some(true) { continue; }
            let kind = entry.get("kind").and_then(Value::as_str).unwrap_or("Operation");
            let action = if state == "awaiting_input" {
                format!("<form method=\"post\" action=\"./request\"><input type=\"hidden\" name=\"id\" value=\"{}\"><button>Respond</button></form>", escape(id))
            } else { String::new() };
            rows.insert(id, format!("<li>{}: {} {}</li>", escape(kind), escape(&state.replace('_', " ")), action));
        }
    }
    if rows.is_empty() { return "<p>No active operations or pending requests.</p>".into(); }
    format!("<ul>{}</ul>", rows.into_values().collect::<String>())
}

fn completed(completion: &Completion) -> String {
    let description = match &completion.outcome {
        Terminal::Finished { state, .. } => format!("Finished: {}", state.replace('_', " ")),
        Terminal::Expired => "Result expired; execution outcome is no longer retained".into(),
        Terminal::Fault(fault) => format!("Accepted, but monitoring failed: {}", fault.message),
    };
    let body = if let Terminal::Finished { document, value, .. } = &completion.outcome {
        let bounded = match document {
            Some(document) => misa_proto::chunk::encoded_size_with_limit(document, 32 * 1024).is_ok(),
            None => misa_proto::chunk::encoded_size_with_limit(value, 32 * 1024).is_ok(),
        };
        if bounded {
            let report = document.clone().unwrap_or_else(|| misa_client::request::report("Result", value));
            let rendered = crate::render_scoped(&report, &format!("outcome-{}:", crate::next_id()));
            if rendered.len() <= 64 * 1024 { rendered } else { "<p>Result is too large for recent outcomes. Open the authoritative operation or conversation.</p>".into() }
        } else { "<p>Result is too large for recent outcomes. Open the authoritative operation or conversation.</p>".into() }
    } else { String::new() };
    format!("<details><summary><code>{}</code> {}</summary>{}</details>", escape(&completion.operation.id), escape(&description), body)
}
