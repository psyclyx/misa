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

/// The wasmtime engine, re-exported so a caller composes one without depending on wasmtime.
pub use wasmtime::Engine;

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
    pub fn load(engine: &Engine, bytes: &[u8]) -> Result<Plugin, PluginFault> {
        let component = Component::new(engine, bytes).map_err(|error| {
            PluginFault::host(format!("this is not a component this engine can run: {error}"))
        })?;
        let linker: Linker<()> = Linker::new(engine);
        let mut store = Store::new(engine, ());
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
        };
        if descriptor.id.is_empty() {
            return Err(PluginFault::refused("a plugin with no id cannot be named in a diagnostic"));
        }
        Ok(Plugin { inner: Mutex::new(Guest { store, bindings }), descriptor })
    }

    pub fn descriptor(&self) -> &Descriptor {
        &self.descriptor
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
        Ok(())
    }

    /// One handler per event kind the plugin declared, ready for a [`misa_reframe::Registry`].
    pub fn handlers(self: &Arc<Self>) -> Vec<(String, Arc<dyn misa_reframe::Handler>)> {
        self.descriptor
            .events
            .iter()
            .map(|kind| {
                let handler: Arc<dyn misa_reframe::Handler> = Arc::new(crate::PluginHandler::new(
                    self.clone(),
                    kind.clone(),
                ));
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
    pub fn view(&self, role: &str, capabilities: &str, db: &Value, window: u32) -> Result<Node, PluginFault> {
        let db = to_json(db, "this session's state")?;
        let mut inner = self.lock()?;
        let Guest { store, bindings } = &mut *inner;
        let tree = bindings
            .interface0
            .call_view(store, role, capabilities, &db, window)
            .map_err(|error| PluginFault::trap(&self.descriptor.id, error))?
            .map_err(|fault| PluginFault::guest(&fault))?;
        let node = tree_of(&tree)?;
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
