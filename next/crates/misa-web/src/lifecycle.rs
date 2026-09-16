//! Local lifecycle preparation over installed daemon commands.
use super::{Shared, Instance, MAX_INSTANCES};
use std::{collections::BTreeMap, time::{Duration, Instant}};
use axum::{extract::{State, Query, Form}, response::{Response, IntoResponse, Html, Redirect}, http::StatusCode};
use misa_client::{form::Form as CommandForm, interface::Interface};
use misa_proto::invocation::Outcome;
use crate::{escape, actions};

fn page(status: StatusCode, title: &str, body: String) -> Response {
    (status, [("cache-control", "no-store")], Html(format!("<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width\"><title>{}</title><link rel=\"stylesheet\" href=\"/style.css\"></head><body><main><h1>{}</h1>{}<p><a href=\"/daemons\">Daemon overview</a></p></main></body></html>", escape(title), escape(title), body))).into_response()
}
fn command(fields: &BTreeMap<String,String>) -> Result<&str, &'static str> {
    match fields.get("action_id").or_else(||fields.get("command")).map(String::as_str) {
        Some(id @ ("daemon.session.create" | "daemon.session.resume" | "daemon.session.close")) => Ok(id),
        _ => Err("Choose a lifecycle command"),
    }
}
pub(super) async fn open(State(hub): State<Shared>, Query(fields): Query<BTreeMap<String,String>>) -> Response {
    let id = fields.get("daemon").map(String::as_str).unwrap_or("");
    let daemon = hub.lock().await.daemons.get(id).cloned();
    let Some(daemon) = daemon else { return page(StatusCode::NOT_FOUND, "Unknown daemon", String::new()); };
    let command = match command(&fields) { Ok(command) => command, Err(error) => return page(StatusCode::BAD_REQUEST, "Preparation failed", escape(error)) };
    let interface = match Interface::load(&daemon.client, daemon.client.welcome().scope).await { Ok(value) => value, Err(fault) => return page(StatusCode::BAD_GATEWAY, "Daemon unavailable", escape(&fault.message)) };
    let model = match CommandForm::command(&interface, command) { Ok(model) => model, Err(fault) => return page(StatusCode::BAD_REQUEST, "Command unavailable", escape(&fault.message)) };
    let mut drafts = BTreeMap::new();
    if command != "daemon.session.close" { drafts.insert("id".into(), format!("session-{}", crate::next_id())); }
    for key in ["id", "incarnation", "conversation"] { if let Some(value) = fields.get(key) { drafts.insert(key.into(), value.clone()); } }
    let mut body = if command == "daemon.session.close" { "<p>Closing the session owner stops its work for all clients. Use Close presentation to release only a local view.</p>".into() } else { String::new() };
    body.push_str(&actions::markup_at(&model, true, &BTreeMap::new(), &drafts, "/lifecycle", &[("daemon", id), ("owner", &interface.scope.incarnation)]));
    page(StatusCode::OK, "Prepare session command", body)
}
pub(super) async fn perform(State(hub): State<Shared>, Form(fields): Form<BTreeMap<String,String>>) -> Response {
    let id = fields.get("daemon").map(String::as_str).unwrap_or("");
    let daemon = hub.lock().await.daemons.get(id).cloned();
    let Some(daemon) = daemon else { return page(StatusCode::NOT_FOUND, "Unknown daemon", String::new()); };
    let command = match command(&fields) { Ok(command) => command, Err(error) => return page(StatusCode::BAD_REQUEST, "Command not sent", escape(error)) };
    let interface = match Interface::load(&daemon.client, daemon.client.welcome().scope).await { Ok(value) => value, Err(fault) => return page(StatusCode::BAD_GATEWAY, "Command not sent", escape(&fault.message)) };
    if fields.get("owner") != Some(&interface.scope.incarnation) { return page(StatusCode::CONFLICT, "Daemon changed", "Reopen the command form against the current daemon before submitting.".into()); }
    let prepared = CommandForm::command(&interface, command).and_then(|model| model.prepare(&fields.iter().filter_map(|(key,value)| key.strip_prefix("field.").map(|id|(id.into(),value.clone()))).collect()));
    let (_, input) = match prepared { Ok(value) => value, Err(fault) => return page(StatusCode::BAD_REQUEST, "Command not sent", escape(&fault.message)) };
    let outcome = daemon.client.invoke(interface.scope.clone(), interface.commands[command].clone(), input, Duration::from_secs(30)).await;
    match outcome.map(|reply|reply.outcome) {
        Ok(Outcome::Completed { value }) if command != "daemon.session.close" => {
            let entry = match misa_client::lifecycle::opened(&daemon, &value).await { Ok(entry) => entry, Err(fault) => return page(StatusCode::BAD_GATEWAY, "Session opened; navigation unavailable", escape(&fault.message)) };
            let remote = match crate::connect_session(&daemon, &entry.id).await { Ok(remote) => remote, Err(error) => return page(StatusCode::BAD_GATEWAY, "Session opened; navigation unavailable", escape(&error)) };
            if remote.interaction.interface.scope != entry.scope() { return page(StatusCode::CONFLICT, "Session changed", "The opened session owner has already changed. Return to the overview.".into()); }
            let key = remote.instance.clone();
            let mut hub = hub.lock().await;
            hub.expire();
            if hub.sessions.len() >= MAX_INSTANCES { return page(StatusCode::TOO_MANY_REQUESTS, "Session opened", "Close a local presentation, then select the created session in the overview.".into()); }
            hub.sessions.insert(key.clone(), Instance { remote, used: Instant::now() });
            Redirect::to(&format!("/view/{key}/")).into_response()
        }
        Ok(Outcome::Completed { .. }) => Redirect::to("/daemons").into_response(),
        Ok(Outcome::Rejected { fault }) => page(StatusCode::BAD_REQUEST, "Command rejected", escape(&fault.message)),
        Ok(Outcome::Accepted { operation }) => page(StatusCode::ACCEPTED, "Command accepted", format!("<p>Operation {} is running. Check the daemon overview before issuing another command.</p>", escape(&operation.id))),
        Ok(Outcome::Indeterminate { .. }) | Err(_) => page(StatusCode::BAD_GATEWAY, "Command outcome unknown", "The command may have taken effect. Check the overview and saved conversations before submitting again.".into()),
    }
}
pub(super) async fn archive(State(hub): State<Shared>, Query(fields): Query<BTreeMap<String,String>>) -> Response {
    let id = fields.get("daemon").map(String::as_str).unwrap_or("");
    let daemon = hub.lock().await.daemons.get(id).cloned();
    let Some(daemon) = daemon else { return page(StatusCode::NOT_FOUND, "Unknown daemon", String::new()); };
    let prefix = fields.get("q").map(String::as_str).unwrap_or("");
    let candidates = match misa_client::lifecycle::conversations(&daemon, prefix, misa_proto::wire::DEFAULT_CANDIDATES).await { Ok(value) => value, Err(fault) => return page(StatusCode::BAD_GATEWAY, "Archive unavailable", escape(&fault.message)) };
    let mut body = format!("<form method=\"get\" action=\"/archive\"><input type=\"hidden\" name=\"daemon\" value=\"{}\"><label>Search <input name=\"q\" value=\"{}\"></label><button>Search</button></form>", escape(id), escape(prefix));
    for choice in candidates.items {
        body.push_str(&format!("<form method=\"get\" action=\"/lifecycle\"><input type=\"hidden\" name=\"daemon\" value=\"{}\"><input type=\"hidden\" name=\"command\" value=\"daemon.session.resume\"><input type=\"hidden\" name=\"conversation\" value=\"{}\"><button>Resume {}</button><p>{}</p></form>", escape(id), escape(&choice.value), escape(&choice.label), escape(choice.detail.as_deref().unwrap_or(""))));
    }
    if candidates.truncated { body.push_str("<p>More conversations match. Refine your search.</p>"); }
    page(StatusCode::OK, "Saved conversations", body)
}
pub(super) async fn disconnect(State(hub): State<Shared>, Form(fields): Form<BTreeMap<String,String>>) -> Response {
    if let Some(id) = fields.get("daemon") {
        let connections = {
            let mut hub = hub.lock().await;
            hub.daemons.remove(id);
            hub.overviews.remove(id);
            hub.updates.send_modify(|version|*version = version.wrapping_add(1));
            hub.connections.clone()
        };
        connections.disconnect(id).await;
    }
    Redirect::to("/daemons").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use misa_protocol::invocation::CommandOwner;
    #[tokio::test]
    async fn daemon_forms_create_close_and_fence_replaced_owners() {
        let directory = misa_daemon::directory::Directory::new("web-lifecycle").unwrap();
        directory.install_factory(Arc::new(|_, spec| Box::pin(async move {
            Ok(misa_session::Runtime::prepare_with(spec.id, spec.title, spec.conversation,
                Arc::new(misa_kernel::LocalKernel::new(misa_kernel::ScriptedProvider::always("reply"))),
                "scripted", "scripted-1", misa_value::Value::Null, Default::default()))
        }))).unwrap();
        let server = misa_transport::iroh::bind(None, false).await.unwrap();
        let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
        let router = iroh::protocol::Router::builder(server.clone()).accept(misa_proto::scoped::ALPN,
            misa_transport::scoped_server::Handler {
                daemon: server.id().to_string(), scope: directory.scope(),
                resolver: Arc::new(misa_daemon::directory::Routes(directory.clone())),
                admission: Arc::new(misa_transport::admission::Admission::open()),
            }).spawn();
        let connections = Arc::new(misa_client::daemons::Daemons::new(endpoint.clone(), misa_proto::ClientInfo::new("web-lifecycle", "test")));
        let daemon = connections.connect(server.addr()).await.unwrap();
        let id = daemon.identity().to_string();
        let owner = daemon.client.welcome().scope.incarnation;
        let hub = Arc::new(tokio::sync::Mutex::new(super::super::Hub::new(connections.clone(), BTreeMap::from([(id.clone(), daemon.clone())]))));
        let preparation = open(State(hub.clone()), Query(BTreeMap::from([("daemon".into(), id.clone()), ("command".into(), "daemon.session.create".into())]))).await;
        assert_eq!(preparation.status(), StatusCode::OK);
        let html = axum::body::to_bytes(preparation.into_body(), 65536).await.unwrap();
        assert!(std::str::from_utf8(&html).unwrap().contains("name=\"field.id\""));
        assert!(directory.sessions().is_empty(), "opening a local form must not create a session");
        let mut fields = BTreeMap::from([("daemon".into(), id.clone()), ("owner".into(), owner.clone()), ("action_id".into(), "daemon.session.create".into()), ("field.id".into(), "created".into()), ("field.title".into(), "Created in browser".into())]);
        let created = perform(State(hub.clone()), Form(fields.clone())).await;
        assert_eq!(created.status(), StatusCode::SEE_OTHER);
        let scope = hub.lock().await.sessions.values().next().unwrap().remote.interaction.interface.scope.clone();
        fields.insert("owner".into(), "obsolete".into());
        fields.insert("field.id".into(), "must-not-create".into());
        assert_eq!(perform(State(hub.clone()), Form(fields.clone())).await.status(), StatusCode::CONFLICT);
        fields.insert("owner".into(), owner);
        fields.insert("action_id".into(), "daemon.session.close".into());
        fields.insert("field.id".into(), "created".into());
        fields.insert("field.incarnation".into(), "obsolete".into());
        assert_eq!(perform(State(hub.clone()), Form(fields.clone())).await.status(), StatusCode::BAD_REQUEST);
        fields.insert("field.incarnation".into(), scope.incarnation);
        assert_eq!(perform(State(hub.clone()), Form(fields)).await.status(), StatusCode::SEE_OTHER);
        assert!(directory.sessions().is_empty());
        assert_eq!(hub.lock().await.sessions.len(), 1, "owner close preserves local presentation memory");
        disconnect(State(hub.clone()), Form(BTreeMap::from([("daemon".into(), id)]))).await;
        assert!(hub.lock().await.daemons.is_empty());
        assert_eq!(hub.lock().await.sessions.len(), 1);
        drop(hub);
        endpoint.close().await;
        router.shutdown().await.unwrap();
    }
}
