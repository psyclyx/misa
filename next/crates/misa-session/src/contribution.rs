//! What a composition adds to a session's loop.
//!
//! The shipped agent loop is a *composition*, not a law: `agent::registry()` is the handlers
//! and subscriptions a session starts with, and this is how a caller adds to them — a plugin
//! loaded by a daemon, a policy a deployment ships, a test that wants one handler.
//!
//! # Why this is not `misa_plugin`
//!
//! Nothing here knows what a plugin *is*. A contribution is handlers, subscriptions, and the
//! state roots they write into, which is the same three things the shipped loop contributes —
//! all of them `misa_reframe`'s vocabulary. A composition that loaded a wasm component has
//! already turned it into those; this crate never grows a dependency on a wasm runtime for the
//! privilege of naming what it registers.
//!
//! # Why a root has to be declared
//!
//! A patch may only create the *last* key of its path — the rule in [`misa_value::patch`] is
//! that an absent branch is a leaf, never a container to descend through — so a handler whose
//! state lives at `guest.turns` needs somebody to have made `guest` first. That somebody is the
//! composition, which is the only layer that knows what it loaded.

use std::sync::Arc;

use misa_reframe::{Effect, Event, Fault, Handler, Registry, Subscription, Tx};
use misa_value::{Op, Path, Value};

use crate::views;

/// The kind a recorded patch is journalled under.
///
/// A composition's state is a *fold of its patches*: nothing else can reproduce it, because events
/// are ephemeral by design and a plugin's decision is not derivable from anything else in the log.
/// So the patches go in the log as they are made, and a resumed session applies them again — which
/// is what the transcript does with messages, and the reason a plugin's root is durable state.
pub const PATCH_KIND: &str = "plugin.patch";

/// A handler whose writes into a composition's own roots are recorded.
struct Recording {
    inner: std::sync::Arc<dyn Handler>,
    roots: Vec<String>,
}

impl Handler for Recording {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn handle(&self, tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
        let before = tx.patches().len();
        self.inner.handle(tx, event)?;
        let conversation = tx.text("session.conversation");
        // Collected before anything is queued: an effect is a mutable borrow and the patches are
        // being read.
        let recorded = tx.patches()[before..]
            .iter()
            .filter(|(path, _)| {
                Op::root_of(path).is_some_and(|root| self.roots.iter().any(|declared| declared == root))
            })
            .map(|(path, op)| (path.to_string(), op.to_value()))
            .collect::<Vec<_>>();
        for (path, patch) in recorded {
            tx.fx(
                Effect::new("kernel.log.append")
                    .with("conversation", Value::str(&conversation))
                    .with("kind", Value::str(PATCH_KIND))
                    .with("data", Value::map([("path", Value::str(&path)), ("patch", patch)])),
            );
        }
        Ok(())
    }
}

/// A patch a composition's log recorded: a path and what to do at it.
pub(crate) fn recorded(data: &Value) -> Option<(Path, Op)> {
    let path = data.get("path").and_then(Value::as_str)?;
    let op = Op::from_value(data.get("patch")?)?;
    Some((Path::parse(path).ok()?, op))
}

/// What a composition contributes to a session, beyond the shipped loop.
///
/// Registered in the order given, each handler at the priority it names: a contribution that
/// wants to see an event *before* the loop's own handlers registers below zero, and one that
/// wants to act on what they decided registers above it. Both are real; neither is this
/// crate's decision.
#[derive(Clone, Default)]
pub struct Contribution {
    /// Event kind, priority, and the handler for it.
    pub handlers: Vec<(String, i32, Arc<dyn Handler>)>,
    /// Query name and the subscription that answers it.
    pub subscriptions: Vec<(String, Subscription)>,
    /// Roots this contribution writes into, as a name and the value it starts at.
    ///
    /// Empty maps rather than nothing, because a patch descending through an absent key is a
    /// fault: what is declared here is what the contribution may write into.
    pub roots: Vec<(String, Value)>,
    /// Parts of the document this contribution presents, placed by the session.
    pub sections: Vec<crate::views::Section>,
    /// Affordances its views offer, by the action id a client sends back.
    ///
    /// Not a routing table: the loop routes an action as the `intent/action` event like every
    /// other event, and the handler that acts on it is whichever one declared that event kind.
    /// What this is for is the *answer* — the session can say "no action named that" when there
    /// genuinely is not one, because these are the ones there are.
    pub actions: Vec<String>,
}

impl std::fmt::Debug for Contribution {
    /// Counts and names, not closures: what a diagnostic wants to know about a contribution is
    /// how much of it there is and which state it may write.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Contribution")
            .field("handlers", &self.handlers.iter().map(|(kind, priority, handler)| (kind.clone(), *priority, handler.id().to_string())).collect::<Vec<_>>())
            .field("subscriptions", &self.subscriptions.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>())
            .field("roots", &self.roots.iter().map(|(name, _)| name.clone()).collect::<Vec<_>>())
            .finish()
    }
}

impl Contribution {
    /// An empty contribution: the shipped loop and nothing else.
    pub fn new() -> Contribution {
        Contribution::default()
    }

    pub fn with_handler(
        mut self,
        kind: impl Into<String>,
        priority: i32,
        handler: Arc<dyn Handler>,
    ) -> Contribution {
        self.handlers.push((kind.into(), priority, handler));
        self
    }

    pub fn with_subscription(mut self, name: impl Into<String>, subscription: Subscription) -> Contribution {
        self.subscriptions.push((name.into(), subscription));
        self
    }

    /// Contribute a part of the document.
    ///
    /// Built once per client class per revision, and built *by* the session, so a plugin's
    /// presentation reaches every frontend without one line of frontend code.
    pub fn with_section(mut self, section: crate::views::Section) -> Contribution {
        self.sections.push(section);
        self
    }

    /// Declare an affordance this contribution's views offer.
    ///
    /// Refused for one of the session's own: an action is what the session's vocabulary is made
    /// of, and a contribution that claimed `panel.close` would have the session's handler doing
    /// its work under a name it did not choose.
    pub fn with_action(mut self, action: &str) -> Result<Contribution, misa_proto::Fault> {
        if crate::agent::ACTIONS.contains(&action) {
            return Err(misa_proto::Fault::new(
                "composition.action",
                format!("`{action}` is an action this session already has"),
            ));
        }
        if self.actions.iter().any(|declared| declared == action) {
            return Err(misa_proto::Fault::new(
                "composition.action",
                format!("`{action}` is declared twice"),
            ));
        }
        self.actions.push(action.to_string());
        Ok(self)
    }

    /// Declare a root this contribution writes into.
    ///
    /// Refused for a name the session already owns, which is the whole of what stops a
    /// contribution from overwriting what the loop decided: a plugin that claimed `session` or
    /// `messages` would be writing the agent's own state, and the manifest is what says which
    /// names are taken.
    ///
    /// The name must be plain — a root is one top-level name, not a path — and it is
    /// [`views::Ownership::Plugin`]: not a kernel fact, and not a client's presentation.
    pub fn with_root(mut self, name: &str, initial: Value) -> Result<Contribution, misa_proto::Fault> {
        if name.is_empty() || name.contains(['.', '[', ']']) {
            return Err(misa_proto::Fault::argument("composition", "root", None));
        }
        if views::declared(name).is_some() {
            return Err(misa_proto::Fault::new(
                "composition.root",
                format!("`{name}` is a root this session already owns"),
            ));
        }
        if self.roots.iter().any(|(declared, _)| declared == name) {
            return Err(misa_proto::Fault::new(
                "composition.root",
                format!("`{name}` is declared twice"),
            ));
        }
        self.roots.push((name.to_string(), initial));
        Ok(self)
    }

    /// The shipped registry with this contribution registered.
    ///
    /// Every handler is wrapped so that a patch into one of the roots this contribution declared is
    /// *recorded* — after the transaction commits, because effects run then, so a transaction that
    /// faults leaves nothing in the log. The session records rather than the host, because the log
    /// and its vocabulary are the session's and "what is a fact" is the middle layer's decision
    /// everywhere else here too.
    pub(crate) fn registry(&self, shipped: Registry) -> Registry {
        let mut registry = shipped;
        let roots: Vec<String> = self.roots.iter().map(|(name, _)| name.clone()).collect();
        for (kind, priority, handler) in &self.handlers {
            let handler: std::sync::Arc<dyn Handler> = if roots.is_empty() {
                handler.clone()
            } else {
                std::sync::Arc::new(Recording { inner: handler.clone(), roots: roots.clone() })
            };
            registry = registry.on(kind.clone(), *priority, handler);
        }
        for (name, subscription) in &self.subscriptions {
            registry = registry.subscription(name.clone(), subscription.clone());
        }
        registry
    }

    /// The database a session starts from: the shipped state, plus these roots.
    pub(crate) fn initial_state(&self, id: &str, provider: &str, model: &str, created_ms: i64) -> Value {
        let mut state = views::initial_state(id, provider, model, created_ms);
        if self.roots.is_empty() {
            return state;
        }
        let mut fields = state.as_map().cloned().unwrap_or_default();
        for (name, value) in &self.roots {
            fields.insert(name.clone(), value.clone());
        }
        state = Value::Map(Arc::new(fields));
        // The affordances a composition declared are state, because that is the only thing a
        // handler can read: `agent::on_action` consults this list to tell an action it does not
        // know from one somebody else is handling.
        if !self.actions.is_empty() {
            let mut session = state.get("session").and_then(Value::as_map).cloned().unwrap_or_default();
            session.insert(
                "actions".to_string(),
                Value::list(self.actions.iter().map(Value::str).collect::<Vec<_>>()),
            );
            state = {
                let mut fields = state.as_map().cloned().unwrap_or_default();
                fields.insert("session".to_string(), Value::Map(Arc::new(session)));
                Value::Map(Arc::new(fields))
            };
        }
        state
    }
}
