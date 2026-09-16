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

fn stage(before: &Value, outcome: &mut misa_reframe::Outcome) -> Result<bool, Fault> {
    let journal = |effect: &Effect| effect.kind == "kernel.log.append" && effect.get("kind").and_then(Value::as_str) == Some(PATCH_KIND);
    let records: Vec<_> = outcome.effects.iter().filter(|effect| journal(effect)).collect();
    let session = before.get("session").ok_or_else(|| Fault::new("composition.state", "Missing session state"))?;
    let candidate = outcome.changes.last().map(|change| &change.after).unwrap_or(before);
    let command = crate::command_operations::staged(candidate);
    if records.is_empty() && command.is_none() { return Ok(false); }
    if command.is_some() && outcome.effects.iter().any(|effect| !journal(effect) && effect.kind != "wire.event") {
        return Err(Fault::new("command.effects", "Transaction commands cannot start external effects; install an operation-aware command handler"));
    }
    if candidate.get("session").and_then(|session| session.get("plugin_write")).is_some_and(|value| !matches!(value, Value::Null)) {
        return Err(Fault::new("composition.busy", "A contributed state transaction is awaiting journal acknowledgement"));
    }
    let conversation = records.first().and_then(|effect| effect.get("conversation")).cloned().unwrap_or_else(|| session.get("conversation").cloned().unwrap_or(Value::Null));
    if records.iter().any(|effect| effect.get("conversation") != Some(&conversation)) {
        return Err(Fault::new("composition.transaction", "One transaction cannot journal plugin state into different conversations"));
    }
    let mut patches = Vec::new();
    for effect in records {
        let data = effect.get("data").ok_or_else(|| Fault::new("composition.transaction", "Missing patch record"))?;
        if let Some(group) = data.get("patches").and_then(Value::as_list) { patches.extend_from_slice(group); }
        else { patches.push(data.clone()); }
    }
    let sequence = session.get("plugin_sequence").and_then(Value::as_i64).unwrap_or(0).checked_add(1)
        .ok_or_else(|| Fault::new("composition.transaction", "Plugin transaction sequence exhausted"))?;
    let mut data = if patches.len() == 1 {
        let mut fields = patches[0].as_map().cloned().unwrap_or_default();
        fields.insert("write".into(), Value::Int(sequence));
        Value::Map(Arc::new(fields))
    } else { Value::map([("write", Value::Int(sequence)), ("patches", Value::list(patches))]) };
    if let Some(command) = &command { data = crate::command_operations::add_completion(data, command); }
    outcome.effects.retain(|effect| !journal(effect));
    outcome.effects.push(Effect::new("kernel.log.append").with("conversation", conversation).with("kind", Value::str(PATCH_KIND)).with("data", data.clone()));
    let deferred: std::collections::BTreeSet<_> = std::mem::take(&mut outcome.deferred).into_iter().collect();
    for (change_index, change) in outcome.changes.iter_mut().enumerate() {
        let mut patch_index = 0;
        change.patches.retain(|_| {
            let retain = !deferred.contains(&(change_index, patch_index));
            patch_index += 1;
            retain
        });
    }
    let change = outcome.changes.last_mut().ok_or_else(|| Fault::new("composition.transaction", "A journal decision needs a state transaction"))?;
    change.patches.push((Path::parse("session.plugin_sequence").unwrap(), Op::Set(Value::Int(sequence))));
    change.patches.push((Path::parse("session.plugin_write").unwrap(), Op::Set(data)));
    if command.is_some() { crate::command_operations::clear_marker(outcome); }
    Ok(true)
}

/// A handler confined to the roots its composition declared, whose writes are recorded.
///
/// One rule seen twice: a composition declares what it may write, so a patch into a root it
/// declared is a fact the log has to keep, and a patch into anything else is a fault.
struct Confined {
    inner: std::sync::Arc<dyn Handler>,
    roots: Vec<String>,
}

impl Handler for Confined {
    fn id(&self) -> &str {
        self.inner.id()
    }

    fn handle(&self, tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
        let before = tx.patches().len();
        self.inner.handle(tx, event)?;
        // Checked after the handler ran, because that is when its patches exist — and safe for the
        // same reason every fault here is: the transaction is the unit. Nothing it wrote lands and
        // nothing it asked for is performed, including the entries this would have recorded.
        //
        // A patch with no root at all (`[0].text`) fails this too, which is right: there is no root
        // to have declared, and the database would refuse the path a moment later anyway.
        for (path, _) in &tx.patches()[before..] {
            let declared = Op::root_of(path).is_some_and(|root| self.roots.iter().any(|declared| declared == root));
            if !declared {
                return Err(Fault::new(
                    "composition.root",
                    format!("`{path}` is not in a root this composition declared"),
                ));
            }
        }
        misa_value::apply(tx.db(), tx.patches())
            .map_err(|error| Fault::new("patch", error.to_string()))?;
        let conversation = tx.text("session.conversation");
        // Collected before anything is queued: an effect is a mutable borrow and the patches are
        // being read. Every patch is in a declared root by now, so this list is the whole of what
        // the composition wrote.
        let recorded = tx.patches()[before..].iter().map(|(path, op)| {
            Value::map([("path", Value::str(path.to_string())), ("patch", op.to_value())])
        }).collect::<Vec<_>>();
        if !recorded.is_empty() {
            tx.defer_patches_from(before);
            let data = if recorded.len() == 1 { recorded[0].clone() } else { Value::map([("patches", Value::list(recorded))]) };
            tx.fx(Effect::new("kernel.log.append").with("conversation", Value::str(&conversation))
                .with("kind", Value::str(PATCH_KIND)).with("data", data));
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

#[cfg(test)]
mod transaction_tests {
    use super::*;

    #[test]
    fn acknowledgement_can_publish_old_writes_and_stage_new_ones() {
        let contribution = Contribution::default()
            .with_root("counter", Value::map([("value", Value::Int(0))])).unwrap()
            .with_handler("counter/increment", 0, Arc::new(misa_reframe::FnHandler::new("increment", |tx: &mut Tx<'_>, _: &Event| {
                tx.set("counter.value", Value::Int(tx.int("counter.value") + 1))
            })))
            .with_handler("kernel/log.appended", 100, Arc::new(misa_reframe::FnHandler::new("after-ack", |tx: &mut Tx<'_>, event: &Event| {
                if event.get("kind").and_then(Value::as_str) == Some(PATCH_KIND)
                    && event.get("data").and_then(|data| data.get("write")).and_then(Value::as_i64) == Some(1) {
                    tx.set("counter.value", Value::Int(tx.int("counter.value") + 1))?;
                }
                Ok(())
            })));
        let mut owner = misa_reframe::Loop::new(Arc::new(contribution.registry(crate::agent::registry())), Arc::new(crate::AcceptedEffects), contribution.initial_state("test", "scripted", "model", 0));
        let first = owner.dispatch(Event::new("counter/increment"));
        let acknowledge = |outcome: &misa_reframe::Outcome, seq| {
            let effect = outcome.effects.iter().find(|effect| effect.get("kind").and_then(Value::as_str) == Some(PATCH_KIND)).unwrap();
            Event::new("kernel/log.appended").with("conversation", effect.get("conversation").unwrap().clone())
                .with("kind", Value::str(PATCH_KIND)).with("data", effect.get("data").unwrap().clone()).with("seq", Value::Int(seq))
        };
        let second = owner.dispatch(acknowledge(&first, 1));
        assert!(second.committed(), "{:?}", second.as_faults());
        assert_eq!(owner.db().get("counter").unwrap().get("value"), Some(&Value::Int(1)), "the acknowledged write must survive staging the listener's write");
        let final_result = owner.dispatch(acknowledge(&second, 2));
        assert!(final_result.committed());
        assert_eq!(owner.db().get("counter").unwrap().get("value"), Some(&Value::Int(2)));
    }

    #[test]
    fn journal_staging_preserves_sequential_handler_reads() {
        let increment = |name| Arc::new(misa_reframe::FnHandler::new(name, |tx: &mut Tx<'_>, _: &Event| {
            tx.set("counter.value", Value::Int(tx.int("counter.value") + 1))
        })) as Arc<dyn Handler>;
        let contribution = Contribution::default()
            .with_root("counter", Value::map([("value", Value::Int(0))])).unwrap()
            .with_handler("counter/increment", 0, increment("first"))
            .with_handler("counter/increment", 1, increment("second"));
        let mut owner = misa_reframe::Loop::new(
            Arc::new(contribution.registry(crate::agent::registry())),
            Arc::new(crate::AcceptedEffects),
            contribution.initial_state("test", "scripted", "model", 0),
        );
        let result = owner.dispatch(Event::new("counter/increment"));
        assert!(result.committed(), "{:?}", result.as_faults());
        assert_eq!(owner.db().get("counter").unwrap().get("value"), Some(&Value::Int(0)), "publication waits for durable acknowledgement");
        let competing = owner.dispatch(Event::new("counter/increment"));
        assert!(!competing.committed());
        assert_eq!(competing.faults[0].code, "composition.busy");
        assert!(competing.effects.is_empty());
        // Apply the durable decisions exactly as replay would. Handler two must
        // have read handler one's working state, even before any journal ack.
        let mut replay = contribution.initial_state("test", "scripted", "model", 0);
        for effect in result.effects.iter().filter(|effect| effect.kind == "kernel.log.append") {
            let data = effect.get("data").unwrap();
            let records = data.get("patches").and_then(Value::as_list).map(<[Value]>::to_vec).unwrap_or_else(|| vec![data.clone()]);
            for record in records {
                let (path, operation) = recorded(&record).unwrap();
                replay = misa_value::apply_one(&replay, &path, &operation).unwrap();
            }
        }
        assert_eq!(replay.get("counter").unwrap().get("value"), Some(&Value::Int(2)));
        let record = result.effects.iter().find(|effect| effect.get("kind").and_then(Value::as_str) == Some(PATCH_KIND)).unwrap();
        let ack = Event::new("kernel/log.appended").with("conversation", record.get("conversation").unwrap().clone())
            .with("kind", Value::str(PATCH_KIND)).with("data", record.get("data").unwrap().clone()).with("seq", Value::Int(1));
        assert!(owner.dispatch(ack.clone()).committed());
        assert_eq!(owner.db().get("counter").unwrap().get("value"), Some(&Value::Int(2)));
        assert!(owner.dispatch(ack).committed());
        assert_eq!(owner.db().get("counter").unwrap().get("value"), Some(&Value::Int(2)), "duplicate acknowledgement cannot apply twice");
        let next = owner.dispatch(Event::new("counter/increment"));
        assert!(next.committed());
        let next_record = next.effects.iter().find(|effect| effect.get("kind").and_then(Value::as_str) == Some(PATCH_KIND)).unwrap();
        let failure = |data| Event::new("kernel/log.failed").with("conversation", next_record.get("conversation").unwrap().clone())
            .with("kind", Value::str(PATCH_KIND)).with("data", data).with("message", Value::str("disk refused write"));
        assert!(owner.dispatch(failure(record.get("data").unwrap().clone())).committed());
        assert!(!owner.dispatch(Event::new("counter/increment")).committed(), "old failure cannot release a newer decision");
        assert!(owner.dispatch(failure(next_record.get("data").unwrap().clone())).committed());
        assert_eq!(owner.db().get("counter").unwrap().get("value"), Some(&Value::Int(2)), "failed persistence publishes no candidate state");
        assert!(owner.dispatch(Event::new("counter/increment")).committed(), "exact failure releases admission");
    }
}

/// What a composition contributes to a session, beyond the shipped loop.
///
/// Registered in the order given, each handler at the priority it names: a contribution that
/// wants to see an event *before* the loop's own handlers registers below zero, and one that
/// wants to act on what they decided registers above it. Both are real; neither is this
/// crate's decision.
#[derive(Clone, Default)]
pub struct Contribution {
    pub tools: Vec<misa_proto::tool::Binding>,
    pub commands: Vec<crate::commands::CommandRegistration>,
    pub bindings: Vec<misa_proto::invocation::Binding>,
    /// Event kind, priority, and the handler for it.
    pub handlers: Vec<(String, i32, Arc<dyn Handler>)>,
    /// Query name and the subscription that answers it.
    pub subscriptions: Vec<(String, Subscription)>,
    /// Explicit contracts for queries this owner permits clients to select.
    pub query_exports: Vec<misa_proto::query::Definition>,
    pub presentations: Vec<misa_proto::presentation::Presentation>,
    /// Roots this contribution writes into, as a name and the value it starts at.
    ///
    /// Empty maps rather than nothing, because a patch descending through an absent key is a
    /// fault: what is declared here is what the contribution may write into.
    pub roots: Vec<(String, Value)>,
    /// Parts of the document this contribution presents, placed by the session.
    pub sections: Vec<crate::views::Section>,
    pub indicators: Vec<crate::indicators::Indicator>,
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
    pub fn with_tool(mut self, tool:misa_proto::tool::Binding)->Self {self.tools.push(tool);self}
    pub fn with_binding(mut self, binding: misa_proto::invocation::Binding) -> Self {
        self.bindings.push(binding);
        self
    }
    pub fn with_command(mut self, command: crate::commands::CommandRegistration) -> Self {
        self.commands.push(command);
        self
    }
    pub fn with_presentation(mut self, presentation: misa_proto::presentation::Presentation) -> Self {
        self.presentations.push(presentation);
        self
    }
    pub fn export_query(mut self, definition: misa_proto::query::Definition) -> Self {
        self.query_exports.push(definition);
        self
    }
    pub fn with_indicator(mut self, indicator: crate::indicators::Indicator) -> Result<Self, misa_proto::Fault> {
        let mut catalog = crate::indicators::builtins();
        for entry in &self.indicators {
            catalog.register(entry.clone()).map_err(|message| misa_proto::Fault::new("composition.indicator", message))?;
        }
        catalog.register(indicator.clone()).map_err(|message| misa_proto::Fault::new("composition.indicator", message))?;
        self.indicators.push(indicator);
        Ok(self)
    }
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
    /// Built once per revision, and built *by* the session, so a plugin's
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
    /// Refused for a name the session already owns: a plugin that claimed `session` or `messages`
    /// would be writing the agent's own state, and the manifest is what says which names are taken.
    /// This is half of the rule and not the whole of it — the other half is the write, held to these
    /// declarations by the wrapper `registry` puts on every handler, so a patch into a root nobody
    /// declared is refused whether or not anybody declared anything.
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
    /// Every handler is wrapped, and the wrapper is the composition's whole rule about state: a patch
    /// into a root this contribution declared is *recorded*, and a patch into anything else fails the
    /// transaction. Declaring what it may write is how a composition says what its state is, and the
    /// write is where that is enforced — refusing the *declaration* of a name the session owns
    /// (`with_root`) is only half of it, and a handler reaching past its roots is exactly the half a
    /// declaration cannot catch.
    ///
    /// Recording is the session's job rather than the host's, because the log and its vocabulary are
    /// the session's and "what is a fact" is the middle layer's decision everywhere else here too.
    /// It is an *effect* rather than a write because effects run after the commit: a transaction that
    /// faults leaves nothing in the log, so a fault takes its recording with it.
    pub(crate) fn registry(&self, shipped: Registry) -> Registry {
        let mut registry = shipped;
        let roots: Vec<String> = self.roots.iter().map(|(name, _)| name.clone()).collect();
        for (kind, priority, handler) in &self.handlers {
            // Wrapped even when it declared no roots: "declares nothing" and "may write anything" are
            // not the same statement, and the composition that declared nothing may write nothing.
            let handler: std::sync::Arc<dyn Handler> =
                std::sync::Arc::new(Confined { inner: handler.clone(), roots: roots.clone() });
            registry = registry.on(kind.clone(), *priority, handler);
        }
        for (name, subscription) in &self.subscriptions {
            registry = registry.subscription(name.clone(), subscription.clone());
        }
        registry = registry.finalize(stage);
        {
            registry = registry.on_fn("kernel/log.failed", -100, "composition.journal.failed", |tx, event| {
                if event.get("kind").and_then(Value::as_str) != Some(PATCH_KIND)
                    || event.get("conversation").and_then(Value::as_str) != Some(tx.text("session.conversation").as_str())
                    || event.get("data") != tx.get("session.plugin_write") { return Ok(()); }
                tx.set("session.plugin_write", Value::Null)?;
                crate::command_operations::settle(tx, event.get("data").unwrap(), "failed")?;
                let message = event.get("message").and_then(Value::as_str).unwrap_or("Plugin state could not be recorded");
                tx.fx(Effect::new("wire.event").with("event", crate::wire::render(&crate::SessionEvent::Notice {
                    level: crate::Level::Error, text: message.into(),
                })));
                Ok(())
            });
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
        let defaults = Value::Map(Arc::new(self.roots.iter().cloned().collect()));
        state = misa_value::apply_one(&state, &Path::parse("session.plugin_defaults").unwrap(), &Op::Set(defaults))
            .expect("plugin defaults belong to session metadata");
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
