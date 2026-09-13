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
use misa_plugin::{Descriptor, PLUGIN_PRIORITY, Plugin, PluginFault};
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
    {
        // (the build below is the fallback, and it is what the shell is for)
    }
    let next = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("the workspace root")
        .to_path_buf();
    let guest = next.join("wit/guest");
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
        target.join("wasm32-unknown-unknown/release/policy_guest.wasm"),
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
    assert_eq!(
        descriptor,
        &Descriptor {
            id: "policy.guest".into(),
            version: "0.1.0".into(),
            events: vec!["intent/prompt".into(), "intent/action".into(), "intent/cancel".into()],
            queries: vec!["policy.guest.turns".into(), "policy.guest.state".into()],
            effects: vec!["kernel.log.append".into()],
            actions: vec!["refresh".into()],
            roots: vec!["guest".into()],
        }
    );
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
    assert_eq!(effect.field("kind"), "note");
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
    let answered = plugin.query("policy.guest.turns", &[Value::Int(1)], &[], &db, None).expect("an answer");
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
fn a_view_is_built_the_way_any_other_tree_is() {
    let plugin = loaded();
    let db = Value::Null;
    let tree = plugin.view(&db, 40).expect("a tree");
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
async fn a_plugin_presents_a_section_a_session_places() {
    // The whole of a plugin's presentation: it returns a tree, the session places it in the
    // document under a role built from the plugin's id, and every frontend draws it — with no
    // frontend code, which is the point of putting it in the document rather than in a query each
    // client would have to know about.
    let plugin = loaded();
    let (runtime, _kernel) = session(contribution(&plugin));
    let node = view(&runtime);
    misa_proto::view::validate(&node).expect("a tree a client may be sent");

    let section = misa_proto::view::find(&node, "plugin.policy.guest").expect("the plugin's section");
    assert_eq!(section.role, "plugin.policy.guest");
    assert_eq!(section.label.as_deref(), Some("policy.guest"));

    // The plugin's own ids are inside the session's namespace, and its affordance is on the node it
    // put it on — with the plugin's own action id, not a mangled one.
    let root = misa_proto::view::find(&node, "plugin.policy.guest.guest").expect("the plugin's root, namespaced");
    assert_eq!(root.actions.len(), 1);
    assert_eq!(root.actions[0].id, "refresh");
    assert_eq!(root.actions[0].label.as_deref(), Some("Refresh"));

    // And what the plugin said about the client it was drawing for is in the document, because the
    // session handed it the database and the requested history window.
    let text = misa_render::to_plain(&misa_render::render(&node, &misa_render::Theme::plain(), 100));
    assert!(text.contains("drawn for"), "{text}");
    assert!(text.contains("semantic"), "semantic plugin view: {text}");
}

#[tokio::test]
async fn an_action_from_a_plugins_tree_reaches_the_plugin() {
    // No router anywhere: the client's action is the loop's own intent/action event, the session
    // does not fault for it because the plugin declared it, and the plugin — which declared that
    // event kind — does the work.
    let plugin = loaded();
    let (runtime, _kernel) = session(contribution(&plugin));
    let _ = view(&runtime);

    let faults = runtime.intent(Intent::Action {
        node: "plugin.policy.guest.guest".into(),
        action: "refresh".into(),
        args: Value::Null,
        fields: Vec::new(),
    });
    assert!(faults.is_empty(), "{faults:?}");

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
    let contribution = contribution(&plugin).with_root("spin", Value::Bool(true)).expect("a root");
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
    let contribution = contribution(&plugin).with_root("refuse", Value::Bool(true)).expect("a root");
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
        let entries = kernel
            .entries()
            .into_iter()
            .filter(|entry| entry.conversation == conversation)
            .collect::<Vec<_>>();
        if !entries.is_empty() {
            return entries;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("no entry arrived for `{conversation}`");
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
    for action in &plugin.descriptor().actions {
        contribution = contribution.with_action(action).expect("an affordance the session does not have");
    }
    let presenting = plugin.clone();
    contribution = contribution.with_section(misa_session::views::Section {
        plugin: plugin.descriptor().id.clone(),
        build: Arc::new(move |db, window| {
            presenting.view(db, window).map_err(|fault| fault.message)
        }),
    });
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
    match runtime.read(&Query::new(misa_proto::VIEW_QUERY)).expect("a view") {
        Reading::View(node) => node,
        Reading::Data(_) => panic!("the view query answered with data"),
    }
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
    assert_eq!(entries[0].kind, "note");
    assert_eq!(entries[0].data.as_str(), Some("seen a prompt"));
}
