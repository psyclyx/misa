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

use crate::Level;
use misa_proto::view::{Action, ActionOn, BlobRef, Field, FieldKind, Kind, Node, Span, State};
use misa_reframe::{Inputs, Query, Registry, Subscription, read_query};
use misa_value::Value;

use crate::agent;

use misa_proto::VIEW_QUERY;

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
                (
                    "effort",
                    crate::catalog::default_effort(&Value::Null, provider, model)
                        .map_or(Value::Null, |effort| Value::str(effort)),
                ),
                ("status", Value::str("idle")),
                ("turn", Value::Int(0)),
                ("requests", Value::Int(0)),
                ("started_ms", Value::Int(created_ms)),
            ]),
        ),
        ("messages", Value::list([])),
        ("attempts", Value::list([])),
        ("operations", Value::list([])),
        ("prompt_operations", Value::list([])),
        ("command_operations", Value::list([])),
        ("input_requests", Value::list([])),
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
            "session.summary.base",
            read_query(|db, _query| {
                let session = db.get("session");
                let text = |name: &str| {
                    session
                        .and_then(|value| value.get(name))
                        .and_then(Value::as_str)
                        .unwrap_or("")
                };
                let requests = db
                    .get("input_requests")
                    .and_then(Value::as_list)
                    .unwrap_or(&[])
                    .iter()
                    .filter(|request| {
                        request.get("state").and_then(Value::as_str) == Some("awaiting_input")
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let running_tools = session
                    .and_then(|session| session.get("running_tools"))
                    .and_then(Value::as_list)
                    .map_or(0, |tools| tools.len());
                let pending_tools = requests
                    .iter()
                    .filter(|request| {
                        request.get("kind").and_then(Value::as_str) == Some("tool_approval")
                    })
                    .count();
                let transaction_work = db
                    .get("command_operations")
                    .and_then(Value::as_list)
                    .unwrap_or(&[])
                    .iter()
                    .any(|operation| {
                        operation.get("terminal").and_then(Value::as_bool) != Some(true)
                    });
                let working = transaction_work
                    || match text("status") {
                        "idle" => false,
                        "tools" => running_tools > pending_tools,
                        _ => true,
                    };
                let operations = db
                    .get("prompt_operations")
                    .and_then(Value::as_list)
                    .unwrap_or(&[])
                    .iter()
                    .filter(|operation| {
                        operation.get("terminal").and_then(Value::as_bool) != Some(true)
                    })
                    .chain(
                        db.get("command_operations")
                            .and_then(Value::as_list)
                            .unwrap_or(&[])
                            .iter()
                            .filter(|operation| {
                                operation.get("terminal").and_then(Value::as_bool) != Some(true)
                            }),
                    )
                    .chain(
                        db.get("operations")
                            .and_then(Value::as_list)
                            .unwrap_or(&[])
                            .iter()
                            .filter(|operation| {
                                matches!(
                                    operation.get("state").and_then(Value::as_str),
                                    Some(
                                        "running" | "awaiting_input" | "submitting" | "cancelling"
                                    )
                                )
                            }),
                    )
                    .map(|operation| {
                        Value::map([
                            ("id", operation.get("id").cloned().unwrap_or(Value::Null)),
                            (
                                "kind",
                                operation.get("kind").cloned().unwrap_or(Value::Null),
                            ),
                            (
                                "state",
                                operation.get("state").cloned().unwrap_or(Value::Null),
                            ),
                            (
                                "generation",
                                operation
                                    .get("generation")
                                    .cloned()
                                    .unwrap_or(Value::Int(1)),
                            ),
                        ])
                    });
                Value::map([
                    ("id", Value::str(text("id"))),
                    ("activity", Value::str(text("status"))),
                    ("working", Value::Bool(working)),
                    ("attention", Value::Int(requests.len() as i64)),
                    ("operations", Value::list(operations)),
                    ("requests", Value::list(requests)),
                    ("provider", Value::str(text("provider"))),
                    ("model", Value::str(text("model"))),
                ])
            }),
        )
        .subscription(
            "session.usage.total",
            Subscription::Derived {
                inputs: Inputs::Fixed(vec![Query::new("session.attempts")]),
                compute: std::sync::Arc::new(|inputs, _, _| {
                    Ok({
                        let rows = inputs.first().and_then(Value::as_list).unwrap_or(&[]);
                        Value::map(
                            ["input_tokens", "output_tokens", "cost_micros"]
                                .into_iter()
                                .map(|key| {
                                    (
                                        key,
                                        Value::Int(
                                            rows.iter()
                                                .filter_map(|row| {
                                                    row.get(key).and_then(Value::as_i64)
                                                })
                                                .fold(0i64, i64::saturating_add),
                                        ),
                                    )
                                }),
                        )
                    })
                }),
            },
        )
        .subscription(
            crate::observation::SUMMARY,
            Subscription::Derived {
                inputs: Inputs::Fixed(vec![
                    Query::new("session.summary.base"),
                    Query::new("session.usage.total"),
                ]),
                compute: std::sync::Arc::new(|inputs, _, _| {
                    Ok({
                        let mut summary = inputs
                            .first()
                            .and_then(Value::as_map)
                            .cloned()
                            .unwrap_or_default();
                        summary.insert(
                            "usage".into(),
                            inputs.get(1).cloned().unwrap_or(Value::Null),
                        );
                        Value::Map(std::sync::Arc::new(summary))
                    })
                }),
            },
        )
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
            Subscription::Derived {
                inputs: Inputs::Fixed(vec![Query::new("session.attempts")]),
                compute: std::sync::Arc::new(|inputs, _query, _previous| {
                    Ok({
                        let micros: i64 = inputs
                            .first()
                            .and_then(Value::as_list)
                            .map(|rows| {
                                rows.iter()
                                    .filter_map(|row| {
                                        row.get("cost_micros").and_then(Value::as_i64)
                                    })
                                    .sum()
                            })
                            .unwrap_or(0);
                        Value::Int(micros)
                    })
                }),
            },
        )
}

/// A part of the document a composition contributes: a plugin's tree, built where it goes.
///
/// The contribution is built from session data; every client receives the same subtree.
#[derive(Clone)]
pub struct Section {
    /// Stable namespace for this contribution's tree.
    pub namespace: String,
    /// Database inputs this content builder reads. An empty path explicitly means the whole db.
    pub inputs: Vec<misa_value::Path>,
    /// Build it. A failure is a sentence in the document and not a broken tree: a plugin that
    /// cannot present itself must not be able to stop a session from answering.
    pub build: Arc<dyn Fn(&Value) -> Result<Node, String> + Send + Sync>,
}

impl Section {
    /// Project a named subscription into a semantic view. The query graph owns
    /// dependencies and memoization; a section owns only the resulting tree.
    pub fn query(
        namespace: impl Into<String>,
        registry: Arc<Registry>,
        query: Query,
        project: impl Fn(&Value) -> Result<Node, String> + Send + Sync + 'static,
    ) -> Self {
        let memo = std::sync::Mutex::new((misa_reframe::Scope::new(), None::<Node>));
        Self {
            namespace: namespace.into(),
            inputs: vec![misa_value::Path::root()],
            build: Arc::new(move |db| {
                let mut memo = memo.lock().map_err(|_| "View query scope is poisoned")?;
                let changed = memo
                    .0
                    .evaluate(db, &registry, &query)
                    .map_err(|fault| fault.message)?;
                if changed.is_some() || memo.1.is_none() {
                    let value = memo.0.current(&query).ok_or("View query has no value")?;
                    memo.1 = Some(project(&value)?);
                }
                Ok(memo.1.as_ref().unwrap().clone())
            }),
        }
    }
}

impl std::fmt::Debug for Section {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Section")
            .field("namespace", &self.namespace)
            .finish()
    }
}

/// Build the session's view.
///
/// The sections are what compositions contributed, placed after the transcript: a plugin's
/// furniture sits under the conversation it is about, and above the panel, the notices, and the
/// composer, which are about *now*. The composer is last because it is always last.
pub fn document(db: &Value, sections: &[Section]) -> Node {
    let session = db.get("session");
    let mut root = Node::section("session").id("session");
    root.label = Some(title(session));

    root.children.push(transcript(db));
    for section in sections {
        root.children.push(section_node(section, db));
    }
    if let Some(panel) = panel(db) {
        root.children.push(panel);
    }
    if let Some(notices) = notices(db) {
        root.children.push(notices);
    }
    if let Some(queue) = queue(db) {
        root.children.push(queue);
    }
    if let Some(attachments) = attachments(db) {
        root.children.push(attachments);
    }
    root.children.push(agent::composer());
    if let Some(cancel) = cancel(db) {
        root.children.push(cancel);
    }
    misa_proto::sync::address(&mut root);
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

pub(crate) fn transcript(db: &Value) -> Node {
    let empty;
    let messages = match db.get("messages").and_then(Value::as_list) {
        Some(messages) => messages,
        None => {
            empty = Vec::new();
            &empty
        }
    };
    let mut node = Node::section("transcript").id("transcript");
    let mut group: Option<Node> = None;
    let mut attempt: Option<Value> = None;
    // The message that opened the group. Its timestamp is the footer's when the
    // group has no attempt yet, so every group carries a separator.
    let mut opener: Option<Value> = None;
    for message in messages.iter() {
        let nodes = message_nodes(message);
        if nodes.is_empty() {
            continue;
        }
        if text_at(message, "role") == "user" || group.is_none() {
            if let Some(previous) = group.take() {
                node.children.push(finish_group(
                    previous,
                    attempt.take(),
                    opener.take().as_ref(),
                ));
            }
            opener = Some(message.clone());
            let id = format!(
                "group.{}",
                message.get("seq").and_then(Value::as_i64).unwrap_or(0)
            );
            group = Some(Node::section("message.group").id(&id));
        }
        if text_at(message, "role") == "assistant" {
            attempt = message.get("attempt").cloned();
        }
        group
            .as_mut()
            .expect("a message has a group")
            .children
            .extend(nodes);
    }
    if let Some(group) = group {
        node.children
            .push(finish_group(group, attempt, opener.as_ref()));
    }
    if node.children.is_empty() {
        node.children.push(Node::new(
            "transcript.empty",
            Kind::Status {
                text: "nothing said yet".into(),
            },
        ));
    }
    node
}

/// Every block a message contributes, in order: the message itself, then its
/// tool calls as siblings rather than children.
///
/// A tool call nested under the assistant message would inherit the message's
/// rail and padding instead of standing as the separate block it is.
pub(crate) fn message_nodes(message: &Value) -> Vec<Node> {
    let mut nodes = Vec::new();
    let Some(node) = message_node(message) else {
        return nodes;
    };
    let id = format!(
        "msg.{}",
        message.get("seq").and_then(Value::as_i64).unwrap_or(0)
    );
    nodes.push(node);
    if text_at(message, "role") == "assistant" {
        for (position, call) in calls(message).into_iter().enumerate() {
            nodes.push(call_node(&id, &call, position));
        }
    }
    nodes
}

/// A message group's footer is shared semantic accounting, not an operation log.
///
/// Every group carries one. An assistant attempt supplies the timing and spend;
/// a group that only has its opening user message falls back to that message's
/// own instant, so the transcript still has a turn boundary.
pub(crate) fn finish_group(
    mut group: Node,
    attempt: Option<Value>,
    opener: Option<&Value>,
) -> Node {
    let footer = match &attempt {
        Some(attempt) => attempt_footer(&group.id, attempt),
        None => message_footer(&group.id, opener),
    };
    group.children.push(footer);
    group
}

/// A message's own separator footer, used until an assistant attempt owns the
/// group's accounting. The instant is the UTC millisecond integer the session
/// stored; turning it into a wall clock belongs to the client.
pub(crate) fn message_footer(group_id: &str, message: Option<&Value>) -> Node {
    let mut footer = Node::new("message.group.footer", Kind::Rule).id(format!("{group_id}.footer"));
    if let Some(at) = message
        .and_then(|message| message.get("at_ms"))
        .and_then(Value::as_i64)
    {
        footer.children.push(Node::new(
            "value.timestamp",
            Kind::Fact {
                value: Value::Int(at),
            },
        ));
    }
    // Marginal token facts, when the log happens to carry them.
    for key in ["input_tokens", "output_tokens"] {
        if let Some(tokens) = message
            .and_then(|message| message.get(key))
            .and_then(Value::as_i64)
        {
            footer.children.push(Node::new(
                "value.tokens",
                Kind::Fact {
                    value: Value::Int(tokens),
                },
            ));
        }
    }
    footer
}

/// Build the settled-attempt facts independently so incremental and snapshot views
/// have the same turn boundary.
pub(crate) fn attempt_footer(group_id: &str, attempt: &Value) -> Node {
    let mut footer = Node::new("message.group.footer", Kind::Rule).id(format!("{group_id}.footer"));
    if let Some(started) = attempt.get("started_ms").and_then(Value::as_i64) {
        footer.children.push(Node::new(
            "value.timestamp",
            Kind::Fact {
                value: Value::Int(started),
            },
        ));
    }
    match attempt.get("status").and_then(Value::as_str) {
        Some("streaming" | "running") => footer.children.push(Node::new(
            "value.text",
            Kind::Text {
                spans: vec![Span::plain(
                    attempt
                        .get("status")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )],
            },
        )),
        Some("error") => footer.children.push(Node::new(
            "value.text",
            Kind::Text {
                spans: vec![Span::plain("failed")],
            },
        )),
        Some("cancelled") => footer.children.push(Node::new(
            "value.text",
            Kind::Text {
                spans: vec![Span::plain("interrupted")],
            },
        )),
        _ => {}
    }
    // A throughput number is only meaningful once the attempt has stopped
    // producing tokens; a streaming rate is a guess that the next frame undoes.
    let settled = !matches!(
        attempt.get("status").and_then(Value::as_str),
        Some("streaming" | "running")
    );
    if let Some(elapsed) = attempt.get("elapsed_ms").and_then(Value::as_i64) {
        footer.children.push(Node::new(
            "value.duration",
            Kind::Fact {
                value: Value::Int(elapsed),
            },
        ));
        let output = attempt
            .get("output_tokens")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if settled && elapsed > 0 && output > 0 {
            footer.children.push(Node::new(
                "value.rate",
                Kind::Fact {
                    value: Value::Float(output as f64 * 1000.0 / elapsed as f64),
                },
            ));
        }
    }
    if let Some(cost) = attempt.get("cost_micros").and_then(Value::as_i64) {
        footer.children.push(Node::new(
            "value.money",
            Kind::Fact {
                value: Value::Int(cost),
            },
        ));
    }
    footer
}

pub(crate) fn message_node(message: &Value) -> Option<Node> {
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
            node.children
                .extend(body("message.user", &id, text_at(message, "text")));
            for (position, attachment) in message_attachments(message).into_iter().enumerate() {
                let mut attachment = attachment_node(&attachment, position);
                attachment.id = format!("{id}.{}", attachment.id);
                node.children.push(attachment);
            }
            Some(node)
        }
        // A note the session wrote to the model: a background command finishing, and whatever
        // else the loop learns on its own. In the transcript rather than the notice list,
        // because somebody reading back should be able to see what the model was told and when.
        "system" => {
            let mut node =
                Node::text("message.system", [Span::plain(text_at(message, "text"))]).id(&id);
            node.state = state;
            Some(node)
        }
        "assistant" => {
            let error = text_at(message, "error");
            if !error.is_empty() {
                let text = text_at(message, "text");
                let text = if text.is_empty() {
                    compact_error(error)
                } else {
                    format!("{text}\n\n{}", compact_error(error))
                };
                let mut node = Node::section("error").id(&id);
                node.state = state;
                node.children.extend(body("error", &id, &text));
                return Some(node);
            }
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
                            summary: thinking_summary(&thinking),
                        },
                    )
                    .id(format!("{id}.thinking"))
                    .child(Node::text(
                        "message.assistant.thinking.text",
                        [Span::plain(thinking)],
                    )),
                );
            }
            // The first text node carries the id a streamed delta appends to, so a
            // client can grow an answer in place.
            node.children
                .extend(body("message.assistant", &id, text_at(message, "text")));
            Some(node)
        }
        _ => None,
    }
}

/// One tool call: what it was asked to do, and what came back.
///
/// A collapsible, because a tool result is usually long and usually not what the
/// reader came for. Clients remember their own expansion against the node id.
pub(crate) fn call_node(message: &str, call: &Value, position: usize) -> Node {
    let name = text_at(call, "name");
    let id = call
        .get("id")
        .and_then(Value::as_str)
        .map(|id| format!("{message}.call.{id}"))
        .unwrap_or_else(|| format!("{message}.call.{position}"));
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
                id: format!("{id}.args"),
                label: "Arguments".into(),
                value: clip(
                    &format!("{}", call.get("args").cloned().unwrap_or(Value::Null)),
                    512,
                ),
                hint: None,
                read_only: true,
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
                if ok {
                    "tool.result.diff"
                } else {
                    "tool.result.error.diff"
                },
                Kind::Code { lang: None, text },
            )
        } else {
            Node::text(
                if ok {
                    "tool.result"
                } else {
                    "tool.result.error"
                },
                [Span::plain(text)],
            )
        }
        .id(format!("{id}.result"))
        .state(if ok { State::Done } else { State::Failed });
        child.label = Some(if ok {
            "Result".into()
        } else {
            "Failure".into()
        });
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
pub(crate) fn section_node(section: &Section, db: &Value) -> Node {
    let role = section.namespace.clone();
    let mut wrapper = Node::section(&role).id(&role);
    wrapper.label = None;
    let built = (section.build)(db).and_then(|mut tree| {
        misa_proto::view::validate(&tree).map_err(|fault| fault.to_string())?;
        misa_proto::sync::address(&mut tree);
        misa_proto::view::validate(&tree).map_err(|fault| fault.to_string())?;
        Ok(tree)
    });
    wrapper.children.push(match built {
        Ok(tree) => namespaced(&role, tree),
        // A fault is data, and here it is a sentence in place of a tree. A session that answered
        // with nothing would be a session whose view a plugin can break.
        Err(reason) => Node::new(
            "plugin.failed",
            Kind::Status {
                text: format!("{} could not present itself: {reason}", section.namespace),
            },
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
    node.children = node
        .children
        .into_iter()
        .map(|child| namespaced(prefix, child))
        .collect();
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
pub(crate) fn panel(db: &Value) -> Option<Node> {
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
        node.children
            .push(Node::text("panel.text", [Span::plain(text)]));
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
                        .child(Node::text(
                            "panel.label",
                            [Span::plain(text_at(row, "label"))],
                        ))
                        .child(Node::new(
                            role,
                            Kind::Fact {
                                value: row.get("value").cloned().unwrap_or(Value::Null),
                            },
                        )),
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
            secret: field
                .get("secret")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
        .collect();
    if !fields.is_empty() {
        let mut form = Node::new("panel.input", Kind::Fields { fields }).id("panel.input");
        if actions
            .iter()
            .any(|action| text_at(action, "id") == "panel.submit")
        {
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

pub(crate) fn notices(db: &Value) -> Option<Node> {
    let rows = db.get("notices").and_then(Value::as_list)?;
    if rows.is_empty() {
        return None;
    }
    let mut node = Node::section("notices").id("notices");
    for notice in rows.iter().rev().take(4) {
        let level = text_at(notice, "level");
        node.children.push(
            Node::new(
                match level {
                    "error" => "notice.error",
                    "warn" => "notice.warn",
                    _ => "notice",
                },
                Kind::Status {
                    text: text_at(notice, "text").to_string(),
                },
            )
            .id(notice_id(notice)),
        );
    }
    Some(node)
}

pub(crate) fn notice_id(notice: &Value) -> String {
    format!(
        "notice.{}",
        notice.get("id").and_then(Value::as_i64).unwrap_or(0)
    )
}

pub(crate) fn queue(db: &Value) -> Option<Node> {
    let rows = db.get("session")?.get("queue")?.as_list()?;
    if rows.is_empty() {
        return None;
    }
    let mut node = Node::section("queue")
        .id("queue")
        .child(count_fact("queue", rows.len()));
    for row in rows {
        node.children.push(queue_item(row));
    }
    // The stock queue exposes the same two decisions as the reference: restore the
    // pending input for editing, or steer it into the active turn. The latter is a
    // client key binding (Alt-Enter), so it is represented by the shared affordance
    // rather than a second destructive queue mutation.
    node.actions = vec![
        Action {
            id: "queue.edit".into(),
            on: ActionOn::Click,
            label: Some("Edit pending message".into()),
            args: Value::Null,
        },
        Action {
            id: "queue.steer".into(),
            on: ActionOn::Click,
            label: Some("Send now".into()),
            args: Value::Null,
        },
    ];
    Some(node)
}

pub(crate) fn queue_item(row: &Value) -> Node {
    Node::text("queue.item", [Span::plain(text_at(row, "text"))]).id(format!(
        "queue.{}",
        row.get("id").and_then(Value::as_i64).unwrap_or(0)
    ))
}

pub(crate) fn attachments(db: &Value) -> Option<Node> {
    let rows = db.get("session")?.get("attachments")?.as_list()?;
    if rows.is_empty() {
        return None;
    }
    let mut node = Node::section("attachments")
        .id("attachments")
        .child(count_fact("attachments", rows.len()));
    for (position, row) in rows.iter().enumerate() {
        node.children.push(attachment_node(row, position));
    }
    Some(node)
}

pub(crate) fn count_fact(owner: &str, count: usize) -> Node {
    Node::new(
        format!("{owner}.count"),
        Kind::Fact {
            value: Value::Int(count as i64),
        },
    )
    .id(format!("{owner}.count"))
    .label(owner)
}

/// The one agent action a view offers while a turn is in flight.
pub(crate) fn cancel(db: &Value) -> Option<Node> {
    let status = db
        .get("session")
        .map(|session| text_at(session, "status"))?;
    if status == "idle" {
        return None;
    }
    // The activity indicator already says "working" and animates. The turn node
    // survives only as the cancel affordance and the redraw tick's identity; a
    // section with no text renders no redundant line.
    Some(Node::section("turn").id("turn").action(Action {
        id: "turn.cancel".into(),
        on: ActionOn::Click,
        label: Some("Stop".into()),
        args: Value::Null,
    }))
}

/// The blocks of a settled message body.
///
/// Markdown is parsed here, in the middle layer, so that no frontend has to parse it and
/// every frontend agrees on what a quote is. The first text block keeps `{id}.text`;
/// in-flight content with that identity lives in the separate stream channel.
fn body(prefix: &str, id: &str, text: &str) -> Vec<Node> {
    let mut blocks = misa_markdown::blocks(prefix, text);
    if blocks.is_empty() {
        // An empty settled body still has a semantic line.
        blocks.push(Node::text(format!("{prefix}.text"), Vec::new()));
    }
    if let Some(first) = blocks.first_mut()
        && matches!(first.kind, Kind::Text { .. })
    {
        first.id = format!("{id}.text");
    }
    blocks
}

/// Provider failures often arrive as an HTTP prefix followed by a JSON envelope.
/// Keep the useful message and provider detail in the transcript, but do not expose
/// request metadata, user ids, or the wire representation as if it were model prose.
fn compact_error(raw: &str) -> String {
    let raw = raw.trim();
    let Some(separator) = raw.find(": {") else {
        return raw.to_string();
    };
    let prefix = raw[..separator].trim();
    let payload = &raw[separator + 2..];
    let Ok(value) = serde_json::from_str::<serde_json::Value>(payload) else {
        return raw.to_string();
    };
    let error = value.get("error").unwrap_or(&value);
    let message = error.get("message").and_then(serde_json::Value::as_str);
    let detail = error
        .pointer("/metadata/raw")
        .and_then(serde_json::Value::as_str)
        .or_else(|| error.get("detail").and_then(serde_json::Value::as_str));
    match (message, detail) {
        (Some(message), Some(detail)) if message != detail => {
            format!("{prefix}: {message} — {detail}")
        }
        (Some(message), _) => format!("{prefix}: {message}"),
        _ => raw.to_string(),
    }
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

/// The short form of a thinking block: its opening lines and how many remain.
///
/// One line of a long block tells the reader nothing about how much more there is. A
/// bounded preview plus an explicit count makes the disclosure worth opening, while the
/// client still decides whether to show this short form or the whole body.
fn thinking_summary(text: &str) -> Vec<Span> {
    const PREVIEW_LINES: usize = 3;
    let mut spans = vec![Span::strong("thinking".to_string())];
    // One pass, and no Vec of every line: a settled block can be thousands of lines
    // long, and this runs whenever the message that owns it is rebuilt.
    let mut total = 0usize;
    for line in text.lines() {
        if total < PREVIEW_LINES {
            let separator = if total == 0 { " · " } else { "\n" };
            spans.push(Span::plain(format!("{separator}{}", line.trim_end())));
        }
        total += 1;
    }
    if total == 0 {
        spans.push(Span::plain(""));
    }
    let hidden = total.saturating_sub(PREVIEW_LINES);
    if hidden > 0 {
        spans.push(Span::plain(format!(
            "\n… {hidden} {} hidden",
            if hidden == 1 { "line" } else { "lines" }
        )));
    }
    spans
}

/// An image reference and its alternative text, independent of the client.
pub fn image_node(blob: BlobRef, alt: &str, width: u32, height: u32) -> Node {
    Node::new(
        "image",
        Kind::Image {
            blob,
            alt: alt.to_string(),
            width,
            height,
        },
    )
}

/// The attachments a message carries, in the order they were attached.
fn message_attachments(message: &Value) -> Vec<Value> {
    message
        .get("attachments")
        .and_then(Value::as_list)
        .map(<[Value]>::to_vec)
        .unwrap_or_default()
}

/// One attachment of a message.
///
/// The bytes stay in the blob store; clients choose how to render the reference and alt.
///
/// The dimensions are zero, which says "unknown": nothing in the store records them, and a
/// client that wants them can read them from the bytes it fetches. A client that lays out
/// before the fetch can reserve whatever room it likes.
fn attachment_node(attachment: &Value, _position: usize) -> Node {
    let hash = text_at(attachment, "hash");
    let media = text_at(attachment, "media");
    let len = attachment
        .get("len")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        .max(0) as u64;
    let name = text_at(attachment, "source");
    let what = if media.is_empty() {
        "file".to_string()
    } else {
        media.to_string()
    };
    let alt = if name.is_empty() {
        format!("{what} ({len} bytes)")
    } else {
        format!("{name} — {what} ({len} bytes)")
    };
    let blob = BlobRef {
        hash: hash.to_string(),
        len,
        media: (!media.is_empty()).then_some(media.to_string()),
    };
    let mut node = image_node(blob, &alt, 0, 0);
    node.id = format!("attachment.{hash}");
    node.actions.push(Action {
        id: "attachment.save".into(),
        on: ActionOn::Click,
        label: Some("Save attachment".into()),
        args: Value::Null,
    });
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

/// The roots the shipped session declares, with who owns each and how long it
/// lives.
///
/// The previous system's manifest, in Rust, and checked by a test rather than only
/// written down: a fact is the kernel's, an observation is derived, and anything
/// ephemeral must not be journalled.
pub const MANIFEST: &[(&str, Ownership, Lifetime)] = &[
    ("session", Ownership::Kernel, Lifetime::Ephemeral),
    ("messages", Ownership::Kernel, Lifetime::Log),
    ("attempts", Ownership::Kernel, Lifetime::Log),
    ("operations", Ownership::Kernel, Lifetime::Log),
    ("prompt_operations", Ownership::Kernel, Lifetime::Log),
    ("command_operations", Ownership::Kernel, Lifetime::Log),
    ("input_requests", Ownership::Kernel, Lifetime::Log),
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
        let mut session = base
            .get("session")
            .expect("a session")
            .as_map()
            .expect("a map")
            .clone();
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
                        ("attempt", Value::map([("status", Value::str("done"))])),
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
    fn a_thinking_summary_previews_a_few_lines_and_counts_the_rest() {
        let text = super::thinking_summary("one\ntwo\nthree\nfour\nfive");
        let rendered: String = text.iter().map(|span| span.text.as_str()).collect();
        assert_eq!(rendered, "thinking · one\ntwo\nthree\n… 2 lines hidden");
        // A block short enough to read in full is not told that anything is hidden.
        let rendered: String = super::thinking_summary("one\ntwo")
            .iter()
            .map(|span| span.text.as_str())
            .collect();
        assert_eq!(rendered, "thinking · one\ntwo");
    }

    #[test]
    fn a_tree_is_built_and_valid() {
        let node = document(&state(), &[]);
        misa_proto::view::validate(&node).expect("the shipped view is valid");
        assert_eq!(node.role, "session");
    }

    #[test]
    fn the_transcript_holds_the_messages_in_order_with_stable_ids() {
        let node = document(&state(), &[]);
        let transcript = find(&node, "transcript").expect("a transcript");
        let group = &transcript.children[0];
        assert_eq!(group.role, "message.group");
        assert_eq!(group.children.last().unwrap().role, "message.group.footer");
        let ids: Vec<Option<&str>> = group
            .children
            .iter()
            .filter(|child| child.role.starts_with("message.") && child.id.starts_with("msg."))
            .map(|child| Some(child.id.as_str()))
            .collect();
        assert_eq!(ids, vec![Some("msg.1"), Some("msg.2")]);
    }

    #[test]
    fn a_user_group_carries_its_own_timestamp_separator() {
        // A group with no assistant attempt still needs a turn boundary, and its
        // instant is the message's own UTC millisecond value.
        let base = state();
        let state = misa_value::apply_one(
            &base,
            &misa_value::Path::parse("messages").unwrap(),
            &misa_value::Op::Set(Value::list([Value::map([
                ("seq", Value::Int(1)),
                ("role", Value::str("user")),
                ("text", Value::str("hello")),
                ("state", Value::str("done")),
                ("at_ms", Value::Int(1_758_067_200_000)),
            ])])),
        )
        .unwrap();
        let node = document(&state, &[]);
        let group = &find(&node, "transcript").unwrap().children[0];
        let footer = group.children.last().expect("a group footer");
        assert_eq!(footer.role, "message.group.footer");
        assert_eq!(footer.children[0].role, "value.timestamp");
        assert_eq!(
            footer.children[0].kind,
            Kind::Fact {
                value: Value::Int(1_758_067_200_000)
            }
        );
    }

    #[test]
    fn a_rate_fact_is_emitted_only_once_the_attempt_is_settled() {
        let attempt = |status| {
            Value::map([
                ("status", Value::str(status)),
                ("elapsed_ms", Value::Int(2_000)),
                ("output_tokens", Value::Int(100)),
            ])
        };
        let streaming = attempt_footer("group.1", &attempt("streaming"));
        assert!(
            !streaming
                .children
                .iter()
                .any(|child| child.role == "value.rate")
        );
        let settled = attempt_footer("group.1", &attempt("done"));
        let rate = settled
            .children
            .iter()
            .find(|child| child.role == "value.rate")
            .expect("a settled rate");
        assert_eq!(
            rate.kind,
            Kind::Fact {
                value: Value::Float(50.0)
            }
        );
    }

    #[test]
    fn a_provider_failure_is_an_error_block_in_the_same_transcript() {
        let base = initial_state("demo", "openrouter", "stealth/union-alpha", 0);
        let state = Value::map([
            ("session", base.get("session").cloned().unwrap()),
            ("attempts", Value::list([])),
            ("notices", Value::list([])),
            (
                "messages",
                Value::list([
                    Value::map([
                        ("seq", Value::Int(1)),
                        ("role", Value::str("user")),
                        ("text", Value::str("foo")),
                        ("state", Value::str("done")),
                    ]),
                    Value::map([
                        ("seq", Value::Int(2)),
                        ("role", Value::str("assistant")),
                        ("text", Value::str("")),
                        (
                            "error",
                            Value::str(
                                "429 application/json: {\"error\":{\"message\":\"Provider returned error\",\"metadata\":{\"raw\":\"temporarily rate-limited\"}},\"user_id\":\"private\"}",
                            ),
                        ),
                        ("state", Value::str("failed")),
                    ]),
                ]),
            ),
        ]);
        let node = document(&state, &[]);
        let error = find(&node, "msg.2").expect("the failed response");
        assert_eq!(error.role, "error");
        assert_eq!(error.state, Some(State::Failed));
        let text = find(&node, "msg.2.text").expect("the error text");
        let rendered = match &text.kind {
            Kind::Text { spans } => spans
                .iter()
                .map(|span| span.text.as_str())
                .collect::<String>(),
            other => panic!("expected error text, got {other:?}"),
        };
        assert!(rendered.contains("429 application/json: Provider returned error"));
        assert!(rendered.contains("temporarily rate-limited"));
        assert!(!rendered.contains("user_id"));
    }

    #[test]
    fn a_streamed_delta_has_a_node_to_append_to() {
        let node = document(&state(), &[]);
        assert!(
            find(&node, "msg.2.text").is_some(),
            "the first text node must carry the delta id"
        );
    }

    #[test]
    fn a_fenced_block_becomes_a_code_node_rather_than_prose() {
        let node = document(&state(), &[]);
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
        let node = document(&state(), &[]);
        let call = find(&node, "msg.2.call.call.1").expect("the tool call");
        assert_eq!(call.state, Some(State::Done));
        assert_eq!(call.label.as_deref(), Some("echo"));
        assert!(
            call.children
                .iter()
                .any(|child| child.role == "tool.result")
        );
    }

    #[test]
    fn a_result_that_is_a_patch_is_laid_out_as_a_diff() {
        let base = initial_state("demo", "scripted", "scripted-1", 0);
        let state = Value::map([
            (
                "session",
                base.get("session").cloned().unwrap_or(Value::Null),
            ),
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
                            (
                                "result",
                                Value::str("--- a/main.rs\n+++ b/main.rs\n@@ -1 +1 @@\n-old\n+new"),
                            ),
                        ])]),
                    ),
                ])]),
            ),
        ]);
        let node = document(&state, &[]);
        let result = find(&node, "msg.1.call.call.1.result").expect("the result");
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
        let tree = document(&state(), &[]);
        assert!(
            matches!(&find(&tree, "msg.2.call.call.1").unwrap().kind, Kind::Collapsible { summary } if !summary.is_empty())
        );
    }

    #[test]
    fn reused_provider_call_ids_have_distinct_message_scopes() {
        let db = state();
        let message = db.get("messages").unwrap().as_list().unwrap()[1].clone();
        let message = misa_value::apply_one(
            &message,
            &misa_value::Path::parse("seq").unwrap(),
            &misa_value::Op::Set(Value::Int(3)),
        )
        .unwrap();
        let db = misa_value::apply_one(
            &db,
            &misa_value::Path::parse("messages").unwrap(),
            &misa_value::Op::Append(message),
        )
        .unwrap();
        let view = document(&db, &[]);
        misa_proto::view::validate(&view).unwrap();
        assert!(find(&view, "msg.2.call.call.1").is_some());
        assert!(find(&view, "msg.3.call.call.1").is_some());
    }

    #[test]
    fn an_empty_transcript_says_so_instead_of_being_empty() {
        let node = document(&initial_state("demo", "p", "m", 0), &[]);
        assert!(
            find(&node, "transcript").expect("a transcript").children[0].role == "transcript.empty"
        );
        misa_proto::view::validate(&node).unwrap();
    }

    #[test]
    fn there_is_no_cancel_action_when_nothing_is_running() {
        let idle = document(&initial_state("demo", "p", "m", 0), &[]);
        assert!(find(&idle, "turn").is_none());
    }

    #[test]
    fn a_running_turn_keeps_cancel_without_a_redundant_status_line() {
        // The activity indicator already says "working"; the turn node survives
        // only as the cancel affordance and must render no line of its own.
        let state = misa_value::apply_one(
            &state(),
            &misa_value::Path::parse("session.status").unwrap(),
            &misa_value::Op::Set(Value::str("thinking")),
        )
        .unwrap();
        let node = document(&state, &[]);
        let turn = find(&node, "turn").expect("a cancel affordance");
        assert!(matches!(turn.kind, Kind::Section));
        assert!(turn.actions.iter().any(|action| action.id == "turn.cancel"));
    }

    #[test]
    fn the_composer_offers_exactly_one_action_and_the_session_owns_its_meaning() {
        let node = document(&state(), &[]);
        let composer = find(&node, "composer").expect("a composer");
        assert_eq!(composer.actions.len(), 1);
        assert_eq!(composer.actions[0].id, "composer.submit");
        assert_eq!(composer.actions[0].on, ActionOn::Submit);
    }

    #[test]
    fn every_root_the_shipped_state_uses_is_declared() {
        let state = state();
        for root in state.as_map().expect("a map").keys() {
            assert!(
                declared(root).is_some(),
                "the session writes an undeclared root `{root}`"
            );
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
        let blob = BlobRef {
            hash: "a".repeat(64),
            len: 4,
            media: Some("image/png".into()),
        };
        let node = image_node(blob.clone(), "a chart", 10, 10);
        assert!(
            matches!(node.kind, Kind::Image { blob: actual, alt, .. } if actual == blob && alt == "a chart")
        );
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
        let node = document(&state(), &[]);
        let mut ids = Vec::new();
        ids_of(&node, &mut ids);
        assert!(
            ids.contains(&"composer".to_string()),
            "the walk found the session's own nodes"
        );
        for id in ids {
            assert!(
                !id.starts_with("plugin."),
                "the session wrote `{id}`, which is a plugin's namespace"
            );
        }
    }
}
