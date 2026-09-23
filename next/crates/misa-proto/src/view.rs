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
//!   ([`StreamUpdate`](crate::sync::StreamUpdate)) or keep a
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
        Node::new(
            role,
            Kind::Text {
                spans: spans.into_iter().collect(),
            },
        )
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
    /// A code block with its language and its text.
    ///
    /// The language is the fence's own label, when the author wrote one. How the
    /// block is then highlighted is the client's; the tree carries no capture.
    Code {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lang: Option<String>,
        text: String,
    },
    /// A bullet or numbered list. Each item is a list of nodes.
    ///
    /// `markers` is parallel to `items` and only used for task lists: `None` is an
    /// ordinary item, `Some(true)` a checked box and `Some(false)` an empty one. It
    /// is empty when the list is not a task list, and old data decodes without it.
    List {
        ordered: bool,
        items: Vec<Vec<Node>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        markers: Vec<Option<bool>>,
    },
    /// A table. Header cells and body cells are inline content.
    ///
    /// `align` is parallel to the header's columns and comes from the delimiter
    /// row's colons, so a client can line each column up the way the author
    /// asked without parsing the source. It is empty on a tree written before
    /// the field existed, and every column is then left-aligned.
    Table {
        head: Vec<Vec<Span>>,
        rows: Vec<Vec<Vec<Span>>>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        align: Vec<Alignment>,
    },
    /// A definition list: terms and the definitions that explain them.
    ///
    /// The order of `entries` is the document's order. One entry may carry a
    /// term that spans several source lines (the newline is content, exactly as
    /// it is in a paragraph) and several definitions, which is what a term with
    /// more than one `:` line means.
    Definition {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        entries: Vec<Definition>,
    },
    /// Named values: a tool call's arguments, a result's summary, a dialog.
    Fields { fields: Vec<Field> },
    /// A node with a short form and a long form.
    Collapsible { summary: Vec<Span> },
    /// An image, by content hash. A client that cannot draw it shows `alt`.
    Image {
        blob: BlobRef,
        alt: String,
        width: u32,
        height: u32,
    },
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

/// How a table column lines its cells up, taken from the delimiter row's colons.
///
/// A column with no colons is left-aligned, which is also what an older tree
/// without any alignment at all means.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alignment {
    #[default]
    Left,
    Center,
    Right,
}

/// One term and the definitions that explain it, the whole of a definition
/// list's vocabulary.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Definition {
    /// The term, as inline runs. A term that the author wrote across several
    /// lines keeps the newlines between them: they are prose, not structure.
    pub term: Vec<Span>,
    /// One or more definitions for the term, each a run of inline content.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub definitions: Vec<Vec<Span>>,
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
    /// `***strong emphasis***`: both, in one span so a client marks it once.
    StrongEmphasis,
    Emphasis,
    Strikethrough,
    /// Underline, from `<u>`.
    Underline,
    /// A highlighted run, from `==x==` or `<mark>`.
    Highlight,
    /// A lowered run, from `~x~` or `<sub>`.
    Subscript,
    /// A raised run, from `^x^` or `<sup>`.
    Superscript,
    /// A key or chord, from `<kbd>`.
    Kbd,
    Code,
    Link {
        href: String,
    },
}

impl SpanKind {
    fn is_plain(&self) -> bool {
        matches!(self, SpanKind::Plain)
    }
}

/// Constructors for the runs a producer writes most often.
impl Span {
    pub fn plain(text: impl Into<String>) -> Span {
        Span {
            text: text.into(),
            kind: SpanKind::Plain,
        }
    }

    pub fn strong(text: impl Into<String>) -> Span {
        Span {
            text: text.into(),
            kind: SpanKind::Strong,
        }
    }

    pub fn code(text: impl Into<String>) -> Span {
        Span {
            text: text.into(),
            kind: SpanKind::Code,
        }
    }

    pub fn link(text: impl Into<String>, href: impl Into<String>) -> Span {
        Span {
            text: text.into(),
            kind: SpanKind::Link { href: href.into() },
        }
    }
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
    /// Facts a client may project into a selected-choice preview. The session owns the facts;
    /// the client owns wording, ordering, and geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ChoiceMetadata>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChoiceMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub efforts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ChoicePricing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak: Option<ChoicePeak>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChoicePricing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_micros_per_thousand: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_micros_per_thousand: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_micros_per_thousand: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_micros_per_thousand: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_micros: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChoicePeak {
    pub multiplier_ppm: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weekdays: Vec<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub windows: Vec<ChoicePeakWindow>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChoicePeakWindow {
    pub start_hour: i64,
    pub end_hour: i64,
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
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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

/// The deepest tree a session may emit.
pub const MAX_DEPTH: usize = 64;

/// Check every rule a client is entitled to rely on before a tree is sent.
///
/// A tree that fails is not sent: the session reports a fault and keeps the last
/// valid view, which is what the previous system did at the frame boundary and
/// what a client with no validation of its own needs.
pub fn validate(node: &Node) -> Result<(), ViewFault> {
    let mut ids = std::collections::HashSet::new();
    validate_at(node, &mut ids, "0", 0)
}

fn fault(path: &str, reason: impl Into<String>) -> ViewFault {
    ViewFault {
        path: path.to_string(),
        reason: reason.into(),
    }
}

fn validate_at(
    node: &Node,
    ids: &mut std::collections::HashSet<String>,
    path: &str,
    depth: usize,
) -> Result<(), ViewFault> {
    if depth > MAX_DEPTH {
        return Err(fault(path, format!("deeper than {MAX_DEPTH} nodes")));
    }
    if node.role.is_empty() {
        return Err(fault(path, "has no role"));
    }
    if !node
        .role
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
    {
        return Err(fault(
            path,
            format!("role `{}` does not start lowercase", node.role),
        ));
    }
    if let Some(offence) = node.role.chars().find(|ch| {
        !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '_' | '-'))
    }) {
        return Err(fault(
            path,
            format!("role `{}` contains `{offence}`", node.role),
        ));
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
            return Err(fault(
                path,
                format!("action `{}` uses the reserved `client.` prefix", action.id),
            ));
        }
    }
    match &node.kind {
        Kind::Section | Kind::Quote | Kind::Rule | Kind::Status { .. } => {}
        Kind::Text { spans } => check_spans(spans, path)?,
        Kind::Heading { level, spans } => {
            // A document heading is `h1` through `h6`. A level outside that is a
            // session that decided something no renderer agreed to draw.
            if *level == 0 || *level > 6 {
                return Err(fault(
                    path,
                    format!("has a heading level of {level}, which is outside 1..=6"),
                ));
            }
            check_spans(spans, path)?;
        }
        Kind::Code { text, .. } => {
            check_text(text, path)?;
        }
        Kind::List { items, markers, .. } => {
            if !markers.is_empty() && markers.len() != items.len() {
                return Err(fault(path, "has a marker for every item or none at all"));
            }
            for (index, item) in items.iter().enumerate() {
                for (position, child) in item.iter().enumerate() {
                    validate_at(
                        child,
                        ids,
                        &format!("{path}.items[{index}][{position}]"),
                        depth + 1,
                    )?;
                }
            }
        }
        Kind::Table { head, rows, align } => {
            if !align.is_empty() && align.len() != head.len() {
                return Err(fault(
                    path,
                    "has an alignment for every column or none at all",
                ));
            }
            for cell in head {
                check_spans(cell, path)?;
            }
            for row in rows {
                for cell in row {
                    check_spans(cell, path)?;
                }
            }
        }
        Kind::Definition { entries } => {
            // A definition list that names no terms is not a list, and a term
            // with nothing defining it is a paragraph. Both are the parser's
            // business, not a shape a client should have to draw.
            if entries.is_empty() {
                return Err(fault(path, "has no definition entries"));
            }
            for entry in entries {
                if entry.term.is_empty() {
                    return Err(fault(path, "has a definition entry with no term"));
                }
                if entry.definitions.is_empty() {
                    return Err(fault(path, "has a term with no definitions"));
                }
                check_spans(&entry.term, path)?;
                for definition in &entry.definitions {
                    check_spans(definition, path)?;
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
            if blob.hash.len() != 64
                || !blob
                    .hash
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
            {
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
                        format!(
                            "carries a `{}` as a fact, which is not a scalar",
                            other.kind()
                        ),
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
        validate_at(
            child,
            ids,
            &format!("{path}.children[{position}]"),
            depth + 1,
        )?;
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
    }
    Ok(())
}

/// Text may not carry anything a renderer would have to interpret as control.
///
/// Tab is refused rather than expanded: an indent is the client's decision, and a
/// session that wants one says so with structure.
fn check_text(text: &str, path: &str) -> Result<(), ViewFault> {
    if let Some(offence) = text.chars().find(|ch| *ch != '\n' && ch.is_control()) {
        return Err(fault(
            path,
            format!("contains the control character {}", offence.escape_debug()),
        ));
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
                    Kind::Collapsible {
                        summary: vec![Span::plain("thinking")],
                    },
                )
                .id("m2")
                .state(State::Streaming)
                .child(Node::new(
                    "message.thinking",
                    Kind::Code {
                        lang: None,
                        text: "hmm".into(),
                    },
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
        let bad = Node::section("a")
            .id("same")
            .child(Node::section("b").id("same"));
        assert!(validate(&bad).unwrap_err().reason.contains("duplicate id"));
    }

    #[test]
    fn anonymous_nodes_are_allowed_and_not_tracked() {
        let tree = Node::section("a")
            .child(Node::section("b"))
            .child(Node::section("c"));
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
        let bad = Node::new(
            "dialog",
            Kind::Fields {
                fields: vec![Field {
                    id: String::new(),
                    label: "Name".into(),
                    value: String::new(),
                    hint: None,
                    read_only: false,
                    secret: false,
                    kind: FieldKind::Inline,
                }],
            },
        );
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
        assert!(replace(
            &mut root,
            "m2",
            Node::section("message.assistant").id("m2")
        ));
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
            Node::new(
                "a.text",
                Kind::Text {
                    spans: vec![Span::plain("hello"), Span::strong("!")],
                },
            ),
            Node::new(
                "a.code",
                Kind::Code {
                    lang: Some("rust".into()),
                    text: "let x = 1;".into(),
                },
            ),
            Node::new(
                "a.list",
                Kind::List {
                    ordered: true,
                    items: vec![vec![Node::text("item", [Span::plain("one")])]],
                    markers: Vec::new(),
                },
            ),
            Node::new(
                "a.table",
                Kind::Table {
                    head: vec![vec![Span::plain("name")]],
                    rows: vec![vec![vec![Span::plain("value")]]],
                    align: vec![Alignment::Center],
                },
            ),
            Node::new(
                "a.definition",
                Kind::Definition {
                    entries: vec![Definition {
                        term: vec![Span::plain("Term")],
                        definitions: vec![vec![Span::plain("a definition")]],
                    }],
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
                        kind: FieldKind::Choice {
                            options: vec![Choice {
                                value: "gpt".into(),
                                label: "GPT".into(),
                                detail: Some("a model".into()),
                                metadata: None,
                            }],
                            selected: Some("gpt".into()),
                        },
                    }],
                },
            ),
            Node::new(
                "a.collapsible",
                Kind::Collapsible {
                    summary: vec![Span::plain("thinking")],
                },
            ),
            Node::new(
                "a.image",
                Kind::Image {
                    blob: BlobRef {
                        hash: "a".repeat(64),
                        len: 8,
                        media: Some("image/png".into()),
                    },
                    alt: "a chart".into(),
                    width: 640,
                    height: 480,
                },
            ),
            Node::new(
                "a.status",
                Kind::Status {
                    text: "idle".into(),
                },
            ),
            Node::new(
                "a.meter",
                Kind::Meter {
                    label: "Budget".into(),
                    value: 0.5,
                    max: 1.0,
                },
            ),
            Node::new(
                "a.fact",
                Kind::Fact {
                    value: Value::Int(1),
                },
            ),
            Node::new(
                "a.heading",
                Kind::Heading {
                    level: 2,
                    spans: vec![Span::plain("A heading")],
                },
            ),
            Node::new("a.quote", Kind::Quote)
                .child(Node::text("a.quote.text", [Span::plain("quoted")])),
            Node::new("a.rule", Kind::Rule),
        ]
    }

    #[test]
    fn every_shape_a_node_can_have_survives_the_wire() {
        for node in every_shape() {
            // Both with the node's own label and without: the two used to be the same key.
            for node in [node.clone(), node.clone().label("a name")] {
                validate(&node)
                    .unwrap_or_else(|fault| panic!("{:?} does not validate: {fault}", node.kind));
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
        let node = Node::new(
            "cost.meter",
            Kind::Meter {
                label: "Spend".into(),
                value: 1.25,
                max: 10.0,
            },
        )
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

    #[test]
    fn list_markers_must_match_items() {
        let list = |markers| {
            Node::new(
                "a.list",
                Kind::List {
                    ordered: false,
                    items: vec![vec![Node::text("i", [Span::plain("x")])]],
                    markers,
                },
            )
        };
        assert!(validate(&list(Vec::new())).is_ok());
        assert!(validate(&list(vec![Some(true)])).is_ok());
        assert!(validate(&list(vec![Some(true), Some(false)])).is_err());
    }

    #[test]
    fn a_definition_list_needs_a_term_and_a_definition() {
        let definition = |entries| Node::new("a.definition", Kind::Definition { entries });
        assert!(
            validate(&definition(Vec::new())).is_err(),
            "a list with no entries is not a list"
        );
        assert!(
            validate(&definition(vec![Definition {
                term: vec![Span::plain("Term")],
                definitions: vec![vec![Span::plain("body")]],
            }]))
            .is_ok()
        );
        assert!(
            validate(&definition(vec![Definition {
                term: Vec::new(),
                definitions: vec![vec![Span::plain("body")]],
            }]))
            .is_err(),
            "a term is required"
        );
        assert!(
            validate(&definition(vec![Definition {
                term: vec![Span::plain("Term")],
                definitions: Vec::new(),
            }]))
            .is_err(),
            "a definition is required"
        );
    }

    #[test]
    fn a_definition_without_definitions_decodes_from_an_older_tree() {
        // The field is defaulted so a tree that named a term before any body
        // arrived still decodes; validation is what refuses it as a final view.
        let node = Node::new(
            "a.definition",
            Kind::Definition {
                entries: vec![Definition {
                    term: vec![Span::plain("T")],
                    definitions: Vec::new(),
                }],
            },
        );
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&node, &mut bytes).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("definitions"),
            "an empty definition list should not reach the wire"
        );
        let back: Node =
            ciborium::de::from_reader(&bytes[..]).expect("a defaulted definition decodes");
        match back.kind {
            Kind::Definition { entries } => {
                assert_eq!(entries.len(), 1);
                assert!(entries[0].definitions.is_empty());
            }
            other => panic!("expected a definition, got {other:?}"),
        }
    }

    #[test]
    fn a_table_alignment_is_parallel_to_its_header() {
        let table = |align| {
            Node::new(
                "a.table",
                Kind::Table {
                    head: vec![vec![Span::plain("a")], vec![Span::plain("b")]],
                    rows: Vec::new(),
                    align,
                },
            )
        };
        // No alignment at all is an older tree, and it is valid.
        assert!(validate(&table(Vec::new())).is_ok());
        assert!(validate(&table(vec![Alignment::Left, Alignment::Right])).is_ok());
        assert!(validate(&table(vec![Alignment::Left])).is_err());
    }

    #[test]
    fn a_table_without_alignment_decodes_as_left_aligned() {
        // A tree written before the field existed decodes, and the alignment it
        // never carried reads as the left default.
        let node = Node::new(
            "a.table",
            Kind::Table {
                head: vec![vec![Span::plain("a")]],
                rows: vec![vec![vec![Span::plain("1")]]],
                align: Vec::new(),
            },
        );
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&node, &mut bytes).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("align"),
            "an empty alignment should not reach the wire"
        );
        let back: Node = ciborium::de::from_reader(&bytes[..]).unwrap();
        assert_eq!(back, node);
        assert_eq!(Alignment::default(), Alignment::Left);
    }
}
