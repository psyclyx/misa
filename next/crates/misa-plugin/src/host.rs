//! Loading a component, calling it, and translating everything that crosses.

use std::sync::{Arc, Mutex};

use misa_proto::view::Node;
use misa_reframe::{Effect, Event, Interpreter, Subscription};
use misa_value::{Op, Value};
use misa_proto::{Query, schema::Schema};
pub use misa_proto::query::ResultContract as QueryResultContract;
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
    Effect as GuestEffect, Event as GuestEvent, Fault as GuestFault,
    Op as GuestOp, OptionValue, Patch as GuestPatch, QueryRequest,
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
#[derive(Clone, Debug, PartialEq)]
pub struct Descriptor {
    pub tools: Vec<misa_proto::tool::Binding>,
    pub commands: Vec<CommandDefinition>,
    pub bindings: Vec<misa_proto::invocation::Binding>,
    pub presentations: Vec<Presentation>,
    pub id: String,
    pub version: String,
    /// Event kinds it handles, one handler each.
    pub events: Vec<String>,
    /// Query names it answers, one subscription each.
    pub queries: Vec<QueryDefinition>,
    /// Effect kinds it may ask for.
    pub effects: Vec<String>,
    /// State roots it writes into, as plain names.
    ///
    /// The composition makes these before the first event arrives, because a patch may only
    /// create the *last* key of its path: a plugin whose root nobody made is a fault in
    /// somebody's transaction, and a declaration is what stops that being a surprise.
    pub roots: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CommandDefinition { pub id: String, pub input: Schema, pub event: String }
impl CommandDefinition {
    pub fn export(&self) -> misa_proto::invocation::Command {
        misa_proto::invocation::Command { id: self.id.clone(), input: self.input.clone(),
            result: Schema::Choice { values: vec![misa_proto::schema::Literal::Null] } }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct QueryDefinition {
    pub id: String,
    pub contract: String,
    pub arguments: Vec<Schema>,
    pub result: QueryResultContract,
    pub source: QuerySource,
}
#[derive(Clone, Debug, PartialEq)]
pub enum QuerySource {
    Read { roots: Vec<String>, schema: Schema },
    Derived { inputs: Vec<Query> },
}

impl QueryDefinition {
    pub fn export(&self) -> misa_proto::query::Definition {
        misa_proto::query::Definition { id: self.id.clone(), contract: self.contract.clone(), arguments: self.arguments.clone(), result: self.result.clone() }
    }
}
pub use misa_proto::presentation::{Presentation, Variant as PresentationVariant};

fn result_contract(text: &str) -> Result<QueryResultContract, PluginFault> {
    let contract: QueryResultContract = serde_json::from_str(text).map_err(|error| PluginFault::refused(format!("invalid result contract: {error}")))?;
    if let QueryResultContract::Data { schema } = &contract { schema.check().map_err(|error| PluginFault::refused(error.to_string()))?; }
    Ok(contract)
}

fn schema(text: &str) -> Result<Schema, PluginFault> {
    let schema: Schema = serde_json::from_str(text).map_err(|error| PluginFault::refused(format!("invalid query schema: {error}")))?;
    schema.check().map_err(|error| PluginFault::refused(error.to_string()))?;
    Ok(schema)
}

fn query_definition(definition: exports::misa::policy::policy_api::QueryDefinition) -> Result<QueryDefinition, PluginFault> {
    use exports::misa::policy::policy_api::QuerySource as Source;
    Ok(QueryDefinition {
        id: definition.id,
        contract: definition.contract,
        arguments: definition.arguments.iter().map(|text| schema(text)).collect::<Result<_, _>>()?,
        result: result_contract(&definition.output)?,
        source: match definition.source {
            Source::Read(contract) => QuerySource::Read { roots: contract.roots, schema: schema(&contract.schema)? },
            Source::Derived(inputs) => QuerySource::Derived { inputs: inputs.into_iter().map(|query| Ok(Query {
                id: query.id, args: query.args.iter().map(|arg| from_json(arg, "query dependency argument")).collect::<Result<_, _>>()?,
            })).collect::<Result<_, PluginFault>>()? },
        },
    })
}

fn validate_arguments(definition: &QueryDefinition, args: &[Value]) -> Result<(), PluginFault> {
    if definition.arguments.len() != args.len() {
        return Err(PluginFault::refused(format!("`{}` expects {} arguments", definition.id, definition.arguments.len())));
    }
    for (schema, value) in definition.arguments.iter().zip(args) {
        schema.validate(value).map_err(|error| PluginFault::refused(error.to_string()))?;
    }
    Ok(())
}

fn validate_local_graph(queries: &[QueryDefinition]) -> Result<(), PluginFault> {
    fn visit(id: &str, queries: &[QueryDefinition], active: &mut Vec<String>, done: &mut std::collections::BTreeSet<String>) -> Result<(), PluginFault> {
        if active.iter().any(|entry| entry == id) { return Err(PluginFault::refused(format!("query dependency cycle at `{id}`"))); }
        if done.contains(id) { return Ok(()); }
        // External dependencies are resolved by the complete owner registry.
        let Some(query) = queries.iter().find(|query| query.id == id) else { return Ok(()); };
        if active.len() >= 64 { return Err(PluginFault::refused("query dependencies too deep")); }
        active.push(id.into());
        if let QuerySource::Derived { inputs } = &query.source {
            for input in inputs { visit(&input.id, queries, active, done)?; }
        }
        active.pop(); done.insert(id.into()); Ok(())
    }
    let mut done = std::collections::BTreeSet::new();
    for query in queries { visit(&query.id, queries, &mut vec![], &mut done)?; }
    Ok(())
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
        let queries: Vec<QueryDefinition> = declared.queries.into_iter().map(query_definition).collect::<Result<_, _>>()?;
        let descriptor = Descriptor {
            tools: declared.tools.into_iter().map(|tool|misa_proto::tool::Binding{name:tool.name,description:tool.description,command:tool.command}).collect(),
            commands: declared.commands.into_iter().map(|command| Ok(CommandDefinition {
                id: command.id, input: schema(&command.input)?, event: command.event,
            })).collect::<Result<_, PluginFault>>()?,
            bindings: declared.bindings.into_iter().map(|binding| Ok(misa_proto::invocation::Binding {
                id: binding.id, binding: misa_proto::invocation::ActionBinding {
                    command: binding.command,
                    bound: serde_json::from_str(&binding.bound).map_err(|error| PluginFault::refused(format!("invalid bound arguments: {error}")))?,
                    inputs: serde_json::from_str(&binding.inputs).map_err(|error| PluginFault::refused(format!("invalid binding inputs: {error}")))?,
                },
            })).collect::<Result<_, PluginFault>>()?,
            presentations: declared.presentations.into_iter().map(|presentation| Ok(Presentation {
                id: format!("plugin.{}.{}", declared.id, presentation.id), title: presentation.title,
                variants: presentation.variants.into_iter().map(|variant| Ok(PresentationVariant {
                    id: variant.id, requirements: variant.requirements,
                    member: misa_proto::observation::Member {
                        contract: queries.iter().find(|query| query.id == variant.query.id).ok_or_else(|| PluginFault::refused("presentation names undeclared query"))?.contract.clone(),
                        query: Query { id: variant.query.id, args: variant.query.args.iter().map(|arg| from_json(arg, "presentation query argument")).collect::<Result<_, _>>()? },
                        encoding: misa_proto::observation::Encoding::Document, optional: false,
                    },
                })).collect::<Result<_, PluginFault>>()?,
            })).collect::<Result<_, PluginFault>>()?,
            id: declared.id,
            version: declared.version,
            events: declared.events,
            queries,
            effects: declared.effects,
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
        for kind in &self.descriptor.events {
            if kind.is_empty() {
                return Err(PluginFault::refused(format!(
                    "`{}` declares an unnamed event or query",
                    self.descriptor.id
                )));
            }
        }
        let mut ids = std::collections::BTreeSet::new();
        for query in &self.descriptor.queries {
            query.export().check().map_err(|fault| PluginFault::refused(fault.message))?;
            if !plain_id(&query.id) || !ids.insert(&query.id) {
                return Err(PluginFault::refused(format!("Invalid or duplicate query `{}`", query.id)));
            }
            if let QuerySource::Read { roots, .. } = &query.source {
                plain_roots(&query.id, roots)?;
            }
            if let QuerySource::Derived { inputs } = &query.source {
                for input in inputs {
                    if !plain_id(&input.id) { return Err(PluginFault::refused("Invalid dependency name")); }
                    if let Some(target) = self.descriptor.queries.iter().find(|target| target.id == input.id) {
                        validate_arguments(target, &input.args)?;
                    }
                }
            }
        }
        validate_local_graph(&self.descriptor.queries)?;
        let prefix = format!("{}.", self.descriptor.id);
        let event_prefix = format!("plugin.{}.", self.descriptor.id);
        let mut commands = std::collections::BTreeSet::new();
        for command in &self.descriptor.commands {
            if !plain_id(&command.id) || !command.id.starts_with(&prefix) || !commands.insert(&command.id)
                || !command.event.starts_with(&event_prefix) || !self.descriptor.events.contains(&command.event) {
                return Err(PluginFault::refused("Invalid command or undeclared command event"));
            }
            command.export().validate().map_err(|fault| PluginFault::refused(fault.message))?;
        }
        let mut bindings = std::collections::BTreeSet::new();
        let mut tools=std::collections::BTreeSet::new();
        for tool in &self.descriptor.tools {
            if tool.name.len()>64 || !tool.name.starts_with(&format!("{}_",self.descriptor.id.replace('.',"_"))) || !tool.name.bytes().all(|byte|byte.is_ascii_alphanumeric() || b"_-".contains(&byte)) || tool.description.is_empty() || !tools.insert(&tool.name) || !commands.contains(&tool.command) {
                return Err(PluginFault::refused("Invalid tool or undeclared tool command"));
            }
        }
        for binding in &self.descriptor.bindings {
            if !plain_id(&binding.id) || !binding.id.starts_with(&prefix) || !bindings.insert(&binding.id) || !commands.contains(&binding.binding.command) {
                return Err(PluginFault::refused("Invalid binding or undeclared bound command"));
            }
            let command = self.descriptor.commands.iter().find(|command| command.id == binding.binding.command).expect("declared command was checked");
            binding.binding.validate_for(&command.export()).map_err(|fault| PluginFault::refused(fault.message))?;
        }
        let exports = self.descriptor.queries.iter().map(|query| (query.id.clone(), query.export())).collect();
        misa_proto::presentation::validate_catalog(&self.descriptor.presentations, &exports)
            .map_err(|fault| PluginFault::refused(fault.message))?;
        plain_roots(&self.descriptor.id, &self.descriptor.roots)
    }

    /// The installing composition grants reads separately from the guest's requests.
    pub fn authorize_reads(&self, allowed_roots: &[String]) -> Result<(), PluginFault> {
        for query in &self.descriptor.queries {
            if let QuerySource::Read { roots, .. } = &query.source {
                for root in roots {
                    if !allowed_roots.contains(root) { return Err(PluginFault::refused(format!("query `{}` has no read grant for `{root}`", query.id))); }
                }
            }
        }
        Ok(())
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
    /// Guest failures remain query faults, never successful values with another schema.
    pub fn subscriptions(self: &Arc<Self>) -> Vec<(String, Subscription)> {
        self.descriptor
            .queries
            .iter()
            .map(|definition| {
                let plugin = self.clone();
                let query_name = definition.id.clone();
                let subscription = match &definition.source {
                    QuerySource::Read { .. } => misa_reframe::try_read_query(move |db, query, previous| {
                        plugin.query(&query_name, &query.args, &[], Some(db), previous)
                            .map_err(|fault| misa_reframe::Fault::new(fault.code, fault.message))
                    }),
                    QuerySource::Derived { inputs } => misa_reframe::try_derived_query(misa_reframe::Inputs::Fixed(inputs.clone()), move |inputs, query, previous| {
                        plugin.query(&query_name, &query.args, inputs, None, previous)
                            .map_err(|fault| misa_reframe::Fault::new(fault.code, fault.message))
                    }),
                };
                (definition.id.clone(), subscription)
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
        db: Option<&Value>,
        previous: Option<&Value>,
    ) -> Result<Value, PluginFault> {
        let definition = self.descriptor.queries.iter().find(|definition| definition.id == id)
            .ok_or_else(|| PluginFault::refused(format!("undeclared query `{id}`")))?;
        validate_arguments(definition, args)?;
        let read_data = match &definition.source {
            QuerySource::Read { roots, schema } => {
                if !inputs.is_empty() { return Err(PluginFault::refused("read query cannot receive derived inputs")); }
                let db = db.ok_or_else(|| PluginFault::refused("read query requires its data contract"))?;
                let selected = Value::Map(Arc::new(roots.iter().filter_map(|root| db.get(root).map(|value| (root.clone(), value.clone()))).collect()));
                schema.validate(&selected).map_err(|error| PluginFault::refused(error.to_string()))?;
                Some(to_json(&selected, "declared read data")?)
            }
            QuerySource::Derived { inputs: declared } => {
                if db.is_some() || inputs.len() != declared.len() { return Err(PluginFault::refused("derived query requires only its declared inputs")); }
                None
            }
        };
        let request = QueryRequest {
            id: id.to_string(),
            args: args.iter().map(|arg| to_json(arg, "a query argument")).collect::<Result<Vec<_>, _>>()?,
        };
        let inputs = inputs
            .iter()
            .map(|input| to_json(input, "a query input"))
            .collect::<Result<Vec<_>, _>>()?;
        let previous = match previous {
            Some(value) => optional_json(value, "the previous answer")?,
            None => None,
        };
        let mut inner = self.lock()?;
        let Guest { store, bindings } = &mut *inner;
        fuelled(store)?;
        let answered = bindings
            .interface0
            .call_query(store, &request, &inputs, read_data.as_deref(), previous.as_deref())
            .map_err(|error| PluginFault::trap(&self.descriptor.id, error))?
            .map_err(|fault| PluginFault::guest(&fault))?;
        let value = from_json(&answered, "a query answer")?;
        match &definition.result {
            QueryResultContract::Data { schema } => schema.validate(&value).map_err(|error| PluginFault::refused(format!("query `{id}` result: {error}")))?,
            QueryResultContract::Document {} => { self.document(&value)?; }
        }
        Ok(value)
    }

    /// A view tree, converted and validated.
    ///
    /// The validation is [`misa_proto::view::validate`] — the same function a session's own
    /// view is held to — so a plugin's tree is not merely *its* tree: it is a tree every
    /// client may assume things about, or it is a fault.
    pub fn document(&self, value: &Value) -> Result<Node, PluginFault> {
        let node: Node = serde_json::from_str(&to_json(value, "semantic document")?)
            .map_err(|error| PluginFault::refused(format!("invalid semantic document: {error}")))?;
        named(&node)?;
        offered(&self.descriptor.id, &node, &self.descriptor.bindings.iter().map(|binding| binding.id.clone()).collect::<Vec<_>>())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use exports::misa::policy::policy_api::{Op as GuestOp, Patch as GuestPatch};

    #[test]
    fn result_contracts_and_argument_schemas_are_validated() {
        assert!(result_contract(r#"{"kind":"document"}"#).is_ok());
        assert!(result_contract(r#"{"kind":"data","schema":{"type":"choice","values":[]}}"#).is_err());
        assert!(result_contract(r#"{"kind":"document","schema":{"type":"int"}}"#).is_err());
        let query = QueryDefinition { id: "test".into(), contract: "test@1".into(), arguments: vec![Schema::Int], result: QueryResultContract::Document {},
            source: QuerySource::Derived { inputs: vec![] } };
        assert!(validate_arguments(&query, &[]).is_err());
        assert!(validate_arguments(&query, &[Value::str("1")]).is_err());
        assert!(validate_arguments(&query, &[Value::Int(1)]).is_ok());
    }

    #[test]
    fn presentation_variants_use_declared_preference_order_and_exact_capabilities() {
        let variant = |id: &str, requirements: Vec<String>| PresentationVariant {
            id: id.into(), requirements, member: misa_proto::observation::Member {
                query: Query::new(format!("test.{id}")), contract: "test@1".into(), encoding: misa_proto::observation::Encoding::Document, optional: false,
            },
        };
        let presentation = Presentation { id: "test".into(), title: "Test".into(), variants: vec![
            variant("rich", vec!["images@1".into()]), variant("basic", vec![]),
        ] };
        assert_eq!(presentation.select(&[]).unwrap().id, "basic");
        assert_eq!(presentation.select(&["images@1".into()]).unwrap().id, "rich");
        assert_eq!(presentation.select(&["images@2".into()]).unwrap().id, "basic");
    }

    #[test]
    fn local_dependency_cycles_fail_before_running_a_guest() {
        let query = |id: &str, dependency: &str| QueryDefinition {
            id: id.into(), contract: format!("{id}@1"), arguments: vec![],
            result: QueryResultContract::Data { schema: Schema::Int },
            source: QuerySource::Derived { inputs: vec![Query::new(dependency)] },
        };
        assert!(validate_local_graph(&[query("a", "b"), query("b", "a")]).unwrap_err().message.contains("cycle"));
        // External names are checked once the installing owner has composed all definitions.
        assert!(validate_local_graph(&[query("a", "external")]).is_ok());
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

}
