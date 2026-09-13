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

use misa_plugin::{Descriptor, Engine, PLUGIN_PRIORITY, Plugin, PluginFault};
use misa_reframe::{Effect, Event, Interpreter, Loop, Registry};
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
    let engine = Engine::default();
    Arc::new(Plugin::load(&engine, component_once()).expect("the fixture loads"))
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
            events: vec!["intent/prompt".into(), "intent/cancel".into()],
            queries: vec!["policy.guest.turns".into()],
            effects: vec!["kernel.log.append".into()],
        }
    );
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
    let tree = plugin.view("session", "plain", &db, 40).expect("a tree");
    assert_eq!(tree.id, "guest");
    assert_eq!(tree.role, "guest.panel");
    assert_eq!(tree.children.len(), 1);
    assert_eq!(tree.children[0].id, "guest.note");
    assert_eq!(tree.children[0].role, "guest.note");
    // It is the protocol's own tree, so the protocol's own validator accepts it — which is the
    // only promise a plugin's view makes.
    misa_proto::view::validate(&tree).expect("a tree a client may be sent");
}

#[test]
fn a_component_that_is_not_a_policy_plugin_is_refused_rather_than_run() {
    let engine = Engine::default();
    let fault: PluginFault = Plugin::load(&engine, b"not a component at all").expect_err("refused");
    assert_eq!(fault.code, "plugin.host");
    assert!(fault.message.contains("component"), "{}", fault.message);
}
