//! The re-frame loop: events in, facts and effects out.
//!
//! The previous system's loop is kept, because it was right, and the parts that
//! made it hard to live in are dropped.
//!
//! # Kept
//!
//! - **Events are the only way to ask for anything.** One dispatch is one
//!   transaction: handlers see one consistent database, their patches land
//!   together or not at all, and the effects they asked for run after the commit.
//! - **Coeffects are the only inputs.** Anything nondeterministic — the clock,
//!   configuration — enters here, so a handler is a function of its event, the
//!   database, and its coeffects. That is what makes a session reproducible.
//! - **Subscriptions are declared-input queries**, in a bounded scope that
//!   recomputes only what a changed input forces. Nothing traverses another
//!   owner's state and nothing derives a fact from presentation.
//! - **State is immutable and shared.** A patch rebuilds the path it touches;
//!   everything else keeps its allocation. That is what makes the scope's
//!   comparison cheap and the sharing real.
//! - **Effects are data**, validated before the commit and executed after. A
//!   handler that wants an answer emits an effect and handles its completion
//!   event, so no handler does IO.
//! - **A fault is contained.** A handler that fails, a patch that does not
//!   apply, or an effect the interpreter will not accept rolls the whole
//!   transaction back and is reported. The loop keeps its previous state and
//!   keeps running.
//!
//! # Dropped, deliberately
//!
//! - **No handler-side queries.** A handler reads the database it was given.
//!   Consumers query. The previous system let a handler call `misa.sub` against a
//!   speculative fork, which made a handler's result depend on another handler's
//!   ordering. A handler that needs a derived value can name a narrow state root
//!   instead, and the derived value stays where it is cheap: at the boundary
//!   where a consumer asked for it.
//! - **No global interceptor chain.** Handlers are declared for an event kind and
//!   run in priority then id order. That is the whole ordering story.
//! - **No whole-database return.** Only patches are accepted, and a root patch
//!   may only merge, so a handler cannot discard state it did not know about.
//!
//! # Openness, where it is needed and nowhere else
//!
//! Handler ids, event kinds, effect kinds, and query ids are strings, because
//! policy names them and a plugin must be able to add a name without a change
//! here. The loop's own surface is typed, and a fault is a value, not a panic.

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::Mutex;

pub use misa_proto::Query;
use misa_value::{Op, Path, Value, apply_one};

pub mod scope;

pub use scope::Scope;

/// A fact that asks for something to happen.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    pub kind: String,
    pub data: Value,
}

impl Event {
    pub fn new(kind: impl Into<String>) -> Self {
        Event { kind: kind.into(), data: Value::Null }
    }

    pub fn with(mut self, key: &'static str, value: impl Into<Value>) -> Self {
        self.data = merge_field(self.data, key, value.into());
        self
    }

    pub fn with_value(mut self, data: Value) -> Self {
        self.data = data;
        self
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }

    pub fn field(&self, key: &str) -> &str {
        self.get(key).and_then(Value::as_str).unwrap_or("")
    }
}

fn merge_field(base: Value, key: &'static str, value: Value) -> Value {
    let mut map = match base {
        Value::Map(existing) => (*existing).clone(),
        Value::Null => BTreeMap::new(),
        other => {
            let mut map = BTreeMap::new();
            map.insert("value".to_string(), other);
            map
        }
    };
    map.insert(key.to_string(), value);
    Value::Map(Arc::new(map))
}

/// Something a handler asked for. Executed after the transaction commits.
///
/// The kind is open because policy names its own effects, and closed in effect
/// because the interpreter validates every one of them before anything commits.
#[derive(Clone, Debug, PartialEq)]
pub struct Effect {
    pub kind: String,
    pub data: Value,
}

impl Effect {
    pub fn new(kind: impl Into<String>) -> Self {
        Effect { kind: kind.into(), data: Value::Null }
    }

    pub fn with(mut self, key: &'static str, value: impl Into<Value>) -> Self {
        self.data = merge_field(self.data, key, value.into());
        self
    }

    pub fn with_value(mut self, data: Value) -> Self {
        self.data = data;
        self
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }

    pub fn field(&self, key: &str) -> &str {
        self.get(key).and_then(Value::as_str).unwrap_or("")
    }
}

/// A contained failure. Never a panic, never a partially applied transaction.
#[derive(Clone, Debug, PartialEq)]
pub struct Fault {
    pub code: String,
    pub message: String,
    /// The event that was being handled, when there was one.
    pub event: Option<String>,
    /// The handler or effect that raised it, when known.
    pub source: Option<String>,
    /// Whatever the client needs to recover.
    ///
    /// A fault that only said "missing argument" would make every client re-derive
    /// which picker to open. `argument.required` carries the source, so the error
    /// path narrows to the same place the declaration pointed at.
    pub data: Value,
}

impl Fault {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Fault { code: code.into(), message: message.into(), event: None, source: None, data: Value::Null }
    }

    pub fn in_event(mut self, event: impl Into<String>) -> Self {
        self.event = Some(event.into());
        self
    }

    pub fn from_source(mut self, source: impl Into<String>) -> Self {
        self.source = Some(source.into());
        self
    }

    /// Attach what a client needs to recover from this.
    pub fn with_data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }

    /// A command was sent without an argument it cannot run without.
    ///
    /// The one fault that is a *next step* rather than a problem: it names the
    /// source, so a client opens its picker without asking anything.
    pub fn argument(command: &str, arg: &str, source: Option<&str>) -> Self {
        let mut data = std::collections::BTreeMap::new();
        data.insert("command".to_string(), Value::str(command));
        data.insert("argument".to_string(), Value::str(arg));
        if let Some(source) = source {
            data.insert("source".to_string(), Value::str(source));
        }
        Fault::new("argument.required", format!("`/{command}` needs an argument for `{arg}`"))
            .with_data(Value::Map(Arc::new(data)))
    }

    /// A handler refused the event, or failed while handling it.
    pub fn handler(message: impl Into<String>) -> Self {
        Fault::new("handler", message)
    }

    /// The interpreter will not accept an effect, so nothing committed.
    pub fn effect(message: impl Into<String>) -> Self {
        Fault::new("effect", message)
    }

    /// The patch a handler produced was not valid for the database.
    pub fn patch(message: impl Into<String>) -> Self {
        Fault::new("patch", message)
    }

    /// A query could not be answered.
    pub fn query(message: impl Into<String>) -> Self {
        Fault::new("query", message)
    }

    /// A dispatch chain did not settle.
    pub fn chain(message: impl Into<String>) -> Self {
        Fault::new("chain", message)
    }
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Fault {}

/// What a handler may read.
#[derive(Clone, Debug, Default)]
pub struct Coeffects {
    /// Wall-clock milliseconds. The only clock a handler may read.
    pub clock_ms: i64,
    /// Installed configuration, immutable for the session's life.
    pub config: Value,
}

/// The interface a handler writes through: read, patch, ask.
///
/// There is deliberately no way to obtain a mutable database, so the previous
/// system's rule that "returning a whole db is rejected" is not a rule here but a
/// property of the type.
pub struct Tx<'a> {
    db: &'a Value,
    cofx: &'a Coeffects,
    patches: Vec<(Path, Op)>,
    deferred: Vec<usize>,
    effects: Vec<Effect>,
    dispatches: Vec<Event>,
}

impl<'a> Tx<'a> {
    pub fn db(&self) -> &Value {
        self.db
    }

    pub fn cofx(&self) -> &Coeffects {
        self.cofx
    }

    /// The value at a path.
    pub fn get(&self, path: &str) -> Option<&Value> {
        self.db.get_path(&Path::parse(path).ok()?)
    }

    pub fn int(&self, path: &str) -> i64 {
        self.get(path).and_then(Value::as_i64).unwrap_or(0)
    }

    /// The text at a path, owned.
    ///
    /// Owned rather than borrowed because a handler that reads a string usually
    /// then patches something, and a borrow that outlives the read would make every
    /// such handler fight the borrow checker. Reading is rare enough that the
    /// allocation is not worth avoiding.
    pub fn text(&self, path: &str) -> String {
        self.get(path).and_then(Value::as_str).unwrap_or_default().to_string()
    }

    pub fn flag(&self, path: &str) -> bool {
        self.get(path).and_then(Value::as_bool).unwrap_or(false)
    }

    /// What has been queued in this transaction so far.
    ///
    /// For a handler that wants to *record* what it wrote — a journal, and nothing here does
    /// anything else with it. It is not a way to read a result: nothing is applied until the
    /// transaction commits, and a transaction that faults leaves no trace but this list.
    pub fn patches(&self) -> &[(Path, Op)] {
        &self.patches
    }

    /// Keep these writes visible to subsequent handlers, but require owner
    /// finalization to decide their publication at the end of dispatch.
    pub fn defer_patches_from(&mut self, index: usize) {
        self.deferred.extend(index..self.patches.len());
    }

    /// Queue a patch. Order is preserved, and a later patch sees an earlier one.
    pub fn patch(&mut self, path: &str, op: Op) -> Result<(), Fault> {
        let path = Path::parse(path).map_err(|err| Fault::patch(err.to_string()))?;
        self.patches.push((path, op));
        Ok(())
    }

    /// Replace the value at a path.
    pub fn set(&mut self, path: &str, value: impl Into<Value>) -> Result<(), Fault> {
        self.patch(path, Op::Set(value.into()))
    }

    /// Remove the value at a path.
    pub fn delete(&mut self, path: &str) -> Result<(), Fault> {
        self.patch(path, Op::Delete)
    }

    /// Grow the list at a path. Appending is the one op that is not idempotent,
    /// so it is spelled out rather than being what a list write does by default.
    pub fn push(&mut self, path: &str, value: impl Into<Value>) -> Result<(), Fault> {
        self.patch(path, Op::Append(value.into()))
    }

    /// Ask for an effect. Executed after the commit, never before.
    pub fn fx(&mut self, effect: Effect) {
        self.effects.push(effect);
    }

    /// Ask for another event. Handled after this transaction commits, in order.
    pub fn dispatch(&mut self, event: Event) {
        self.dispatches.push(event);
    }
}

/// A registered reaction to one event kind.
pub trait Handler: Send + Sync {
    /// The id this handler was registered under, for diagnostics.
    fn id(&self) -> &str;

    fn handle(&self, tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault>;
}

/// A handler behind an Arc is a handler.
///
/// A composition builds handlers somewhere else and hands them over: a plugin's handlers are
/// shared with the plugin that owns the instance, so a list of them is a list of
/// Arc&lt;dyn Handler&gt; before it is a Registry. Without this, registering one would mean
/// unwrapping it and giving up the sharing.
impl<H: Handler + ?Sized> Handler for Arc<H> {
    fn id(&self) -> &str {
        (**self).id()
    }

    fn handle(&self, tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
        (**self).handle(tx, event)
    }
}

/// A closure handler. Most handlers are small enough to write inline.
pub struct FnHandler<F> {
    id: String,
    body: F,
}

impl<F> FnHandler<F> {
    pub fn new(id: impl Into<String>, body: F) -> Self {
        FnHandler { id: id.into(), body }
    }
}

impl<F> Handler for FnHandler<F>
where
    F: Fn(&mut Tx<'_>, &Event) -> Result<(), Fault> + Send + Sync,
{
    fn id(&self) -> &str {
        &self.id
    }

    fn handle(&self, tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
        (self.body)(tx, event)
    }
}

/// A derived query can access only its declared inputs. The previous result is
/// an optional immutable optimization hint, never a correctness dependency.
pub type Compute = Arc<dyn Fn(&[Value], &Query, Option<&Value>) -> Result<Value, Fault> + Send + Sync>;
pub type Read = Arc<dyn Fn(&Value, &Query, Option<&Value>) -> Result<Value, Fault> + Send + Sync>;

/// A pure query with declared inputs.
#[derive(Clone)]
pub enum Subscription {
    Read { read: Read },
    Derived { inputs: Inputs, compute: Compute },
}

/// A subscription that reads the database directly.
///
/// The honest declaration when a query is over a whole root: naming every path it
/// touches would be a lie, so it names the database, and the scope invalidates it
/// on the database's identity rather than on a list of paths.
pub fn read_query(compute: impl Fn(&Value, &Query) -> Value + Send + Sync + 'static) -> Subscription {
    try_read_query(move |db, query, _previous| Ok(compute(db, query)))
}

pub fn try_read_query(compute: impl Fn(&Value, &Query, Option<&Value>) -> Result<Value, Fault> + Send + Sync + 'static) -> Subscription {
    Subscription::Read { read: Arc::new(compute) }
}

/// A pure projection of named query results, independent of any consumer.
pub fn derived_query(
    inputs: impl IntoIterator<Item = Query>,
    compute: impl Fn(&[Value]) -> Value + Send + Sync + 'static,
) -> Subscription {
    Subscription::Derived {
        inputs: Inputs::Fixed(inputs.into_iter().collect()),
        compute: Arc::new(move |inputs, _, _| Ok(compute(inputs))),
    }
}

pub fn try_derived_query(inputs: Inputs, compute: impl Fn(&[Value], &Query, Option<&Value>) -> Result<Value, Fault> + Send + Sync + 'static) -> Subscription {
    Subscription::Derived { inputs, compute: Arc::new(compute) }
}

/// What a query depends on.
#[derive(Clone)]
pub enum Inputs {
    /// A fixed list of dependency queries; their values may change. An empty
    /// list describes a constant computation. Database reads use Subscription::Read.
    Fixed(Vec<Query>),
    /// Dependencies that depend on the query's own arguments.
    Dynamic(Arc<dyn Fn(&Query) -> Result<Vec<Query>, Fault> + Send + Sync>),
}

/// Everything a loop can be asked to do, installed once.
///
/// Installation is closed: nothing at runtime adds a handler or a subscription.
/// The previous system kept that rule and it is worth keeping — it is what makes
/// the whole loop inspectable before it runs.
#[derive(Default)]
pub struct Registry {
    handlers: BTreeMap<String, Vec<(i32, Arc<dyn Handler>)>>,
    subscriptions: BTreeMap<String, Arc<Subscription>>,
    installation_faults: Vec<Fault>,
    finalizers: Vec<Arc<dyn Fn(&Value, &mut Outcome) -> Result<bool, Fault> + Send + Sync>>,
}

impl Registry {
    /// Owner policy applied after the complete handler chain, before publication
    /// or effects. Any failure rolls back the entire dispatch.
    /// Return true after changing patches or effects so the loop rebuilds the
    /// committed changes and validates the final effects before publication.
    pub fn finalize(mut self, finalize: impl Fn(&Value, &mut Outcome) -> Result<bool, Fault> + Send + Sync + 'static) -> Self {
        self.finalizers.push(Arc::new(finalize));
        self
    }
    pub fn new() -> Self {
        Registry::default()
    }

    /// Register a handler for one event kind.
    ///
    /// Order is ascending priority, then registration id, so it does not depend on
    /// the order these calls happen to be written in.
    pub fn on(mut self, event_kind: impl Into<String>, priority: i32, handler: impl Handler + 'static) -> Self {
        let kind = event_kind.into();
        let entry = self.handlers.entry(kind).or_default();
        entry.push((priority, Arc::new(handler)));
        entry.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.id().cmp(right.1.id())));
        self
    }

    /// Register a closure handler.
    pub fn on_fn<F>(self, event_kind: impl Into<String>, priority: i32, id: impl Into<String>, body: F) -> Self
    where
        F: Fn(&mut Tx<'_>, &Event) -> Result<(), Fault> + Send + Sync + 'static,
    {
        self.on(event_kind, priority, FnHandler::new(id, body))
    }

    pub fn subscription(mut self, id: impl Into<String>, subscription: Subscription) -> Self {
        let id = id.into();
        if id.is_empty() || self.subscriptions.contains_key(&id) {
            self.installation_faults.push(Fault::query(format!("invalid or duplicate subscription `{id}`")));
        } else {
            self.subscriptions.insert(id, Arc::new(subscription));
        }
        self
    }

    /// Validate the closed composition before its owner starts. Dynamic dependencies
    /// are checked when their query arguments are available.
    pub fn validate(&self) -> Result<(), Fault> {
        if let Some(fault) = self.installation_faults.first() { return Err(fault.clone()); }
        fn visit(registry: &Registry, id: &str, active: &mut Vec<String>, done: &mut std::collections::BTreeSet<String>) -> Result<(), Fault> {
            if active.iter().any(|entry| entry == id) { return Err(Fault::query(format!("subscription dependency cycle at `{id}`"))); }
            if done.contains(id) { return Ok(()); }
            let definition = registry.definition(id).ok_or_else(|| Fault::query(format!("no subscription named `{id}`")))?;
            if active.len() >= scope::DEFAULT_DEPTH { return Err(Fault::query("subscription dependencies are too deep")); }
            active.push(id.into());
            if let Subscription::Derived { inputs: Inputs::Fixed(inputs), .. } = definition.as_ref() {
                for query in inputs { visit(registry, &query.id, active, done)?; }
            }
            active.pop();
            done.insert(id.into());
            Ok(())
        }
        let mut done = std::collections::BTreeSet::new();
        for id in self.subscriptions.keys() { visit(self, id, &mut Vec::new(), &mut done)?; }
        Ok(())
    }

    pub fn handlers_for(&self, kind: &str) -> &[(i32, Arc<dyn Handler>)] {
        self.handlers.get(kind).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The installed definition of one query, if there is one.
    pub fn definition(&self, id: &str) -> Option<&Arc<Subscription>> {
        self.subscriptions.get(id)
    }

    /// Every event kind something reacts to.
    pub fn event_kinds(&self) -> impl Iterator<Item = &str> {
        self.handlers.keys().map(String::as_str)
    }

    /// Every query this loop can answer.
    pub fn query_ids(&self) -> impl Iterator<Item = &str> {
        self.subscriptions.keys().map(String::as_str)
    }
}

/// Decides whether an effect can run.
///
/// Consulted inside the transaction, before anything commits, which is what makes
/// "a transaction that asked for something impossible leaves no trace" true. The
/// session implements this: it owns the kernel, the wire, and the clock.
pub trait Interpreter: Send + Sync {
    fn accepts(&self, effect: &Effect) -> Result<(), String>;
}

/// Accepts everything. For a loop with no effects of its own and for tests.
pub struct AcceptsEverything;

impl Interpreter for AcceptsEverything {
    fn accepts(&self, _effect: &Effect) -> Result<(), String> {
        Ok(())
    }
}

/// A committed database transaction, preserved for incremental consumers.
#[derive(Clone, Debug)]
pub struct Change {
    pub before: Value,
    pub after: Value,
    pub patches: Vec<(Path, Op)>,
}

/// What one dispatch did.
#[derive(Clone, Debug, Default)]
pub struct Outcome {
    /// (change, patch) positions awaiting owner finalization.
    pub deferred: Vec<(usize, usize)>,
    /// Successful event transactions in this atomic dispatch, in application order.
    pub changes: Vec<Change>,
    /// Effects to execute, in the order their handlers asked for them.
    pub effects: Vec<Effect>,
    /// Events that were handled as part of this dispatch, in order.
    pub handled: Vec<String>,
    /// Contained failures. A non-empty list means nothing committed.
    pub faults: Vec<Fault>,
    /// The revision after this dispatch, or the previous one if nothing committed.
    pub rev: u64,
}

impl Outcome {
    pub fn committed(&self) -> bool {
        self.faults.is_empty()
    }

    /// The faults as wire faults, for a client to read.
    pub fn as_faults(&self) -> Vec<misa_proto::Fault> {
        self.faults
            .iter()
            .map(|fault| {
                let mut wire = misa_proto::Fault::new(&fault.code, fault.message.clone());
                // Whatever the fault carried, plus the attribution the loop knows.
                // Attribution is named `handler`, not `source`: `source` in a fault's
                // data is a completion source, which is a different thing and the one
                // a client acts on.
                let mut data = match &fault.data {
                    Value::Map(map) => (**map).clone(),
                    _ => BTreeMap::new(),
                };
                if let Some(event) = &fault.event {
                    data.insert("event".to_string(), Value::str(event));
                }
                if let Some(handler) = &fault.source {
                    data.insert("handler".to_string(), Value::str(handler));
                }
                if !data.is_empty() {
                    wire = wire.with(Value::Map(Arc::new(data)));
                }
                wire
            })
            .collect()
    }
}

/// The largest resolved dispatch chain. A self-dispatching handler is a bug, and
/// this bound turns it into a fault instead of a hang.
pub const DEFAULT_CHAIN_LIMIT: usize = 1024;

/// One instance of the loop.
pub struct Loop {
    db: Value,
    registry: Arc<Registry>,
    interpreter: Arc<dyn Interpreter>,
    scope: Scope,
    cofx: Coeffects,
    rev: u64,
    chain_limit: usize,
    /// Queries a consumer asked for, re-evaluated after every commit.
    live: Mutex<Vec<Query>>,
}

impl Loop {
    pub fn new(registry: Arc<Registry>, interpreter: Arc<dyn Interpreter>, db: Value) -> Self {
        registry.validate().expect("invalid owner query composition");
        Loop {
            db,
            registry,
            interpreter,
            scope: Scope::new(),
            cofx: Coeffects::default(),
            rev: 0,
            chain_limit: DEFAULT_CHAIN_LIMIT,
            live: Mutex::new(Vec::new()),
        }
    }

    pub fn db(&self) -> &Value {
        &self.db
    }

    pub fn rev(&self) -> u64 {
        self.rev
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn scope(&self) -> &Scope {
        &self.scope
    }

    pub fn set_clock(&mut self, clock_ms: i64) {
        self.cofx.clock_ms = clock_ms;
    }

    pub fn set_config(&mut self, config: Value) {
        self.cofx.config = config;
    }

    pub fn clock(&self) -> i64 {
        self.cofx.clock_ms
    }

    pub fn with_chain_limit(mut self, limit: usize) -> Self {
        self.chain_limit = limit.max(1);
        self
    }

    /// Register a query a consumer is watching.
    pub fn watch(&mut self, query: Query) {
        let mut live = self.live.lock().expect("live query set is never poisoned");
        if !live.contains(&query) {
            live.push(query);
        }
    }

    pub fn unwatch(&mut self, query: &Query) {
        let mut live = self.live.lock().expect("live query set is never poisoned");
        live.retain(|entry| entry != query);
        self.scope.forget(query);
    }

    /// Re-evaluate every watched query, returning the ones whose value changed.
    ///
    /// This is what a transport sends: everything that moved, and nothing else.
    pub fn refresh(&mut self) -> Vec<(Query, Value)> {
        let live: Vec<Query> = self.live.lock().expect("live query set is never poisoned").clone();
        let mut changed = Vec::new();
        for query in live {
            // A query that cannot be answered now may be answerable later, so it
            // stays watched and the fault is simply not a value.
            if let Ok(Some(value)) = self.scope.evaluate(&self.db, &self.registry, &query) {
                changed.push((query, value));
            }
        }
        changed
    }

    /// The value of a query, computing it if the scope does not hold it.
    pub fn query(&mut self, query: &Query) -> Result<Value, Fault> {
        self.scope.evaluate(&self.db, &self.registry, query)?;
        self.scope
            .current(query)
            .ok_or_else(|| Fault::query(format!("`{}` produced no value", query.id)))
    }

    /// Dispatch one event.
    ///
    /// One transaction per event in the chain: every handler for the kind runs,
    /// patches land together, effects are validated, and only then does anything
    /// commit. A fault rolls back everything that event did and clears the rest of
    /// the chain, because later events would see a database an earlier handler
    /// meant to change.
    pub fn dispatch(&mut self, event: Event) -> Outcome {
        self.dispatch_checked(event, |_| Ok(()))
    }

    /// Reserve owner resources against the final validated transaction before
    /// publishing its state. Refusal rolls back the complete dispatch chain.
    pub fn dispatch_checked(&mut self, event: Event, admit: impl FnOnce(&Outcome) -> Result<(), Fault>) -> Outcome {
        let before = self.db.clone();
        let mut outcome = Outcome { rev: self.rev, ..Outcome::default() };
        let mut queue = VecDeque::new();
        queue.push_back(event);
        let mut chain = 0usize;

        while let Some(event) = queue.pop_front() {
            if chain >= self.chain_limit {
                outcome.faults.push(
                    Fault::chain(format!(
                        "a dispatch chain of {} events did not settle",
                        self.chain_limit
                    ))
                    .in_event(event.kind.clone()),
                );
                break;
            }
            chain += 1;
            outcome.handled.push(event.kind.clone());
            match self.dispatch_one(&event) {
                Ok((effects, next, change, deferred)) => {
                    outcome.deferred.extend(deferred.into_iter().map(|patch| (outcome.changes.len(), patch)));
                    if let Some(change) = change { outcome.changes.push(change); }
                    outcome.effects.extend(effects);
                    queue.extend(next);
                }
                Err(faults) => {
                    outcome.faults.extend(faults);
                    break;
                }
            }
        }

        if outcome.committed() && !self.registry.finalizers.is_empty() {
            let mut finalized = false;
            for finalize in &self.registry.finalizers {
                match finalize(&before, &mut outcome) {
                    Ok(changed) => finalized |= changed,
                    Err(fault) => { outcome.faults.push(fault); break; }
                }
            }
            if outcome.committed() && finalized {
                let mut working = before.clone();
                for change in &mut outcome.changes {
                    change.before = working.clone();
                    match misa_value::apply(&working, &change.patches) {
                        Ok(value) => { working = value; change.after = working.clone(); }
                        Err(error) => { outcome.faults.push(Fault::patch(error.to_string())); break; }
                    }
                }
                if outcome.committed() {
                    for effect in &outcome.effects {
                        if let Err(error) = self.interpreter.accepts(effect) { outcome.faults.push(Fault::effect(error)); break; }
                    }
                }
                if outcome.committed() { self.db = working; }
            }
        }
        if outcome.committed() && !outcome.deferred.is_empty() {
            outcome.faults.push(Fault::new("transaction.unfinalized", "Owner did not finalize deferred writes"));
        }
        if outcome.committed() {
            if let Err(fault)=admit(&outcome) {outcome.faults.push(fault);}
        }
        if outcome.committed() {
            self.rev += 1;
            outcome.rev = self.rev;
        } else {
            self.db = before;
            outcome.effects.clear();
            outcome.changes.clear();
            outcome.deferred.clear();
        }
        outcome
    }

    /// One event, one transaction, one commit.
    fn dispatch_one(&mut self, event: &Event) -> Result<(Vec<Effect>, Vec<Event>, Option<Change>, Vec<usize>), Vec<Fault>> {
        let handlers = self.registry.handlers_for(&event.kind).to_vec();
        if handlers.is_empty() {
            // An event nothing reacts to is not an error: a policy observes what
            // it cares about, and a session is a composition of several policies.
            return Ok((Vec::new(), Vec::new(), None, Vec::new()));
        }

        let mut working = self.db.clone();
        let mut committed_patches = Vec::new();
        let mut deferred = Vec::new();
        let mut effects = Vec::new();
        let mut dispatches = Vec::new();
        let mut faults = Vec::new();

        for (_, handler) in handlers {
            let mut tx = Tx {
                db: &working,
                cofx: &self.cofx,
                patches: Vec::new(),
                deferred: Vec::new(),
                effects: Vec::new(),
                dispatches: Vec::new(),
            };
            if let Err(mut fault) = handler.handle(&mut tx, event) {
                fault.event.get_or_insert_with(|| event.kind.clone());
                fault.source.get_or_insert_with(|| handler.id().to_string());
                faults.push(fault);
                break;
            }

            let (patches, asked, next, held) = tx.into_parts();

            let mut applied = working.clone();
            let mut failed = None;
            for (path, op) in &patches {
                match apply_one(&applied, path, op) {
                    Ok(value) => applied = value,
                    Err(err) => {
                        failed = Some(
                            Fault::patch(err.to_string())
                                .in_event(event.kind.clone())
                                .from_source(handler.id()),
                        );
                        break;
                    }
                }
            }
            if let Some(fault) = failed {
                faults.push(fault);
                break;
            }

            // Nothing commits unless the interpreter will accept everything this
            // handler asked for.
            let mut refused = None;
            for effect in &asked {
                if let Err(reason) = self.interpreter.accepts(effect) {
                    refused = Some(
                        Fault::effect(reason)
                            .in_event(event.kind.clone())
                            .from_source(handler.id()),
                    );
                    break;
                }
            }
            if let Some(fault) = refused {
                faults.push(fault);
                break;
            }

            working = applied;
            deferred.extend(held.into_iter().map(|index| committed_patches.len() + index));
            committed_patches.extend(patches);
            effects.extend(asked);
            dispatches.extend(next);
        }

        if !faults.is_empty() {
            return Err(faults);
        }

        let change = (!committed_patches.is_empty()).then(|| Change {
            before: self.db.clone(), after: working.clone(), patches: committed_patches,
        });
        self.db = working;
        Ok((effects, dispatches, change, deferred))
    }
}

impl<'a> Tx<'a> {
    fn into_parts(self) -> (Vec<(Path, Op)>, Vec<Effect>, Vec<Event>, Vec<usize>) {
        (self.patches, self.effects, self.dispatches, self.deferred)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn base() -> Value {
        Value::map([
            ("count", Value::Int(0)),
            ("log", Value::list([])),
            ("ui", Value::map([("verbose", Value::Bool(false))])),
        ])
    }

    /// Only `announce` may run. Everything else must be refused before the commit.
    struct OnlyAnnounce;

    #[test]
    fn admission_refusal_rolls_back_prepared_state_and_effects() {
        let mut owner = Loop::new(registry(), Arc::new(OnlyAnnounce), base());
        let refused = owner.dispatch_checked(Event::new("tick"), |prepared| {
            assert_eq!(prepared.effects.len(), 1);
            assert!(!prepared.changes.is_empty());
            Err(Fault::new("capacity", "No reserved execution slot"))
        });
        assert_eq!(refused.faults[0].code, "capacity");
        assert_eq!(owner.db(), &base());
        assert_eq!(owner.rev(), 0);
        assert!(refused.effects.is_empty() && refused.changes.is_empty());
        assert!(owner.dispatch(Event::new("tick")).committed());
        assert_eq!(owner.db().get("count").and_then(Value::as_i64), Some(1));
    }

    #[test]
    fn deferred_writes_require_finalization_and_rollback_effects() {
        let registry = Registry::new().on_fn("write", 0, "staged", |tx, _| {
            tx.set("count", Value::Int(1))?;
            tx.defer_patches_from(0);
            tx.fx(Effect::new("announce"));
            Ok(())
        });
        let mut owner = Loop::new(Arc::new(registry), Arc::new(OnlyAnnounce), base());
        let result = owner.dispatch(Event::new("write"));
        assert!(!result.committed());
        assert_eq!(result.faults[0].code, "transaction.unfinalized");
        assert_eq!(owner.db(), &base());
        assert!(result.effects.is_empty() && result.changes.is_empty());
    }

    impl Interpreter for OnlyAnnounce {
        fn accepts(&self, effect: &Effect) -> Result<(), String> {
            match effect.kind.as_str() {
                "announce" => Ok(()),
                other => Err(format!("unknown effect `{other}`")),
            }
        }
    }

    fn registry() -> Arc<Registry> {
        Arc::new(
            Registry::new()
                .on_fn("tick", 0, "tick", |tx, _| {
                    let count = tx.int("count");
                    tx.set("count", Value::Int(count + 1))?;
                    tx.push("log", Value::str("tick"))?;
                    tx.fx(Effect::new("announce").with("what", Value::str("tick")));
                    Ok(())
                })
                .on_fn("boom", 0, "boom", |_tx, _| Err(Fault::handler("no")))
                .on_fn("boom", 10, "after-boom", |tx, _| tx.set("count", Value::Int(99)))
                .on_fn("emit-impossible", 0, "impossible", |tx, _| {
                    tx.fx(Effect::new("not.a.real.effect"));
                    tx.set("count", Value::Int(7))
                })
                .on_fn("recursive", 0, "recursive", |tx, _| {
                    tx.dispatch(Event::new("recursive"));
                    Ok(())
                })
                .on_fn("touch.ui", 0, "touch", |tx, _| tx.set("ui.verbose", Value::Bool(true)))
                .on_fn("bump", 0, "bump", |tx, _| {
                    let count = tx.int("count");
                    tx.set("count", Value::Int(count + 1))
                })
                .on_fn("root", 0, "root", |tx, _| tx.patch("", Op::Set(Value::map([]))))
                .subscription(
                    "log",
                    read_query(|db, _query| db.get("log").cloned().unwrap_or(Value::Null)),
                )
                .subscription(
                    "log.length",
                    Subscription::Derived {
                        inputs: Inputs::Fixed(vec![Query::new("log")]),
                        compute: Arc::new(|inputs, _query, _previous| Ok({
                            Value::Int(inputs[0].as_list().map(<[Value]>::len).unwrap_or(0) as i64)
                        })),
                    },
                ),
        )
    }

    fn test_loop() -> Loop {
        Loop::new(registry(), Arc::new(OnlyAnnounce), base())
    }

    #[test]
    fn a_handler_patches_and_asks_for_effects() {
        let mut loop_ = test_loop();
        let outcome = loop_.dispatch(Event::new("tick"));
        assert!(outcome.committed(), "{:?}", outcome.faults);
        assert_eq!(outcome.effects.len(), 1);
        assert_eq!(outcome.effects[0].kind, "announce");
        assert_eq!(loop_.db().get("count").and_then(Value::as_i64), Some(1));
        assert_eq!(loop_.db().get("log").and_then(Value::as_list).map(<[Value]>::len), Some(1));
        assert_eq!(outcome.rev, 1);
    }

    #[test]
    fn a_handler_behind_an_arc_is_a_handler() {
        // A composition builds its handlers somewhere else and hands them over — a plugin's
        // handlers are shared with the plugin that owns the instance — so \`Arc<dyn Handler>\`
        // has to be registrable as it is, and its id has to be the id a diagnostic names.
        let shared: Arc<dyn Handler> = Arc::new(FnHandler::new("shared", |tx: &mut Tx<'_>, _: &Event| {
            tx.push("log", Value::str("shared"))?;
            Ok(())
        }));
        let registry = Registry::new().on("tick", 0, shared);
        let mut loop_ = Loop::new(Arc::new(registry), Arc::new(OnlyAnnounce), base());
        let outcome = loop_.dispatch(Event::new("tick"));
        assert!(outcome.committed(), "{:?}", outcome.faults);
        assert_eq!(loop_.db().get("log").and_then(Value::as_list).map(<[Value]>::len), Some(1));
        assert!(
            outcome.handled.iter().any(|handled| handled == "tick"),
            "{:?}",
            outcome.handled
        );
    }

    #[test]
    fn a_handler_that_faults_takes_its_whole_transaction_with_it() {
        let mut loop_ = test_loop();
        loop_.dispatch(Event::new("tick"));
        let before = loop_.db().clone();
        let outcome = loop_.dispatch(Event::new("boom"));
        assert!(!outcome.committed());
        assert_eq!(outcome.faults[0].code, "handler");
        assert_eq!(outcome.faults[0].event.as_deref(), Some("boom"));
        assert_eq!(outcome.faults[0].source.as_deref(), Some("boom"));
        assert!(before.same(loop_.db()), "a faulted transaction changed the database");
        assert_eq!(outcome.rev, 1, "a faulted dispatch advanced the revision");
    }

    #[test]
    fn a_lower_priority_handler_does_not_run_after_one_faults() {
        let mut loop_ = test_loop();
        loop_.dispatch(Event::new("boom"));
        assert_eq!(loop_.db().get("count").and_then(Value::as_i64), Some(0));
    }

    #[test]
    fn an_effect_the_interpreter_refuses_stops_the_commit() {
        let mut loop_ = test_loop();
        let before = loop_.db().clone();
        let outcome = loop_.dispatch(Event::new("emit-impossible"));
        assert!(!outcome.committed());
        assert_eq!(outcome.faults[0].code, "effect");
        assert!(outcome.faults[0].message.contains("not.a.real.effect"));
        assert!(before.same(loop_.db()), "a refused effect still committed state");
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn a_dispatch_chain_is_bounded_and_reports_instead_of_hanging() {
        let mut loop_ = test_loop().with_chain_limit(8);
        let outcome = loop_.dispatch(Event::new("recursive"));
        assert!(!outcome.committed());
        assert_eq!(outcome.faults[0].code, "chain");
        assert_eq!(outcome.handled.len(), 8);
    }

    #[test]
    fn a_chained_event_is_handled_in_the_same_dispatch() {
        let registry = Arc::new(
            Registry::new()
                .on_fn("first", 0, "first", |tx, _| {
                    tx.dispatch(Event::new("second"));
                    tx.set("count", Value::Int(1))
                })
                .on_fn("second", 0, "second", |tx, _| {
                    let count = tx.int("count");
                    tx.set("count", Value::Int(count + 10))
                }),
        );
        let mut loop_ = Loop::new(registry, Arc::new(AcceptsEverything), base());
        let outcome = loop_.dispatch(Event::new("first"));
        assert!(outcome.committed());
        assert_eq!(outcome.handled, vec!["first", "second"]);
        assert_eq!(loop_.db().get("count").and_then(Value::as_i64), Some(11));
        assert_eq!(outcome.rev, 1, "one dispatch is one revision, however long its chain");
        assert_eq!(outcome.changes.len(), 2);
        assert_eq!(outcome.changes[0].after, outcome.changes[1].before);
        assert_eq!(outcome.changes[1].after, *loop_.db());
    }

    #[test]
    fn a_later_chained_fault_rolls_back_all_changes_and_effects() {
        let registry = Arc::new(Registry::new()
            .on_fn("first", 0, "first", |tx, _| {
                tx.set("count", Value::Int(9))?;
                tx.fx(Effect::new("reported"));
                tx.dispatch(Event::new("second"));
                Ok(())
            })
            .on_fn("second", 0, "second", |_, _| Err(Fault::new("test", "failure"))));
        let mut loop_ = Loop::new(registry, Arc::new(AcceptsEverything), base());
        let before = loop_.db().clone();
        let outcome = loop_.dispatch(Event::new("first"));
        assert!(!outcome.committed());
        assert!(outcome.changes.is_empty());
        assert!(outcome.effects.is_empty());
        assert_eq!(loop_.rev(), 0);
        assert!(loop_.db().same(&before));
    }

    #[test]
    fn an_event_nothing_watches_is_not_an_error() {
        let mut loop_ = test_loop();
        let outcome = loop_.dispatch(Event::new("nobody.cares"));
        assert!(outcome.committed());
        assert!(outcome.effects.is_empty());
    }

    #[test]
    fn the_root_may_only_be_merged() {
        let mut loop_ = test_loop();
        let outcome = loop_.dispatch(Event::new("root"));
        assert!(!outcome.committed());
        assert_eq!(outcome.faults[0].code, "patch");
        assert_eq!(outcome.faults[0].source.as_deref(), Some("root"));
    }

    #[test]
    fn a_query_recomputes_only_when_an_input_is_a_different_value() {
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let registry = Arc::new(
            Registry::new()
                .on_fn("bump", 0, "bump", |tx, _| {
                    let count = tx.int("count");
                    tx.set("count", Value::Int(count + 1))
                })
                .on_fn("touch.ui", 0, "touch", |tx, _| tx.set("ui.verbose", Value::Bool(true)))
                .on_fn("append", 0, "append", |tx, _| tx.push("log", Value::str("x")))
                .subscription(
                    "log",
                    read_query(|db, _query| db.get("log").cloned().unwrap_or(Value::Null)),
                )
                .subscription(
                    "log.length",
                    Subscription::Derived {
                        inputs: Inputs::Fixed(vec![Query::new("log")]),
                        compute: Arc::new(move |_inputs, _query, _previous| Ok({
                            counter.fetch_add(1, Ordering::SeqCst);
                            Value::Int(0)
                        })),
                    },
                ),
        );
        let mut loop_ = Loop::new(registry, Arc::new(AcceptsEverything), base());
        let query = Query::new("log.length");
        loop_.watch(query.clone());

        loop_.refresh();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        // A write to a subtree the query does not depend on is not a reason to
        // recompute. The database is a different value, so the read recomputes,
        // but it returns the same allocation for `log`, and that is exactly what
        // the dependent query compares.
        loop_.dispatch(Event::new("touch.ui"));
        loop_.refresh();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "an unrelated write forced a recomputation");
        // A write to the readu2019s own branch is.
        loop_.dispatch(Event::new("append"));
        loop_.refresh();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn refresh_reports_only_the_queries_that_changed() {
        let mut loop_ = test_loop();
        loop_.watch(Query::new("log.length"));
        assert_eq!(loop_.refresh().len(), 1, "a new query has a value to report");
        loop_.dispatch(Event::new("nobody.cares"));
        assert!(loop_.refresh().is_empty(), "an unchanged query was reported");
        loop_.dispatch(Event::new("tick"));
        let changed = loop_.refresh();
        assert_eq!(changed.len(), 1);
        assert_eq!(changed[0].0.id, "log.length");
        assert_eq!(changed[0].1.as_i64(), Some(1));
    }

    #[test]
    fn watch_and_unwatch_bound_the_scope() {
        let mut loop_ = test_loop();
        let query = Query::new("log.length");
        loop_.watch(query.clone());
        loop_.refresh();
        // The dependency keeps its memo: it is still a valid answer to a question
        // somebody asked. Only the forgotten query goes.
        assert_eq!(loop_.scope().len(), 2);
        loop_.unwatch(&query);
        assert_eq!(loop_.scope().len(), 1);
    }

    #[test]
    fn a_query_for_a_subscription_that_does_not_exist_names_it() {
        let mut loop_ = test_loop();
        let error = loop_.query(&Query::new("nope")).unwrap_err();
        assert_eq!(error.code, "query");
        assert!(error.message.contains("nope"));
    }

    #[test]
    fn faults_carry_what_a_client_needs_and_nothing_more() {
        let mut loop_ = test_loop();
        let outcome = loop_.dispatch(Event::new("emit-impossible"));
        let fault = &outcome.as_faults()[0];
        assert_eq!(fault.code, "effect");
        assert_eq!(fault.data.get("event").and_then(Value::as_str), Some("emit-impossible"));
        assert_eq!(fault.data.get("handler").and_then(Value::as_str), Some("impossible"));
    }

    #[test]
    fn a_handler_reads_typed_helpers_without_unwrapping_options() {
        let mut loop_ = test_loop();
        let registry = Arc::new(Registry::new().on_fn("read", 0, "read", |tx, _| {
            assert_eq!(tx.text("ui.verbose"), "");
            assert!(!tx.flag("ui.verbose"));
            assert_eq!(tx.int("missing.deeply.nested"), 0);
            assert!(tx.get("missing").is_none());
            Ok(())
        }));
        let mut second = Loop::new(registry, Arc::new(AcceptsEverything), loop_.db().clone());
        assert!(second.dispatch(Event::new("read")).committed());
        let _ = loop_.dispatch(Event::new("bump"));
    }

    #[test]
    fn a_handler_can_see_what_it_has_queued() {
        // For a handler that records what it wrote. It is not a way to read a result: nothing is
        // applied until the transaction commits.
        let registry = Registry::new().on_fn("tick", 0, "writes", |tx: &mut Tx<'_>, _: &Event| {
            tx.set("a", Value::Int(1))?;
            tx.set("b.c", Value::Int(2))?;
            assert_eq!(tx.patches().len(), 2, "a handler sees its own writes");
            assert_eq!(tx.patches()[1].0.to_string(), "b.c");
            Ok(())
        });
        let mut loop_ = Loop::new(Arc::new(registry), Arc::new(AcceptsEverything), Value::map([("a", Value::Null), ("b", Value::map([]))]));
        let outcome = loop_.dispatch(Event::new("tick"));
        assert!(outcome.committed(), "{:?}", outcome.faults);
        assert_eq!(loop_.db().get("b").and_then(|b| b.get("c")).and_then(Value::as_i64), Some(2));
    }
}

/// Reading an effect's or an event's data.
///
/// An effect is data with an open kind, so its payload has to be read back out.
/// These helpers make that a one-line operation with a defined answer for a
/// missing or mistyped field, which is what keeps an open vocabulary from turning
/// every handler into a defensive parse.
pub mod fields {
    use crate::{Effect, Event};
    use misa_value::Value;

    /// A string field, empty when absent or not a string.
    pub fn text(effect: &Effect, key: &str) -> String {
        effect.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
    }

    /// An integer field, zero when absent or not an integer.
    pub fn int(effect: &Effect, key: &str) -> i64 {
        effect.get(key).and_then(Value::as_i64).unwrap_or(0)
    }

    /// A boolean field, false when absent.
    pub fn flag(effect: &Effect, key: &str) -> bool {
        effect.get(key).and_then(Value::as_bool).unwrap_or(false)
    }

    /// A field's value, or `Null`.
    pub fn value(effect: &Effect, key: &str) -> Value {
        effect.get(key).cloned().unwrap_or(Value::Null)
    }

    /// The string an event carries under a key.
    pub fn event_text(event: &Event, key: &str) -> String {
        event.get(key).and_then(Value::as_str).unwrap_or_default().to_string()
    }

    /// The integer an event carries under a key.
    pub fn event_int(event: &Event, key: &str) -> i64 {
        event.get(key).and_then(Value::as_i64).unwrap_or(0)
    }

    /// The value an event carries under a key.
    pub fn event_value(event: &Event, key: &str) -> Value {
        event.get(key).cloned().unwrap_or(Value::Null)
    }
}
