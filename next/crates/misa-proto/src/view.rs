//! The semantic view tree.
//!
//! A node is a [`role`](Node::role) — an open, dotted semantic name such as
//! `message.assistant` or `tool.call` — plus a [`Kind`], which is the structural
//! shape of what is here, plus children.
//!
//! # What is deliberately absent
//!
//! There is no colour, no font, no size, no weight (except the inline `Strong`
//! and `Emphasis` runs that mean "the author emphasised this", not "make it
//! bold"), no width, no row, no column, no z-index, no transition. Adding one
//! would be a visible change to the architecture, not a field on a struct, and
//! that is the point: a client and a session cannot drift into an argument about
//! presentation because neither can express the other's side of it.
//!
//! # What is present that a naive API would omit
//!
//! - **Stable ids.** A node that represents the same thing across two revisions
//!   carries the same id, so a client can stream an append into it
//!   ([`wire::SessionEvent::TextDelta`](crate::wire::SessionEvent)) or keep a
//!   scroll anchor without re-rendering the world.
//! - **Structure, not strings.** Markdown arrives as [`Kind::List`],
//!   [`Kind::Code`], [`Kind::Table`], and inline runs. No frontend parses
//!   Markdown, and all three agree on what a quote is.
//! - **State.** [`State`] says a node is pending, streaming, or failed. That is a
//!   domain fact with a visible consequence, and the client needs it to decide
//!   whether to show a caret or to autoscroll.
//! - **Actions.** A node may offer named actions. The client renders an
//!   affordance and sends the action back by name; it never needs to know what
//!   the action does, only what it is called.
//! - **A short form.** [`Kind::Collapsible`] says a node has a summary and a
//!   body. Which of the two is showing is the client's decision.

use misa_value::Value;
use serde::{Deserialize, Serialize};

/// A node's identity within one session's view.
///
/// Empty means anonymous: fine for a purely structural node, and it must not be
/// addressed by an event or an action.
pub type NodeId = String;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: NodeId,
    /// An open, dotted, lowercase semantic name. The one part of this vocabulary
    /// a plugin may extend, because a plugin that adds a tool needs to name the
    /// thing it added.
    pub role: String,
    /// The structural shape, one field rather than flattened into the node's own.
    ///
    /// It used to be `#[serde(flatten)]`, which read a little better and was wrong: a
    /// flattened enum shares the node's own key space, so `Node.label` and
    /// `Kind::Meter.label` were the same key on the wire. Serializing wrote one of them and
    /// deserializing gave it to the node, leaving the meter without a label and the whole
    /// view undecodable — a session that emitted a meter could not be read by anybody.
    /// Nesting costs one level of brackets and removes the whole class of collision.
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<State>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Node>,
}

/// Where a node stands. A domain fact whose consequence happens to be visible.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Pending,
    Streaming,
    Done,
    Failed,
    Cancelled,
}

impl Node {
    pub fn new(role: impl Into<String>, kind: Kind) -> Self {
        Node {
            id: String::new(),
            role: role.into(),
            kind,
            label: None,
            state: None,
            actions: Vec::new(),
            children: Vec::new(),
        }
    }

    pub fn id(mut self, id: impl Into<String>) -> Self {
        self.id = id.into();
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn state(mut self, state: State) -> Self {
        self.state = Some(state);
        self
    }

    pub fn child(mut self, child: Node) -> Self {
        self.children.push(child);
        self
    }

    pub fn children(mut self, children: impl IntoIterator<Item = Node>) -> Self {
        self.children.extend(children);
        self
    }

    pub fn action(mut self, action: Action) -> Self {
        self.actions.push(action);
        self
    }

    /// A plain section: the ordinary grouping node.
    pub fn section(role: impl Into<String>) -> Self {
        Node::new(role, Kind::Section)
    }

    /// A run of inline content.
    pub fn text(role: impl Into<String>, spans: impl IntoIterator<Item = Span>) -> Self {
        Node::new(role, Kind::Text { spans: spans.into_iter().collect() })
    }

    pub fn child_in(&self, id: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.id == id)
    }
}

/// The structural shape of a node.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum Kind {
    /// An ordered group of children. The default.
    Section,
    /// Inline content: text with semantic runs.
    Text { spans: Vec<Span> },
    /// A heading, and the level it sits at.
    ///
    /// The level is *structure*, not size: two clients may draw a level-2 heading
    /// differently and both be right, but neither of them may decide that a
    /// document's `##` was really a paragraph. It is a field rather than a suffix
    /// on the role because a role has no place to put a number that a client is
    /// entitled to rely on.
    Heading {
        /// 1 through 6, as a document source counts them.
        level: u8,
        spans: Vec<Span>,
    },
    /// A block quotation. The blocks it quotes are the node's children.
    Quote,
    /// A thematic break: a horizontal rule between blocks.
    Rule,
    /// A code block with its language, its text, and derived syntax captures.
    ///
    /// Captures are computed once, session-side, because a grammar and a parser
    /// are expensive and versioned. A client maps a capture name to whatever it
    /// has; an unknown name is plain text.
    Code {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lang: Option<String>,
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        captures: Vec<Capture>,
    },
    /// A bullet or numbered list. Each item is a list of nodes.
    List { ordered: bool, items: Vec<Vec<Node>> },
    /// A table. Header cells and body cells are inline content.
    Table { head: Vec<Vec<Span>>, rows: Vec<Vec<Vec<Span>>> },
    /// Named values: a tool call's arguments, a result's summary, a dialog.
    Fields { fields: Vec<Field> },
    /// A node with a short form and a long form.
    Collapsible { summary: Vec<Span> },
    /// An image, by content hash. A client that cannot draw it shows `alt`.
    Image { blob: BlobRef, alt: String, width: u32, height: u32 },
    /// A one-line state statement.
    Status { text: String },
    /// A bounded progress statement, for a budget or a queue depth.
    Meter { label: String, value: f64, max: f64 },
    /// A typed scalar, for a client to render its own way.
    ///
    /// The one place the tree carries a value rather than words, and it exists
    /// because the alternative is worse. A session that writes "≈$1.24" has decided
    /// a currency symbol, a rounding, and a locale; a session that writes `1240000`
    /// and says the role is `value.money` has decided only what the number *is*. The
    /// role is the node's own role, so the same mechanism covers a token count, a
    /// percentage, a duration, and a ratio.
    ///
    /// The value must be a scalar: a fact is one number or one word, not a subtree.
    /// `validate` enforces that, which is what keeps this from becoming a bag of
    /// values that every frontend has to interpret.
    Fact { value: Value },
}

/// One inline run. Adjacent runs with the same kind are merged by convention, not
/// required to be.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Span {
    pub text: String,
    #[serde(default, skip_serializing_if = "SpanKind::is_plain")]
    pub kind: SpanKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "span", rename_all = "snake_case")]
pub enum SpanKind {
    #[default]
    Plain,
    Strong,
    Emphasis,
    Strikethrough,
    Code,
    Link { href: String },
    /// A syntax capture name from [`Capture::token`], as in `keyword`.
    Token { name: String },
}

impl SpanKind {
    fn is_plain(&self) -> bool {
        matches!(self, SpanKind::Plain)
    }
}

/// Constructors for the runs a producer writes most often.
impl Span {
    pub fn plain(text: impl Into<String>) -> Span {
        Span { text: text.into(), kind: SpanKind::Plain }
    }

    pub fn strong(text: impl Into<String>) -> Span {
        Span { text: text.into(), kind: SpanKind::Strong }
    }

    pub fn code(text: impl Into<String>) -> Span {
        Span { text: text.into(), kind: SpanKind::Code }
    }

    pub fn link(text: impl Into<String>, href: impl Into<String>) -> Span {
        Span { text: text.into(), kind: SpanKind::Link { href: href.into() } }
    }

    pub fn token(text: impl Into<String>, name: impl Into<String>) -> Span {
        Span { text: text.into(), kind: SpanKind::Token { name: name.into() } }
    }
}

/// A byte range of a [`Kind::Code`] node's text and the capture covering it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Capture {
    pub start: u32,
    pub end: u32,
    pub token: String,
}

/// A named field, the whole of the dialog vocabulary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Field {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
    /// What kind of input this is. Nested for the same reason a node's shape is: a
    /// flattened enum shares the field's own keys, and `Field.label` is one of them.
    pub kind: FieldKind,
    /// A value to read rather than edit, independent of its shape.
    #[serde(default)]
    pub read_only: bool,
    /// A value clients must mask rather than echo, independent of its shape.
    #[serde(default)]
    pub secret: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum FieldKind {
    /// A single line of text.
    #[default]
    Inline,
    /// Several lines of text.
    Block,
    Bool,
    /// One of a fixed set. The session owns the options; the client owns the
    /// widget.
    Choice {
        options: Vec<Choice>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        selected: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Choice {
    pub value: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Something the node offers. The client renders an affordance and sends the id
/// back; what it means is the session's business.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Action {
    pub id: String,
    #[serde(default)]
    pub on: ActionOn,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub args: Value,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActionOn {
    #[default]
    Click,
    /// The node's field values are submitted with the action.
    Submit,
}

/// A reference to content too large or too binary to inline.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BlobRef {
    /// Lowercase hex of the content hash.
    pub hash: String,
    pub len: u64,
    /// A media type, when the producer knows one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
}

/// Why a view tree was refused. A fault here is a session bug, never a client bug,
/// and it is reported as such.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ViewFault {
    /// The path to the offending node, as `0.children[2]`.
    pub path: String,
    pub reason: String,
}

impl std::fmt::Display for ViewFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "view node at `{}` {}", self.path, self.reason)
    }
}

impl std::error::Error for ViewFault {}

/// The largest tree a session may emit.
pub const MAX_NODES: usize = 200_000;
/// The deepest tree a session may emit.
pub const MAX_DEPTH: usize = 64;
/// The largest inline text run.
pub const MAX_TEXT: usize = 4 * 1024 * 1024;

/// Check every rule a client is entitled to rely on before a tree is sent.
///
/// A tree that fails is not sent: the session reports a fault and keeps the last
/// valid view, which is what the previous system did at the frame boundary and
/// what a client with no validation of its own needs.
pub fn validate(node: &Node) -> Result<(), ViewFault> {
    let mut ids = std::collections::HashSet::new();
    let mut count = 0usize;
    validate_at(node, &mut ids, &mut count, "0", 0)
}

fn fault(path: &str, reason: impl Into<String>) -> ViewFault {
    ViewFault { path: path.to_string(), reason: reason.into() }
}

fn validate_at(
    node: &Node,
    ids: &mut std::collections::HashSet<String>,
    count: &mut usize,
    path: &str,
    depth: usize,
) -> Result<(), ViewFault> {
    *count += 1;
    if *count > MAX_NODES {
        return Err(fault(path, format!("more than {MAX_NODES} nodes")));
    }
    if depth > MAX_DEPTH {
        return Err(fault(path, format!("deeper than {MAX_DEPTH} nodes")));
    }
    if node.role.is_empty() {
        return Err(fault(path, "has no role"));
    }
    if !node.role.chars().next().is_some_and(|first| first.is_ascii_lowercase()) {
        return Err(fault(path, format!("role `{}` does not start lowercase", node.role)));
    }
    if let Some(offence) = node
        .role
        .chars()
        .find(|ch| !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-')))
    {
        return Err(fault(path, format!("role `{}` contains `{offence}`", node.role)));
    }
    if !node.id.is_empty() && !ids.insert(node.id.clone()) {
        return Err(fault(path, format!("duplicate id `{}`", node.id)));
    }
    for action in &node.actions {
        if action.id.is_empty() {
            return Err(fault(path, "offers an action with no id"));
        }
        if action.id.starts_with("client.") {
            // Reserved: a session may not name an affordance only the client can
            // honour, because that would be a session deciding what a client draws.
            return Err(fault(path, format!("action `{}` uses the reserved `client.` prefix", action.id)));
        }
    }
    match &node.kind {
        Kind::Section | Kind::Quote | Kind::Rule | Kind::Status { .. } => {}
        Kind::Text { spans } => check_spans(spans, path)?,
        Kind::Heading { level, spans } => {
            // A document heading is `h1` through `h6`. A level outside that is a
            // session that decided something no renderer agreed to draw.
            if *level == 0 || *level > 6 {
                return Err(fault(path, format!("has a heading level of {level}, which is outside 1..=6")));
            }
            check_spans(spans, path)?;
        }
        Kind::Code { text, captures, .. } => {
            check_text(text, path)?;
            let mut last_end = 0u32;
            for capture in captures {
                if capture.token.is_empty() {
                    return Err(fault(path, "has a capture with no token"));
                }
                if capture.start >= capture.end {
                    return Err(fault(path, format!("has an empty capture at {}", capture.start)));
                }
                if capture.end as usize > text.len() {
                    return Err(fault(path, format!("capture ends past the text at {}", capture.end)));
                }
                if capture.start < last_end {
                    return Err(fault(path, "has overlapping or unordered captures"));
                }
                if !text.is_char_boundary(capture.start as usize) || !text.is_char_boundary(capture.end as usize) {
                    return Err(fault(path, format!("capture at {} is not on a character boundary", capture.start)));
                }
                last_end = capture.end;
            }
        }
        Kind::List { items, .. } => {
            for (index, item) in items.iter().enumerate() {
                for (position, child) in item.iter().enumerate() {
                    validate_at(child, ids, count, &format!("{path}.items[{index}][{position}]"), depth + 1)?;
                }
            }
        }
        Kind::Table { head, rows } => {
            for cell in head {
                check_spans(cell, path)?;
            }
            for row in rows {
                for cell in row {
                    check_spans(cell, path)?;
                }
            }
        }
        Kind::Fields { fields } => {
            for field in fields {
                if field.id.is_empty() {
                    return Err(fault(path, "has a field with no id"));
                }
                if !ids.insert(format!("field:{}", field.id)) {
                    return Err(fault(path, format!("duplicate field id `{}`", field.id)));
                }
                check_text(&field.value, path)?;
                check_text(&field.label, path)?;
            }
        }
        Kind::Collapsible { summary, .. } => check_spans(summary, path)?,
        Kind::Image { blob, alt, .. } => {
            if blob.hash.len() != 64 || !blob.hash.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) {
                return Err(fault(path, "names a blob that is not a lowercase hex hash"));
            }
            check_text(alt, path)?;
        }
        Kind::Fact { value } => {
            // A fact is one number or one word. Anything else and this stopped being
            // a typed scalar and became a subtree the client must interpret.
            match value {
                Value::Null | Value::Bool(_) | Value::Int(_) | Value::Float(_) | Value::Str(_) => {}
                other => {
                    return Err(fault(
                        path,
                        format!("carries a `{}` as a fact, which is not a scalar", other.kind()),
                    ));
                }
            }
            if let Value::Str(text) = value {
                check_text(text, path)?;
            }
        }
        Kind::Meter { label, value, max } => {
            check_text(label, path)?;
            if !value.is_finite() || !max.is_finite() || *max <= 0.0 {
                return Err(fault(path, "has a meter with a non-finite or empty bound"));
            }
        }
    }
    for (position, child) in node.children.iter().enumerate() {
        validate_at(child, ids, count, &format!("{path}.children[{position}]"), depth + 1)?;
    }
    Ok(())
}

fn check_spans(spans: &[Span], path: &str) -> Result<(), ViewFault> {
    for span in spans {
        check_text(&span.text, path)?;
        if let SpanKind::Link { href } = &span.kind {
            if href.is_empty() || href.len() > 4096 {
                return Err(fault(path, "has a link with an empty or oversized target"));
            }
            if href.chars().any(char::is_control) {
                return Err(fault(path, "has a link containing a control character"));
            }
        }
        if let SpanKind::Token { name } = &span.kind
            && name.is_empty()
        {
            return Err(fault(path, "has a span with an empty capture name"));
        }
    }
    Ok(())
}

/// Text may not carry anything a renderer would have to interpret as control.
///
/// Tab is refused rather than expanded: an indent is the client's decision, and a
/// session that wants one says so with structure.
fn check_text(text: &str, path: &str) -> Result<(), ViewFault> {
    if text.len() > MAX_TEXT {
        return Err(fault(path, format!("holds more than {MAX_TEXT} bytes of text")));
    }
    if let Some(offence) = text.chars().find(|ch| *ch != '\n' && ch.is_control()) {
        return Err(fault(path, format!("contains the control character {}", offence.escape_debug())));
    }
    Ok(())
}

/// Every node in depth-first order, parents before children.
pub fn walk<'a>(node: &'a Node, out: &mut Vec<&'a Node>) {
    out.push(node);
    for child in &node.children {
        walk(child, out);
    }
}

/// Find a node by id.
pub fn find<'a>(node: &'a Node, id: &str) -> Option<&'a Node> {
    if node.id == id {
        return Some(node);
    }
    node.children.iter().find_map(|child| find(child, id))
}

/// Replace one node's subtree in place, matched by id. Returns whether it matched.
pub fn replace(root: &mut Node, id: &str, replacement: Node) -> bool {
    if root.id == id {
        *root = replacement;
        return true;
    }
    for child in root.children.iter_mut() {
        if replace(child, id, replacement.clone()) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Node {
        Node::section("conversation")
            .id("root")
            .child(
                Node::text("message.user", [Span::plain("hello")])
                    .id("m1")
                    .state(State::Done),
            )
            .child(
                Node::new(
                    "message.assistant",
                    Kind::Collapsible { summary: vec![Span::plain("thinking")] },
                )
                .id("m2")
                .state(State::Streaming)
                .child(Node::new(
                    "message.thinking",
                    Kind::Code { lang: None, text: "hmm".into(), captures: vec![] },
                )),
            )
    }

    #[test]
    fn a_good_tree_validates() {
        validate(&tree()).unwrap();
    }

    #[test]
    fn roles_must_be_lowercase_dotted_names() {
        let bad = Node::section("Message.User");
        assert!(validate(&bad).is_err());
        let bad = Node::section("message user");
        assert!(validate(&bad).is_err());
        let bad = Node::section("");
        assert!(validate(&bad).is_err());
    }

    #[test]
    fn ids_must_be_unique() {
        let bad = Node::section("a").id("same").child(Node::section("b").id("same"));
        assert!(validate(&bad).unwrap_err().reason.contains("duplicate id"));
    }

    #[test]
    fn anonymous_nodes_are_allowed_and_not_tracked() {
        let tree = Node::section("a").child(Node::section("b")).child(Node::section("c"));
        validate(&tree).unwrap();
    }

    #[test]
    fn text_may_not_carry_control_characters() {
        let bad = Node::text("a", [Span::plain("bell\u{7}")]);
        assert!(validate(&bad).is_err());
        // A newline is content, not control.
        let ok = Node::text("a", [Span::plain("two\nlines")]);
        validate(&ok).unwrap();
        // A tab is not: an indent is the client's decision.
        let bad = Node::text("a", [Span::plain("did\tshift")]);
        assert!(validate(&bad).is_err());
    }

    #[test]
    fn captures_must_be_ordered_non_empty_and_in_range() {
        let code = |captures| Node::new("c", Kind::Code { lang: Some("rust".into()), text: "let x".into(), captures });
        validate(&code(vec![Capture { start: 0, end: 3, token: "keyword".into() }])).unwrap();
        assert!(validate(&code(vec![Capture { start: 2, end: 2, token: "k".into() }])).is_err());
        assert!(validate(&code(vec![Capture { start: 0, end: 99, token: "k".into() }])).is_err());
        assert!(
            validate(&code(vec![
                Capture { start: 4, end: 5, token: "a".into() },
                Capture { start: 0, end: 3, token: "b".into() },
            ]))
            .is_err()
        );
    }

    #[test]
    fn a_session_may_not_name_a_client_affordance() {
        let bad = Node::section("a").action(Action {
            id: "client.scroll_to_top".into(),
            on: ActionOn::Click,
            label: None,
            args: Value::Null,
        });
        assert!(validate(&bad).unwrap_err().reason.contains("reserved"));
    }

    #[test]
    fn fields_must_be_addressable() {
        let bad = Node::new("dialog", Kind::Fields { fields: vec![Field {
            id: String::new(),
            label: "Name".into(),
            value: String::new(),
            hint: None,
            read_only: false,
            secret: false,
            kind: FieldKind::Inline,
        }] });
        assert!(validate(&bad).is_err());
    }

    #[test]
    fn depth_is_bounded() {
        let mut node = Node::section("leaf");
        for _ in 0..MAX_DEPTH + 2 {
            node = Node::section("wrap").child(node);
        }
        assert!(validate(&node).unwrap_err().reason.contains("deeper than"));
    }

    #[test]
    fn find_and_replace_walk_the_tree() {
        let mut root = tree();
        assert!(find(&root, "m1").is_some());
        assert!(find(&root, "nope").is_none());
        assert!(replace(&mut root, "m2", Node::section("message.assistant").id("m2")));
        assert!(!replace(&mut root, "nope", Node::section("x")));
        validate(&root).unwrap();
    }

    #[test]
    fn walk_visits_parents_first() {
        let root = tree();
        let mut nodes = Vec::new();
        walk(&root, &mut nodes);
        assert_eq!(nodes.first().unwrap().id, "root");
        assert_eq!(nodes.last().unwrap().role, "message.thinking");
    }

    #[test]
    fn a_tree_round_trips_through_cbor() {
        let root = tree();
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&root, &mut bytes).unwrap();
        let back: Node = ciborium::de::from_reader(&bytes[..]).unwrap();
        assert_eq!(root, back);
    }

    /// One node for every shape the vocabulary can express, each with a label.
    ///
    /// The label is the point. When `Kind` was flattened into the node, a `Meter`'s own
    /// `label` was the same wire key as the node's, so a meter — and therefore the whole
    /// view containing it — could not be decoded by anybody, while a tree of sections and
    /// text round-tripped happily. Anything that reaches a client goes through this
    /// function, so every shape has to survive it.
    fn every_shape() -> Vec<Node> {
        vec![
            Node::new("a.section", Kind::Section),
            Node::new("a.text", Kind::Text { spans: vec![Span::plain("hello"), Span::strong("!")] }),
            Node::new(
                "a.code",
                Kind::Code {
                    lang: Some("rust".into()),
                    text: "let x = 1;".into(),
                    captures: vec![Capture { start: 0, end: 3, token: "keyword".into() }],
                },
            ),
            Node::new(
                "a.list",
                Kind::List {
                    ordered: true,
                    items: vec![vec![Node::text("item", [Span::plain("one")])]],
                },
            ),
            Node::new(
                "a.table",
                Kind::Table {
                    head: vec![vec![Span::plain("name")]],
                    rows: vec![vec![vec![Span::plain("value")]]],
                },
            ),
            Node::new(
                "a.fields",
                Kind::Fields {
                    fields: vec![Field {
                        id: "model".into(),
                        label: "Model".into(),
                        value: "gpt".into(),
                        hint: Some("which one".into()),
                        read_only: false,
                        secret: false,
                        kind: FieldKind::Choice { options: vec![Choice {
                            value: "gpt".into(),
                            label: "GPT".into(),
                            detail: Some("a model".into()),
                        }], selected: Some("gpt".into()) },
                    }],
                },
            ),
            Node::new("a.collapsible", Kind::Collapsible { summary: vec![Span::plain("thinking")] }),
            Node::new(
                "a.image",
                Kind::Image {
                    blob: BlobRef { hash: "a".repeat(64), len: 8, media: Some("image/png".into()) },
                    alt: "a chart".into(),
                    width: 640,
                    height: 480,
                },
            ),
            Node::new("a.status", Kind::Status { text: "idle".into() }),
            Node::new("a.meter", Kind::Meter { label: "Budget".into(), value: 0.5, max: 1.0 }),
            Node::new("a.fact", Kind::Fact { value: Value::Int(1) }),
            Node::new("a.heading", Kind::Heading { level: 2, spans: vec![Span::plain("A heading")] }),
            Node::new("a.quote", Kind::Quote).child(Node::text("a.quote.text", [Span::plain("quoted")])),
            Node::new("a.rule", Kind::Rule),
        ]
    }

    #[test]
    fn every_shape_a_node_can_have_survives_the_wire() {
        for node in every_shape() {
            // Both with the node's own label and without: the two used to be the same key.
            for node in [node.clone(), node.clone().label("a name")] {
                validate(&node).unwrap_or_else(|fault| panic!("{:?} does not validate: {fault}", node.kind));
                let mut bytes = Vec::new();
                ciborium::ser::into_writer(&node, &mut bytes).unwrap();
                let back: Node = ciborium::de::from_reader(&bytes[..])
                    .unwrap_or_else(|err| panic!("{:?} does not decode: {err}", node.kind));
                assert_eq!(back, node, "a node came back changed");
            }
        }
    }

    #[test]
    fn a_node_keeps_its_own_label_when_its_shape_has_one_too() {
        // The collision, named: a meter's label is the meter's, and a node's label is the
        // node's, and both survive.
        let node = Node::new("cost.meter", Kind::Meter {
            label: "Spend".into(),
            value: 1.25,
            max: 10.0,
        })
        .label("this turn");
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&node, &mut bytes).unwrap();
        let back: Node = ciborium::de::from_reader(&bytes[..]).expect("a labelled meter decodes");
        assert_eq!(back.label.as_deref(), Some("this turn"));
        assert!(
            matches!(back.kind, Kind::Meter { ref label, .. } if label == "Spend"),
            "the meter's own label went missing"
        );
    }
}
