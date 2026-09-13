//! The browser frontend, served by a client that talks to an agent daemon.
//!
//! # Three processes, and where this one sits
//!
//! ```text
//! browser  ──HTTP──▶  misa-web  ──iroh──▶  misa-daemon
//! (HTML, CSS,         (a client:            (the kernel and
//!  one SSE stream)     subscribes, sends     the session)
//!                      intents)
//! ```
//!
//! The browser never speaks the protocol and never sees a view tree. This process
//! is the client: it holds the subscription, owns the presentation policy that the
//! terminal and pixel frontends also own, and renders the tree to HTML. The browser
//! gets a document and a stream of replacements.
//!
//! # Why so little JavaScript
//!
//! Because the server can render, and the parts of "an app" that usually need a
//! framework are the parts that already exist in HTML: a `<form>` submits, a
//! `<details>` discloses, a `<table>` lays out, a `<button>` acts. What is left is
//! *liveness*, and liveness is one `EventSource` and one line that swaps a region —
//! see [`SCRIPT`], which is the whole of it.
//!
//! The form works with JavaScript disabled. That is not a fallback for a niche
//! audience; it is the check that the interaction was designed as a document
//! rather than as an app.
//!
//! # Theming
//!
//! Every node carries its *role* as a class, and nothing else. [`STYLE`] maps a role
//! to a look with custom properties, so a theme is a stylesheet and switching one is
//! swapping a stylesheet. No colour crosses the wire, which is the same rule the
//! other two frontends follow, expressed in the browser's own idiom.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Form, Multipart, Path, State};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::Router;
use tokio_stream::StreamExt as _;
use misa_proto::view::{ActionOn, BlobRef, FieldKind, Kind, Node, Span, SpanKind, State as NodeState};
use misa_proto::wire::{Intent, SessionInfo};
use misa_proto::{SubId, Query};
use misa_session::Runtime;
use tokio::sync::broadcast;

/// The stylesheet. Roles from the view tree, and nothing else.
pub const STYLE: &str = include_str!("style.css");

/// The script. All of it.
pub const SCRIPT: &str = include_str!("app.js");

/// Render a view as the `<main>` region of a document.
///
/// Every element is chosen because it *means* the same thing the node means: a
/// section is a `<section>`, a list is a list, a collapsible is a `<details>`, a
/// form is a form. That is the whole of the mapping, and it is why this renderer is
/// legible.
pub fn render_main(view: &Node) -> String {
    let mut out = String::new();
    render_node(view, &mut out);
    out
}

/// A document a browser can open directly, with or without JavaScript.
pub fn document(session: &SessionInfo, view: &Node) -> String {
    document_with(session, &render_main(view), "")
}

/// The same, with the client's own part of the region in front of the session's view.
///
/// The lead is the client's and not the session's: it says what *this process* is holding and
/// has not sent, which is a fact no session can know. It goes inside `#main` because that is
/// the element the stream replaces — one event, one thing to keep in step — and it is refreshed
/// by the post-redirect-get every form here already does.
pub fn document_with(session: &SessionInfo, region: &str, lead: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n\
<title>{title}</title>\n<link rel=\"stylesheet\" href=\"/style.css\">\n\
</head>\n<body data-session=\"{session_id}\">\n{toolbar}<main id=\"main\">{lead}{region}</main>\n{declarations}\
<script src=\"/app.js\" defer></script>\n</body>\n</html>\n",
        title = escape(&session.title),
        session_id = escape(&session.id),
        toolbar = toolbar(),
        lead = lead,
        region = region,
        declarations = declarations(session)
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
            "<form method=\"post\" action=\"/detach\"><p>{} attachment{} ready to send",
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
        "<form method=\"post\" action=\"/attach\" enctype=\"multipart/form-data\">\
<label>Attach a file <input type=\"file\" name=\"file\"></label> \
<button type=\"submit\">Upload</button></form></aside>",
    );
    out
}

/// The session's commands, as a `<datalist>`.
///
/// The browser's own completion, driven by the session's own declaration, with no
/// script and no round trip. A client that wants ranking, previews, or frecency is
/// welcome to build one — that is what `misa-client`'s picker is — but the plainest
/// possible frontend still knows what the session can do.
pub fn declarations(session: &SessionInfo) -> String {
    if session.commands.is_empty() {
        return String::new();
    }
    let mut out = String::from("<datalist id=\"misa-commands\">");
    for command in &session.commands {
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

fn render_node(node: &Node, out: &mut String) {
    let role = escape(&node.role);
    let id = if node.id.is_empty() {
        String::new()
    } else {
        format!(" id=\"{}\"", escape(&node.id))
    };
    let state = node
        .state
        .map(|state| format!(" data-state=\"{}\"", state_word(state)))
        .unwrap_or_default();
    let kind = format!(" kind.{}.", node.role);
    let _ = kind;
    // A thematic break is a void element: it has no closing tag to match.
    if let Kind::Rule = &node.kind {
        out.push_str(&format!("<hr class=\"n-{role}\"{id}{state}>"));
        return;
    }

    // A node that offers a submit action is a form, and its fields are its inputs.
    // That is the only reason a view node ever becomes a form, and it is enough for
    // every dialog the shipped session has.
    let submit = node.actions.iter().find(|action| action.on == ActionOn::Submit);
    if let Some(action) = submit {
        out.push_str(&format!(
            "<form class=\"n-{role}\"{id}{state} method=\"post\" action=\"/intent\">\
<input type=\"hidden\" name=\"node\" value=\"{node_id}\">\
<input type=\"hidden\" name=\"action\" value=\"{action}\">",
            role = role,
            id = id,
            state = state,
            node_id = escape(&node.id),
            action = escape(&action.id)
        ));
    } else {
        let element = element_for(node);
        out.push_str(&format!("<{element} class=\"n-{role}\"{id}{state}>"));
    }

    if let Some(label) = &node.label {
        out.push_str(&format!("<span class=\"label\">{}</span>", escape(label)));
    }

    match &node.kind {
        Kind::Section => {}
        Kind::Text { spans } => inline(spans, out),
        Kind::Heading { spans, .. } => inline(spans, out),
        // A quote's blocks and a rule's emptiness are structure, not content: the
        // element carries them and the stylesheet draws them. A rule is written before
        // this match, as a void element, so nothing goes inside it here.
        Kind::Quote => {}
        Kind::Rule => {}
        Kind::Code { lang, text, captures } => {
            out.push_str("<pre><code");
            if let Some(lang) = lang {
                out.push_str(&format!(" data-lang=\"{}\"", escape(lang)));
            }
            out.push('>');
            code(text, captures, out);
            out.push_str("</code></pre>");
        }
        Kind::List { ordered, items } => {
            // Each item is its own `<li>`; a nested list inside one is the child
            // nodes of that item, which the loop below emits.
            let mut items_out = String::new();
            for item in items {
                items_out.push_str("<li>");
                for child in item {
                    render_node(child, &mut items_out);
                }
                items_out.push_str("</li>");
            }
            out.push_str(&items_out);
            if *ordered {
                out.insert_str(0, "");
            }
        }
        Kind::Table { head, rows } => {
            out.push_str("<table>");
            if !head.is_empty() {
                out.push_str("<thead><tr>");
                for cell in head {
                    out.push_str("<th>");
                    inline(cell, out);
                    out.push_str("</th>");
                }
                out.push_str("</tr></thead>");
            }
            out.push_str("<tbody>");
            for row in rows {
                out.push_str("<tr>");
                for cell in row {
                    out.push_str("<td>");
                    inline(cell, out);
                    out.push_str("</td>");
                }
                out.push_str("</tr>");
            }
            out.push_str("</tbody></table>");
        }
        Kind::Fields { fields } => {
            out.push_str("<dl>");
            for field in fields {
                out.push_str(&format!("<dt>{}</dt><dd>", escape(&field.label)));
                match &field.kind {
                    _ if field.read_only => {
                        if field.secret {
                            out.push_str("••••");
                        } else if matches!(field.kind, FieldKind::Block) {
                            out.push_str(&format!("<pre>{}</pre>", escape(&field.value)));
                        } else {
                            out.push_str(&escape(&field.value));
                        }
                    },
                    _ if field.secret => out.push_str(&format!(
                        "<input type=\"password\" name=\"{}\" value=\"\">",
                        escape(&field.id)
                    )),
                    FieldKind::Block => out.push_str(&format!(
                        "<textarea name=\"{name}\" rows=\"3\" aria-label=\"{label}\" list=\"misa-commands\">{value}</textarea>",
                        name = escape(&field.id),
                        label = escape(&field.label),
                        value = escape(&field.value)
                    )),
                    FieldKind::Bool => out.push_str(&format!(
                        "<input type=\"checkbox\" name=\"{}\"{}>",
                        escape(&field.id),
                        if field.value == "true" { " checked" } else { "" }
                    )),
                    FieldKind::Choice { options, selected } => {
                        out.push_str(&format!("<select name=\"{}\">", escape(&field.id)));
                        for option in options {
                            out.push_str(&format!(
                                "<option value=\"{value}\"{selected}>{label}</option>",
                                value = escape(&option.value),
                                label = escape(&option.label),
                                selected = if Some(&option.value) == selected.as_ref() { " selected" } else { "" }
                            ));
                        }
                        out.push_str("</select>");
                    }
                    FieldKind::Inline => out.push_str(&format!(
                        "<input type=\"text\" name=\"{}\" value=\"{}\">",
                        escape(&field.id),
                        escape(&field.value)
                    )),
                }
                if let Some(hint) = &field.hint {
                    out.push_str(&format!("<small>{}</small>", escape(hint)));
                }
                out.push_str("</dd>");
            }
            out.push_str("</dl>");
        }
        Kind::Collapsible { summary } => {
            // `<details>` is why this node exists as a node: a short form and a long
            // form are a thing HTML already has a word for.
            out.push_str("<summary>");
            inline(summary, out);
            out.push_str("</summary>");
        }
        Kind::Image { blob, alt, .. } => {
            // The browser renders the shared image node and keeps its alternative text.
            out.push_str(&format!(
                "<img src=\"/blob/{hash}\" alt=\"{alt}\">",
                hash = escape(&blob.hash),
                alt = escape(alt)
            ));
        }
        Kind::Status { text } => out.push_str(&escape(text)),
        // A fact is written by the client's own formatter, and marked up so a
        // stylesheet can treat a number differently from a word. `<data>` says
        // exactly that: here is a value, and here is how to write it.
        Kind::Fact { value } => out.push_str(&format!(
            "<data value=\"{}\">{}</data>",
            escape(&value.to_string()),
            escape(&misa_render::fact::format(&node.role, value))
        )),
        Kind::Meter { label, value, max } => {
            out.push_str(&format!(
                "<meter min=\"0\" max=\"{max}\" value=\"{value}\" aria-label=\"{label}\"></meter>\
<span class=\"value\">{text}</span>",
                max = max,
                value = value,
                label = escape(label),
                text = format!("{value}/{max}")
            ));
        }
    }

    for child in &node.children {
        render_node(child, out);
    }

    // A node that is not a form may still offer a click action, and a button that is not in a
    // form posts nothing: each one gets a one-button form of its own, which is the whole of
    // what it takes for a panel's buttons to work in a browser with no script at all.
    if submit.is_none() {
        for action in &node.actions {
            if action.id == "attachment.save" {
                out.push_str(&format!("<form method=\"post\" action=\"/download\"><input type=\"hidden\" name=\"node\" value=\"{}\"><button type=\"submit\">{}</button></form>", escape(&node.id), escape(action.label.as_deref().unwrap_or("Save attachment"))));
                continue;
            }
            out.push_str(&format!(
                "<form class=\"n-{role}.action\" method=\"post\" action=\"/intent\">\
<input type=\"hidden\" name=\"node\" value=\"{node_id}\">\
<input type=\"hidden\" name=\"action\" value=\"{action}\">\
<button type=\"submit\">{label}</button></form>",
                role = role,
                node_id = escape(&node.id),
                action = escape(&action.id),
                label = escape(action.label.as_deref().unwrap_or(&action.id))
            ));
        }
    }
    if submit.is_some() {
        out.push_str("<button type=\"submit\">");
        out.push_str(&escape(submit.and_then(|action| action.label.as_deref()).unwrap_or("Send")));
        out.push_str("</button></form>");
    } else {
        let element = element_for(node);
        out.push_str(&format!("</{element}>"));
    }
}

fn element_for(node: &Node) -> String {
    match &node.kind {
        Kind::Text { .. } => "p".into(),
        // HTML has one element per heading level, so the level picks the tag.
        Kind::Heading { level, .. } => format!("h{}", (*level).clamp(1, 6)),
        Kind::Code { .. } => "div".into(),
        Kind::List { ordered: true, .. } => "ol".into(),
        Kind::List { .. } => "ul".into(),
        Kind::Table { .. } => "div".into(),
        Kind::Fields { .. } => "dl".into(),
        Kind::Collapsible { .. } => "details".into(),
        Kind::Image { .. } => "figure".into(),
        Kind::Status { .. } => "p".into(),
        Kind::Fact { .. } => "data".into(),
        Kind::Meter { .. } => "p".into(),
        Kind::Quote => "blockquote".into(),
        // A rule is written before this is reached; named here so the match stays total.
        Kind::Rule => "hr".into(),
        Kind::Section => "section".into(),
    }
}

fn inline(spans: &[Span], out: &mut String) {
    for span in spans {
        if span.text.contains('\n') {
            // A newline inside a run is a paragraph the session chose; the browser
            // is told with elements rather than with `white-space`.
            let mut first = true;
            for part in span.text.split('\n') {
                if !first {
                    out.push_str(&format!("</{}>", span_element(&span.kind)));
                    out.push_str(&format!("<{}>", span_element(&span.kind)));
                }
                first = false;
                out.push_str(&escape(part));
            }
            continue;
        }
        let element = span_element(&span.kind);
        out.push_str(&format!("<{element}>"));
        out.push_str(&escape(&span.text));
        out.push_str(&format!("</{element}>"));
    }
}

fn span_element(kind: &SpanKind) -> String {
    match kind {
        SpanKind::Plain => "span".into(),
        SpanKind::Strong => "strong".into(),
        SpanKind::Emphasis => "em".into(),
        SpanKind::Strikethrough => "del".into(),
        SpanKind::Code => "code".into(),
        SpanKind::Link { href } => format!("a href=\"{}\"", escape(href)),
        SpanKind::Token { name } => format!("span data-token=\"{}\"", escape(name)),
    }
}

/// A code block, with its captures as `<span data-token>`.
///
/// No highlighting is computed here: the session did that, and this maps a capture
/// name to a class. A grammar that knows more than the stylesheet does is not an
/// error, it is plain text.
fn code(text: &str, captures: &[misa_proto::view::Capture], out: &mut String) {
    if captures.is_empty() {
        out.push_str(&escape(text));
        return;
    }
    let mut cursor = 0usize;
    for capture in captures {
        let start = capture.start as usize;
        let end = capture.end as usize;
        if start < cursor || end > text.len() {
            continue;
        }
        out.push_str(&escape(&text[cursor..start]));
        out.push_str(&format!("<span data-token=\"{}\">", escape(&capture.token)));
        out.push_str(&escape(&text[start..end]));
        out.push_str("</span>");
        cursor = end;
    }
    out.push_str(&escape(&text[cursor..]));
}

fn state_word(state: NodeState) -> &'static str {
    match state {
        NodeState::Pending => "pending",
        NodeState::Streaming => "streaming",
        NodeState::Done => "done",
        NodeState::Failed => "failed",
        NodeState::Cancelled => "cancelled",
    }
}

/// HTML-escape everything a session sends. A view tree is data and may contain
/// anything a model wrote, so nothing reaches a browser unescaped.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(character),
        }
    }
    out
}

/// The client server: one runtime, one HTML region, and a stream of replacements.
pub struct App {
    runtime: Arc<Runtime>,
    /// The last rendered region, so a browser that connects mid-turn is sent a
    /// document rather than waiting for the next change.
    ///
    /// Behind its own `Arc` because an app with a blob store is a second `App` over one
    /// renderer, and two apps that held two copies of the last region would be two answers to
    /// "what is on screen".
    latest: Arc<std::sync::Mutex<Arc<String>>>,
    updates: broadcast::Sender<Arc<String>>,
    /// Where an `<img>` in the document gets its bytes. Nothing, for a session with no store
    /// behind it, which is a session whose images are drawn as their alt text.
    blobs: Option<Arc<Source>>,
    /// The blobs this process has uploaded and not yet sent. The client's own state, and the
    /// only state it keeps: what a session holds is read from the session.
    pending: std::sync::Mutex<Vec<BlobRef>>,
}

impl App {
    pub fn new(runtime: Arc<Runtime>) -> Arc<App> {
        let (updates, _) = broadcast::channel(64);
        let app = Arc::new(App {
            runtime,
            latest: Arc::new(std::sync::Mutex::new(Arc::new(String::new()))),
            updates,
            blobs: None,
            pending: std::sync::Mutex::new(Vec::new()),
        });
        app.clone().spawn_renderer();
        app
    }

    /// The same app, with somewhere to fetch the bytes a view's images name.
    ///
    /// The region and the stream are shared with the app this came from, so the renderer that
    /// is already running keeps feeding the browser: a second one would render every change
    /// twice, which is work for nothing.
    pub fn with_blobs(self: &Arc<App>, blobs: Arc<Source>) -> Arc<App> {
        Arc::new(App {
            runtime: self.runtime.clone(),
            latest: self.latest.clone(),
            updates: self.updates.clone(),
            blobs: Some(blobs),
            // The pending list is shared: an app with a store is the same client, and two lists
            // would be two answers to "what is about to be sent".
            pending: std::sync::Mutex::new(Vec::new()),
        })
    }

    /// Render whenever the session's revision changes.
    ///
    /// One renderer for every browser, and the result broadcast: five tabs on one
    /// session are one subscription, which is what a server-side client buys that a
    /// JavaScript one cannot.
    fn spawn_renderer(self: Arc<Self>) {
        let mut revision = self.runtime.watch_rev();
        tokio::spawn(async move {
            loop {
                self.render();
                if revision.changed().await.is_err() {
                    return;
                }
            }
        });
    }

    fn render(&self) {
        let html = match self.runtime.view() {
            Ok(view) => render_main(&view),
            Err(fault) => format!("<p class=\"n-notice.error\">{}</p>", escape(&fault.message)),
        };
        let html = Arc::new(html);
        *self.latest.lock().expect("the last region is never poisoned") = html.clone();
        let _ = self.updates.send(html);
    }

    pub fn runtime(&self) -> &Arc<Runtime> {
        &self.runtime
    }
}

/// Where the bytes a view names come from.
///
/// Two sources because there are two kinds of client process here, and the difference is
/// exactly the architecture's difference: the local one holds the store, and the remote one
/// has to ask the daemon over the same ticket it uses for the session. A frontend that decided
/// *how* to fetch would be a frontend with a transport inside it.
pub enum Source {
    /// The store is in this process, which is what `--local` is.
    Local(Arc<misa_kernel::Blobs>),
    /// The bytes are on the daemon that hosts the session.
    Remote(Arc<misa_net::blob::Store>),
}

impl Source {
    /// Put bytes here, and get back the name they now have.
    ///
    /// The one write this client makes. It is content addressing on both sides — the name is
    /// the hash of the bytes — so an upload that has been made before costs a question and no
    /// bytes, which is what makes attaching the same screenshot twice free.
    pub async fn put(&self, bytes: Vec<u8>, media: Option<&str>) -> Result<BlobRef, String> {
        match self {
            Source::Local(blobs) => blobs.put(&bytes, media),
            Source::Remote(store) => store.share(bytes, media).await,
        }
    }

    /// The bytes a hash names, and what they are.
    pub async fn get(&self, hash: &str) -> Result<Option<(String, Vec<u8>)>, String> {
        let unknown = || "application/octet-stream".to_string();
        match self {
            Source::Local(blobs) => {
                Ok(blobs.get(hash).map(|bytes| (blobs.media(hash).unwrap_or_else(unknown), bytes)))
            }
            Source::Remote(store) => {
                Ok(store.get(hash).await?.map(|blob| (blob.media.unwrap_or_else(unknown), blob.bytes)))
            }
        }
    }
}

/// One blob, for an `<img>` a view asked for.
///
/// Immutable, because a blob is named by the hash of its own content: the bytes behind a name
/// can never change, so a browser may keep them as long as it likes and scrolling back through
/// a transcript fetches nothing. That is what makes serving every request straight from the
/// store — with no cache here, and no invalidation to get wrong — the right shape.
async fn blob_response(source: Option<&Source>, hash: &str) -> Response {
    let Some(source) = source else {
        return (StatusCode::NOT_FOUND, "this client has no blob store").into_response();
    };
    if !misa_proto::blob::valid_hash(hash) {
        // A name that is not a content hash names nothing, and answering before asking is what
        // keeps a path-shaped name from being a path.
        return (StatusCode::NOT_FOUND, "not a content hash").into_response();
    }
    match source.get(hash).await {
        Ok(Some((media, bytes))) => (
            [
                (header::CONTENT_TYPE, media),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable".to_string()),
            ],
            bytes,
        )
            .into_response(),
        Ok(None) => (StatusCode::NOT_FOUND, "no such blob").into_response(),
        Err(message) => (StatusCode::BAD_GATEWAY, message).into_response(),
    }
}

fn save_intent(node: String) -> misa_proto::wire::Intent {
    misa_proto::wire::Intent::Action {
        node,
        action: "attachment.save".into(),
        args: misa_value::Value::Null,
        fields: vec![],
    }
}

async fn download_response(source: Option<&Source>, download: misa_proto::wire::Download) -> Response {
    let Some(blob) = download.blob else {
        return (StatusCode::NOT_FOUND, download.error).into_response();
    };
    let mut response = blob_response(source, &blob.hash).await;
    if response.status() == StatusCode::OK {
        let name = if !download.name.is_empty()
            && download.name.bytes().all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        {
            download.name
        } else {
            "attachment.bin".into()
        };
        response.headers_mut().insert(
            axum::http::header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{name}\"").parse().unwrap(),
        );
    }
    response
}

async fn local_download(
    State(app): State<Arc<App>>,
    Form(fields): Form<std::collections::HashMap<String, String>>,
) -> Response {
    let mut events = app.runtime.subscribe_events();
    let context =
        misa_proto::wire::RequestContext { recipient: misa_proto::wire::RequestContext::connection(), id: next_id() };
    let faults = app.runtime.intent_from(save_intent(fields.get("node").cloned().unwrap_or_default()), Some(context));
    if let Some(fault) = faults.first() {
        return (StatusCode::BAD_REQUEST, fault.message.clone()).into_response();
    }
    let answer = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while let Ok(emission) = events.recv().await {
            if emission.recipient == Some(context.recipient) {
                if let misa_proto::wire::SessionEvent::DownloadReady { id, download } = emission.event {
                    if id == context.id {
                        return Some(download);
                    }
                }
            }
        }
        None
    })
    .await;
    match answer {
        Ok(Some(download)) => download_response(app.blobs.as_deref(), download).await,
        _ => (StatusCode::GATEWAY_TIMEOUT, "The session did not answer the save request").into_response(),
    }
}

struct DownloadRequest {
    node: String,
    reply: tokio::sync::oneshot::Sender<Result<misa_proto::wire::Download, String>>,
}

async fn remote_download(
    State(state): State<Arc<Remote>>,
    Form(fields): Form<std::collections::HashMap<String, String>>,
) -> Response {
    let (reply, answer) = tokio::sync::oneshot::channel();
    if state
        .downloads
        .send(DownloadRequest { node: fields.get("node").cloned().unwrap_or_default(), reply })
        .await
        .is_err()
    {
        return (StatusCode::BAD_GATEWAY, "The session is disconnected").into_response();
    }
    match tokio::time::timeout(std::time::Duration::from_secs(20), answer).await {
        Ok(Ok(Ok(download))) => download_response(state.blobs.as_deref(), download).await,
        Ok(Ok(Err(error))) => (StatusCode::BAD_REQUEST, error).into_response(),
        _ => (StatusCode::GATEWAY_TIMEOUT, "The session did not answer the save request").into_response(),
    }
}

async fn blob(State(app): State<Arc<App>>, Path(hash): Path<String>) -> Response {
    blob_response(app.blobs.as_deref(), &hash).await
}

/// The routes. A document, a stream, a form, and two assets.
pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(page))
        .route("/events", get(events))
        .route("/intent", post(intent))
        .route("/attach", post(attach_file))
        .route("/detach", post(detach))
        .route("/blob/{hash}", get(blob))
        .route("/download", post(local_download))
        .route("/style.css", get(|| async { ([("content-type", "text/css")], STYLE) }))
        .route("/app.js", get(|| async { ([("content-type", "text/javascript")], SCRIPT) }))
        .with_state(app)
}

/// Take one uploaded file, put it in the store, and remember its name.
///
/// One file per submission, because a file input is one file and a second one would be a
/// second decision about what "attach three files at once" means. The bytes go straight into
/// the content-addressed store the views already point at, so what is uploaded is addressable
/// by hash the moment it arrives, and nothing but a name ever enters an intent.
async fn attach_file(State(app): State<Arc<App>>, multipart: Multipart) -> Response {
    upload(app.blobs.as_deref(), &app.pending, multipart).await
}

/// Forget what is pending, without deleting anything: a blob belongs to the store once it is
/// written, and an upload somebody changed their mind about is not a reason to touch it.
async fn detach(State(app): State<Arc<App>>) -> Response {
    app.pending.lock().expect("the pending list is never poisoned").clear();
    Redirect::to("/").into_response()
}

/// What a file input's submission becomes.
async fn upload(
    source: Option<&Source>,
    pending: &std::sync::Mutex<Vec<BlobRef>>,
    mut multipart: Multipart,
) -> Response {
    let Some(source) = source else {
        return upload_fault("this client has nowhere to put a file");
    };
    let field = match multipart.next_field().await {
        Ok(Some(field)) => field,
        Ok(None) => return upload_fault("there was nothing to attach"),
        Err(error) => return upload_fault(&format!("the upload could not be read: {error}")),
    };
    let media = field.content_type().map(str::to_string);
    let bytes = match field.bytes().await {
        Ok(bytes) => bytes,
        Err(error) => return upload_fault(&format!("the upload could not be read: {error}")),
    };
    // Refused here as well as there, because a daemon should not have to receive a gigabyte to
    // say no to it.
    if bytes.len() > misa_proto::blob::MAX_BLOB_BYTES {
        return upload_fault(&format!(
            "{} bytes is larger than the {MAX_BLOB_BYTES} byte bound",
            bytes.len(),
            MAX_BLOB_BYTES = misa_proto::blob::MAX_BLOB_BYTES
        ));
    }
    match source.put(bytes.to_vec(), media.as_deref()).await {
        Ok(blob) => {
            pending.lock().expect("the pending list is never poisoned").push(blob);
            Redirect::to("/").into_response()
        }
        Err(message) => upload_fault(&message),
    }
}

fn upload_fault(message: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Html(format!("<p class=\"n-notice.error\">{}</p>", escape(message))),
    )
        .into_response()
}

async fn page(State(app): State<Arc<App>>) -> Html<String> {
    let view = app.runtime.view().ok();
    let lead = lead(&app.pending.lock().expect("the pending list is never poisoned"));
    match view {
        Some(view) => Html(document_with(&app.runtime.info(), &render_main(&view), &lead)),
        None => Html(format!(
            "<!doctype html><html><body><main id=\"main\">{}</main></body></html>",
            escape("this session cannot build a view right now")
        )),
    }
}

/// The live region, as a server-sent event stream.
///
/// One event type and one field: the whole `<main>`. That is deliberately the
/// least clever thing that works. A finer-grained patch protocol would need the
/// browser to know node ids, which would mean the browser participated in the view
/// tree — and the point of rendering on the server is that it does not.
async fn events(State(app): State<Arc<App>>) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, std::convert::Infallible>>> {
    let current = app.latest.lock().expect("the last region is never poisoned").clone();
    let lead = lead(&app.pending.lock().expect("the pending list is never poisoned"));
    region_stream(current, lead, app.updates.subscribe())
}
async fn intent(State(app): State<Arc<App>>, Form(form): Form<HashMap<String, String>>) -> Response {
    let mut pending = app.pending.lock().expect("the pending list is never poisoned");
    let intent = submitted_intent(&form, &pending);
    let spent = spent(&intent);
    let faults = app.runtime.intent(intent);
    let _ = faults;
    if spent {
        pending.clear();
    }
    // Post, redirect, get: a form submission is a request for a new document, and the
    // browser should end up with a URL it can bookmark and reload.
    Redirect::to("/").into_response()
}

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

/// Serve one session to browsers on a loopback address.
pub async fn serve(runtime: Arc<Runtime>, address: std::net::SocketAddr) -> Result<(), String> {
    listen(App::new(runtime), address).await
}

/// The same, with a store to fetch the bytes a view's images name.
pub async fn serve_with(
    runtime: Arc<Runtime>,
    blobs: Arc<Source>,
    address: std::net::SocketAddr,
) -> Result<(), String> {
    listen(App::new(runtime).with_blobs(blobs), address).await
}

async fn listen(app: Arc<App>, address: std::net::SocketAddr) -> Result<(), String> {
    let listener = tokio::net::TcpListener::bind(address).await.map_err(|err| err.to_string())?;
    let bound = listener.local_addr().map_err(|err| err.to_string())?;
    tracing::info!("serving at http://{bound}");
    axum::serve(listener, router(app))
        .await
        .map_err(|err| err.to_string())
}
/// Attach to a session over iroh and serve its view as HTML.
///
/// This is the shape the architecture asks for: the browser's server is an agent
/// client, and the browser is only a browser. It renders from the same
/// [`render_main`] the local path uses, and the only difference is where the view
/// arrives from.
pub async fn attach(ticket: &str, address: std::net::SocketAddr) -> Result<(), String> {
    // A ticket, or a pairing string: whatever the daemon printed or the QR said.
    let (parsed, code) = misa_proto::Pairing::given(&ticket)?;
    let endpoint = misa_net::iroh::bind_for(&parsed.node).await?;
    let target = misa_net::iroh::address_of(&parsed.node)?;
    if let Some(code) = &code {
        let message = misa_net::iroh::Client::pair(&endpoint, target.clone(), code, "a browser").await?;
        tracing::info!(%message, "paired");
    }
    // The address the session is reached at serves that session's blobs too: a ticket names one
    // node, so a client that can reach a session can fetch what its views point at.
    let blobs = Arc::new(Source::Remote(misa_net::blob::Store::new(endpoint.clone(), target.clone())));
    let info = misa_proto::ClientInfo::new("misa-web", env!("CARGO_PKG_VERSION"));
    let mut client = misa_net::iroh::Client::connect(&endpoint, target, info, &parsed.session).await?;
    client.subscribe(SubId(1), Query::new(misa_proto::VIEW_QUERY)).await?;

    let session = client.session().cloned();
    let region = Region::new();

    // One task owns the connection, because a client is a stream and an intent is a write
    // to the same stream. A browser's form submission arrives on a channel, which is what
    // makes it indistinguishable from an intent sent by a terminal: both land here.
    let (intents, mut outgoing) = tokio::sync::mpsc::unbounded_channel::<misa_proto::wire::Intent>();
    let stream = region.clone();
    let (downloads, mut download_requests) = tokio::sync::mpsc::channel::<DownloadRequest>(16);
    tokio::spawn(async move {
        let mut pending_downloads = std::collections::HashMap::<u64, tokio::sync::oneshot::Sender<Result<misa_proto::wire::Download, String>>>::new();
        loop {
            tokio::select! {
                message = client.next() => match message {
                    Ok(Some(misa_proto::SessionMsg::Download { id, download })) => {
                        if let Some(reply) = pending_downloads.remove(&id) { let _ = reply.send(Ok(download)); }
                    }
                    Ok(Some(misa_proto::SessionMsg::Fault { id: Some(id), fault })) => {
                        if let Some(reply) = pending_downloads.remove(&id) { let _ = reply.send(Err(fault.message)); }
                    }
                    Ok(Some(message)) => match stream.receive(&message) {
                        Ok(true) => {},
                        Err(_) => { let _ = client.subscribe(SubId(1), Query::new(misa_proto::VIEW_QUERY)).await; },
                        Ok(false) => {},
                    },

                    Ok(None) | Err(_) => return,
                },
                request = download_requests.recv() => if let Some(request) = request {
                    let id = next_id();
                    pending_downloads.insert(id, request.reply);
                    if client.intent(id, save_intent(request.node)).await.is_err() { return; }
                },
                intent = outgoing.recv() => match intent {
                    Some(intent) => {
                        if client.intent(next_id(), intent).await.is_err() {
                            return;
                        }
                    }
                    None => return,
                },
            }
        }
    });

    let listener = tokio::net::TcpListener::bind(address).await.map_err(|err| err.to_string())?;
    tracing::info!("serving at http://{}", listener.local_addr().map_err(|err| err.to_string())?);

    let state = Arc::new(Remote {
        region,
        session,
        intents,
        downloads,
        blobs: Some(blobs),
        pending: Arc::new(std::sync::Mutex::new(Vec::new())),
    });
    axum::serve(listener, remote_router(state)).await.map_err(|err| err.to_string())
}

mod updates;
pub use updates::Region;

/// A session reached over iroh, with no kernel in this process.
pub struct Remote {
    downloads: tokio::sync::mpsc::Sender<DownloadRequest>,
    region: Region,
    session: Option<SessionInfo>,
    /// Where an intent from the browser goes: to the task that owns the connection.
    intents: tokio::sync::mpsc::UnboundedSender<misa_proto::wire::Intent>,
    /// Where an `<img>` goes to get its bytes, and where an upload goes: the daemon's blob
    /// store, over its own connection, which is the one part of this process that is not the
    /// session stream.
    blobs: Option<Arc<Source>>,
    /// The blobs this process has uploaded and not yet sent, shared with the strip that shows
    /// them.
    pending: Arc<std::sync::Mutex<Vec<BlobRef>>>,
}

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
        .route("/attach", post(remote_attach))
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
    Form(form): Form<HashMap<String, String>>,
) -> Response {
    let mut pending = state.pending.lock().expect("the pending list is never poisoned");
    let intent = submitted_intent(&form, &pending);
    let spent = spent(&intent);
    let _ = state.intents.send(intent);
    if spent {
        pending.clear();
    }
    Redirect::to("/").into_response()
}

/// One uploaded file, put in the daemon's store over the blob connection.
async fn remote_attach(State(state): State<Arc<Remote>>, multipart: Multipart) -> Response {
    upload(state.blobs.as_deref(), &state.pending, multipart).await
}

async fn remote_detach(State(state): State<Arc<Remote>>) -> Response {
    state.pending.lock().expect("the pending list is never poisoned").clear();
    Redirect::to("/").into_response()
}

/// One blob, asked of the daemon this process is attached to.
async fn remote_blob(State(state): State<Arc<Remote>>, Path(hash): Path<String>) -> Response {
    blob_response(state.blobs.as_deref(), &hash).await
}

async fn remote_page(State(state): State<Arc<Remote>>) -> Html<String> {
    let lead = lead(&state.pending.lock().expect("the pending list is never poisoned"));
    let session = state.session.clone();
    match session {
        Some(session) => Html(document_with(&session, &state.region.get(), &lead)),
        None => Html(format!("<!doctype html><html><body><main id=\"main\">{lead}{}</main></body></html>", state.region.get())),
    }
}

async fn remote_events(
    State(state): State<Arc<Remote>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, std::convert::Infallible>>> {
    let lead = lead(&state.pending.lock().expect("the pending list is never poisoned"));
    state.region.events(lead)
}

/// The stream a browser reads: the region it should be showing, then every change
/// to it. One stream type for both paths, so the browser cannot tell them apart.
fn region_stream(
    current: Arc<String>,
    lead: String,
    updates: broadcast::Receiver<Arc<String>>,
) -> Sse<impl tokio_stream::Stream<Item = Result<SseEvent, std::convert::Infallible>>> {
    let first = format!("{lead}{current}");
    let stream = tokio_stream::once(Ok(SseEvent::default().data(first))).chain(
        tokio_stream::wrappers::BroadcastStream::new(updates).filter_map(move |update| {
            let lead = lead.clone();
            update.ok().map(move |html| Ok(SseEvent::default().data(format!("{lead}{html}"))))
        }),
    );
    Sse::new(stream).keep_alive(KeepAlive::default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::view::{Action, BlobRef, Capture, Field};

    #[tokio::test]
    async fn attachment_button_downloads_kernel_confirmed_bytes_as_a_file() {
        let kernel = Arc::new(misa_kernel::LocalKernel::new(misa_kernel::ScriptedProvider::always("done")));
        let blobs = kernel.blobs().clone();
        let stored = blobs.put(PNG, Some("image/png")).unwrap();
        let runtime = Runtime::start("save", "Save", None, kernel, "scripted", "test", misa_value::Value::Null);
        runtime.intent(misa_proto::wire::Intent::Prompt { text: "save it".into(), attachments: vec![stored.clone()] });
        fn target(node: &Node) -> Option<String> {
            if node.actions.iter().any(|action| action.id == "attachment.save") { return Some(node.id.clone()); }
            node.children.iter().find_map(target)
        }
        let mut revision = runtime.watch_rev();
        let (tree, node) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let tree = runtime.view().unwrap();
                if let Some(node) = target(&tree) { break (tree, node); }
                revision.changed().await.unwrap();
            }
        }).await.expect("attachment was not durably recorded");
        let html = render_main(&tree);
        assert!(html.contains("action=\"/download\""));
        assert!(html.contains("Save attachment"));
        let app = App::new(runtime).with_blobs(Arc::new(Source::Local(blobs)));
        let response = local_download(State(app), Form(std::collections::HashMap::from([("node".into(), node)]))).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[axum::http::header::CONTENT_DISPOSITION], format!("attachment; filename=\"{}.png\"", stored.hash));
        assert_eq!(axum::body::to_bytes(response.into_body(), 1024).await.unwrap().as_ref(), PNG);
    }

    #[tokio::test]
    async fn usage_dashboard_is_typed_and_renders_on_terminal_and_browser() {
        use misa_proto::wire::Intent;
        use misa_value::Value;
        let kernel = misa_kernel::LocalKernel::new(misa_kernel::ScriptedProvider::new([]));
        let runtime = misa_session::Runtime::start(
            "usage",
            "Usage",
            None,
            std::sync::Arc::new(kernel),
            "scripted",
            "test",
            Value::Null,
        );
        assert!(runtime.intent(Intent::Command { name: "usage".into(), args: Value::Null }).is_empty());
        let facts = misa_kernel::usage::parse("kimi", &serde_json::json!({"usage":{"limit":100,"used":25}}));
        assert!(
            runtime
                .dispatch(
                    misa_reframe::Event::new("kernel/usage").with("id", Value::str("usage.1")).with("facts", facts)
                )
                .is_empty()
        );
        let tree = runtime.view().unwrap();
        misa_proto::view::validate(&tree).unwrap();
        let html = render_main(&tree);
        let terminal = misa_render::to_plain(&misa_render::render(&tree, &misa_render::Theme::plain(), 100));
        for expected in ["Kimi", "remaining", "75", "$0.00"] {
            assert!(html.contains(expected), "{expected}: {html}");
            assert!(terminal.contains(expected), "{expected}: {terminal}");
        }
        assert!(html.contains("value.money"));
    }

    /// Bytes the store will recognise as a picture, which is what a store sniffs for.
    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3];

    fn composer() -> Node {
        Node::new(
            "composer",
            Kind::Fields {
                fields: vec![Field {
                    id: "prompt".into(),
                    label: "Message".into(),
                    value: String::new(),
                    hint: None,
                    read_only: false,
                    secret: false,
                    kind: FieldKind::Block,
                }],
            },
        )
        .id("composer")
        .action(Action {
            id: "composer.submit".into(),
            on: ActionOn::Submit,
            label: Some("Send".into()),
            args: misa_value::Value::Null,
        })
        .child(Node::text("composer.hint", [Span::plain("enter to send")]))
    }

    fn view() -> Node {
        Node::section("session")
            .id("session")
            .child(
                Node::text("message.user", [Span::plain("what is <this>?")])
                    .id("msg.1")
                    .state(NodeState::Done),
            )
            .child(
                Node::new(
                    "tool.call",
                    Kind::Collapsible { summary: vec![Span::plain("echo")] },
                )
                .id("call.1")
                .child(Node::new(
                    "tool.result",
                    Kind::Code {
                        lang: Some("rust".into()),
                        text: "let x = 1;".into(),
                        captures: vec![Capture { start: 0, end: 3, token: "keyword".into() }],
                    },
                )),
            )
            .child(Node::new(
                "tool.batch",
                Kind::List {
                    ordered: true,
                    items: vec![
                        vec![Node::text("x", [Span::plain("first")])],
                        vec![Node::text("x", [Span::plain("second")])],
                    ],
                },
            ))
            .child(composer())
    }

    #[test]
    fn a_submission_with_attachments_is_made_the_way_the_protocol_declares_one() {
        // A form of text fields cannot carry bytes, so when this client is holding some the
        // submission stops being "forward the session's action" and becomes the declared
        // text-with-attachments intent. Every other action is forwarded untouched: what an
        // action means is the session's business, and a client that guessed would be a client
        // with a second opinion about the session's own vocabulary.
        let pending = vec![BlobRef { hash: "a".repeat(64), len: 12, media: Some("image/png".into()) }];
        let mut form = HashMap::new();
        form.insert("node".to_string(), "composer".to_string());
        form.insert("action".to_string(), "composer.submit".to_string());
        form.insert("prompt".to_string(), "look at this".to_string());
        assert_eq!(
            submitted_intent(&form, &pending),
            Intent::Prompt { text: "look at this".into(), attachments: pending.clone() }
        );

        // Nothing pending: the session's own action, with its fields, exactly as submitted.
        match submitted_intent(&form, &[]) {
            Intent::Action { action, fields, .. } => {
                assert_eq!(action, "composer.submit");
                assert!(fields.iter().any(|field| field.id == "prompt" && field.value == "look at this"));
            }
            other => panic!("a form with nothing pending became a `{}`", other.name()),
        }

        // Something pending, but not a submission: still the action, because an attachment is
        // not a reason to turn a button into a prompt.
        let mut other = HashMap::new();
        other.insert("action".to_string(), "queue.take".to_string());
        match submitted_intent(&other, &pending) {
            Intent::Action { action, .. } => assert_eq!(action, "queue.take"),
            other => panic!("a queue button became a `{}`", other.name()),
        }
    }

    #[test]
    fn the_strip_says_what_is_waiting_and_offers_the_way_to_add_more() {
        // The upload form is there even with nothing pending, because a client that could only
        // attach once it already had an attachment could never attach the first one.
        let empty = lead(&[]);
        assert!(empty.contains("action=\"/attach\""), "{empty}");
        assert!(empty.contains("multipart/form-data"), "{empty}");
        assert!(!empty.contains("/detach"), "{empty}");

        let pending = vec![
            BlobRef { hash: "a".repeat(64), len: 12, media: Some("image/png".into()) },
            BlobRef { hash: "b".repeat(64), len: 3, media: None },
        ];
        let strip = lead(&pending);
        assert!(strip.contains("2 attachments"), "{strip}");
        assert!(strip.contains(&"a".repeat(64)), "{strip}");
        assert!(strip.contains("image/png"), "{strip}");
        assert!(strip.contains("action=\"/detach\""), "{strip}");
        // The bytes are never in the document: only their names are, which is what keeps a
        // transcript and a strip small however large the file is.
        assert!(strip.len() < 1_000, "{strip}");
    }

    #[tokio::test]
    async fn a_blob_is_served_from_the_store_under_a_name_that_cannot_change() {
        let blobs = Arc::new(misa_kernel::Blobs::in_memory());
        let stored = blobs.put(PNG, None).expect("a blob");
        let source = Source::Local(blobs);

        let response = blob_response(Some(&source), &stored.hash).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        // Named by the hash of its own content, so the bytes behind the name can never change:
        // a browser may keep them for as long as it likes, and a transcript that scrolls back
        // fetches nothing.
        let cache = response.headers()[header::CACHE_CONTROL].to_str().unwrap();
        assert!(cache.contains("immutable"), "{cache}");

        let body = axum::body::to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(&body[..], PNG);
    }

    #[tokio::test]
    async fn a_name_that_is_not_a_content_hash_is_answered_before_it_is_asked() {
        let blobs = Arc::new(misa_kernel::Blobs::in_memory());
        let source = Source::Local(blobs);
        for name in ["../../etc/shadow", "not a hash", &"a".repeat(63), &"A".repeat(64)] {
            let response = blob_response(Some(&source), name).await;
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "`{name}` was answered");
        }
        // A client with no store behind it says so rather than pretending the blob is missing
        // from a store it does not have.
        let hash = "a".repeat(64);
        assert_eq!(blob_response(None, &hash).await.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn a_message_becomes_a_paragraph_and_its_roles_become_classes() {
        let html = render_main(&view());
        assert!(html.contains("<p class=\"n-message.user\" id=\"msg.1\" data-state=\"done\">"), "{html}");
        assert!(html.contains("what is &lt;this&gt;?"), "{html}");
    }

    #[test]
    fn nothing_a_session_sends_reaches_the_page_unescaped() {
        let node = Node::text("message.assistant", [Span::plain("<script>alert(1)</script>")]);
        let html = render_main(&node);
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn a_collapsible_is_a_details_element_because_html_already_has_one() {
        let html = render_main(&view());
        assert!(html.contains("<details"), "{html}");
        assert!(html.contains("<summary>"), "{html}");
        assert!(!html.contains("<details open"), "a closed node was rendered open");
    }

    #[test]
    fn a_capture_becomes_a_token_element_and_no_highlighting_is_computed_here() {
        let html = render_main(&view());
        assert!(html.contains("<span data-token=\"keyword\">let</span>"), "{html}");
        assert!(html.contains("= 1;"), "{html}");
    }

    #[test]
    fn a_form_is_a_form_and_works_without_the_script() {
        let html = render_main(&view());
        assert!(html.contains("method=\"post\" action=\"/intent\""), "{html}");
        assert!(html.contains("<textarea name=\"prompt\""), "{html}");
        assert!(
            html.contains("<input type=\"hidden\" name=\"action\" value=\"composer.submit\">"),
            "{html}"
        );
        assert!(html.contains("type=\"submit\""), "{html}");
    }

    #[test]
    fn secret_policy_masks_every_field_shape() {
        for kind in [FieldKind::Inline, FieldKind::Block, FieldKind::Bool,
            FieldKind::Choice { options: vec![], selected: Some("private".into()) }] {
            for read_only in [false, true] {
                let node = Node::new("secret", Kind::Fields { fields: vec![Field {
                    id: "value".into(), label: "Secret".into(), value: "private".into(), hint: None,
                    kind: kind.clone(), read_only, secret: true,
                }] });
                let html = render_main(&node);
                assert!(!html.contains("private"), "{html}");
                assert!(html.contains(if read_only { "••••" } else { "type=\"password\"" }), "{html}");
            }
        }
    }

    #[test]
    fn a_read_only_block_keeps_its_shape_without_offering_an_edit() {
        let node = Node::new("report", Kind::Fields { fields: vec![Field {
            id: "body".into(), label: "Report".into(), value: "one\ntwo".into(), hint: None,
            kind: FieldKind::Block, read_only: true, secret: false,
        }] });
        let html = render_main(&node);
        assert!(html.contains("<pre>one\ntwo</pre>"), "{html}");
        assert!(!html.contains("textarea"), "{html}");
    }

    #[test]
    fn a_panel_is_a_report_whose_buttons_work_without_the_script() {
        // The shape a session opens for `/login`: a row that is a fact, a field somebody types
        // into, and a dismiss. A row is not a text box, and each button posts on its own —
        // a button outside a form is a button that does nothing.
        let panel = Node::section("panel")
            .id("authorize")
            .label("Authorize `kimi-coding`")
            .child(
                Node::new(
                    "panel.row",
                    Kind::Fields {
                        fields: vec![Field {
                            id: "row.0".into(),
                            label: "code".into(),
                            value: "AAAA-BBBB".into(),
                            hint: None,
                            read_only: true,
                            secret: false,
                            kind: FieldKind::Inline,
                        }],
                    },
                )
                .id("panel.row.0"),
            )
            .action(Action {
                id: "panel.close".into(),
                on: ActionOn::Click,
                label: Some("Dismiss".into()),
                args: misa_value::Value::Null,
            });
        let html = render_main(&panel);
        assert!(html.contains("<dt>code</dt><dd>AAAA-BBBB</dd>"), "{html}");
        assert!(!html.contains("<input type=\"text\" name=\"row.0\""), "a row became an input: {html}");
        assert!(
            html.contains("method=\"post\" action=\"/intent\"") && html.contains("value=\"panel.close\""),
            "a panel's button does not post anything: {html}"
        );
        assert!(html.contains("value=\"authorize\""), "the form does not name the node: {html}");
    }

    #[test]
    fn an_image_is_a_reference_rather_than_bytes() {
        let node = Node::new(
            "screenshot",
            Kind::Image {
                blob: BlobRef { hash: "a".repeat(64), len: 9, media: Some("image/png".into()) },
                alt: "a chart".into(),
                width: 10,
                height: 10,
            },
        );
        let html = render_main(&node);
        assert!(html.contains("/blob/"), "{html}");
        assert!(html.contains("alt=\"a chart\""), "{html}");
    }

    #[test]
    fn a_document_is_a_document() {
        let session = SessionInfo {
            id: "demo".into(),
            title: "a demo".into(),
            conversation: None,
            created_ms: 0,
            policy: Vec::new(),
            queries: Vec::new(),
            commands: vec![misa_proto::wire::Command::new("model", "Model", "choose a model")
                .arg(misa_proto::wire::Arg::new("model", "Model").required().from("models"))],
            sources: vec![misa_proto::wire::Source::resident("models", "Models")],
        };
        let html = document(&session, &view());
        assert!(html.starts_with("<!doctype html>"), "{html}");
        assert!(html.contains("<main id=\"main\">"), "{html}");
        assert!(html.contains("/style.css"), "{html}");
        assert!(html.contains("/app.js"), "{html}");
        assert!(html.contains("data-session=\"demo\""), "{html}");
        assert!(html.contains("id=\"theme\""), "{html}");
    }

    #[test]
    fn nested_disclosures_do_not_rewrite_their_ancestors() {
        let tree = Node::section("session").id("session")
            .child(Node::text("message", [Span::plain("before")]))
            .child(Node::new("tool.call", Kind::Collapsible {
                summary: vec![Span::plain("details")],
            }).id("call").child(Node::text("body", [Span::plain("inside")])))
            .child(Node::text("message", [Span::plain("after")]));
        let html = render_main(&tree);
        assert!(html.starts_with("<section "), "{html}");
        assert_eq!(html.matches("<details ").count(), 1, "{html}");
        assert_eq!(html.matches("</details>").count(), 1, "{html}");
        assert!(html.ends_with("</section>"), "{html}");
        assert!(html.find("before").unwrap() < html.find("<details ").unwrap());
        assert!(html.find("</details>").unwrap() < html.find("after").unwrap());
    }

    #[test]
    fn the_stylesheet_themes_roles_and_nothing_else() {
        assert!(STYLE.contains(".n-message\\.user"), "no role rule in the stylesheet");
        assert!(STYLE.contains("data-token"), "captures have no styling");
        // A stylesheet that named a colour inline per node would be a stylesheet
        // that had left the theming model.
        assert!(!STYLE.contains("style=\""), "{STYLE}");
    }
    #[test]
    fn a_submitted_form_becomes_the_intent_both_paths_send() {
        let mut form = HashMap::new();
        form.insert("node".to_string(), "panel".to_string());
        form.insert("action".to_string(), "panel.submit".to_string());
        form.insert("value".to_string(), "sk-a-secret".to_string());
        match intent_from_form(&form) {
            Intent::Action { node, action, fields, .. } => {
                assert_eq!(node, "panel");
                assert_eq!(action, "panel.submit");
                // The two hidden fields are not values a panel asked for.
                assert_eq!(fields.len(), 1);
                assert_eq!(fields[0].id, "value");
            }
            other => panic!("expected an action, got {other:?}"),
        }
    }

    #[test]
    fn a_fact_is_marked_up_with_its_value_and_written_by_the_clients_formatter() {
        let node = Node::section("value.money")
            .child(Node::new("value.money", Kind::Fact { value: misa_value::Value::Int(1_240_000) }));
        let html = render_main(&node);
        assert!(html.contains("<data value=\"1240000\">$1.24</data>"), "{html}");
    }

    #[test]
    fn the_declarations_become_the_browsers_own_completion() {
        let session = SessionInfo {
            id: "demo".into(),
            title: "a demo".into(),
            conversation: None,
            created_ms: 0,
            policy: Vec::new(),
            queries: Vec::new(),
            commands: vec![misa_proto::wire::Command::new("model", "Model", "choose a model")
                .arg(misa_proto::wire::Arg::new("model", "Model").required().from("models"))],
            sources: Vec::new(),
        };
        let html = declarations(&session);
        assert!(html.contains("<datalist id=\"misa-commands\">"), "{html}");
        assert!(html.contains("value=\"/model\""), "{html}");
        // No script, no round trip, and it works with JavaScript switched off.
        assert!(!html.contains("<script"), "{html}");
    }

    #[test]
    fn a_session_with_nothing_declared_adds_nothing_to_the_document() {
        let session = SessionInfo {
            id: "demo".into(),
            title: "a demo".into(),
            conversation: None,
            created_ms: 0,
            policy: Vec::new(),
            queries: Vec::new(),
            commands: Vec::new(),
            sources: Vec::new(),
        };
        assert!(declarations(&session).is_empty());
    }

    #[test]
    fn a_heading_picks_its_level_and_a_quote_becomes_a_blockquote() {
        let node = Node::section("message.assistant")
            .child(Node::new(
                "message.assistant.markdown.heading",
                Kind::Heading { level: 2, spans: vec![Span::plain("Title")] },
            ))
            .child(
                Node::new("message.assistant.markdown.quote", Kind::Quote)
                    .child(Node::text("message.assistant.markdown.paragraph", [Span::plain("quoted")])),
            )
            .child(Node::new("message.assistant.markdown.rule", Kind::Rule));
        let html = render_main(&node);
        assert!(html.contains("<h2 class=\"n-message.assistant.markdown.heading\"><span>Title</span></h2>"), "{html}");
        assert!(html.contains("<blockquote"), "{html}");
        // A rule is void: one tag, and no closing tag to mismatch.
        assert!(html.contains("<hr class=\"n-message.assistant.markdown.rule\">"), "{html}");
        assert!(!html.contains("</hr>"), "{html}");
    }
}
