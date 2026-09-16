//! A plugin, over a real component: the whole path from an event to patches and effects.
//!
//! Behind `guest-fixture`, because it builds `wit/guest` for `wasm32-unknown-unknown`, and that
//! needs a rustc with the target — the shell has one, and `cargo test --workspace` must not
//! start a nested cargo build. It is the same bargain `misa-skia`'s `paint` feature makes.
//!
//! ```sh
//! cargo test -p misa-plugin --features guest-fixture
//! ```
//!
//! `MISA_PLUGIN_FIXTURE=/path/to/policy.component.wasm` uses a component somebody already made
//! — the README's two commands — instead of building one, which is what to do when the
//! interesting answer is "is the host wrong, or is the guest?".

#![cfg(feature = "guest-fixture")]

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;

use misa_kernel::{LocalKernel, Provider, ScriptedProvider};
use misa_plugin::{ PLUGIN_PRIORITY, Plugin, PluginFault};
use misa_reframe::{Effect, Event, Interpreter, Loop, Query, Registry};
use misa_proto::wire::Intent;
use misa_session::{Reading, Runtime};
use misa_value::Value;

/// An interpreter that accepts exactly the effect kinds a test names.
///
/// Which is the whole of what a composition is, from a plugin's side: a list of effect kinds
/// somebody decided this session can run.
struct Accepts(Vec<&'static str>);

impl Interpreter for Accepts {
    fn accepts(&self, effect: &Effect) -> Result<(), String> {
        if self.0.contains(&effect.kind.as_str()) {
            return Ok(());
        }
        Err(format!("`{}` is not something this composition does", effect.kind))
    }
}

/// The fixture component, built if it is not already lying around.
fn component() -> Vec<u8> {
    if let Ok(path) = std::env::var("MISA_PLUGIN_FIXTURE") {
        return std::fs::read(&path).expect("the component MISA_PLUGIN_FIXTURE names");
    }
    build_component("guest")
}
fn build_component(package:&str)->Vec<u8> {
    let next = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("the workspace root")
        .to_path_buf();
    let guest = next.join("wit").join(package);
    let target = std::env::temp_dir().join(format!("misa-guest-fixture-{}", std::process::id()));
    // The guest is built the way the README says, with the outer build's target settings taken
    // out of the environment: a `CARGO_TARGET_DIR` or a `RUSTFLAGS` meant for a host build is
    // what makes a nested cargo build produce something that is not a wasm module.
    let status = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["build", "--release", "--target", "wasm32-unknown-unknown"])
        .arg("--manifest-path")
        .arg(guest.join("Cargo.toml"))
        .arg("--target-dir")
        .arg(&target)
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .status()
        .expect("cargo, and a rustc with the wasm32-unknown-unknown target: run this from the shell");
    assert!(status.success(), "the fixture did not build");
    let module = std::fs::read(
        target.join(format!("wasm32-unknown-unknown/release/policy_{package}.wasm")),
    )
    .expect("the module the guest built");

    // The guest's `component-type` section is what says which world the module implements — the
    // same thing `wasm-tools component new` reads, through the same library.
    wit_component::ComponentEncoder::default()
        .module(&module)
        .expect("a core module")
        .encode()
        .expect("a component")
}

/// The fixture, built once for the whole binary: a nested cargo build per test would make this
/// file take two minutes to say the same thing.
fn component_once() -> &'static [u8] {
    static COMPONENT: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    COMPONENT.get_or_init(component)
}

fn loaded() -> Arc<Plugin> {
    Arc::new(Plugin::load(component_once()).expect("the fixture loads"))
}

#[test]
fn a_plugin_tells_the_host_what_it_is_before_it_runs() {
    let plugin = loaded();
    let descriptor = plugin.descriptor();
    assert_eq!(descriptor.id, "policy.guest");
    assert_eq!(descriptor.version, "0.2.0");
    assert_eq!(descriptor.queries.len(), 4);
    assert_eq!(descriptor.presentations.len(), 1);
    assert_eq!(descriptor.presentations[0].select(&[]).unwrap().member.query.id, "policy.guest.document");
    // The root it declared is the one it writes into: the composition makes it, and that is
    // what makes its patch land.
    assert_eq!(plugin.descriptor().roots, vec!["guest".to_string()]);
}

#[test]
fn what_a_plugin_declares_is_checked_against_the_composition_that_would_run_it() {
    let plugin = loaded();
    // The composition has the effect it asked for.
    plugin
        .validate(&Accepts(vec!["kernel.log.append"]))
        .expect("a composition that can do what it asked for");
    // And one that does not refuses it, naming the plugin and the effect — at install, before
    // an event ever reaches it.
    let fault = plugin.validate(&Accepts(vec![])).expect_err("refused");
    assert_eq!(fault.code, "plugin.refused");
    assert!(fault.message.contains("policy.guest"), "{}", fault.message);
    assert!(fault.message.contains("kernel.log.append"), "{}", fault.message);
}

#[test]
fn a_plugin_is_configured_with_what_the_composition_gives_it() {
    let plugin = loaded();
    plugin
        .configure(&[("greeting".into(), "hello".into())])
        .expect("a plugin that takes options");
}

#[test]
fn an_event_becomes_patches_and_effects_the_loop_applies() {
    let plugin = loaded();
    plugin.validate(&Accepts(vec!["kernel.log.append"])).expect("validated");
    let mut registry = Registry::new();
    for (kind, handler) in plugin.handlers() {
        registry = registry.on(kind, PLUGIN_PRIORITY, handler);
    }
    for (name, subscription) in plugin.subscriptions() {
        registry = registry.subscription(name, subscription);
    }
    let mut loop_ = Loop::new(
        Arc::new(registry),
        Arc::new(Accepts(vec!["kernel.log.append"])),
        Value::map([
            ("session", Value::map([("id", Value::str("demo"))])),
            // A plugin writes into state the composition declared. `guest` is that
            // declaration here: a path whose parent does not exist is a fault, which is the
            // composition's boundary showing through rather than a plugin inventing a root.
            ("guest", Value::map([])),
        ]),
    );

    let outcome = loop_.dispatch(Event::new("intent/prompt").with("text", Value::str("hello")));
    assert!(outcome.committed(), "{:?}", outcome.faults);
    // The plugin said it handled `intent/prompt`, so it was called for it.
    assert!(outcome.handled.contains(&"intent/prompt".to_string()), "{:?}", outcome.handled);

    // Its patch is in the database the loop committed: json in, a value out. `Value::get` is
    // one key, so the path is walked the way the patch walked it.
    let patched = loop_
        .db()
        .get("guest")
        .and_then(|guest| guest.get("turns"))
        .cloned()
        .expect("the plugin's patch landed");
    assert!(patched.get("seen").is_some(), "the patch is the json the plugin sent: {patched:?}");

    // And its effect is queued for the interpreter, in the loop's own shape.
    let effect = outcome.effects.iter().find(|effect| effect.kind == "kernel.log.append").expect("the plugin's effect");
    assert_eq!(effect.field("kind"), "guest.policy.guest.note");
    assert_eq!(effect.field("conversation"), "guest");
}

#[test]
fn a_plugin_that_refuses_an_event_rolls_its_transaction_back() {
    let plugin = loaded();
    let mut registry = Registry::new();
    for (kind, handler) in plugin.handlers() {
        registry = registry.on(kind, PLUGIN_PRIORITY, handler);
    }
    let mut loop_ = Loop::new(
        Arc::new(registry),
        Arc::new(Accepts(vec!["kernel.log.append"])),
        Value::map([("guest", Value::map([("turns", Value::str("before"))]))]),
    );
    let rev = loop_.rev();

    // It declared that it handles `intent/cancel`, so it is called for it — and its own answer
    // is a fault. That is the shape a guest's `Err` takes on this side: a transaction that
    // leaves no trace and a reason somebody can read.
    let outcome = loop_.dispatch(Event::new("intent/cancel"));
    let faults = outcome.as_faults();
    assert_eq!(faults.len(), 1, "{faults:?}");
    assert_eq!(faults[0].code, "handler");
    assert!(faults[0].message.contains("policy.guest"), "{}", faults[0].message);
    assert!(faults[0].message.contains("not an event this plugin handles"), "{}", faults[0].message);
    assert_eq!(loop_.rev(), rev, "nothing committed");
    assert_eq!(
        loop_.db().get("guest").and_then(|guest| guest.get("turns")).and_then(Value::as_str),
        Some("before")
    );
}

#[test]
fn a_query_is_a_subscription_the_loop_can_read() {
    let plugin = loaded();
    let db = Value::map([("session", Value::map([("id", Value::str("demo"))]))]);
    let answered = plugin.query("policy.guest.turns", &[], &[], None, None).expect("an answer");
    assert_eq!(answered.get("answered").and_then(Value::as_str), Some("policy.guest.turns"));

    // The same call through the loop, which is how a client would reach it: a subscription
    // whose value is the plugin's answer.
    let mut registry = Registry::new();
    for (name, subscription) in plugin.subscriptions() {
        registry = registry.subscription(name, subscription);
    }
    let mut loop_ = Loop::new(Arc::new(registry), Arc::new(Accepts(vec![])), db);
    let value = loop_.query(&misa_reframe::Query::new("policy.guest.turns")).expect("a value");
    assert_eq!(value.get("answered").and_then(Value::as_str), Some("policy.guest.turns"));
}

#[test]
fn read_contracts_isolate_data_and_derived_queries_receive_only_dependencies() {
    let plugin = loaded();
    assert!(plugin.authorize_reads(&[]).is_err());
    plugin.authorize_reads(plugin.roots()).unwrap();
    let db = Value::map([
        ("guest", Value::map([("count", Value::Int(3))])),
        ("private", Value::str("must not cross query boundary")),
    ]);
    let mut registry = Registry::new();
    for (id, definition) in plugin.subscriptions() { registry = registry.subscription(id, definition); }
    registry.validate().unwrap();
    let mut scope = misa_reframe::Scope::new();
    let result = scope.evaluate(&db, &registry, &Query::new("policy.guest.copy")).unwrap().unwrap();
    assert_eq!(result, Value::map([("guest", Value::map([("count", Value::Int(3))]))]));
    assert!(plugin.query("policy.guest.turns", &[Value::Int(1)], &[], None, None).is_err());
    assert!(plugin.query("policy.guest.copy", &[], &[], Some(&db), None).is_err());
    assert!(plugin.query("undeclared", &[], &[], None, None).is_err());
    let invalid = Value::map([("guest", Value::Int(3))]);
    assert!(plugin.query("policy.guest.state", &[], &[], Some(&invalid), None).is_err());
}

#[test]
fn guest_query_faults_remain_faults_through_the_scope() {
    let plugin = loaded();
    let mut registry = Registry::new();
    for (id, definition) in plugin.subscriptions() { registry = registry.subscription(id, definition); }
    let mut scope = misa_reframe::Scope::new();
    let query = Query::new("policy.guest.document");
    let refusing = Value::map([("guest", Value::map([("refuse", Value::Bool(true))]))]);
    let fault = scope.evaluate(&refusing, &registry, &query).unwrap_err();
    assert_eq!(fault.code, "policy.guest.no-view");
    assert!(scope.current(&query).is_none());
    assert!(scope.evaluate(&Value::map([]), &registry, &query).unwrap().is_some());
}

#[test]
fn a_view_is_built_the_way_any_other_tree_is() {
    let plugin = loaded();
    let db = Value::Null;
    let value = plugin.query("policy.guest.document", &[], &[], Some(&db), None).expect("document query");
    let tree = plugin.document(&value).expect("a tree");
    assert_eq!(tree.id, "guest");
    assert_eq!(tree.role, "guest.panel");
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].id, "guest.summary");
    assert_eq!(tree.children[0].role, "guest.summary");
    // It is the protocol's own tree, so the protocol's own validator accepts it — which is the
    // only promise a plugin's view makes.
    misa_proto::view::validate(&tree).expect("a tree a client may be sent");
}

#[test]
fn a_component_that_is_not_a_policy_plugin_is_refused_rather_than_run() {
    let fault: PluginFault = Plugin::load(b"not a component at all").expect_err("refused");
    assert_eq!(fault.code, "plugin.host");
    assert!(fault.message.contains("component"), "{}", fault.message);
}

#[tokio::test]
async fn a_plugin_presentation_is_selected_and_placed_by_a_client() {
    // A client discovers the plugin document through the catalog and chooses its
    // baseline variant independently of the canonical conversation.
    let plugin = loaded();
    let (runtime, _kernel) = session(contribution(&plugin));
    let node = view(&runtime);
    misa_proto::view::validate(&node).expect("a tree a client may be sent");

    assert!(misa_proto::view::find(&runtime.view().unwrap(), "guest").is_none(), "plugin leaked into canonical conversation");

    // Each selected document keeps its own node identity space and bindings.
    let root = misa_proto::view::find(&node, "guest").expect("the plugin document root");
    assert_eq!(root.actions.len(), 1);
    assert_eq!(root.actions[0].id, "policy.guest.refresh");
    assert_eq!(root.actions[0].label.as_deref(), Some("Refresh"));

    // The plugin receives only its declared read data independently of the client.
    let text = misa_render::to_plain(&misa_render::render(&node, &misa_render::Theme::plain(), 100));
    assert!(text.contains("drawn for"), "{text}");
    assert!(text.contains("semantic"), "semantic plugin view: {text}");
}

#[tokio::test]
async fn an_action_from_a_plugins_tree_reaches_the_plugin() {
    // Bindings prepare inputs; the owner independently validates the invocation.
    let plugin = loaded();
    let (runtime, _kernel) = session(contribution(&plugin));
    let _ = view(&runtime);

    use misa_proto::invocation::{Invocation, Outcome};
    use misa_protocol::invocation::{CallContext, Dispatcher};
    let dispatcher = Dispatcher::new(CallContext { principal: "paired-test".into(), connection: 1 }, 4, Default::default(), Default::default());
    let binding = &plugin.descriptor().bindings[0].binding;
    let mut invocation = Invocation { id: 1, scope: runtime.scope(), command: binding.command.clone(), input: Value::map([("confirm", Value::str("yes"))]) };
    assert!(matches!(dispatcher.dispatch(runtime.as_ref(), invocation.clone()).await.outcome, Outcome::Rejected { .. }));
    invocation.input = binding.prepare(&std::collections::BTreeMap::new()).unwrap();
    assert!(matches!(dispatcher.dispatch(runtime.as_ref(), invocation).await.outcome, Outcome::Accepted { .. }));
    // Legacy node action traffic cannot invoke the installed command implicitly.
    assert!(!runtime.intent(Intent::Action { node: "plugin.policy.guest.main.guest".into(), action: "policy.guest.refresh".into(), args: Value::Null, fields: vec![] }).is_empty());

    wait_for_guest(&runtime, "acted").await;

    let state = match runtime.read(&Query::new("policy.guest.state")).expect("an answer") {
        Reading::Data(value) => value,
        Reading::View(_) => panic!("a plugin's query answered with a view"),
    };
    assert_eq!(
        state.get("guest").and_then(|guest| guest.get("acted")).and_then(Value::as_bool),
        Some(true),
        "the plugin was not told: {state:?}"
    );
}

#[tokio::test]
async fn a_plugin_that_runs_away_is_stopped_by_its_budget_and_the_session_still_answers() {
    // A guest that never returns is the failure mode a host has to design away rather than hope
    // about. The budget is fuel, so it costs no threads and no timers, and what a person sees is a
    // sentence where the widget was.
    let plugin = loaded();
    let mut contribution = contribution(&plugin);
    contribution.roots.iter_mut().find(|(name, _)| name == "guest").unwrap().1 = Value::map([("spin", Value::Bool(true))]);
    let (runtime, _kernel) = session(contribution);
    let node = view(&runtime);
    misa_proto::view::validate(&node).expect("a tree a client may be sent");

    let text = misa_render::to_plain(&misa_render::render(&node, &misa_render::Theme::plain(), 100));
    assert!(text.contains("budget"), "{text}");
    assert!(text.contains("policy.guest"), "{text}");
    // And the rest of the session is untouched: a runaway plugin is not a session that stops
    // answering.
    assert!(misa_proto::view::find(&node, "composer").is_some());
}

#[tokio::test]
async fn a_plugin_that_refuses_to_present_is_a_sentence_too() {
    let plugin = loaded();
    let mut contribution = contribution(&plugin);
    contribution.roots.iter_mut().find(|(name, _)| name == "guest").unwrap().1 = Value::map([("refuse", Value::Bool(true))]);
    let (runtime, _kernel) = session(contribution);
    let node = view(&runtime);
    let text = misa_render::to_plain(&misa_render::render(&node, &misa_render::Theme::plain(), 100));
    // The guest's own words, in place of its tree.
    assert!(text.contains("cannot draw that"), "{text}");
}

/// Wait for the kernel to have entries in a conversation, the way a client waits: by looking.
///
/// Async, and awaited: an effect is executed by a task of the session's own, and a test whose
/// runtime is one thread that blocks is a test that starves the writer it is waiting for.
async fn wait_for_entries(kernel: &misa_kernel::LocalKernel, conversation: &str) -> Vec<misa_kernel::LogEntry> {
    for _ in 0..200 {
        let entries = kernel.store().load(conversation,0,100_000).expect("read the requested log directly");
        if !entries.is_empty() {
            return entries;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("no entry arrived for `{conversation}`; recorded entries: {:?}",kernel.entries());
}

/// Wire a plugin the way a daemon does: handlers, queries, affordances, presentation, and the
/// state roots it declared.
fn contribution(plugin: &Arc<Plugin>) -> misa_session::Contribution {
    let mut contribution = misa_session::Contribution::new();
    for (kind, handler) in plugin.handlers() {
        contribution = contribution.with_handler(kind, PLUGIN_PRIORITY, handler);
    }
    for (name, subscription) in plugin.subscriptions() {
        contribution = contribution.with_subscription(name, subscription);
    }
    for definition in &plugin.descriptor().queries {
        contribution = contribution.export_query(definition.export());
    }
    for command in &plugin.descriptor().commands {
        let registration = match &command.request {
            Some(form) => misa_session::commands::CommandRegistration::input_event(&command.id, command.input.clone(), &command.event, form.clone()).unwrap(),
            None => misa_session::commands::CommandRegistration::event(&command.id, command.input.clone(), &command.event),
        };
        contribution = contribution.with_command(registration);
    }
    for tool in &plugin.descriptor().tools { contribution=contribution.with_tool(tool.clone()); }
    for binding in &plugin.descriptor().bindings { contribution = contribution.with_binding(binding.clone()); }
    plugin.authorize_reads(plugin.roots()).expect("granted own roots");
    for presentation in &plugin.descriptor().presentations {
        contribution = contribution.with_presentation(presentation.clone());
    }
    for root in plugin.roots() {
        contribution = contribution.with_root(root, Value::map([])).expect("a root of its own");
    }
    contribution
}

/// A session running one plugin, with a kernel a test can look at.
fn session(contribution: misa_session::Contribution) -> (Arc<Runtime>, Arc<LocalKernel>) {
    let provider: Arc<dyn Provider> = ScriptedProvider::always("an answer");
    let kernel = Arc::new(LocalKernel::new(provider));
    let runtime = Runtime::start_with(
        "demo",
        "a demo session",
        None,
        kernel.clone(),
        "scripted",
        "scripted-1",
        Value::Null,
        contribution,
    );
    (runtime, kernel)
}

/// The view a client would draw.
fn view(runtime: &Runtime) -> misa_proto::view::Node {
    use misa_proto::observation::{Selection, Content};
    let Reading::Data(value) = runtime.read(&Query::new(misa_proto::presentation::CATALOG)).unwrap() else { panic!() };
    let catalog: Vec<misa_proto::presentation::Presentation> = serde_json::from_value(serde_json::to_value(value).unwrap()).unwrap();
    let members = catalog.into_iter().filter(|presentation| presentation.id != "status").map(|presentation| {
        let mut member = presentation.select(&[]).unwrap().member.clone();
        member.optional = presentation.id != "conversation";
        (presentation.id, member)
    }).collect();
    let snapshot = runtime.read_selection(&Selection { scope: runtime.scope(), members }).unwrap();
    let mut root = misa_proto::Node::section("client.composition").id("client.composition");
    for (id, content) in snapshot.members {
        match content {
            Content::Document(document) => root.children.push(document.tree),
            Content::Unavailable(fault) => root.children.push(misa_proto::Node::new("error", misa_proto::view::Kind::Status { text: format!("{id}: {}", fault.message) }).id(id)),
            _ => panic!("presentation must be a document"),
        }
    }
    root
}

/// A session spawns the tasks that carry kernel reports back, so this one is async like the
/// rest of the loop's tests.
#[tokio::test]
async fn a_session_runs_what_a_plugin_declared() {
    // The whole point, end to end: a daemon loads a component, a session registers what it
    // declared, and a prompt in that session reaches wasm — whose patch lands in the state the
    // composition made for it and whose effect is run by the session's own interpreter.
    let plugin = loaded();
    plugin.validate(&misa_session::AcceptedEffects).expect("this session can run it");

    let (runtime, kernel) = session(contribution(&plugin));

    let faults = runtime.intent(Intent::Prompt { text: "hello".into(), attachments: vec![] });
    assert!(faults.is_empty(), "{faults:?}");

    wait_for_guest(&runtime, "turns").await;

    // The plugin's patch landed in the root the composition made for it. It is read back the way
    // a client would: through the plugin's own query, which answers with the state it was handed.
    let state = match runtime
        .read(&Query::new("policy.guest.state"))
        .expect("an answer from the plugin")
    {
        Reading::Data(value) => value,
        Reading::View(_) => panic!("a plugin's query answered with a view"),
    };
    let turns = state
        .get("guest")
        .and_then(|guest| guest.get("turns"))
        .expect("the plugin's root holds what it wrote");
    assert!(turns.get("seen").is_some(), "{turns:?}");

    // And its effect reached the kernel: `kernel.log.append` was in this session's accepted set,
    // so the interpreter ran it and the daemon's log has the entry the plugin asked for.
    let entries = wait_for_entries(&kernel, "guest").await;
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0].kind, "guest.policy.guest.note");
    assert_eq!(entries[0].data.as_str(), Some("seen a prompt"));
}

async fn wait_for_guest(runtime: &Runtime, key: &str) {
    for _ in 0..500 {
        if let Reading::Data(state) = runtime.read(&Query::new("policy.guest.state")).unwrap() {
            if state.get("guest").and_then(|guest| guest.get(key)).is_some() { return; }
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("durable plugin patch {key} was not acknowledged");
}

#[tokio::test]
async fn pet_command_and_model_tool_share_state_while_presentations_are_local_choices() {
    use misa_proto::{invocation::{Invocation,Outcome},observation::{Content,Selection}};
    use misa_protocol::invocation::{CallContext,Dispatcher};
    let plugin=Arc::new(Plugin::load(&build_component("pet")).unwrap());
    plugin.validate(&misa_session::AcceptedEffects).unwrap();
    let kernel=Arc::new(LocalKernel::new(ScriptedProvider::new([
        misa_kernel::Turn::call("pet_feed",Value::map([("amount",Value::Int(3))]),misa_kernel::Turn::say("Pet fed")),
    ])));
    let runtime=Runtime::start_with("pet-test","Pet",None,kernel,"scripted","scripted-1",Value::Null,contribution(&plugin));
    let dispatcher=Dispatcher::new(CallContext{principal:"pet-owner".into(),connection:1},8,Default::default(),Default::default());
    let call=|id,command:&str,input|Invocation{id,scope:runtime.scope(),command:command.into(),input};
    assert!(matches!(dispatcher.dispatch(runtime.as_ref(),call(1,"pet.feed",Value::map([("amount",Value::Int(2))]))).await.outcome,Outcome::Accepted{..}));
    let treats=||match runtime.read(&Query::new("pet.state")).unwrap(){Reading::Data(value)=>value.get("treats").and_then(Value::as_i64).unwrap(),_=>panic!()};
    tokio::time::timeout(std::time::Duration::from_secs(5),async{while treats()!=2{tokio::task::yield_now().await;}}).await.expect("admitted plugin patch becomes durable state");
    assert!(matches!(dispatcher.dispatch(runtime.as_ref(),call(2,"session.prompt",Value::map([("text",Value::str("feed pet")),("attachments",Value::list([]))]))).await.outcome,Outcome::Accepted{..}));
    tokio::time::timeout(std::time::Duration::from_secs(5),async{while treats()!=5{tokio::task::yield_now().await;}}).await.unwrap();
    let presentation=&plugin.descriptor().presentations[0];
    assert_eq!(presentation.select(&[]).unwrap().id,"portable");
    assert_eq!(presentation.select(&["semantic.meter@1".into()]).unwrap().id,"rich");
    let preferences=misa_client::composition::Preferences::default();
    assert!(preferences.resolve(&plugin.descriptor().presentations,&[],&[]).unwrap().is_empty());
    let mut preferences=preferences;
    preferences.set(&plugin.descriptor().presentations,&[],&presentation.id,misa_client::composition::Choice::Auto).unwrap();
    let selected=preferences.resolve(&plugin.descriptor().presentations,&[],&[]).unwrap();
    let snapshot=runtime.read_selection(&Selection{scope:runtime.scope(),members:selected}).unwrap();
    let Content::Document(document)=&snapshot.members[&presentation.id] else{panic!()};
    assert!(misa_render::to_plain(&misa_render::render(&document.tree,&misa_render::Theme::plain(),80)).contains("5 treats"));
    assert!(misa_proto::view::find(&runtime.view().unwrap(),"pet").is_none());
    preferences.set(&plugin.descriptor().presentations,&[],&presentation.id,misa_client::composition::Choice::Hidden).unwrap();
    assert!(preferences.resolve(&plugin.descriptor().presentations,&[],&[]).unwrap().is_empty());
}

#[tokio::test]
async fn guest_declared_form_keeps_model_tool_pending_until_owner_response_is_durable() {
    use misa_proto::{invocation::{Invocation,Outcome},observation::{Content,Encoding,Member,Selection}};
    use misa_protocol::invocation::{CallContext,Dispatcher};
    use std::collections::BTreeMap;
    let plugin=Arc::new(Plugin::load(&build_component("pet")).unwrap());
    plugin.validate(&misa_session::AcceptedEffects).unwrap();
    let kernel=Arc::new(LocalKernel::new(ScriptedProvider::new([misa_kernel::Turn::call("pet_ask_feed",Value::map([]),misa_kernel::Turn::say("Owner fed the pet"))])));
    let runtime=Runtime::start_with("pet-input","Pet input",None,kernel.clone(),"scripted","test",Value::Null,contribution(&plugin));
    let context=CallContext{principal:"pet-owner".into(),connection:1};
    let dispatcher=Dispatcher::new(context.clone(),8,Default::default(),Default::default());
    let invoke=|id,command:&str,input|Invocation{id,scope:runtime.scope(),command:command.into(),input};
    let Outcome::Accepted{operation:prompt}=dispatcher.dispatch(runtime.as_ref(),invoke(1,"session.prompt",Value::map([("text",Value::str("ask me how many treats"))]))).await.outcome else{panic!()};
    let request=tokio::time::timeout(std::time::Duration::from_secs(5),async{loop{
        let Reading::Data(value)=runtime.read(&Query::new("requests.summary")).unwrap()else{panic!()};
        if let Some(request)=value.as_list().unwrap().iter().find(|request|request.get("kind").and_then(Value::as_str)==Some("form")){break request.clone()}
        tokio::task::yield_now().await;
    }}).await.unwrap();
    assert!(!kernel.entries().iter().any(|entry|entry.kind=="tool_result"));
    let id=request.get("id").and_then(Value::as_str).unwrap();
    let selection=Selection{scope:runtime.scope(),members:BTreeMap::from([("request".into(),Member{query:Query::new("operation.request").arg(Value::str(id)),contract:"operation.request@1".into(),encoding:Encoding::Value,optional:false})])};
    assert!(misa_protocol::owner::Owner::read(runtime.as_ref(),&CallContext{principal:"other".into(),connection:2},&selection).is_err());
    let snapshot=misa_protocol::owner::Owner::read(runtime.as_ref(),&context,&selection).unwrap();
    let Content::Value(detail)=&snapshot.members["request"]else{panic!()};
    assert_eq!(detail.get("title").and_then(Value::as_str),Some("Feed the pet"));
    let response=invoke(2,"input.resolve",Value::map([("request",Value::str(id)),("generation",request.get("generation").unwrap().clone()),("value",Value::map([("amount",Value::Int(4))]))]));
    assert!(matches!(dispatcher.dispatch(runtime.as_ref(),response.clone()).await.outcome,Outcome::Accepted{..}));
    assert!(matches!(dispatcher.dispatch(runtime.as_ref(),response).await.outcome,Outcome::Rejected{..}));
    tokio::time::timeout(std::time::Duration::from_secs(5),async{loop{
        let Reading::Data(value)=runtime.read(&Query::new("operation.result").arg(Value::str(&prompt.id))).unwrap()else{panic!()};
        if value.get("terminal")==Some(&Value::Bool(true)){assert_eq!(value.get("state").and_then(Value::as_str),Some("succeeded"));break;}
        tokio::task::yield_now().await;
    }}).await.unwrap();
    let Reading::Data(state)=runtime.read(&Query::new("pet.state")).unwrap()else{panic!()};
    assert_eq!(state.get("treats"),Some(&Value::Int(4)));
    assert_eq!(kernel.entries().iter().filter(|entry|entry.kind=="tool_result").count(),1);
    runtime.shutdown_complete().await;
}
