//! One cheap coherent summary observation per daemon relationship.
use std::sync::{Arc, Mutex};
use misa_client::{daemons::Daemon, overview::{Overview, Snapshot}};
use misa_protocol::observation::Status;
use misa_proto::directory::Availability;
use tokio::sync::watch;
use crate::escape;

pub(crate) struct Feed {
    html: Arc<Mutex<String>>,
    task: tokio::task::AbortHandle,
}
impl Drop for Feed { fn drop(&mut self) { self.task.abort(); } }
impl Feed {
    pub(crate) fn start(daemon: Arc<Daemon>, updates: watch::Sender<u64>) -> Self {
        let html = Arc::new(Mutex::new("<p>Opening overview…</p>".into()));
        let target = html.clone();
        let task = tokio::spawn(async move {
            let result = Overview::open(&daemon.client).await;
            match result {
                Ok(mut overview) => loop {
                    *target.lock().unwrap() = match overview.snapshot() {
                        Ok(snapshot) => render(daemon.identity(), &snapshot),
                        Err(fault) => format!("<p role=\"status\">Overview unavailable: {}</p>", escape(&fault.message)),
                    };
                    updates.send_modify(|version| *version = version.wrapping_add(1));
                    if let Err(fault) = overview.changed().await {
                        *target.lock().unwrap() = match overview.snapshot() {
                            Ok(mut snapshot) => { snapshot.status = Status::Closed(fault); render(daemon.identity(), &snapshot) }
                            Err(_) => format!("<p role=\"status\">Overview unavailable: {}</p>", escape(&fault.message)),
                        };
                        updates.send_modify(|version| *version = version.wrapping_add(1));
                        break;
                    }
                },
                Err(fault) => {
                    *target.lock().unwrap() = format!("<p role=\"status\">Overview unavailable: {}</p>", escape(&fault.message));
                    updates.send_modify(|version| *version = version.wrapping_add(1));
                }
            }
        });
        Self { html, task: task.abort_handle() }
    }
    pub(crate) fn html(&self) -> String { self.html.lock().unwrap().clone() }
}

fn render(daemon: &str, snapshot: &Snapshot) -> String {
    let current = matches!(snapshot.status, Status::Current);
    let freshness = match &snapshot.status {
        Status::Current => "Current".to_string(),
        Status::Stale(fault) | Status::Closed(fault) => format!("Unavailable: {}", fault.message),
        _ => "Loading; previously observed values may be stale".into(),
    };
    let mut html = format!("<p role=\"status\">{} · publication {}</p>", escape(&freshness), snapshot.position.map(|v|v.to_string()).unwrap_or_else(||"pending".into()));
    for fault in snapshot.unavailable.values() {
        html.push_str(&format!("<p role=\"status\">{}</p>", escape(&fault.message)));
    }
    if snapshot.sessions.is_empty() { html.push_str("<p>No live sessions.</p>"); }
    for entry in &snapshot.sessions {
        let available = current && entry.availability == Availability::Current;
        html.push_str(&format!("<article><form method=\"post\" action=\"/choose\"><input type=\"hidden\" name=\"daemon\" value=\"{}\"><input type=\"hidden\" name=\"session\" value=\"{}\"><input type=\"hidden\" name=\"incarnation\" value=\"{}\"><button {}>{}</button></form>", escape(daemon), escape(&entry.id), escape(&entry.incarnation), if available {""} else {"disabled"}, escape(&entry.title)));
        if let Some(row) = snapshot.rows.iter().find(|row| row.scope == entry.scope()) {
            let state = if !available || row.availability != Availability::Current { "State unavailable" }
                else if row.working { "Working" } else { "Idle" };
            html.push_str(&format!("<p>{state} · {} awaiting input · {} blocking children</p>", row.attention, row.blocking.len()));
            if row.cyclic || !row.unavailable.is_empty() { html.push_str("<p>Some related work is unavailable; totals may be incomplete.</p>"); }
            for request in &row.requests {
                let kind = request.request.get("kind").and_then(misa_value::Value::as_str).unwrap_or("input");
                html.push_str(&format!("<p>Awaiting {} · {}</p>", escape(kind), escape(&format!("{:?}", request.scope.id))));
                if let (misa_proto::observation::ScopeId::Session { id: session }, Some(id), Some(generation)) = (&request.scope.id, request.request.get("id").and_then(misa_value::Value::as_str), request.request.get("generation").and_then(misa_value::Value::as_i64)) {
                    let enabled = available && request.availability == Availability::Current;
                    html.push_str(&format!("<form method=\"post\" action=\"/choose\"><input type=\"hidden\" name=\"daemon\" value=\"{}\"><input type=\"hidden\" name=\"session\" value=\"{}\"><input type=\"hidden\" name=\"incarnation\" value=\"{}\"><input type=\"hidden\" name=\"request\" value=\"{}\"><input type=\"hidden\" name=\"generation\" value=\"{}\"><button {}>Respond to {}</button></form>", escape(daemon), escape(session), escape(&request.scope.incarnation), escape(id), generation, if enabled { "" } else { "disabled" }, escape(kind)));
                }
            }
            html.push_str(&format!("<p>Usage including related work: {} input / {} output tokens</p>", row.inclusive_usage.input_tokens, row.inclusive_usage.output_tokens));
        } else { html.push_str("<p>Work summary unavailable</p>"); }
        html.push_str(&format!("<form method=\"get\" action=\"/lifecycle\"><input type=\"hidden\" name=\"daemon\" value=\"{}\"><input type=\"hidden\" name=\"command\" value=\"daemon.session.close\"><input type=\"hidden\" name=\"id\" value=\"{}\"><input type=\"hidden\" name=\"incarnation\" value=\"{}\"><button {}>Prepare closing session owner</button></form></article>", escape(daemon), escape(&entry.id), escape(&entry.incarnation), if available {""} else {"disabled"}));
    }
    if !snapshot.work.is_empty() {
        html.push_str("<details><summary>Delegated work</summary><ul>");
        for work in &snapshot.work {
            html.push_str(&format!("<li>{}: {} · {}{}", escape(&work.id), escape(&work.state), escape(&work.lifetime), if work.blocking { " · blocks parent" } else { "" }));
            let action = match work.state.as_str() {
                "starting" | "running" => Some(("operation.cancel", "Prepare cancellation")),
                "succeeded" | "failed" | "cancelled" | "expired" | "interrupted" => Some(("daemon.work.forget", "Prepare forgetting result")),
                _ => None,
            };
            if let Some((command, label)) = action {
                html.push_str(&format!("<form method=\"get\" action=\"/lifecycle\"><input type=\"hidden\" name=\"daemon\" value=\"{}\"><input type=\"hidden\" name=\"owner\" value=\"{}\"><input type=\"hidden\" name=\"command\" value=\"{}\"><input type=\"hidden\" name=\"operation\" value=\"{}\"><button {}>{}</button></form>", escape(daemon), escape(&snapshot.scope.incarnation), command, escape(&work.id), if current { "" } else { "disabled" }, label));
            }
            html.push_str("</li>");
        }
        html.push_str("</ul></details>");
    }
    html
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_client::overview::{Row, Usage, Work};
    use misa_proto::{directory::Entry, observation::Scope};
    use misa_value::Value;
    #[test]
    fn overview_preserves_parallel_work_attention_and_freshness() {
        let entry = Entry { id: "parent".into(), incarnation: "one".into(), title: "Parent <session>".into(), availability: Availability::Current, source_position: 5, summary: Value::Null };
        let scope = entry.scope();
        let usage = Usage { input_tokens: 12, output_tokens: 3, cost_micros: 0 };
        let mut snapshot = Snapshot {
            scope: Scope { id: misa_proto::observation::ScopeId::Daemon, incarnation: "runtime".into() }, status: Status::Current, position: Some(9), sessions: vec![entry],
            rows: vec![Row { scope: scope.clone(), source_position: 5, availability: Availability::Current, direct_working: false, working: true, direct_attention: 0, attention: 1, requests: vec![], blocking: vec!["child".into()], unavailable: vec![], cyclic: false, direct_usage: usage.clone(), inclusive_usage: usage.clone() }],
            work: vec![Work { id: "child".into(), parent: scope, child: None, state: "running".into(), lifetime: "independent".into(), blocking: false, parent_operation: None, child_operation: None, conversation: None, usage }],
            unavailable: Default::default(),
        };
        snapshot.rows[0].requests.push(misa_client::overview::Attention {
            scope: Scope { id: misa_proto::observation::ScopeId::Session { id: "child-session".into() }, incarnation: "child-owner".into() },
            source_position: 6, availability: Availability::Current,
            request: Value::map([("id", Value::str("child-request")), ("generation", Value::Int(7)), ("kind", Value::str("approval"))]),
        });
        let current = render("daemon", &snapshot);
        assert!(current.contains("Working · 1 awaiting input · 1 blocking children"));
        assert!(current.contains("child: running · independent"));
        assert!(current.contains("name=\"operation\" value=\"child\""));
        assert!(current.contains("name=\"owner\" value=\"runtime\""));
        assert!(current.contains("Prepare cancellation"));
        assert!(current.contains("name=\"session\" value=\"child-session\""));
        assert!(current.contains("name=\"incarnation\" value=\"child-owner\""));
        assert!(current.contains("name=\"request\" value=\"child-request\""));
        assert!(current.contains("name=\"generation\" value=\"7\""));
        assert!(current.contains("<button >Respond to approval</button>"));
        assert!(current.contains("Parent &lt;session&gt;"));
        assert!(current.contains("name=\"incarnation\" value=\"one\""));
        snapshot.status = Status::Stale(misa_proto::Fault::new("offline", "Disconnected"));
        let stale = render("daemon", &snapshot);
        assert!(stale.contains("button disabled"));
        assert!(stale.contains("State unavailable"));
        assert!(!stale.contains("Working ·"));
        assert!(stale.contains("<button disabled>Respond to approval</button>"));
    }
}
