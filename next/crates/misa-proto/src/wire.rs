//! Messages and subscriptions.
//!
//! # Shape of a connection
//!
//! A client opens one bidirectional stream on the session ALPN. It sends
//! [`ClientMsg`] and reads [`SessionMsg`]. There is no second channel to keep in
//! step: the split between `Value` (a subscription's new value) and `Event` (an
//! ephemeral stream event) is a *semantic* split, not a transport one.
//!
//! - A **subscription** is durable: keyed by [`SubId`], versioned by `rev`, and it
//!   always converges on a value. A client that reconnects re-subscribes and gets
//!   the truth.
//! - A **stream event** is ephemeral and cheap: an append to a node that already
//!   exists, a notice, a status line. Missing one costs latency, never
//!   correctness, because the subscription that describes that node also changes
//!   and delivers the same content in full.
//!
//! That asymmetry is deliberate. It is what lets a token stream arrive thousands
//! of times without sending a transcript thousands of times, while a dropped
//! packet cannot desynchronise a client.

use misa_value::Value;
use serde::{Deserialize, Serialize};

use crate::view::{Field, NodeId};

/// A subscription's handle, chosen by the client and stable for the connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SubId(pub u32);

/// A declared query: a name and its arguments.
///
/// The name is a dotted, lowercase identifier. The arguments are data, so a query
/// is a value a client can log, compare, and retain, and a session can key a
/// scope entry by.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Query {
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Value>,
}

impl Query {
    pub fn new(id: impl Into<String>) -> Self {
        Query { id: id.into(), args: Vec::new() }
    }

    pub fn arg(mut self, value: Value) -> Self {
        self.args.push(value);
        self
    }

    /// The query's canonical form, used as a scope key.
    ///
    /// A separator that cannot appear unescaped in a canonical value key keeps a
    /// query with one argument distinct from a query with two.
    pub fn key(&self) -> String {
        let mut out = self.id.clone();
        for arg in &self.args {
            out.push('\u{1}');
            out.push_str(&arg.canonical_key());
        }
        out
    }
}

/// A kernel-confirmed file offer. The destination is deliberately the client's own state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Download {
    pub blob: Option<crate::view::BlobRef>,
    pub name: String,
    pub error: String,
}

/// Who is connecting. Logged, and useful when several clients attach at once.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

impl ClientInfo {
    pub fn new(name: impl Into<String>, version: impl Into<String>) -> Self {
        ClientInfo { name: name.into(), version: version.into() }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum ClientMsg {
    /// First message on a connection. Nothing else is accepted before it.
    Hello { version: u16, client: ClientInfo },
    /// Ask to be let in, with the code a person was shown.
    ///
    /// The one message a peer that is not admitted may send. Everything else about a session —
    /// that it exists, what it is doing, what its views contain — stays closed to a stranger,
    /// because an endpoint that answers anything is an endpoint that advertises itself.
    ///
    /// A code is single-use and short-lived, and it pairs *this* endpoint key: what is stored
    /// afterwards is the identity iroh authenticated, never the code.
    Pair { code: String, label: String },
    /// Choose which session on this endpoint to talk to.
    ///
    /// Named rather than implied because a session is created after an endpoint
    /// starts listening, and because one connection shape then serves both "what is
    /// here" and "this one".
    Attach { session: String },
    Subscribe { id: SubId, query: Query, since: Option<crate::sync::Version> },
    Unsubscribe { id: SubId },
    /// An intent, correlated by `id` so a session can acknowledge it.
    Intent { id: u64, intent: Intent },
}

impl ClientMsg {
    /// A one-word name for diagnostics.
    pub fn name(&self) -> &'static str {
        match self {
            ClientMsg::Hello { .. } => "hello",
            ClientMsg::Pair { .. } => "pair",
            ClientMsg::Attach { .. } => "attach",
            ClientMsg::Subscribe { .. } => "subscribe",
            ClientMsg::Unsubscribe { .. } => "unsubscribe",
            ClientMsg::Intent { .. } => "intent",
        }
    }
}

/// Everything a client may ask for.
///
/// This is the closed half of the architecture's second rule. A client cannot name
/// an effect, a provider, a model parameter, or a state path: it submits text,
/// resolves an affordance the session itself offered, invokes a command the session
/// itself declared, cancels, or asks for candidates from a source the session
/// declared. What any of those mean is decided on the other side.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "intent", rename_all = "snake_case")]
pub enum Intent {
    /// Interrupt the current turn and submit this prompt before queued prompts.
    Interrupt {
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<crate::view::BlobRef>,
    },
    /// Submit a turn.
    Prompt {
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<crate::view::BlobRef>,
    },
    /// Resolve an action a view node offered.
    Action {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        node: NodeId,
        action: String,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        args: Value,
        /// Field values from the node, for an action with
        /// [`ActionOn::Submit`](crate::view::ActionOn).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<Field>,
    },
    /// Invoke a declared command by name.
    Command {
        name: String,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        args: Value,
    },
    /// Cancel the named work, or everything in flight when unnamed.
    Cancel {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
    },
    /// Ask a session for candidates for a prefix.
    ///
    /// Only for a source declared `OnDemand`, and only when a client has decided it
    /// needs them: matching, ranking, and deciding *when* to show a picker are the
    /// client's, because they are cheap, local, and different on every platform.
    /// Answering "which models are there" or "which files are under this prefix" is
    /// not.
    ///
    /// The reply is [`SessionMsg::Completion`], correlated by this intent's id.
    Complete {
        source: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        prefix: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },
}

impl Intent {
    pub fn name(&self) -> &'static str {
        match self {
            Intent::Interrupt { .. } => "interrupt",
            Intent::Prompt { .. } => "prompt",
            Intent::Action { .. } => "action",
            Intent::Command { .. } => "command",
            Intent::Cancel { .. } => "cancel",
            Intent::Complete { .. } => "complete",
        }
    }
}

/// One argument of a declared command.
///
/// `source` is the whole point: it says *where a value can come from*, and it is
/// declared before anything is typed. A client that has this declaration knows that
/// `/model` needs a model and that models come from somewhere named `models`, so it
/// can offer its own picker without asking. Nothing about how that picker looks, or
/// when it opens, is here — and nothing needs to be, because the declaration is
/// enough for a client to decide.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Arg {
    pub name: String,
    pub label: String,
    /// A command cannot run without it. This is what lets a client narrow a
    /// command into its arguments rather than sending it and being refused.
    #[serde(default)]
    pub required: bool,
    /// A completion source's id, when the argument has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
}

impl Arg {
    pub fn new(name: impl Into<String>, label: impl Into<String>) -> Arg {
        Arg {
            name: name.into(),
            label: label.into(),
            required: false,
            source: None,
            placeholder: None,
        }
    }

    pub fn required(mut self) -> Arg {
        self.required = true;
        self
    }

    pub fn from(mut self, source: impl Into<String>) -> Arg {
        self.source = Some(source.into());
        self
    }
}

/// A command a session offers.
///
/// Declared once, at install, and carried in [`SessionInfo`] — so a client knows
/// what the session can do before it has typed anything, and never has to ask.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Command {
    /// Without a leading slash: the slash is punctuation, and a client draws it.
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Arg>,
}

impl Command {
    pub fn new(id: impl Into<String>, label: impl Into<String>, description: impl Into<String>) -> Command {
        Command { id: id.into(), label: label.into(), description: description.into(), args: Vec::new() }
    }

    pub fn arg(mut self, arg: Arg) -> Command {
        self.args.push(arg);
        self
    }

    /// The first argument that has to be supplied, if any.
    pub fn first_required(&self) -> Option<&Arg> {
        self.args.iter().find(|arg| arg.required)
    }
}

/// Where a set of candidates comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Small and slow-changing: a client subscribes to it and filters locally, so
    /// nothing is asked for while someone is typing.
    Resident,
    /// Large, dynamic, or a capability: a client asks, naming a prefix.
    OnDemand,
}

/// One declared completion source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub label: String,
    pub kind: SourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Source {
    pub fn resident(id: impl Into<String>, label: impl Into<String>) -> Source {
        Source { id: id.into(), label: label.into(), kind: SourceKind::Resident, description: None }
    }

    pub fn on_demand(id: impl Into<String>, label: impl Into<String>, description: impl Into<String>) -> Source {
        Source {
            id: id.into(),
            label: label.into(),
            kind: SourceKind::OnDemand,
            description: Some(description.into()),
        }
    }

    /// The query a resident source is read from.
    ///
    /// A function of the id and nothing else, so a client needs no extra
    /// declaration to know where to look.
    pub fn query(&self) -> String {
        format!("completion.{}", self.id)
    }
}

/// One candidate: the same shape a resident source's items decode to.
///
/// Reusing [`Choice`](crate::view::Choice) means a picker built for one path works
/// unchanged for the other, which is what keeps the two paths indistinguishable to
/// a client once the candidates are in hand.
pub fn candidates(value: &Value) -> Vec<crate::view::Choice> {
    let mut out = Vec::new();
    let Some(items) = value.as_list() else {
        return out;
    };
    for item in items {
        let Some(value) = item.get("value").and_then(Value::as_str) else {
            continue;
        };
        out.push(crate::view::Choice {
            value: value.to_string(),
            label: item
                .get("label")
                .and_then(Value::as_str)
                .unwrap_or(value)
                .to_string(),
            detail: item.get("detail").and_then(Value::as_str).map(str::to_string),
        });
    }
    out
}

/// How many candidates a source will answer with at most.
pub const DEFAULT_CANDIDATES: u32 = 50;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    /// What this session is for, as one line. Rendered by a client as-is.
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<String>,
    pub created_ms: i64,
    /// The composition this session runs: policy names, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policy: Vec<String>,
    /// Query names this session answers, so a client can fail early and say so
    /// plainly instead of subscribing to nothing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub queries: Vec<String>,
    /// What this session can do, declared once and never asked for.
    ///
    /// A client holds these before it types anything, which is what makes "the
    /// client knows when to show a picker" cost nothing: the declaration says a
    /// command needs an argument and where its values come from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<Command>,
    /// Where a command argument's values can come from.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sources: Vec<Source>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

/// An ephemeral, ordered event. Cheap, lossy, and never the source of truth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SessionEvent {
    DownloadReady { id: u64, download: Download },
    /// Append text to a node that already exists in the client's view. The client
    /// appends; it does not re-render from this.
    Stream { update: crate::sync::StreamUpdate },
    /// A line for the client's own place to put such things, not for the view.
    Notice { level: Level, text: String },
    /// What the session is doing right now.
    Status { text: String },
    /// Text for a client's own editor.
    ///
    /// The one place a session hands text back for editing, and only ever in answer
    /// to a request the client itself made — taking a queued prompt back into the
    /// draft. A session that pushed text into an editor unasked would be deciding
    /// what somebody is writing.
    Recover { text: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "msg", rename_all = "snake_case")]
pub enum SessionMsg {
    Download { id: u64, download: Download },
    Welcome { version: u16, session: SessionInfo },
    /// The answer to a [`ClientMsg::Pair`].
    ///
    /// After this, a client says hello like any other; there is no second handshake and no
    /// token to carry, because the thing that was approved is the key the connection arrived
    /// with.
    Paired {
        ok: bool,
        /// What happened, in words: what was paired, or why it was not.
        message: String,
        /// The endpoint id that was approved, so a client can say which key it is.
        endpoint: String,
    },
    /// A subscription's value, at a revision. A later revision supersedes an
    /// earlier one.
    Value { id: SubId, rev: u64, value: Value },
    /// A view, which is its own kind of answer.
    ///
    /// Separate from `Value` because a client draws a view and reads data, and a
    /// protocol that blurred the two would let a view decay into a bag of values
    /// that every frontend then has to interpret.
    View { id: SubId, version: crate::sync::Version, view: crate::view::Node },
    Changes { id: SubId, changes: Vec<crate::sync::Change> },
    Streams { streams: Vec<crate::sync::Stream> },
    /// Candidates for an [`Intent::Complete`], correlated by that intent's id.
    ///
    /// `truncated` is not decoration: a client filtering locally needs to know that
    /// what it holds is not everything, so it can say so rather than pretending a
    /// prefix has no match.
    Completion {
        id: u64,
        source: String,
        candidates: Vec<crate::view::Choice>,
        #[serde(default)]
        truncated: bool,
    },
    /// The subscription cannot be answered. It stays subscribed: a later change
    /// may make it answerable.
    QueryFault { id: SubId, fault: Fault },
    Event { seq: u64, event: SessionEvent },
    Ack { id: u64 },
    /// A refused intent, or a session failure not tied to one.
    Fault {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<u64>,
        fault: Fault,
    },
}

/// A refusal or a contained failure, as a client hears about it.
///
/// Faults are data, not errors. The previous system discovered that a policy fault
/// must roll its transaction back and leave the session running; a client likewise
/// must be able to say what happened without falling over. `code` is a stable
/// identifier, `message` is for a person.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fault {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
}

impl Fault {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Fault { code: code.into(), message: message.into(), data: Value::Null }
    }

    pub fn with(mut self, data: Value) -> Self {
        self.data = data;
        self
    }

    /// The intent was refused because the client asked for something this session
    /// does not accept.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Fault::new("unsupported", message)
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Fault::new("protocol", message)
    }

    pub fn query(message: impl Into<String>) -> Self {
        Fault::new("query", message)
    }
    /// A command was sent without an argument it cannot run without.
    ///
    /// Carries enough for a client to open the right picker without asking, so the
    /// error path narrows to the same place the declaration pointed at. A fault that
    /// only said "missing argument" would make every client re-derive what the
    /// session already knows.
    pub fn argument(command: &str, arg: &str, source: Option<&str>) -> Fault {
        let mut data = std::collections::BTreeMap::new();
        data.insert("command".to_string(), Value::str(command));
        data.insert("argument".to_string(), Value::str(arg));
        if let Some(source) = source {
            data.insert("source".to_string(), Value::str(source));
        }
        Fault::new("argument.required", format!("`/{command}` needs an argument for `{arg}`"))
            .with(Value::Map(std::sync::Arc::new(data)))
    }
}

/// How to reach a session.
///
/// Carried as text so a person can copy one into a chat message. The node part is
/// an endpoint address in whatever form the transport understands; this crate
/// neither parses nor validates it, because it has no opinion about transports.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ticket {
    pub node: String,
    pub session: String,
}
impl Ticket {
    pub fn new(node: impl Into<String>, session: impl Into<String>) -> Self {
        Ticket { node: node.into(), session: session.into() }
    }
}

impl std::fmt::Display for Ticket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "misa:{}:{}", self.node, self.session)
    }
}

/// A ticket and the code that pairs it, as one string.
///
/// The only place this is ever carried by hand is a camera pointed at a terminal, so it is one
/// line and it is unambiguous: `misa-pair:` then a ticket, then `#`, then the code. A `#` cannot
/// appear in a ticket — a node is an endpoint id and an address, a session is a name — so the
/// two halves can never be confused for each other, however an address is spelled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pairing {
    pub ticket: Ticket,
    pub code: String,
}

impl Pairing {
    pub fn new(ticket: Ticket, code: impl Into<String>) -> Pairing {
        Pairing { ticket, code: code.into() }
    }
}

impl Pairing {
    /// What a client was handed: a ticket, and the code that pairs it when there is one.
    ///
    /// One entry point for both shapes, because a person copies one string off a screen — the
    /// ticket a daemon printed, or the pairing line beside its QR code — and a client that made
    /// them find the code separately would be a client with a second thing to get wrong.
    pub fn given(text: &str) -> Result<(Ticket, Option<String>), String> {
        let text = text.trim();
        if text.starts_with("misa-pair:") {
            let pairing: Pairing = text.parse()?;
            return Ok((pairing.ticket, Some(pairing.code)));
        }
        Ok((text.parse::<Ticket>()?, None))
    }
}

impl std::fmt::Display for Pairing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "misa-pair:{}#{}", self.ticket, self.code)
    }
}

impl std::str::FromStr for Pairing {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let rest = text
            .trim()
            .strip_prefix("misa-pair:")
            .ok_or("a pairing string starts with `misa-pair:`")?;
        let (ticket, code) = rest
            .rsplit_once('#')
            .ok_or("a pairing string is `misa-pair:<ticket>#<code>`")?;
        if code.trim().is_empty() {
            return Err("a pairing string needs a code".into());
        }
        Ok(Pairing::new(ticket.parse()?, code.trim()))
    }
}

impl std::str::FromStr for Ticket {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let rest = text.strip_prefix("misa:").ok_or("a ticket starts with `misa:`")?;
        let (node, session) = rest.rsplit_once(':').ok_or("a ticket is `misa:<node>:<session>`")?;
        if node.is_empty() || session.is_empty() {
            return Err("a ticket needs both a node and a session".into());
        }
        Ok(Ticket::new(node, session))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{ActionOn, BlobRef, FieldKind, Span};

    fn round_trip_cbor<T>(value: &T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de>,
    {
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(value, &mut bytes).unwrap();
        ciborium::de::from_reader(&bytes[..]).unwrap()
    }

    #[test]
    fn every_client_message_round_trips() {
        let messages = [
            ClientMsg::Pair { code: "K7QX-3M2P".into(), label: "a phone".into() },
            ClientMsg::Hello {
                version: crate::PROTOCOL_VERSION,
                client: ClientInfo::new("misa-tui", "0.1.0"),
            },
            ClientMsg::Subscribe { since: None, id: SubId(1), query: Query::new("session.view") },
            ClientMsg::Unsubscribe { id: SubId(1) },
            ClientMsg::Intent {
                id: 7,
                intent: Intent::Prompt { text: "hello".into(), attachments: Vec::new() },
            },
            ClientMsg::Unsubscribe { id: SubId(3) },
            ClientMsg::Attach { session: "demo".into() },
        ];
        for message in messages {
            assert_eq!(round_trip_cbor(&message), message);
        }
    }

    #[test]
    fn every_session_message_round_trips() {
        let messages = [
            SessionMsg::Welcome {
                version: crate::PROTOCOL_VERSION,
                session: SessionInfo {
                    id: "demo".into(),
                    title: "a demo session".into(),
                    conversation: Some("c1".into()),
                    created_ms: 0,
                    policy: vec!["agent.default".into()],
                    queries: vec!["session.view".into()],
                    commands: vec![Command::new("model", "Model", "choose a model").arg(Arg::new("model", "Model").required().from("models"))],
                    sources: vec![Source::resident("models", "Models")],
                },
            },
            SessionMsg::Value { id: SubId(2), rev: 4, value: Value::list([Value::Int(1)]) },
            SessionMsg::QueryFault { id: SubId(2), fault: Fault::unsupported("no such query") },
            SessionMsg::Event {
                seq: 9,
                event: SessionEvent::Stream { update: crate::sync::StreamUpdate::Append { id: "m1".into(), offset: 0, text: "…".into() } },
            },
            SessionMsg::Ack { id: 7 },
            SessionMsg::Fault { id: Some(7), fault: Fault::new("refused", "no") },
        ];
        for message in messages {
            assert_eq!(round_trip_cbor(&message), message);
        }
    }

    #[test]
    fn a_submit_intent_carries_only_what_the_session_asked_for() {
        let intent = Intent::Action {
            node: "dialog.model".into(),
            action: "dialog.submit".into(),
            args: Value::Null,
            fields: vec![Field {
                id: "model".into(),
                label: "Model".into(),
                value: "claude".into(),
                hint: None,
                read_only: false,
                secret: false,
                kind: FieldKind::Choice { options: Vec::new(), selected: Some("claude".into()) },
            }],
        };
        assert_eq!(round_trip_cbor(&intent), intent);
        assert_eq!(
            Intent::Action {
                node: String::new(),
                action: "a".into(),
                args: Value::Null,
                fields: Vec::new()
            }
            .name(),
            "action"
        );
    }



    #[test]
    fn a_query_key_distinguishes_its_arguments() {
        let one = Query::new("session.view").arg(Value::Int(1));
        let two = Query::new("session.view").arg(Value::Int(2));
        let three = Query::new("session.view").arg(Value::str("1"));
        assert_ne!(one.key(), two.key());
        assert_ne!(one.key(), three.key());
        assert_eq!(one.key(), one.clone().key());
        assert_ne!(one.key(), Query::new("session.view").key());
    }

    #[test]
    fn a_ticket_is_text_a_person_can_carry() {
        let ticket = Ticket::new("abc123", "demo");
        assert_eq!(ticket.to_string(), "misa:abc123:demo");
        assert_eq!("misa:abc123:demo".parse::<Ticket>().unwrap(), ticket);
        assert!("abc123:demo".parse::<Ticket>().is_err());
        assert!("misa::demo".parse::<Ticket>().is_err());
    }

    #[test]
    fn a_pairing_string_carries_a_ticket_and_a_code_with_no_way_to_confuse_them() {
        // An address is the awkward part: it has colons, an `@`, and possibly commas, so the
        // separator has to be a character none of those can be.
        let ticket = Ticket::new("abc123@127.0.0.1:5000,10.0.0.4:5000", "demo");
        let pairing = Pairing::new(ticket.clone(), "K7QX-3M2P");
        let text = pairing.to_string();
        assert!(text.starts_with("misa-pair:"), "{text}");
        let parsed: Pairing = text.parse().expect("a pairing string");
        assert_eq!(parsed.ticket, ticket);
        assert_eq!(parsed.code, "K7QX-3M2P");
        // And a ticket on its own is not a pairing string, because it has no code.
        assert!("misa:abc123:demo".parse::<Pairing>().is_err());
        assert!("misa-pair:misa:abc123:demo".parse::<Pairing>().is_err());
        assert!("misa-pair:misa:abc123:demo#".parse::<Pairing>().is_err());
        assert!("misa:abc123:demo#code".parse::<Pairing>().is_err());
    }

    #[test]
    fn an_attachment_is_a_blob_reference_not_a_blob() {
        let intent = Intent::Prompt {
            text: "look".into(),
            attachments: vec![BlobRef { hash: "a".repeat(64), len: 10, media: Some("image/png".into()) }],
        };
        assert_eq!(round_trip_cbor(&intent), intent);
    }

    #[test]
    fn an_action_on_round_trips() {
        assert_eq!(round_trip_cbor(&ActionOn::Submit), ActionOn::Submit);
        let spans = vec![Span::link("docs", "https://example.invalid")];
        assert_eq!(round_trip_cbor(&spans), spans);
    }
    #[test]
    fn a_command_declares_where_its_argument_comes_from() {
        let command = Command::new("model", "Model", "choose the model for this session")
            .arg(Arg::new("model", "Model").required().from("models"));
        let required = command.first_required().expect("a required argument");
        assert_eq!(required.name, "model");
        assert_eq!(required.source.as_deref(), Some("models"));
        assert_eq!(Source::resident("models", "Models").query(), "completion.models");
        assert!(matches!(
            Source::on_demand("paths", "Paths", "the daemon's filesystem").kind,
            SourceKind::OnDemand
        ));
    }

    #[test]
    fn candidates_decode_the_same_way_from_either_path() {
        // A resident source's items and an on-demand reply carry one shape, so a
        // picker built for one works unchanged for the other.
        let value = Value::list([
            Value::map([
                ("value", Value::str("claude")),
                ("label", Value::str("Claude")),
                ("detail", Value::str("200k context")),
            ]),
            // An item with no value names nothing and is skipped rather than
            // becoming a candidate that cannot be chosen.
            Value::map([("label", Value::str("nameless"))]),
            Value::map([("value", Value::str("gpt")), ("label", Value::str("GPT"))]),
        ]);
        let decoded = candidates(&value);
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].value, "claude");
        assert_eq!(decoded[0].detail.as_deref(), Some("200k context"));
        assert_eq!(decoded[1].label, "GPT");
        assert!(candidates(&Value::Null).is_empty());
    }

    #[test]
    fn a_completion_reply_is_correlated_with_the_intent_that_asked() {
        let ask = Intent::Complete {
            source: "paths".into(),
            prefix: "src/".into(),
            limit: None,
        };
        assert_eq!(round_trip_cbor(&ask), ask);
        assert_eq!(ask.name(), "complete");
        let reply = SessionMsg::Completion {
            id: 12,
            source: "paths".into(),
            candidates: vec![crate::view::Choice {
                value: "src/main.rs".into(),
                label: "main.rs".into(),
                detail: None,
            }],
            truncated: true,
        };
        assert_eq!(round_trip_cbor(&reply), reply);
    }

    #[test]
    fn a_missing_argument_fault_says_which_picker_to_open() {
        let fault = Fault::argument("model", "model", Some("models"));
        assert_eq!(fault.code, "argument.required");
        assert_eq!(fault.data.get("command").and_then(Value::as_str), Some("model"));
        assert_eq!(fault.data.get("source").and_then(Value::as_str), Some("models"));
    }
}
