//! Browser presentation instances over scoped daemon relationships.
//! Semantic HTML rendering, local form preparation and HTTP delivery have
//! separate owners; all shared state arrives through the shared client replica.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Form, Multipart, Path, State};
use axum::response::sse::{Event as SseEvent, Sse};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use misa_proto::view::{BlobRef, FieldKind};
#[cfg(test)]
use misa_proto::view::{ActionOn, Kind, Node, Span, State as NodeState};
use misa_proto::wire::Intent;
use misa_value::Value;
#[cfg(test)]
use misa_session::Runtime;

/// The stylesheet. Roles from the view tree, and nothing else.
pub const STYLE: &str = include_str!("style.css");

/// The script. All of it.
pub const SCRIPT: &str = include_str!("app.js");

mod html;
pub use html::{escape, render_main};
pub(crate) use html::render_scoped;

fn render_report(title: &str, value: &Value) -> String {
    render_scoped(&misa_client::request::report(title, value), &format!("report-{}:", next_id()))
}
fn read_report(title: &str, result: &misa_client::ReadValue, member: &str) -> Result<String, misa_proto::Fault> {
    Ok(render_scoped(&misa_client::interface::report(result, member, title)?, &format!("report-{}:", next_id())))
}

fn outcome_report(outcome: &misa_proto::invocation::Outcome) -> Option<String> {
    match outcome {
        misa_proto::invocation::Outcome::Completed { value } if *value != Value::Null => Some(render_report("Result", value)),
        _ => None,
    }
}

fn document_parts(title: &str, memory: &str, commands: &[misa_proto::wire::Command], region: &str, lead: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
<title>{title}</title>\n<link rel=\"stylesheet\" href=\"./style.css\">\n\
</head>\n<body data-session=\"{session_id}\">\n<nav><a href=\"/daemons\">Daemons and sessions</a></nav>{toolbar}<main id=\"main\">{lead}{region}</main>\n{declarations}\
<script src=\"./commands.js\" defer></script><script src=\"./app.js\" defer></script>\n</body>\n</html>\n",
        title = escape(title),
        session_id = escape(memory),
        toolbar = toolbar(),
        lead = lead,
        region = region,
        declarations = command_declarations(commands)
    )
}

fn toolbar() -> &'static str {
    "<nav aria-label=\"Display\"><label>Theme <select id=\"theme\"><option value=\"system\">System</option><option value=\"dark\">Dark</option><option value=\"plain\">Light</option></select></label></nav>"
}

/// What this client is holding and has not sent yet, and the way to add to it.
///
/// Written by the client rather than by the session, because it is the client's own state. It
/// is also the one direction of transfer no other frontend here has: a file input reads a file
/// off the machine the *person* is sitting at, which the daemon can neither see nor be told
/// about except by bytes.
///
/// The upload form is always here, even with nothing pending: a client that can only attach
/// once it already has an attachment could never attach the first one.
fn lead(pending: &[BlobRef]) -> String {
    let mut out = String::from("<aside class=\"attachments\">");
    if !pending.is_empty() {
        out.push_str(&format!(
            "<form method=\"post\" action=\"./detach\"><p>{} attachment{} ready to send",
            pending.len(),
            if pending.len() == 1 { "" } else { "s" }
        ));
        for blob in pending {
            let what = blob.media.as_deref().unwrap_or("a file");
            out.push_str(&format!(
                " <code>{}</code> ({}, {} bytes)",
                escape(&blob.hash),
                escape(what),
                blob.len
            ));
        }
        out.push_str("</p><button type=\"submit\">Discard</button></form>");
    }
    out.push_str(
        "<form method=\"post\" action=\"./attach\" enctype=\"multipart/form-data\">\
<label>Attach a file <input type=\"file\" name=\"file\"></label> \
<button type=\"submit\">Upload</button></form></aside>",
    );
    out
}

/// The session's commands, as a `<datalist>`.
///
/// The browser's own completion, driven by the session's own declaration, with no
/// script and no round trip. A client that wants ranking, previews, or frecency is
/// welcome to build one — that is what `misa-kit`'s picker is — but the plainest
/// possible frontend still knows what the session can do.
fn command_declarations(commands: &[misa_proto::wire::Command]) -> String {
    if commands.is_empty() {
        return String::new();
    }
    let mut out = String::from("<datalist id=\"misa-commands\">");
    for command in commands {
        let label = if command.args.is_empty() {
            command.description.clone()
        } else {
            format!(
                "{} — {}",
                command.description,
                command.args.iter().map(|arg| arg.label.clone()).collect::<Vec<_>>().join(" ")
            )
        };
        out.push_str(&format!(
            "<option value=\"/{}\" label=\"{}\">{}</option>",
            escape(&command.id),
            escape(&label),
            escape(&label)
        ));
    }
    out.push_str("</datalist>");
    out
}

mod blobs;
pub use blobs::Source;
use blobs::{blob_response, remote_download, upload};
#[cfg(test)]
use blobs::local_download;

/// The composer's action, which is the one action this client understands by name.
///
/// It is worth one constant: a submission is the only place where a client can hold something
/// the session's own action cannot carry — the bytes it just read — and the protocol already
/// declares how a submission with attachments is made ([`Intent::Prompt`]). Every other action
/// is forwarded without being read.
const COMPOSER: &str = "composer.submit";

/// A submitted form, as the intent it should be.
///
/// Normally the session's own action, forwarded: what an action means is the session's
/// business. Except when this client is holding attachments, which a form of text fields cannot
/// carry — then the submission is made the way the protocol declares submissions with
/// attachments, which is the one intent that can name bytes.
#[cfg(test)]
fn submitted_intent(form: &HashMap<String, String>, pending: &[BlobRef]) -> Intent {
    let action = form.get("action").map(String::as_str).unwrap_or_default();
    if !pending.is_empty() && action == COMPOSER {
        return Intent::Prompt {
            text: form.get("prompt").cloned().unwrap_or_default(),
            attachments: pending.to_vec(),
        };
    }
    intent_from_form(form)
}

/// Whether an intent has taken the attachments this client was holding.
///
/// Handing them over spends them: the session's own message now names them, and a copy still in
/// the strip would send the same picture again with the next prompt. Nothing is lost by
/// clearing — the bytes are in the store under the same name, so a second attempt costs one
/// question and no bytes.
fn spent(intent: &Intent) -> bool {
    matches!(intent, Intent::Prompt { attachments, .. } if !attachments.is_empty())
}

/// A submitted form, as an intent.
///
/// The hidden `node` and `action` fields are what a view node's action carries; every other
/// field is what a panel's inputs carried. One function because both paths must read a form
/// the same way, and a second copy would be a second place to forget a field.
fn intent_from_form(form: &HashMap<String, String>) -> Intent {
    let fields = form
        .iter()
        .filter(|(key, _)| key.as_str() != "action" && key.as_str() != "node")
        .map(|(key, value)| misa_proto::view::Field {
            id: key.clone(),
            label: key.clone(),
            value: value.clone(),
            hint: None,
            read_only: false,
            secret: false,
            kind: FieldKind::Inline,
        })
        .collect::<Vec<_>>();
    Intent::Action {
        node: form.get("node").cloned().unwrap_or_default(),
        action: form.get("action").cloned().unwrap_or_default(),
        args: misa_value::Value::Null,
        fields,
    }
}

/// Attach to a session over iroh and serve its view as HTML.
///
/// This is the shape the architecture asks for: the browser's server is an agent
/// client, and the browser is only a browser. It renders from the same
/// [`render_main`] the local path uses, and the only difference is where the view
/// arrives from.
pub async fn attach(ticket: &str, address: std::net::SocketAddr) -> Result<(), String> {
    hub::serve(&[ticket.to_string()], address).await
}

pub mod hub;

mod remote;
mod presentations;
mod requests;
mod activity;
mod overview;
mod actions;
mod commands;

mod updates;
pub use updates::Region;

/// A session reached over iroh, with no kernel in this process.
pub struct Remote {
    closed: tokio::sync::watch::Sender<bool>,
    activity: activity::Activity,
    claimed: std::sync::atomic::AtomicBool,
    presentation_changes: tokio::sync::mpsc::Sender<presentations::Change>,
    presentation_gate: tokio::sync::Mutex<()>,
    preferences: Arc<std::sync::Mutex<misa_client::composition::Preferences>>,
    region: Region,
    session: misa_proto::directory::Entry,
    daemon: Arc<misa_client::daemons::Daemon>,
    interaction: Arc<misa_client::interaction::Interaction>,
    task: tokio::task::AbortHandle,
    instance: String,
    blobs: Option<Arc<Source>>,
    pending: Arc<std::sync::Mutex<Vec<BlobRef>>>,
}
impl Remote {
    fn close(&self) {
        self.closed.send_replace(true);
        self.task.abort();
        self.activity.close();
        self.region.close();
    }
}
impl Drop for Remote { fn drop(&mut self) { self.close(); } }

/// A correlation id for an intent this process sends.
fn next_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// The routes for a remote session. The same documents, the same stream, and no
/// runtime to render from.
/// The routes for a remote session. The same documents, the same stream, and no
/// runtime to render from in this process.
pub fn remote_router(state: Arc<Remote>) -> Router {
    Router::new()
        .route("/", get(remote_page))
        .route("/events", get(remote_events))
        .route("/intent", post(remote_intent))
        .route("/presentations", post(presentations::configure))
        .route("/requests", get(requests::list))
        .route("/request", post(requests::open))
        .route("/request/events", get(requests::events))
        .route("/request.js", get(|| async { ([("content-type", "text/javascript")], include_str!("request.js")) }))
        .route("/respond", post(requests::respond))
        .route("/actions", get(actions::list))
        .route("/action", post(actions::open))
        .route("/perform", post(actions::perform))
        .route("/commands", get(commands::list))
        .route("/command", get(commands::open))
        .route("/completions", get(commands::candidates))
        .route("/commands.js", get(|| async { ([("content-type", "text/javascript")], include_str!("commands.js")) }))
        .route("/attach", post(remote_attach).layer(axum::extract::DefaultBodyLimit::max(misa_proto::blob::MAX_BLOB_BYTES + 8192)))
        .route("/detach", post(remote_detach))
        .route("/blob/{hash}", get(remote_blob))
        .route("/download", post(remote_download))
        .route("/style.css", get(|| async { ([("content-type", "text/css")], STYLE) }))
        .route("/app.js", get(|| async { ([("content-type", "text/javascript")], SCRIPT) }))
        .with_state(state)
}

/// A form submission from a browser, as the intent it is.
///
/// The same shape the local path uses, so a browser cannot tell which kind of session it is
/// talking to — and neither can a test.
async fn remote_intent(
    State(state): State<Arc<Remote>>,
    headers: axum::http::HeaderMap,
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let json = headers.get(header::ACCEPT).and_then(|value| value.to_str().ok()).is_some_and(|value| value.contains("application/json"));
    let refused = |error: String| {
        if json { return (StatusCode::BAD_REQUEST, axum::Json(serde_json::json!({"ok":false,"error":error}))).into_response(); }
        (StatusCode::BAD_REQUEST, Html(format!("<!doctype html><html><body><p role=\"alert\">{}</p><textarea readonly>{}</textarea><a href=\"./\">Back to session</a></body></html>", escape(&error), escape(form.get("prompt").map(String::as_str).unwrap_or(""))))).into_response()
    };
    let submitted = state.pending.lock().expect("pending attachments").clone();
    if form.get("action").map(String::as_str) == Some(COMPOSER) {
        let declarations = remote::declarations(&state.interaction);
        if let misa_kit::intent::Parsed::Needs { command, given, .. } = misa_kit::intent::parse(form.get("prompt").map(String::as_str).unwrap_or(""), &declarations) {
            return match commands::prepare(&state, &command, &given.into_iter().collect()).await {
                Ok(html) if json => axum::Json(serde_json::json!({"ok":true,"preparation":true,"report":html})).into_response(),
                Ok(html) => requests::page(StatusCode::OK, "Prepare command", format!("{html}<script src=\"./commands.js\"></script>")),
                Err(fault) => refused(fault.message),
            };
        }
    }
    if let Some(id) = form.get("action").filter(|id| id.as_str() != COMPOSER) {
        if let Ok(model) = misa_client::form::Form::action(&state.interaction.interface, id) {
            if model.fields.iter().any(|(id, field)| !field.optional && !form.contains_key(id)) {
                let html = actions::markup(&model);
                return if json { axum::Json(serde_json::json!({"ok":true,"report":html})).into_response() }
                    else { requests::page(StatusCode::OK, &model.title, html) };
            }
        }
    }
    let intent = match remote::submitted(&state, &form, &submitted) { Ok(intent) => intent, Err(error) => return refused(error) };
    let spent = spent(&intent);
    let result = match remote::prepare(&state.interaction, intent) {
        Ok(misa_client::interaction::Prepared::Read { member }) => {
            let title = member.query.id.clone();
            let selection = misa_proto::observation::Selection { scope: state.interaction.interface.scope.clone(), members: std::collections::BTreeMap::from([("report".into(), member)]) };
            return match state.daemon.client.read(selection, std::time::Duration::from_secs(20)).await {
                Ok(result) => match read_report(&title, &result, "report") {
                    Ok(html) if json => axum::Json(serde_json::json!({"ok":true,"report":html})).into_response(),
                    Ok(html) => Html(format!("<!doctype html><html><head><link rel=\"stylesheet\" href=\"./style.css\"></head><body><main>{html}<a href=\"./\">Back to session</a></main></body></html>")).into_response(),
                    Err(fault) => (StatusCode::BAD_GATEWAY, fault.message).into_response(),
                },
                Err(fault) => (StatusCode::BAD_GATEWAY, fault.message).into_response(),
            };
        },
        Ok(prepared) => remote::invoke(&state, prepared).await,
        Err(error) => return refused(error),
    };
    match result {
        Ok(outcome @ (misa_proto::invocation::Outcome::Completed { .. } | misa_proto::invocation::Outcome::Accepted { .. })) => {
            if spent { state.pending.lock().expect("pending attachments").retain(|blob| !submitted.contains(blob)); }
            let report = outcome_report(&outcome);
            if json { axum::Json(serde_json::json!({"ok":true,"outcome":outcome,"report":report})).into_response() }
            else if let Some(report) = report { requests::page(StatusCode::OK, "Result", report) }
            else { Redirect::to("./").into_response() }
        },
        Ok(outcome) => {
            let (status, message) = match &outcome {
                misa_proto::invocation::Outcome::Rejected { fault } => (StatusCode::BAD_REQUEST, &fault.message),
                misa_proto::invocation::Outcome::Indeterminate { fault } => (StatusCode::BAD_GATEWAY, &fault.message),
                _ => unreachable!(),
            };
            if json { (status, axum::Json(serde_json::json!({"ok":false,"error":message,"outcome":outcome}))).into_response() } else { refused(message.clone()) }
        },
        Err(error) => {
            let outcome = misa_proto::invocation::Outcome::Indeterminate {
                fault: misa_proto::Fault::new("invocation_unconfirmed", &error),
            };
            if json { (StatusCode::BAD_GATEWAY, axum::Json(serde_json::json!({"ok":false,"error":error,"outcome":outcome}))).into_response() } else { refused(error) }
        },
    }
}

/// One uploaded file, put in the daemon's store over the blob connection.
async fn remote_attach(State(state): State<Arc<Remote>>, multipart: Multipart) -> Response {
    let mut closed = state.closed.subscribe();
    if *closed.borrow() { return (StatusCode::GONE, "Presentation closed").into_response(); }
    tokio::select! {
        biased;
        _ = closed.changed() => (StatusCode::GONE, "Presentation closed").into_response(),
        response = upload(state.blobs.as_deref(), &state.pending, multipart) => response,
    }
}

async fn remote_detach(State(state): State<Arc<Remote>>) -> Response {
    state.pending.lock().expect("the pending list is never poisoned").clear();
    Redirect::to("./").into_response()
}

/// One blob, asked of the daemon this process is attached to.
async fn remote_blob(State(state): State<Arc<Remote>>, Path(hash): Path<String>) -> Response {
    blob_response(state.blobs.as_deref(), &hash).await
}

async fn remote_page(State(state): State<Arc<Remote>>) -> Html<String> {
    let lead = lead(&state.pending.lock().expect("the pending list is never poisoned"));
    let memory = format!("{}:{}:{}", state.daemon.identity(), state.interaction.interface.scope.incarnation, state.instance);
    Html(document_parts(&state.session.title, &memory, &remote::declarations(&state.interaction), &state.region.get(), &lead).replacen("<body ", &format!("<body data-preferences=\"{}\" ", escape(&format!("{}:{}", state.daemon.identity(), state.session.id))), 1).replacen("<main", &format!("{}<section id=\"activity\" aria-label=\"Operations and requests\">{}</section><main", presentations::controls(&state), state.region.activity_html()), 1).replace("</nav>", "<form method=\"post\" action=\"./close\"><button>Close presentation</button></form></nav>"))
}

async fn remote_events(
    State(state): State<Arc<Remote>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, std::convert::Infallible>>> {
    let lead = lead(&state.pending.lock().expect("the pending list is never poisoned"));
    state.region.events(lead, state.clone())
}

#[cfg(test)]
mod tests;
