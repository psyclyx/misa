//! Loading a component, calling it, and translating everything that crosses.

use std::sync::{Arc, Mutex};

use misa_proto::view::{Action as ViewAction, ActionOn, Kind, MAX_DEPTH, Node, State};
use misa_reframe::{Effect, Event, Inputs, Interpreter, Subscription};
use misa_value::{Op, Value};
use wasmtime::component::{Component, Linker};
use wasmtime::Store;

// The world a policy plugin implements, generated from the design file.
//
// `wit/policy.wit` is parsed *here*, at compile time, which is why the file is checked by
// every build of this crate: the types below are its types, and a change to it that does not
// parse, or that no longer matches what this module does with it, is a build failure rather
// than a plugin that loads and misbehaves.
wasmtime::component::bindgen!({
    path: "../../wit/policy.wit",
    world: "policy",
});

use exports::misa::policy::policy_api::{
    Effect as GuestEffect, Event as GuestEvent, Fault as GuestFault, Node as GuestNode,
    Op as GuestOp, OptionValue, Patch as GuestPatch, QueryRequest, ViewTree,
};

/// The engine every plugin in this process shares.
///
/// One per process rather than one per plugin: compiling a component is the expensive part, and
/// two plugins have no reason to have two code generators between them. It is built here rather
/// than handed in because its configuration is the *host's* requirement and not a caller's
/// choice — fuel has to be on for a call to be bounded, and a composition that had to remember
/// that would be a composition that can hang a session.
fn engine() -> &'static wasmtime::Engine {
    static ENGINE: std::sync::OnceLock<wasmtime::Engine> = std::sync::OnceLock::new();
    ENGINE.get_or_init(|| {
        let mut config = wasmtime::Config::new();
        config.consume_fuel(true);
        wasmtime::Engine::new(&config).expect("the host's own engine configuration is valid")
    })
}

/// Set the budget for one call into a guest.
///
/// One function because every call has to go through it — a store with fuel switched on and none
/// set traps on the *first* instruction, which is how a host that forgot once would look like a
/// plugin that cannot answer `describe`.
fn fuelled(store: &mut Store<()>) -> Result<(), PluginFault> {
    store.set_fuel(CALL_FUEL).map_err(|error| PluginFault::host(error.to_string()))
}

/// Fuel for one call into a plugin.
///
/// Generous for what a plugin does — read a few json fields, build a tree, walk a database — and
/// small enough that a runaway loop is a fault in tens of milliseconds rather than a session
/// that never answers again. Per *call*, not per plugin: a plugin that spent its budget once may
/// spend another on the next event.
const CALL_FUEL: u64 = 100_000_000;

/// What a plugin said it is, at load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Descriptor {
    pub id: String,
    pub version: String,
    /// Event kinds it handles, one handler each.
    pub events: Vec<String>,
    /// Query names it answers, one subscription each.
    pub queries: Vec<String>,
    /// Effect kinds it may ask for.
    pub effects: Vec<String>,
    /// Affordances its views offer, by the action id a client sends back.
    pub actions: Vec<String>,
    /// State roots it writes into, as plain names.
    ///
    /// The composition makes these before the first event arrives, because a patch may only
    /// create the *last* key of its path: a plugin whose root nobody made is a fault in
    /// somebody's transaction, and a declaration is what stops that being a surprise.
    pub roots: Vec<String>,
}

/// One patch a plugin asked for: a path, and what to do at it.
#[derive(Clone, Debug, PartialEq)]
pub struct Patch {
    /// `a.b[0].c`, or empty for the root.
    pub path: String,
    pub op: Op,
}

/// A plugin failure, in the one shape everything else here reports failures in.
///
/// `code` says *who* is at fault, which is the distinction worth keeping: a guest that said no
/// keeps its own code, and everything the host refused — json it could not read, a path that
/// is not one, a tree deeper than the protocol allows, a trap — is `plugin.host`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginFault {
    pub code: String,
    pub message: String,
}

impl PluginFault {
    fn host(message: impl Into<String>) -> PluginFault {
        PluginFault { code: "plugin.host".into(), message: message.into() }
    }

    fn refused(message: impl Into<String>) -> PluginFault {
        PluginFault { code: "plugin.refused".into(), message: message.into() }
    }

    fn guest(fault: &GuestFault) -> PluginFault {
        let where_ = match &fault.event {
            Some(event) => format!(" while handling `{event}`"),
            None => String::new(),
        };
        PluginFault {
            code: if fault.code.is_empty() { "plugin.fault".into() } else { fault.code.clone() },
            message: format!("{}{where_}", fault.message),
        }
    }

    fn trap(id: &str, error: wasmtime::Error) -> PluginFault {
        // A budget that ran out is the one trap worth a sentence of its own: "trapped" would
        // send a plugin author looking for a bug in the wrong place.
        if error.downcast_ref::<wasmtime::Trap>() == Some(&wasmtime::Trap::OutOfFuel) {
            return PluginFault {
                code: "plugin.out-of-fuel".into(),
                message: format!(
                    "`{id}` used its whole budget for one call ({CALL_FUEL} units) and was stopped"
                ),
            };
        }
        PluginFault::host(format!("the component `{id}` trapped: {error}"))
    }
}

/// A loaded plugin: its store, its instance, and what it declared.
pub struct Plugin {
    inner: Mutex<Guest>,
    descriptor: Descriptor,
}

impl std::fmt::Debug for Plugin {
    /// The descriptor, and not the store: a wasm instance prints as a page of indices.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Plugin")
            .field("id", &self.descriptor.id)
            .field("version", &self.descriptor.version)
            .finish()
    }
}

/// `(store, instance)`, behind the plugin's lock.
struct Guest {
    store: Store<()>,
    bindings: Policy,
}

impl Plugin {
    /// Compile a component, instantiate it, and ask it what it is.
    ///
    /// Nothing runs until [`Plugin::validate`] has compared its declarations against the
    /// composition, so a plugin that asks for something this session cannot do is refused
    /// before an event reaches it.
    pub fn load(bytes: &[u8]) -> Result<Plugin, PluginFault> {
        let engine = engine();
        let component = Component::new(engine, bytes).map_err(|error| {
            PluginFault::host(format!("this is not a component this engine can run: {error}"))
        })?;
        let linker: Linker<()> = Linker::new(engine);
        let mut store = Store::new(engine, ());
        fuelled(&mut store)?;
        let bindings = Policy::instantiate(&mut store, &component, &linker).map_err(|error| {
            PluginFault::host(format!(
                "the component asked for something this host does not provide: {error}"
            ))
        })?;
        let declared = bindings
            .interface0
            .call_describe(&mut store)
            .map_err(|error| PluginFault::host(format!("`describe` did not answer: {error}")))?;
        let descriptor = Descriptor {
            id: declared.id,
            version: declared.version,
            events: declared.events,
            queries: declared.queries,
            effects: declared.effects,
            actions: declared.actions,
            roots: declared.roots,
        };
        if !plain_id(&descriptor.id) {
            return Err(PluginFault::refused(format!(
                "`{}` is not an id a session can present under: an id is a dotted lowercase name,                  like a role, because that is what a theme is written against",
                descriptor.id
            )));
        }
        Ok(Plugin { inner: Mutex::new(Guest { store, bindings }), descriptor })
    }

    pub fn descriptor(&self) -> &Descriptor {
        &self.descriptor
    }

    /// The state roots this plugin declared it writes into.
    pub fn roots(&self) -> &[String] {
        &self.descriptor.roots
    }

    /// Hand the plugin the composition's options. Called once, after load.
    ///
    /// Options are how a composition parameterises a plugin without giving it a capability to
    /// read configuration: what is not passed is not available.
    pub fn configure(&self, options: &[(String, String)]) -> Result<(), PluginFault> {
        let options = options
            .iter()
            .map(|(key, value)| OptionValue { key: key.clone(), value: value.clone() })
            .collect::<Vec<_>>();
        let mut inner = self.lock()?;
        let Guest { store, bindings } = &mut *inner;
        fuelled(store)?;
        bindings
            .interface0
            .call_configure(store, &options)
            .map_err(|error| PluginFault::trap(&self.descriptor.id, error))?
            .map_err(|fault| PluginFault::guest(&fault))
    }

    /// Compare what the plugin declared with what this composition will accept.
    ///
    /// The whole of "a plugin declares; it does not register": every effect it named is put to
    /// the interpreter *now*, so a plugin that wanted a capability this session does not have
    /// is refused at install rather than in somebody's transaction later.
    pub fn validate(&self, interpreter: &dyn Interpreter) -> Result<(), PluginFault> {
        for kind in &self.descriptor.effects {
            if let Some(why) = not_a_plugin_effect(kind) {
                return Err(PluginFault::refused(format!("`{}`: {why}", self.descriptor.id)));
            }
            interpreter.accepts(&Effect::new(kind.clone())).map_err(|why| {
                PluginFault::refused(format!(
                    "`{}` declares the effect `{kind}`, which this composition does not accept: {why}",
                    self.descriptor.id
                ))
            })?;
        }
        for kind in self.descriptor.events.iter().chain(&self.descriptor.queries) {
            if kind.is_empty() {
                return Err(PluginFault::refused(format!(
                    "`{}` declares an unnamed event or query",
                    self.descriptor.id
                )));
            }
        }
        plain_roots(&self.descriptor.id, &self.descriptor.roots)
    }

    /// One handler per event kind the plugin declared, ready for a [`misa_reframe::Registry`].
    ///
    /// There is nothing else to register, and nothing special about a plugin's: a plugin that
    /// wants to be acted on declares `intent/action` like any other event kind, and the loop's
    /// registry routes it — by kind, in priority order, through the same path every handler in
    /// this system goes through.
    pub fn handlers(self: &Arc<Self>) -> Vec<(String, Arc<dyn misa_reframe::Handler>)> {
        self.descriptor
            .events
            .iter()
            .map(|kind| {
                let handler: Arc<dyn misa_reframe::Handler> =
                    Arc::new(crate::PluginHandler::new(self.clone(), kind.clone()));
                (kind.clone(), handler)
            })
            .collect()
    }

    /// One subscription per query the plugin declared.
    ///
    /// A subscription cannot fail — the loop's `compute` has no `Result` — so a plugin that
    /// cannot answer returns a value that says so rather than nothing. A client that
    /// subscribed sees the failure instead of an empty list it would have to interpret.
    pub fn subscriptions(self: &Arc<Self>) -> Vec<(String, Subscription)> {
        self.descriptor
            .queries
            .iter()
            .map(|name| {
                let plugin = self.clone();
                let query_name = name.clone();
                let subscription = Subscription {
                    // The database, because a plugin is handed the database: naming every path
                    // it might read would be a list the host does not know.
                    inputs: Inputs::Database,
                    compute: Arc::new(move |db, inputs, query, previous| {
                        match plugin.query(&query_name, &query.args, inputs, db, previous) {
                            Ok(value) => value,
                            Err(fault) => Value::map([
                                ("plugin", Value::str(&plugin.descriptor.id)),
                                ("code", Value::str(&fault.code)),
                                ("error", Value::str(&fault.message)),
                            ]),
                        }
                    }),
                };
                (name.clone(), subscription)
            })
            .collect()
    }

    /// One event, as the loop's vocabulary sees it, and what the plugin wants done about it.
    pub fn handle(&self, event: &Event, db: &Value) -> Result<(Vec<Patch>, Vec<Effect>), PluginFault> {
        let asked = GuestEvent {
            kind: event.kind.clone(),
            data: optional_json(&event.data, "event data")?,
        };
        let db = to_json(db, "this session's state")?;
        let mut inner = self.lock()?;
        let Guest { store, bindings } = &mut *inner;
        fuelled(store)?;
        let answered = bindings
            .interface0
            .call_handle(store, &asked, &db)
            .map_err(|error| PluginFault::trap(&self.descriptor.id, error))?
            .map_err(|fault| PluginFault::guest(&fault))?;
        let (patches, effects) = answered;
        Ok((patches_of(&patches)?, effects_of(&effects)?))
    }

    /// One query, answered as a value.
    ///
    /// `inputs` are the query's declared dependencies, already evaluated by the loop, and
    /// `previous` is the value this query produced last time — a hint that may be absent.
    pub fn query(
        &self,
        id: &str,
        args: &[Value],
        inputs: &[Value],
        db: &Value,
        previous: Option<&Value>,
    ) -> Result<Value, PluginFault> {
        let request = QueryRequest {
            id: id.to_string(),
            args: args.iter().map(|arg| to_json(arg, "a query argument")).collect::<Result<Vec<_>, _>>()?,
        };
        let inputs = inputs
            .iter()
            .map(|input| to_json(input, "a query input"))
            .collect::<Result<Vec<_>, _>>()?;
        let db = to_json(db, "this session's state")?;
        let previous = match previous {
            Some(value) => optional_json(value, "the previous answer")?,
            None => None,
        };
        let mut inner = self.lock()?;
        let Guest { store, bindings } = &mut *inner;
        fuelled(store)?;
        let answered = bindings
            .interface0
            .call_query(store, &request, &inputs, &db, previous.as_deref())
            .map_err(|error| PluginFault::trap(&self.descriptor.id, error))?
            .map_err(|fault| PluginFault::guest(&fault))?;
        from_json(&answered, "a query answer")
    }

    /// A view tree, converted and validated.
    ///
    /// The validation is [`misa_proto::view::validate`] — the same function a session's own
    /// view is held to — so a plugin's tree is not merely *its* tree: it is a tree every
    /// client may assume things about, or it is a fault.
    pub fn view(&self, capabilities: &misa_proto::wire::Capabilities, db: &Value, window: usize) -> Result<Node, PluginFault> {
        // The client's capabilities go over as the json a client declared: the render class,
        // whether it has a native disclosure widget, its size. That is the *whole* of what a
        // plugin learns about a client, which is what keeps presentation policy in one place —
        // the tree it returns is placed by the session for every frontend at once.
        let capabilities = serde_json::to_string(capabilities)
            .map_err(|error| PluginFault::host(format!("the client's capabilities: {error}")))?;
        let db = to_json(db, "this session's state")?;
        let mut inner = self.lock()?;
        let Guest { store, bindings } = &mut *inner;
        fuelled(store)?;
        let tree = bindings
            .interface0
            .call_view(store, &capabilities, &db, window as u32)
            .map_err(|error| PluginFault::trap(&self.descriptor.id, error))?
            .map_err(|fault| PluginFault::guest(&fault))?;
        let node = tree_of(&tree)?;
        named(&node)?;
        offered(&self.descriptor.id, &node, &self.descriptor.actions)?;
        misa_proto::view::validate(&node).map_err(|fault| {
            PluginFault::host(format!("the plugin's view is not one a client may be sent: {fault}"))
        })?;
        Ok(node)
    }

    /// The lock, as a fault rather than a panic.
    ///
    /// Poisoned means a call panicked while holding it, which cannot happen through this API
    /// — every path to a guest is a `Result` — so this is the one place that would rather
    /// report than unwind.
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Guest>, PluginFault> {
        self.inner.lock().map_err(|_| PluginFault::host("the plugin is poisoned by a panic"))
    }
}

/// Whether a plugin's id is a *name*: dotted lowercase, like a role.
///
/// Nothing here parses an id — no part of this host splits one — and this is why: the session
/// presents a plugin's tree under a role it builds from this id (`plugin.<id>`), and a role is
/// what a theme is written against. An id that is not a name is a tree no stylesheet can reach
/// and no diagnostic can point at.
fn plain_id(id: &str) -> bool {
    !id.is_empty()
        && id.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        })
}

/// Every node in a plugin's tree needs an id.
///
/// A client remembers which nodes it opened by id, so a tree whose nodes have none is a tree
/// that cannot be remembered — and one the session would have to name positionally, which moves
/// when the tree grows. Refused here, where the message can tell a plugin author what to do.
fn named(node: &Node) -> Result<(), PluginFault> {
    if node.id.is_empty() {
        return Err(PluginFault::host(
            "a node in the plugin's view has no id, and a client remembers the nodes it opened by id",
        ));
    }
    for child in &node.children {
        named(child)?;
    }
    Ok(())
}

/// Every action in a tree has to be one the plugin declared.
///
/// The session routes an action as the loop's own `intent/action` event, so nothing here has to
/// translate an id — but a client clicking an affordance nobody handles is a silence, and this is
/// where that becomes impossible: a tree that offers an action outside the declaration is refused
/// when the plugin presents it, with the plugin's id and the action on it.
fn offered(id: &str, node: &Node, declared: &[String]) -> Result<(), PluginFault> {
    for action in &node.actions {
        if !declared.contains(&action.id) {
            return Err(PluginFault::refused(format!(
                "`{id}` presented an action `{}` that it does not declare; declared: {:?}",
                action.id, declared
            )));
        }
    }
    for child in &node.children {
        offered(id, child, declared)?;
    }
    Ok(())
}

/// What a plugin declared as a state root has to be: one plain name, once.
///
/// A root is a *name* — the composition's check against the manifest is about names, and the
/// paths inside a root are the plugin's own business — so a plugin that declared `session.status`
/// would be claiming something that is not a root at all. Twice is a mistake rather than two
/// roots, and an empty name is not a name.
fn plain_roots(id: &str, roots: &[String]) -> Result<(), PluginFault> {
    let mut seen: Vec<&String> = Vec::new();
    for root in roots {
        if root.is_empty() || root.contains(['.', '[', ']']) {
            return Err(PluginFault::refused(format!(
                "`{id}` declares `{root}` as a state root, and a root is one plain name"
            )));
        }
        if seen.contains(&root) {
            return Err(PluginFault::refused(format!(
                "`{id}` declares the state root `{root}` twice"
            )));
        }
        seen.push(root);
    }
    Ok(())
}

/// Effect kinds a plugin may not ask for, whatever the composition accepts.
///
/// `wire.event` is the one: its data is a `SessionEvent` in the protocol's own encoding, which
/// json cannot express, so a plugin asking for it would produce an effect the session quietly
/// drops. A plugin reports by patching state and letting the session decide what a client is
/// told — the same rule that keeps presentation out of the middle layer.
fn not_a_plugin_effect(kind: &str) -> Option<&'static str> {
    match kind {
        "wire.event" => Some(
            "a wire event carries a `SessionEvent` in the protocol's own encoding, which json \
             cannot express; a plugin reports by patching state and the session decides what to \
             tell clients",
        ),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// What crosses the boundary, in both directions
// ---------------------------------------------------------------------------

/// A value as json text, which is the only shape this boundary carries data in.
fn to_json(value: &Value, what: &str) -> Result<String, PluginFault> {
    serde_json::to_string(value)
        .map_err(|error| PluginFault::host(format!("{what} cannot be written as json: {error}")))
}

/// The same, for a field the world spells `option<string>`: json `null` is "nothing".
fn optional_json(value: &Value, what: &str) -> Result<Option<String>, PluginFault> {
    match value {
        Value::Null => Ok(None),
        other => to_json(other, what).map(Some),
    }
}

/// Json text as a value.
fn from_json(text: &str, what: &str) -> Result<Value, PluginFault> {
    serde_json::from_str(text)
        .map_err(|error| PluginFault::host(format!("{what} is not json this host can read: {error}")))
}

fn patches_of(patches: &[GuestPatch]) -> Result<Vec<Patch>, PluginFault> {
    patches
        .iter()
        .map(|patch| {
            // Parsed here rather than left to the transaction: a path that is not a path is the
            // plugin's mistake, and the fault should say so instead of rolling back a
            // transaction with a message about a patch.
            misa_value::Path::parse(&patch.path).map_err(|error| {
                PluginFault::host(format!("`{}` is not a path: {error}", patch.path))
            })?;
            let op = match &patch.op {
                GuestOp::Set(value) => Op::Set(from_json(value, "a patch value")?),
                GuestOp::Merge(value) => Op::Merge(from_json(value, "a patch value")?),
                GuestOp::Delete => Op::Delete,
                GuestOp::Append(value) => Op::Append(from_json(value, "a patch value")?),
                GuestOp::AppendAll(value) => {
                    let parsed = from_json(value, "an append-all value")?;
                    let items = match parsed {
                        Value::List(items) => items.as_ref().to_vec(),
                        other => {
                            return Err(PluginFault::host(format!(
                                "append-all takes a list of values, and this plugin sent `{}`",
                                other.kind()
                            )));
                        }
                    };
                    Op::AppendAll(items)
                }
            };
            Ok(Patch { path: patch.path.clone(), op })
        })
        .collect()
}

fn effects_of(effects: &[GuestEffect]) -> Result<Vec<Effect>, PluginFault> {
    effects
        .iter()
        .map(|effect| {
            let data = match &effect.data {
                Some(text) => from_json(text, "an effect's data")?,
                None => Value::Null,
            };
            Ok(Effect::new(effect.kind.clone()).with_value(data))
        })
        .collect()
}

/// A flat view tree, as the nested one the protocol defines.
///
/// The list is in document order with parents first, which is what makes this a loop rather
/// than a recursion: walking it backwards builds every child before its parent, and the depth a
/// plugin may produce is checked against [`MAX_DEPTH`] *here* rather than left to
/// [`misa_proto::view::validate`] — which recurses, and would overflow the stack on a
/// ten-thousand-deep tree before it could say no.
///
/// Two rules make the shape a tree rather than a graph or a forest, and both are refused
/// rather than repaired: a node's parent must come *before* it in the list, and only the root
/// may have no parent. Together they mean every node is reachable, which is why nothing here
/// counts them.
fn tree_of(tree: &ViewTree) -> Result<Node, PluginFault> {
    if tree.nodes.is_empty() {
        return Err(PluginFault::host("a view with no nodes has no root"));
    }
    let root = tree.root as usize;
    if root >= tree.nodes.len() {
        return Err(PluginFault::host(format!(
            "a view's root is node {root}, and there are {} nodes",
            tree.nodes.len()
        )));
    }
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); tree.nodes.len()];
    for (index, node) in tree.nodes.iter().enumerate() {
        match node.parent {
            Some(parent) if (parent as usize) < index => children[parent as usize].push(index),
            Some(parent) => {
                return Err(PluginFault::host(format!(
                    "node {index} names parent {parent}, and a view is in document order with \
                     parents first"
                )));
            }
            None if index == root => {}
            None => {
                return Err(PluginFault::host(format!(
                    "node {index} has no parent and is not the root"
                )));
            }
        }
    }
    let mut built: Vec<Option<Node>> = (0..tree.nodes.len()).map(|_| None).collect();
    let mut depth: Vec<usize> = vec![1; tree.nodes.len()];
    for index in (0..tree.nodes.len()).rev() {
        let mut child_nodes = Vec::with_capacity(children[index].len());
        let mut deepest = 0;
        for child in &children[index] {
            let node = built[*child].take().ok_or_else(|| {
                PluginFault::host(format!("node {child} is not reachable in document order"))
            })?;
            deepest = deepest.max(depth[*child]);
            child_nodes.push(node);
        }
        depth[index] = deepest + 1;
        if depth[index] > MAX_DEPTH {
            return Err(PluginFault::host(format!(
                "the view is {} nodes deep, and a client may only be sent {MAX_DEPTH}",
                depth[index]
            )));
        }
        built[index] = Some(node_of(&tree.nodes[index], child_nodes)?);
    }
    // Every node was built exactly once, and only the root may have no parent, so the root is
    // still here: a node nobody points at, or a second root, is refused above.
    built[root].take().ok_or_else(|| PluginFault::host("a view with no root, which cannot happen"))
}

fn node_of(node: &GuestNode, children: Vec<Node>) -> Result<Node, PluginFault> {
    let state = match &node.state {
        Some(name) => Some(state_of(name)?),
        None => None,
    };
    let actions = node.actions.iter().map(action_of).collect::<Result<Vec<_>, _>>()?;
    Ok(Node {
        id: node.id.clone(),
        role: node.role.clone(),
        kind: kind_of(&node.kind, node.data.as_deref())?,
        label: None,
        state,
        actions,
        children,
    })
}

fn state_of(name: &str) -> Result<State, PluginFault> {
    serde_json::from_value::<State>(serde_json::Value::String(name.to_string()))
        .map_err(|_| PluginFault::host(format!("`{name}` is not a state this protocol has")))
}

/// The node's kind, assembled from the two fields the world splits it into.
///
/// The protocol encodes a kind as a tagged object (`{"shape": "text", "spans": [...]}`), which
/// is exactly `kind` plus `data`, so the plugin's two fields are put back together here and
/// decoded by the same `Deserialize` every other reader of this protocol uses. `shape` wins
/// over anything `data` said, because the kind is the field the world separates out.
fn kind_of(shape: &str, data: Option<&str>) -> Result<Kind, PluginFault> {
    let mut object: serde_json::Map<String, serde_json::Value> = match data {
        Some(text) => match serde_json::from_str(text) {
            Ok(serde_json::Value::Object(object)) => object,
            Ok(serde_json::Value::Null) => Default::default(),
            Ok(_) => return Err(PluginFault::host("a node's data must be a json object")),
            Err(error) => {
                return Err(PluginFault::host(format!(
                    "a node's data is not json this host can read: {error}"
                )));
            }
        },
        None => Default::default(),
    };
    object.insert("shape".to_string(), serde_json::Value::String(shape.to_string()));
    serde_json::from_value(serde_json::Value::Object(object))
        .map_err(|error| PluginFault::host(format!("`{shape}` is not a kind of node a client can be sent: {error}")))
}

fn action_of(action: &exports::misa::policy::policy_api::Action) -> Result<ViewAction, PluginFault> {
    let args = match &action.args {
        Some(text) => from_json(text, "an action's args")?,
        None => Value::Null,
    };
    Ok(ViewAction {
        id: action.id.clone(),
        on: if action.on_submit { ActionOn::Submit } else { ActionOn::Click },
        label: action.label.clone(),
        args,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use exports::misa::policy::policy_api::{Action as GuestAction, Node as GuestNode, Op as GuestOp, Patch as GuestPatch};

    /// A node, as a plugin writes one: a kind and the json its kind needs.
    fn node(id: &str, shape: &str, data: Option<&str>, parent: Option<u32>) -> GuestNode {
        GuestNode {
            id: id.to_string(),
            role: "plugin.thing".to_string(),
            kind: shape.to_string(),
            data: data.map(str::to_string),
            parent,
            actions: Vec::new(),
            state: None,
        }
    }

    #[test]
    fn a_node_kind_is_the_protocol_encoding_of_the_two_fields_the_world_splits_it_into() {
        // `kind` plus `data` is exactly what the protocol writes as `{"shape": …, …}`. The
        // world splits it because a plugin cannot import the protocol's own types, and this is
        // where the two halves are put back together — through the same `Deserialize` every
        // other reader of the protocol uses, so a kind is decoded once.
        match kind_of("text", Some(r#"{"spans":[{"text":"hi"}]}"#)).unwrap() {
            Kind::Text { spans } => {
                assert_eq!(spans.len(), 1);
                assert_eq!(spans[0].text, "hi");
            }
            other => panic!("expected text, got {other:?}"),
        }
        assert_eq!(kind_of("section", None).unwrap(), Kind::Section);
        // The `kind` field wins over anything `data` claims to be: it is the one the world
        // separated out, so it is the one that means it.
        assert_eq!(kind_of("section", Some(r#"{"shape":"rule"}"#)).unwrap(), Kind::Section);
    }

    #[test]
    fn a_kind_a_client_cannot_be_sent_is_a_fault_rather_than_a_tree() {
        let fault = kind_of("nonsense", None).unwrap_err();
        assert_eq!(fault.code, "plugin.host");
        assert!(fault.message.contains("nonsense"), "{}", fault.message);
        // A kind is more than its tag: `text` without spans is not a text node, and the
        // decoder is the one that says so.
        assert!(kind_of("text", Some("{}")).is_err());
        assert!(kind_of("text", Some("[1,2]")).is_err(), "a node's data is an object or nothing");
        assert!(kind_of("text", Some("not json at all")).is_err());
    }

    #[test]
    fn a_state_a_client_does_not_have_is_a_fault() {
        assert_eq!(state_of("streaming").unwrap(), State::Streaming);
        assert_eq!(state_of("done").unwrap(), State::Done);
        let fault = state_of("nearly").unwrap_err();
        assert!(fault.message.contains("nearly"), "{}", fault.message);
    }

    #[test]
    fn a_path_is_parsed_here_so_the_fault_says_whose_mistake_it_was() {
        let good = patches_of(&[GuestPatch {
            path: "guest.turns[0].seen".to_string(),
            op: GuestOp::Set("1".to_string()),
        }])
        .unwrap();
        assert_eq!(good[0].path, "guest.turns[0].seen");
        assert_eq!(good[0].op, Op::Set(Value::Int(1)));

        let fault = patches_of(&[GuestPatch {
            path: "a..b".to_string(),
            op: GuestOp::Delete,
        }])
        .unwrap_err();
        assert_eq!(fault.code, "plugin.host");
        assert!(fault.message.contains("a..b"), "{}", fault.message);
    }

    #[test]
    fn every_op_the_world_has_arrives_as_the_loops_own() {
        let patches = patches_of(&[
            GuestPatch { path: "a".into(), op: GuestOp::Set("\"x\"".into()) },
            GuestPatch { path: "b".into(), op: GuestOp::Merge(r#"{"k":1}"#.into()) },
            GuestPatch { path: "c".into(), op: GuestOp::Delete },
            GuestPatch { path: "d".into(), op: GuestOp::Append("2".into()) },
            GuestPatch { path: "e".into(), op: GuestOp::AppendAll("[1,2]".into()) },
        ])
        .unwrap();
        assert_eq!(patches[0].op, Op::Set(Value::str("x")));
        assert_eq!(patches[1].op, Op::Merge(Value::map([("k", Value::Int(1))])));
        assert_eq!(patches[2].op, Op::Delete);
        assert_eq!(patches[3].op, Op::Append(Value::Int(2)));
        assert_eq!(patches[4].op, Op::AppendAll(vec![Value::Int(1), Value::Int(2)]));

        // Append-all is the one op whose shape matters: it grows a list by every element of
        // one, so a single value is not a thing it can be given.
        let fault = patches_of(&[GuestPatch { path: "e".into(), op: GuestOp::AppendAll("1".into()) }])
            .unwrap_err();
        assert!(fault.message.contains("append-all takes a list"), "{}", fault.message);
    }

    #[test]
    fn an_effect_carries_its_data_or_nothing_at_all() {
        let effects = effects_of(&[
            GuestEffect { kind: "kernel.log.append".into(), data: Some(r#"{"kind":"note"}"#.into()) },
            GuestEffect { kind: "kernel.log.list".into(), data: None },
        ])
        .unwrap();
        assert_eq!(effects[0].kind, "kernel.log.append");
        assert_eq!(effects[0].field("kind"), "note");
        assert_eq!(effects[1].data, Value::Null);

        // Data that is not json is the plugin's mistake, and it is a fault rather than an
        // effect the interpreter would refuse for a reason about the wrong thing.
        assert!(effects_of(&[GuestEffect { kind: "k".into(), data: Some("nope".into()) }]).is_err());
    }

    #[test]
    fn a_flat_tree_becomes_the_nested_one_a_client_draws() {
        let tree = ViewTree {
            nodes: vec![
                node("root", "section", None, None),
                node("a", "status", Some(r#"{"text":"one"}"#), Some(0)),
                node("b", "section", None, Some(0)),
                node("b.1", "fact", Some(r#"{"value":7}"#), Some(2)),
            ],
            root: 0,
        };
        let built = tree_of(&tree).unwrap();
        assert_eq!(built.id, "root");
        assert_eq!(built.children.len(), 2);
        assert_eq!(built.children[0].id, "a");
        assert_eq!(built.children[1].children[0].id, "b.1");
        // And it is a tree a client may be sent, by the same validator a session's own view
        // answers to.
        misa_proto::view::validate(&built).expect("valid");
    }

    #[test]
    fn a_tree_the_protocol_would_refuse_is_refused_here_first() {
        // No nodes at all.
        assert!(tree_of(&ViewTree { nodes: vec![], root: 0 }).is_err());
        // A root that is not a node.
        assert!(tree_of(&ViewTree { nodes: vec![node("a", "section", None, None)], root: 7 }).is_err());
        // A node with no parent that is not the root.
        assert!(
            tree_of(&ViewTree {
                nodes: vec![node("a", "section", None, None), node("orphan", "section", None, None)],
                root: 0,
            })
            .is_err()
        );
        // A child before its parent.
        assert!(
            tree_of(&ViewTree {
                nodes: vec![node("child", "section", None, Some(1)), node("root", "section", None, None)],
                root: 1,
            })
            .is_err()
        );
        // A second root, which is a forest rather than a tree.
        assert!(
            tree_of(&ViewTree {
                nodes: vec![
                    node("root", "section", None, None),
                    node("also-root", "section", None, None),
                ],
                root: 0,
            })
            .is_err()
        );
    }

    #[test]
    fn a_tree_deeper_than_a_client_may_be_sent_is_refused_without_recursing() {
        // The check is here rather than left to `view::validate` because that one recurses: a
        // guest could ask for a stack overflow with a hundred thousand nodes, and a guest does
        // not get to decide how this process dies.
        let deep = MAX_DEPTH + 2;
        let mut nodes = vec![node("n0", "section", None, None)];
        for index in 1..deep {
            nodes.push(node(&format!("n{index}"), "section", None, Some((index - 1) as u32)));
        }
        let fault = tree_of(&ViewTree { nodes, root: 0 }).unwrap_err();
        assert!(fault.message.contains("deep"), "{}", fault.message);
    }

    #[test]
    fn a_plugin_id_is_a_name_because_a_role_is_built_from_it() {
        for good in ["guest", "policy.guest", "guest-2", "a.b.c-1"] {
            assert!(plain_id(good), "{good}");
        }
        for bad in ["", ".", "a..b", "Guest", "guest ", "guest:2", "guest/x"] {
            assert!(!plain_id(bad), "{bad}");
        }
    }

    #[test]
    fn a_tree_may_only_offer_actions_the_plugin_declared() {
        let declared = vec!["refresh".to_string()];
        let mut tree = Node::section("guest").id("guest");
        assert!(offered("guest", &tree, &declared).is_ok());

        tree.actions.push(misa_proto::view::Action {
            id: "refresh".into(),
            on: misa_proto::view::ActionOn::Click,
            label: None,
            args: Value::Null,
        });
        assert!(offered("guest", &tree, &declared).is_ok());

        tree.actions.push(misa_proto::view::Action {
            id: "panel.close".into(),
            on: misa_proto::view::ActionOn::Click,
            label: None,
            args: Value::Null,
        });
        // A plugin cannot offer the session's own affordances by naming one: it either declares
        // it or the tree is refused, and declaring `panel.close` would make the session's
        // handler do something the plugin asked for.
        let fault = offered("guest", &tree, &declared).unwrap_err();
        assert_eq!(fault.code, "plugin.refused");
        assert!(fault.message.contains("panel.close"), "{}", fault.message);
    }

    #[test]
    fn every_node_a_plugin_presents_has_an_id() {
        let named_tree = Node::section("guest").id("guest");
        assert!(named(&named_tree).is_ok());
        let mut anonymous = Node::section("guest").id("guest");
        anonymous.children.push(Node::section("guest.anonymous"));
        let fault = named(&anonymous).unwrap_err();
        assert!(fault.message.contains("no id"), "{}", fault.message);
    }

    #[test]
    fn a_state_root_is_one_plain_name_and_only_once() {
        assert!(plain_roots("guest", &["guest".to_string()]).is_ok());
        assert!(plain_roots("guest", &[]).is_ok());
        for bad in ["", "guest.turns", "guest[0]", "a..b"] {
            let fault = plain_roots("guest", &[bad.to_string()]).expect_err("refused");
            assert_eq!(fault.code, "plugin.refused");
            assert!(fault.message.contains("plain name"), "{}", fault.message);
        }
        let twice = plain_roots("guest", &["guest".to_string(), "guest".to_string()]).unwrap_err();
        assert!(twice.message.contains("twice"), "{}", twice.message);
    }

    #[test]
    fn wire_event_is_the_one_effect_a_plugin_may_not_ask_for() {
        // Its data is a `SessionEvent` in the protocol's own encoding, which json cannot
        // express, so a plugin that asked for it would produce an effect the session quietly
        // drops. Refusing it at install is the honest answer, and the reason says why.
        let why = not_a_plugin_effect("wire.event").expect("refused");
        assert!(why.contains("json"), "{why}");
        assert!(not_a_plugin_effect("kernel.log.append").is_none());
    }

    #[test]
    fn a_value_crosses_as_json_and_bytes_are_the_thing_json_cannot_carry() {
        let value = Value::map([
            ("text", Value::str("hi")),
            ("n", Value::Int(7)),
            ("ratio", Value::Float(0.5)),
            ("list", Value::list([Value::Bool(true), Value::Null])),
        ]);
        let text = to_json(&value, "a value").unwrap();
        assert_eq!(from_json(&text, "a value").unwrap(), value);

        // Bytes have no json: they arrive as a list of numbers and cannot come back as bytes,
        // so the only thing that makes them is the kernel's blob store.
        let bytes = to_json(&Value::Bytes(std::sync::Arc::from(&b"ab"[..])), "bytes").unwrap();
        assert_eq!(bytes, "[97,98]");
        assert_eq!(from_json(&bytes, "bytes").unwrap(), Value::list([Value::Int(97), Value::Int(98)]));
        assert_eq!(optional_json(&Value::Null, "nothing").unwrap(), None);
    }

    #[test]
    fn an_actions_args_are_json_and_its_kind_is_the_protocols() {
        let click = action_of(&GuestAction {
            id: "guest.close".into(),
            label: Some("Close".into()),
            on_submit: false,
            args: None,
        })
        .unwrap();
        assert_eq!(click.on, ActionOn::Click);
        assert_eq!(click.label.as_deref(), Some("Close"));
        assert_eq!(click.args, Value::Null);

        let submit = action_of(&GuestAction {
            id: "guest.send".into(),
            label: None,
            on_submit: true,
            args: Some(r#"{"value":"typed"}"#.into()),
        })
        .unwrap();
        assert_eq!(submit.on, ActionOn::Submit);
        assert_eq!(submit.args.get("value").and_then(Value::as_str), Some("typed"));
        // Args the host cannot read are a fault, not an action with nothing in it.
        assert!(
            action_of(&GuestAction {
                id: "guest.send".into(),
                label: None,
                on_submit: true,
                args: Some("not json".into()),
            })
            .is_err()
        );
    }
}
