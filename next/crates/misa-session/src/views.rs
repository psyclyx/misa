//! The view tree, and the data a client can ask for instead of it.
//!
//! # What this file decides, and what it deliberately does not
//!
//! It decides *what is here*: that a transcript is a sequence of messages, that a
//! user message is text, that an assistant message may contain a tool call, that a
//! tool call has arguments and a result, that a session has a header and a
//! composer. That is presentation policy, and putting it in one place is what makes
//! every frontend agree.
//!
//! It does not decide *how anything looks*. There is no colour, no width, no
//! ordering by screen position, no "show this collapsed because the viewport is
//! short". A node may say that it has a short form and a long form
//! ([`Kind::Collapsible`]); whether the long form is showing belongs to the client,
//! remembered against its stable id.
//!
//! # Stable ids
//!
//! A node that stands for the same thing across two revisions carries the same id,
//! and the ids are derived from the message's own sequence number rather than from
//! its position in the window. Nothing about "the third node" may appear in an id,
//! because the window moves and a client's scroll anchor must not.

use std::sync::Arc;

use misa_proto::view::{Action, ActionOn, BlobRef, Field, FieldKind, Kind, Node, Span, State};
use misa_proto::wire::Level;
use misa_reframe::{Inputs, Query, Registry, Subscription, read_query};
use misa_value::Value;

use crate::agent;

use misa_proto::VIEW_QUERY;

/// How many messages a view shows unless a client asks for a different window.
///
/// A window rather than the whole transcript because the frame is the thing a
/// client has to receive, and a long conversation must not make it unbounded. The
/// client passes its own number, which is why this is a default and not a rule.
pub const DEFAULT_WINDOW: usize = 40;

/// The queries this session advertises.
pub fn queries() -> Vec<String> {
    vec![
        VIEW_QUERY.to_string(),
        "session.status".to_string(),
        "session.conversation".to_string(),
        "session.attempts".to_string(),
    ]
}

/// The state a fresh session starts from.
///
/// Small on purpose. Every root here is one the manifest declares, and a root that
/// is not declared is a bug the manifest test catches.
pub fn initial_state(id: &str, provider: &str, model: &str, created_ms: i64) -> Value {
    Value::map([
        (
            "session",
            Value::map([
                ("id", Value::str(id)),
                ("conversation", Value::str(id)),
                ("provider", Value::str(provider)),
                ("model", Value::str(model)),
                ("status", Value::str("idle")),
                ("turn", Value::Int(0)),
                ("requests", Value::Int(0)),
                ("started_ms", Value::Int(created_ms)),
            ]),
        ),
        ("messages", Value::list([])),
        ("attempts", Value::list([])),
        ("notices", Value::list([])),
        // Nothing open. The root is declared anyway, because what a session may write is a
        // question about the manifest and not about the state it happens to start from.
        ("panel", Value::Null),
    ])
}

/// The data queries. The view query is answered by the runtime, not here.
pub fn subscriptions(registry: Registry) -> Registry {
    registry
        .subscription(
            "session.status",
            read_query(|db, _query| db.get("session").cloned().unwrap_or(Value::Null)),
        )
        .subscription(
            "session.conversation",
            read_query(|db, _query| db.get("messages").cloned().unwrap_or(Value::list([]))),
        )
        .subscription(
            "session.attempts",
            read_query(|db, _query| db.get("attempts").cloned().unwrap_or(Value::list([]))),
        )
        .subscription(
            "session.spend",
            Subscription {
                inputs: Inputs::Fixed(vec![Query::new("session.attempts")]),
                compute: std::sync::Arc::new(|_db, inputs, _query, _previous| {
                    let micros: i64 = inputs
                        .first()
                        .and_then(Value::as_list)
                        .map(|rows| {
                            rows.iter()
                                .filter_map(|row| row.get("cost_micros").and_then(Value::as_i64))
                                .sum()
                        })
                        .unwrap_or(0);
                    Value::Int(micros)
                }),
            },
        )
}

/// A part of the document a composition contributes: a plugin's tree, built where it goes.
///
/// The contribution is built from session data; every client receives the same subtree.
#[derive(Clone)]
pub struct Section {
    /// The plugin's id, which is also the role its tree is placed under (plugin.<id>).
    pub plugin: String,
    /// Build it. A failure is a sentence in the document and not a broken tree: a plugin that
    /// cannot present itself must not be able to stop a session from answering.
    pub build: Arc<dyn Fn(&Value, usize) -> Result<Node, String> + Send + Sync>,
}

impl std::fmt::Debug for Section {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Section").field("plugin", &self.plugin).finish()
    }
}

/// Build the session's view.
///
/// The sections are what compositions contributed, placed after the transcript: a plugin's
/// furniture sits under the conversation it is about, and above the panel, the notices, and the
/// composer, which are about *now*. The composer is last because it is always last.
pub fn document(db: &Value, window: usize, sections: &[Section]) -> Node {
    let session = db.get("session");
    let mut root = Node::section("session").id("session");
    root.label = Some(title(session));

    root.children.push(header(db, session));
    root.children.push(transcript(db, window));
    for section in sections {
        root.children.push(section_node(section, db, window));
    }
    if let Some(panel) = panel(db) {
        root.children.push(panel);
    }
    if let Some(notices) = notices(db) {
        root.children.push(notices);
    }
    root.children.push(agent::composer());
    if let Some(cancel) = cancel(db) {
        root.children.push(cancel);
    }
    root
}

fn title(session: Option<&Value>) -> String {
    session
        .and_then(|session| session.get("id"))
        .and_then(Value::as_str)
        .unwrap_or("session")
        .to_string()
}

fn text_at<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn header(db: &Value, session: Option<&Value>) -> Node {
    let provider = session.map(|session| text_at(session, "provider")).unwrap_or_default();
    let model = session.map(|session| text_at(session, "model")).unwrap_or_default();
    let status = session.map(|session| text_at(session, "status")).unwrap_or("unknown");
    let turns = session.and_then(|session| session.get("turn")).and_then(Value::as_i64).unwrap_or(0);
    let mut node = Node::new("session.header", Kind::Status {
        text: format!("{provider}/{model} · {status} · {turns} turns"),
    })
    .id("header");

    // The indicators. Each one is a *fact* with a role, and no formatting: `$1.24`
    // is the client's business, because a currency symbol, a rounding, and a
    // thousands separator are all decisions a session should not be making. The
    // previous system called these value-renderers and made them a catalog; here
    // they are the node's role, so a plugin that invents `value.byte-size` gets a
    // sensible fallback rather than an error.
    node.children.push(
        Node::new("value.turns", Kind::Fact { value: Value::Int(turns) }).label("turns"),
    );

    let spend = db
        .get("attempts")
        .and_then(Value::as_list)
        .map(|rows| rows.iter().filter_map(|row| row.get("cost_micros").and_then(Value::as_i64)).sum::<i64>())
        .unwrap_or(0);
    if spend > 0 {
        node.children.push(
            Node::new("value.spend", Kind::Fact { value: Value::Int(spend) }).label("spend"),
        );
    }

    // How full the context window is, when a model is known. A meter rather than a
    // fact because it is bounded, and bound is what a meter is for.
    if let Some(model) = crate::catalog::model(model) {
        let used = db
            .get("attempts")
            .and_then(Value::as_list)
            .and_then(|rows| rows.last())
            .map(|row| input_tokens(row))
            .unwrap_or(0);
        node.children.push(Node::new(
            "value.context",
            Kind::Meter {
                label: "context".into(),
                value: used as f64,
                max: model.context_window as f64,
            },
        ));
    }
    node
}

/// The input tokens of the most recent attempt, which is the closest thing to "how
/// full is the window" that the ledger can answer.
fn input_tokens(row: &Value) -> i64 {
    row.get("input_tokens").and_then(Value::as_i64).unwrap_or(0)
}

fn transcript(db: &Value, window: usize) -> Node {
    let empty;
    let messages = match db.get("messages").and_then(Value::as_list) {
        Some(messages) => messages,
        None => {
            empty = Vec::new();
            &empty
        }
    };
    let mut node = Node::section("transcript").id("transcript");
    let start = messages.len().saturating_sub(window.max(1));
    if start > 0 {
        // The session says that it is showing a window and what is above it. What
        // to do about that — a button, a scroll, nothing — is the client's.
        node.children.push(Node::new(
            "transcript.earlier",
            Kind::Status { text: format!("{start} earlier messages") },
        ));
    }
    let mut group: Option<Node> = None;
    for message in messages.iter().skip(start) {
        let Some(child) = message_node(message) else { continue };
        if text_at(message, "role") == "user" || group.is_none() {
            if let Some(previous) = group.take() {
                node.children.push(finish_group(previous));
            }
            let id = format!("group.{}", message.get("seq").and_then(Value::as_i64).unwrap_or(0));
            group = Some(Node::section("message.group").id(&id).child(
                Node::text("message.group.header", [Span::plain("Conversation turn")]).id(format!("{id}.header")),
            ));
        }
        group.as_mut().expect("a message has a group").children.push(child);
    }
    if let Some(group) = group {
        node.children.push(finish_group(group));
    }
    if node.children.is_empty() {
        node.children.push(Node::new(
            "transcript.empty",
            Kind::Status { text: "nothing said yet".into() },
        ));
    }
    node
}

/// A run's boundary and count are semantic facts; surfaces choose their decoration.
fn finish_group(mut group: Node) -> Node {
    let count = group.children.len().saturating_sub(1);
    group.children.push(Node::new("message.group.footer", Kind::Fact { value: Value::Int(count as i64) })
        .id(format!("{}.footer", group.id)).label("messages"));
    group
}

fn message_node(message: &Value) -> Option<Node> {
    let seq = message.get("seq").and_then(Value::as_i64).unwrap_or(0);
    let role = text_at(message, "role");
    let id = format!("msg.{seq}");
    let state = match text_at(message, "state") {
        "streaming" => Some(State::Streaming),
        "failed" => Some(State::Failed),
        "cancelled" => Some(State::Cancelled),
        "done" => Some(State::Done),
        _ => None,
    };
    match role {
        "user" => {
            // A section rather than a bare line: a person's message can carry attachments, and
            // a node that is both a line and a container is a node no client can draw. The
            // renderer already rails a section whose role names a rail, so this is the shape it
            // expects anyway.
            let mut node = Node::section("message.user").id(&id);
            node.state = state;
            node.children.extend(body("message.user", &id, text_at(message, "text")));
            for (position, attachment) in message_attachments(message).into_iter().enumerate() {
                node.children.push(attachment_node(&attachment, position));
            }
            Some(node)
        }
        // A note the session wrote to the model: a background command finishing, and whatever
        // else the loop learns on its own. In the transcript rather than the notice list,
        // because somebody reading back should be able to see what the model was told and when.
        "system" => {
            let mut node = Node::text("message.system", [Span::plain(text_at(message, "text"))]).id(&id);
            node.state = state;
            Some(node)
        }
        "assistant" => {
            let mut node = Node::section("message.assistant").id(&id);
            node.state = state;
            // Thinking first, because that is the order it happened in, and collapsed because
            // somebody reading an answer usually wants the answer: the summary says how much of
            // it there is, and the body is there for whoever wants to read it. Whether to open
            // it is the client's decision — that is what a collapsible is.
            let thinking = text_at(message, "thinking");
            if !thinking.is_empty() {
                node.children.push(
                    Node::new(
                        "message.assistant.thinking",
                        Kind::Collapsible {
                            summary: vec![
                                Span::strong("thinking".to_string()),
                                Span::plain(format!(" · {}", preview(&thinking))),
                            ],

                        },
                    )
                    .id(format!("{id}.thinking"))
                    .child(Node::text("message.assistant.thinking.text", [Span::plain(thinking)])),
                );
            }
            // The first text node carries the id a streamed delta appends to, so a
            // client can grow an answer in place.
            node.children.extend(body("message.assistant", &id, text_at(message, "text")));
            for (position, call) in calls(message).into_iter().enumerate() {
                node.children.push(call_node(&call, position));
            }
            Some(node)
        }
        _ => None,
    }
}

/// One tool call: what it was asked to do, and what came back.
///
/// A collapsible, because a tool result is usually long and usually not what the
/// reader came for. Clients remember their own expansion against the node id.
fn call_node(call: &Value, position: usize) -> Node {
    let name = text_at(call, "name");
    let id = call
        .get("id")
        .and_then(Value::as_str)
        .map(|id| format!("call.{id}"))
        .unwrap_or_else(|| format!("call.{position}"));
    let status = text_at(call, "status");
    let mut node = Node::new(
        "tool.call",
        Kind::Collapsible {
            summary: vec![Span::strong(name.to_string()), Span::plain(" ")],

        },
    )
    .id(&id)
    .label(if name.is_empty() { "tool" } else { name });
    node.state = Some(match status {
        "pending" => State::Pending,
        "running" => State::Streaming,
        "ok" => State::Done,
        "error" => State::Failed,
        _ => State::Done,
    });
    node.children.push(Node::new(
        "tool.call.args",
        Kind::Fields {
            fields: vec![Field {
                id: "args".into(),
                label: "Arguments".into(),
                value: clip(&format!("{}", call.get("args").cloned().unwrap_or(Value::Null)), 512),
                hint: None,
                read_only: false,
                secret: false,
                kind: FieldKind::Inline,
            }],
        },
    ));
    if let Some(result) = call.get("result")
        && !result.is_null()
    {
        let ok = status != "error";
        let text = clip(text_of(result), 8192);
        // A result that is a patch is laid out as one. This is the branch the previous
        // system had as a `content.diff` component: a diff is code, so the kind does not
        // change — what changes is the role, which is what tells every frontend that the
        // lines mean added, removed, and where a hunk begins.
        let mut child = if is_unified_diff(&text) {
            Node::new(
                if ok { "tool.result.diff" } else { "tool.result.error.diff" },
                Kind::Code { lang: None, text, captures: Vec::new() },
            )
        } else {
            Node::text(if ok { "tool.result" } else { "tool.result.error" }, [Span::plain(text)])
        }
        .id(format!("{id}.result"))
        .state(if ok { State::Done } else { State::Failed });
        child.label = Some(if ok { "Result".into() } else { "Failure".into() });
        node.children.push(child);
    }
    node
}

/// Whether a tool's result is a unified diff.
///
/// By shape, and only where the shape is unambiguous: a hunk header, a `diff --git` line, or
/// the `--- `/`+++ ` pair a patch opens with. A body that happens to contain a line starting
/// with `+` is not a diff, which is why the pairing and the hunk header are what is looked
/// for rather than a leading character.
fn is_unified_diff(text: &str) -> bool {
    let mut previous_was_removal_header = false;
    for line in text.lines().take(16) {
        if line.starts_with("@@ ") || line.starts_with("diff --git ") {
            return true;
        }
        if previous_was_removal_header && line.starts_with("+++ ") {
            return true;
        }
        previous_was_removal_header = line.starts_with("--- ");
    }
    false
}

/// One composition's section: what it presents, or a sentence about why it could not.
///
/// The wrapper is the session's, so a theme can style a plugin's whole contribution by its role
/// (plugin.<id>) and so a client can find it without knowing anything about the plugin.
fn section_node(section: &Section, db: &Value, window: usize) -> Node {
    let role = format!("plugin.{}", section.plugin);
    let mut wrapper = Node::section(&role).id(&role);
    wrapper.label = Some(section.plugin.clone());
    wrapper.children.push(match (section.build)(db, window) {
        Ok(tree) => namespaced(&role, tree),
        // A fault is data, and here it is a sentence in place of a tree. A session that answered
        // with nothing would be a session whose view a plugin can break.
        Err(reason) => Node::new(
            "plugin.failed",
            Kind::Status { text: format!("this plugin could not present itself: {reason}") },
        )
        .id(format!("{role}.failed")),
    });
    wrapper
}

/// A plugin's tree, with the session's identity on every node.
///
/// Node ids belong to whoever owns the document, so the session puts its own namespace on a subtree
/// it embeds: two plugins, or a plugin and the session, cannot collide — and a collision would
/// refuse the *whole* tree, so one plugin's mistake would freeze every client's view. This is not
/// routing and nothing is ever parsed back out of it: the plugin's own id stays inside the name, so
/// a client that remembers which nodes it opened keeps remembering the right ones.
fn namespaced(prefix: &str, mut node: Node) -> Node {
    node.id = format!("{prefix}.{}", node.id);
    node.children = node.children.into_iter().map(|child| namespaced(prefix, child)).collect();
    node
}

/// The panel the session is showing, if it is showing one.
///
/// A panel is the half of a dialog that is not the composer: a title, a line explaining
/// itself, rows of facts, the fields somebody types into, and the actions they can take. It
/// is built here rather than by each frontend because it is *presentation policy* — what a
/// dialog has in it — and a plugin that opens one should not have to write it five times.
///
/// Two things about the shape matter to a client:
///
/// - a row is a read-only field, so a surface that gives every field an input
///   does not offer an edit that could never be saved;
/// - the action that submits the form travels on the fields node, and every other action is
///   a button on the panel itself, because those are the two pairings clients already read.
fn panel(db: &Value) -> Option<Node> {
    let panel = db.get("panel")?;
    let id = text_at(panel, "id");
    if id.is_empty() {
        // An id is what says a panel is open at all: the root is set as one map and taken
        // away as a whole, so a panel without an id is a panel that is not there.
        return None;
    }
    let actions = panel.get("actions").and_then(Value::as_list).unwrap_or(&[]);
    let label_of = |want: &str| {
        actions
            .iter()
            .find(|action| text_at(action, "id") == want)
            .map(|action| text_at(action, "label").to_string())
            .unwrap_or_else(|| want.to_string())
    };

    let mut node = Node::section("panel").id(id);
    node.label = Some(text_at(panel, "title").to_string());
    let text = text_at(panel, "text");
    if !text.is_empty() {
        node.children.push(Node::text("panel.text", [Span::plain(text)]));
    }

    // Rows are what a session has to say and a person has to copy: a code, an address, a
    // total. Facts rather than fields, which is a distinction a client can draw.
    let rows = panel.get("rows").and_then(Value::as_list).unwrap_or(&[]);
    if !rows.is_empty() {
        let mut list = Node::section("panel.rows").id("panel.rows");
        for (position, row) in rows.iter().enumerate() {
            if let Some(role) = row.get("role").and_then(Value::as_str) {
                list.children.push(
                    Node::section("panel.row")
                        .id(format!("panel.row.{position}"))
                        .child(Node::text("panel.label", [Span::plain(text_at(row, "label"))]))
                        .child(Node::new(role, Kind::Fact { value: row.get("value").cloned().unwrap_or(Value::Null) })),
                );
                continue;
            }

            list.children.push(
                Node::new(
                    "panel.row",
                    Kind::Fields {
                        fields: vec![Field {
                            id: format!("row.{position}"),
                            label: text_at(row, "label").to_string(),
                            value: text_at(row, "value").to_string(),
                            hint: None,
                            read_only: true,
                            secret: false,
                            kind: FieldKind::Inline,
                        }],
                    },
                )
                .id(format!("panel.row.{position}")),
            );
        }
        node.children.push(list);
    }

    // What somebody types into. A secret is a field like any other and the value never
    // travels back in a view, which is what makes the login panel a field and not a
    // protocol of its own.
    let fields: Vec<Field> = panel
        .get("fields")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .iter()
        .map(|field| Field {
            id: text_at(field, "id").to_string(),
            label: text_at(field, "label").to_string(),
            value: String::new(),
            hint: None,
            kind: FieldKind::Inline,
            read_only: false,
            secret: field.get("secret").and_then(Value::as_bool).unwrap_or(false),
        })
        .collect();
    if !fields.is_empty() {
        let mut form = Node::new("panel.input", Kind::Fields { fields }).id("panel.input");
        if actions.iter().any(|action| text_at(action, "id") == "panel.submit") {
            form.actions.push(Action {
                id: "panel.submit".into(),
                on: ActionOn::Submit,
                label: Some(label_of("panel.submit")),
                args: Value::Null,
            });
        }
        node.children.push(form);
    }

    for action in actions {
        let action_id = text_at(action, "id");
        if action_id == "panel.submit" {
            continue;
        }
        node.actions.push(Action {
            id: action_id.to_string(),
            on: ActionOn::Click,
            label: Some(text_at(action, "label").to_string()),
            args: Value::Null,
        });
    }
    Some(node)
}

fn notices(db: &Value) -> Option<Node> {
    let rows = db.get("notices").and_then(Value::as_list)?;
    if rows.is_empty() {
        return None;
    }
    let mut node = Node::section("notices").id("notices");
    for (index, notice) in rows.iter().rev().take(4).enumerate() {
        let level = text_at(notice, "level");
        node.children.push(
            Node::new(
                match level {
                    "error" => "notice.error",
                    "warn" => "notice.warn",
                    _ => "notice",
                },
                Kind::Status { text: text_at(notice, "text").to_string() },
            )
            .id(format!("notice.{index}")),
        );
    }
    Some(node)
}

/// The one agent action a view offers while a turn is in flight.
fn cancel(db: &Value) -> Option<Node> {
    let status = db.get("session").map(|session| text_at(session, "status"))?;
    if status == "idle" {
        return None;
    }
    Some(
        Node::new("turn", Kind::Status { text: format!("working ({status})") })
            .id("turn")
            .action(Action {
                id: "turn.cancel".into(),
                on: ActionOn::Click,
                label: Some("Stop".into()),
                args: Value::Null,
            }),
    )
}

/// The blocks of a message body, with the node a streamed delta appends to.
///
/// Markdown is parsed here, in the middle layer, so that no frontend has to parse it and
/// every frontend agrees on what a quote is. The first block keeps `{id}.text` when it is a
/// run of text, because that is the id a `TextDelta` grows: a client watching an answer
/// arrive appends to it instead of re-reading a tree per token.
fn body(prefix: &str, id: &str, text: &str) -> Vec<Node> {
    let mut blocks = crate::markdown::blocks(prefix, text);
    if blocks.is_empty() {
        // An empty body still has a line, so a delta always has somewhere to land.
        blocks.push(Node::text(format!("{prefix}.text"), Vec::new()));
    }
    if let Some(first) = blocks.first_mut()
        && matches!(first.kind, Kind::Text { .. })
    {
        first.id = format!("{id}.text");
    }
    blocks
}

fn calls(message: &Value) -> Vec<Value> {
    message
        .get("calls")
        .and_then(Value::as_list)
        .map(<[Value]>::to_vec)
        .unwrap_or_default()
}

fn text_of(value: &Value) -> &str {
    value.as_str().unwrap_or_default()
}

fn clip(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_string();
    }
    let mut end = limit;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// The first line of a block of thinking, clipped, for a summary somebody skims.
///
/// A summary that is the whole first line is a summary of nothing; one that is one line of a
/// page is a summary of the page. Eighty characters is about what fits beside a role in a
/// terminal and in a browser.
fn preview(text: &str) -> String {
    let first = text.lines().find(|line| !line.trim().is_empty()).unwrap_or("").trim();
    match first.char_indices().nth(80) {
        Some((end, _)) => format!("{}…", &first[..end]),
        None => first.to_string(),
    }
}

/// An image reference and its alternative text, independent of the client.
pub fn image_node(blob: BlobRef, alt: &str, width: u32, height: u32) -> Node {
    Node::new("image", Kind::Image { blob, alt: alt.to_string(), width, height })
}

/// The attachments a message carries, in the order they were attached.
fn message_attachments(message: &Value) -> Vec<Value> {
    message.get("attachments").and_then(Value::as_list).map(<[Value]>::to_vec).unwrap_or_default()
}

/// One attachment of a message.
///
/// The bytes stay in the blob store; clients choose how to render the reference and alt.
///
/// The dimensions are zero, which says "unknown": nothing in the store records them, and a
/// client that wants them can read them from the bytes it fetches. A client that lays out
/// before the fetch can reserve whatever room it likes.
fn attachment_node(attachment: &Value, position: usize) -> Node {
    let hash = text_at(attachment, "hash");
    let media = text_at(attachment, "media");
    let len = attachment.get("len").and_then(Value::as_i64).unwrap_or(0).max(0) as u64;
    let name = text_at(attachment, "source");
    let what = if media.is_empty() { "file".to_string() } else { media.to_string() };
    let alt = if name.is_empty() {
        format!("{what} ({len} bytes)")
    } else {
        format!("{name} — {what} ({len} bytes)")
    };
    let blob = BlobRef { hash: hash.to_string(), len, media: (!media.is_empty()).then_some(media.to_string()) };
    let mut node = image_node(blob, &alt, 0, 0);
    node.id = format!("attachment.{position}");
    node
}

/// A notice's level as data, for a client that wants to group them.
pub fn level_of(notice: &Value) -> Level {
    match text_at(notice, "level") {
        "error" => Level::Error,
        "warn" => Level::Warn,
        _ => Level::Info,
    }
}

/// The four roots the shipped session declares, with who owns each and how long it
/// lives.
///
/// The previous system's manifest, in Rust, and checked by a test rather than only
/// written down: a fact is the kernel's, an observation is derived, and anything
/// ephemeral must not be journalled.
pub const MANIFEST: &[(&str, Ownership, Lifetime)] = &[
    ("session", Ownership::Kernel, Lifetime::Ephemeral),
    ("messages", Ownership::Kernel, Lifetime::Log),
    ("attempts", Ownership::Kernel, Lifetime::Log),
    ("notices", Ownership::Presentation, Lifetime::Ephemeral),
    // A panel is a report or a small form, and it is presentation: it exists to be drawn,
    // and a restart has nothing to say about whether somebody had it open.
    ("panel", Ownership::Presentation, Lifetime::Ephemeral),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ownership {
    /// Belongs to the kernel: a fact about what happened.
    Kernel,
    /// Belongs to the client: it exists to be drawn.
    Presentation,
    /// Belongs to a handler this session was composed with, and to nobody else.
    ///
    /// The third category exists because the first two are not a dichotomy: a plugin's state is
    /// not a fact about the world the kernel witnessed, and it is not a client's presentation
    /// either — it is the middle layer's own, written only by the handlers that declared it.
    /// A composition declares these ([`crate::Contribution::with_root`]); the shipped manifest
    /// cannot, because it cannot know what a caller loaded.
    Plugin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifetime {
    /// Durable. Appended, never rewritten.
    Log,
    /// In-memory only, and meaningless after a restart.
    Ephemeral,
}

/// Whether a database root is declared, and how.
pub fn declared(root: &str) -> Option<(Ownership, Lifetime)> {
    MANIFEST
        .iter()
        .find(|(name, _, _)| *name == root)
        .map(|(_, ownership, lifetime)| (*ownership, *lifetime))
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::view::find;

    /// A transcript with one user message, one assistant message containing a
    /// fenced code block, and one settled tool call.
    fn state() -> Value {
        let base = initial_state("demo", "scripted", "scripted-1", 0);
        let mut session = base.get("session").expect("a session").as_map().expect("a map").clone();
        session.insert("turn".into(), Value::Int(2));
        Value::map([
            ("session", Value::Map(std::sync::Arc::new(session))),
            ("attempts", Value::list([])),
            ("notices", Value::list([])),
            (
                "messages",
                Value::list([
                    Value::map([
                        ("seq", Value::Int(1)),
                        ("role", Value::str("user")),
                        ("text", Value::str("hello")),
                        ("state", Value::str("done")),
                    ]),
                    Value::map([
                        ("seq", Value::Int(2)),
                        ("role", Value::str("assistant")),
                        ("text", Value::str("here:\n```rust\nlet x = 1;\n```\ndone")),
                        ("state", Value::str("done")),
                        (
                            "calls",
                            Value::list([Value::map([
                                ("id", Value::str("call.1")),
                                ("name", Value::str("echo")),
                                ("args", Value::str("hi")),
                                ("status", Value::str("ok")),
                                ("result", Value::str("hi")),
                            ])]),
                        ),
                    ]),
                ]),
            ),
        ])
    }

    #[test]
    fn a_tree_is_built_and_valid() {
        let node = document(&state(), 40, &[]);
        misa_proto::view::validate(&node).expect("the shipped view is valid");
        assert_eq!(node.role, "session");
    }

    #[test]
    fn the_transcript_holds_the_messages_in_order_with_stable_ids() {
        let node = document(&state(), 40, &[]);
        let transcript = find(&node, "transcript").expect("a transcript");
        let group = &transcript.children[0];
        assert_eq!(group.role, "message.group");
        assert_eq!(group.children.first().unwrap().role, "message.group.header");
        assert_eq!(group.children.last().unwrap().role, "message.group.footer");
        let ids: Vec<Option<&str>> = group.children.iter().filter(|child| child.id.starts_with("msg.")).map(|child| Some(child.id.as_str())).collect();
        assert_eq!(ids, vec![Some("msg.1"), Some("msg.2")]);
    }

    #[test]
    fn a_streamed_delta_has_a_node_to_append_to() {
        let node = document(&state(), 40, &[]);
        assert!(find(&node, "msg.2.text").is_some(), "the first text node must carry the delta id");
    }

    #[test]
    fn a_fenced_block_becomes_a_code_node_rather_than_prose() {
        let node = document(&state(), 40, &[]);
        let message = find(&node, "msg.2").expect("the assistant message");
        let code = message
            .children
            .iter()
            .find(|child| matches!(child.kind, Kind::Code { .. }))
            .expect("a code node");
        match &code.kind {
            Kind::Code { lang, text, .. } => {
                assert_eq!(lang.as_deref(), Some("rust"));
                assert_eq!(text, "let x = 1;");
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_tool_call_carries_its_arguments_and_its_result() {
        let node = document(&state(), 40, &[]);
        let call = find(&node, "call.call.1").expect("the tool call");
        assert_eq!(call.state, Some(State::Done));
        assert_eq!(call.label.as_deref(), Some("echo"));
        assert!(call.children.iter().any(|child| child.role == "tool.result"));
    }

    #[test]
    fn a_result_that_is_a_patch_is_laid_out_as_a_diff() {
        let base = initial_state("demo", "scripted", "scripted-1", 0);
        let state = Value::map([
            ("session", base.get("session").cloned().unwrap_or(Value::Null)),
            ("attempts", Value::list([])),
            ("notices", Value::list([])),
            (
                "messages",
                Value::list([Value::map([
                    ("seq", Value::Int(1)),
                    ("role", Value::str("assistant")),
                    ("text", Value::str("edited")),
                    ("state", Value::str("done")),
                    (
                        "calls",
                        Value::list([Value::map([
                            ("id", Value::str("call.1")),
                            ("name", Value::str("write")),
                            ("args", Value::str("src/main.rs")),
                            ("status", Value::str("ok")),
                            ("result", Value::str("--- a/main.rs\n+++ b/main.rs\n@@ -1 +1 @@\n-old\n+new")),
                        ])]),
                    ),
                ])]),
            ),
        ]);
        let node = document(&state, 40, &[]);
        let result = find(&node, "call.call.1.result").expect("the result");
        assert_eq!(result.role, "tool.result.diff");
        match &result.kind {
            Kind::Code { text, .. } => assert!(text.starts_with("--- a/main.rs"), "{text}"),
            other => panic!("a patch came through as {other:?}"),
        }
        misa_proto::view::validate(&node).expect("a diff result is a valid view");
    }

    #[test]
    fn a_result_that_merely_mentions_a_plus_line_is_still_text() {
        // The shape has to be unambiguous, or every bulleted list a tool prints becomes a
        // patch. A hunk header or the `--- `/`+++ ` pair is a diff; a leading `+` is not.
        assert!(!is_unified_diff("here is a bullet\n+ and a plus line"));
        assert!(!is_unified_diff("+ one\n+ two"));
        assert!(is_unified_diff("@@ -1 +1 @@\n-a\n+b"));
        assert!(is_unified_diff("diff --git a/x b/x\n--- a/x\n+++ b/x"));
    }

    #[test]
    fn tool_details_are_semantic() {
        let tree = document(&state(), 40, &[]);
        assert!(matches!(&find(&tree, "call.call.1").unwrap().kind, Kind::Collapsible { summary } if !summary.is_empty()));
    }

    #[test]
    fn the_window_is_what_the_caller_asked_for() {
        let node = document(&state(), 1, &[]);
        let transcript = find(&node, "transcript").expect("a transcript");
        assert!(transcript.children.iter().any(|child| child.role == "transcript.earlier"));
        assert_eq!(transcript.children.iter().flat_map(|group| &group.children).filter(|child| child.id.starts_with("msg.")).count(), 1);
    }

    #[test]
    fn an_empty_transcript_says_so_instead_of_being_empty() {
        let node = document(&initial_state("demo", "p", "m", 0), 40, &[]);
        assert!(find(&node, "transcript").expect("a transcript").children[0].role == "transcript.empty");
        misa_proto::view::validate(&node).unwrap();
    }

    #[test]
    fn there_is_no_cancel_action_when_nothing_is_running() {
        let idle = document(&initial_state("demo", "p", "m", 0), 40, &[]);
        assert!(find(&idle, "turn").is_none());
    }

    #[test]
    fn the_composer_offers_exactly_one_action_and_the_session_owns_its_meaning() {
        let node = document(&state(), 40, &[]);
        let composer = find(&node, "composer").expect("a composer");
        assert_eq!(composer.actions.len(), 1);
        assert_eq!(composer.actions[0].id, "composer.submit");
        assert_eq!(composer.actions[0].on, ActionOn::Submit);
    }

    #[test]
    fn every_root_the_shipped_state_uses_is_declared() {
        let state = state();
        for root in state.as_map().expect("a map").keys() {
            assert!(declared(root).is_some(), "the session writes an undeclared root `{root}`");
        }
    }

    #[test]
    fn nothing_journalled_is_presentation() {
        for (root, ownership, lifetime) in MANIFEST {
            if *lifetime == Lifetime::Log {
                assert_eq!(
                    *ownership,
                    Ownership::Kernel,
                    "`{root}` is journalled but not owned by the kernel"
                );
            }
        }
    }

    #[test]
    fn an_image_always_carries_its_hash_and_alt() {
        let blob = BlobRef { hash: "a".repeat(64), len: 4, media: Some("image/png".into()) };
        let node = image_node(blob.clone(), "a chart", 10, 10);
        assert!(matches!(node.kind, Kind::Image { blob: actual, alt, .. } if actual == blob && alt == "a chart"));
    }

    /// Every node the session writes, by id.
    fn ids_of(node: &Node, out: &mut Vec<String>) {
        out.push(node.id.clone());
        for child in &node.children {
            ids_of(child, out);
        }
    }

    #[test]
    fn the_session_leaves_a_namespace_for_what_it_embeds() {
        // A composition's tree is placed under ids the *session* mints (plugin.<id> + the node id),
        // which is collision-free only while no node the session writes is in that namespace. The
        // collision is not a theoretical one: a duplicate id refuses the whole tree a client is
        // sent, so one plugin's mistake would freeze every client's view.
        let node = document(&state(), 40, &[]);
        let mut ids = Vec::new();
        ids_of(&node, &mut ids);
        assert!(ids.contains(&"composer".to_string()), "the walk found the session's own nodes");
        for id in ids {
            assert!(!id.starts_with("plugin."), "the session wrote `{id}`, which is a plugin's namespace");
        }
    }

}
