//! The session: policy over a kernel's facts, and the only layer that knows what
//! an agent is.
//!
//! # Where this sits
//!
//! Three layers, and this is the middle one:
//!
//! - below, a [`Kernel`](misa_kernel::Kernel) holds facts and capability. It does
//!   not know what a turn is.
//! - here, the **agent loop** decides what happens next: continue after tools,
//!   settle an attempt, report a failure, send notice text. It also builds the
//!   **view tree**, which is presentation *policy* — which nodes exist, what groups
//!   them, what a node says. That is why the view lives here and not in a client:
//!   one implementation of "what a transcript is" serves every frontend, and a
//!   plugin that adds a tool can add the view of it.
//! - above, a client owns the surface: theme, layout, focus, scroll, animation. It
//!   never decides what the agent does, and this layer never decides what anything
//!   looks like.
//!
//! # The contract with a client
//!
//! A client may send an [`Intent`] — submit text, resolve an action this layer
//! offered, invoke a command this layer declared, or cancel. It may subscribe, and
//! it is sent a value whenever that value changed. It may do nothing else, because
//! no message in the protocol can express anything else. That is why "the client
//! owns no agent policy" is a property of the message types and not a rule anyone
//! has to remember.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use misa_proto::view::{Node, validate};
use misa_proto::wire::{Capabilities, Intent, SessionEvent, SessionInfo};
use misa_proto::{Fault, Query};
use misa_reframe::{Effect, Event, Interpreter, Loop, Outcome};
use misa_reframe::fields;
use misa_value::Value;
use tokio::sync::{broadcast, mpsc, watch};

use misa_kernel::{Kernel, KernelEvent, Request};

/// Who may attach, as configuration rather than code.
pub mod admission;
pub mod agent;
/// What a session declares: its commands, their arguments, and where a value can
/// come from.
pub mod catalog;
/// Answering a request for candidates.
pub mod completions;
pub mod views;

/// What a query produced.
///
/// The view is its own variant rather than a `Value` because it is a different
/// kind of thing: a client *draws* a view and *reads* data. Keeping them distinct
/// in the type is what stops a view from quietly becoming a bag of values that
/// every client has to interpret.
#[derive(Clone, Debug)]
pub enum Reading {
    View(Node),
    Data(Value),
}

/// One ephemeral event, numbered so a client can tell a repeat from a rerun.
#[derive(Clone, Debug)]
pub struct Emission {
    pub seq: u64,
    pub event: SessionEvent,
}

/// A running agent session.
pub struct Runtime {
    state: Mutex<State>,
    to_kernel: mpsc::UnboundedSender<Request>,
    rev: watch::Sender<u64>,
    events: broadcast::Sender<Emission>,
    seq: AtomicU64,
    info: SessionInfo,
    provider: String,
    model: String,
}

struct State {
    state: Loop,
    /// The last view built, with the revision and client class it was built for.
    ///
    /// Cached because building a tree over a long transcript is the one expensive
    /// thing this layer does, and because a view is a pure function of the database
    /// and the client's capabilities: the same two inputs must produce the same
    /// tree, so caching cannot be observed.
    view: Option<(String, u64, Node)>,
}

/// The composition a session runs. Named so a client can display it and a
/// diagnostic can print it.
pub const POLICY: &[&str] = &["agent.loop", "agent.tools"];

impl Runtime {
    /// Start a session and the task that carries kernel events back into its loop.
    ///
    /// The kernel sits behind a channel on purpose: an effect asks for work, the
    /// work reports an event, and the loop handles it as a fact. No handler awaits,
    /// so the ordering inside a transaction is never at the mercy of a network.
    pub fn start(
        id: impl Into<String>,
        title: impl Into<String>,
        conversation: Option<String>,
        kernel: Arc<dyn Kernel>,
        provider: impl Into<String>,
        model: impl Into<String>,
        config: Value,
    ) -> Arc<Runtime> {
        let id = id.into();
        let provider = provider.into();
        let model = model.into();
        let created_ms = now_ms();

        let registry = Arc::new(agent::registry());
        // Two channels, not one: a request goes out to the kernel and a report comes
        // back. The loop never awaits, and a kernel report re-enters it as an
        // ordinary event rather than as a callback from inside a transaction.
        let (to_kernel, mut from_loop) = mpsc::unbounded_channel::<Request>();
        let (kernel_events, mut kernel_reports) = mpsc::unbounded_channel::<KernelEvent>();
        let (events, _) = broadcast::channel::<Emission>(512);
        let (rev, _) = watch::channel(0u64);
        // A kernel that reports on its own schedule — a background command finishing — needs a
        // reader that is not a request. There is exactly one such stream, so there is one task.
        let unsolicited = kernel.events();

        let mut state = Loop::new(
            registry.clone(),
            Arc::new(OnlyKnownEffects),
            views::initial_state(&id, &provider, &model, created_ms),
        );
        state.set_clock(created_ms);
        state.set_config(config);
        // The transport pushes a snapshot to a client that asks for it; the view is
        // what a client draws before it has sent anything.
        state.watch(Query::new(views::VIEW_QUERY));

        let info = SessionInfo {
            id: id.clone(),
            title: title.into(),
            conversation,
            created_ms,
            policy: POLICY.iter().map(|name| name.to_string()).collect(),
            queries: views::queries(),
            commands: catalog::commands(),
            sources: catalog::sources(),
        };

        let runtime = Arc::new(Runtime {
            state: Mutex::new(State { state, view: None }),
            to_kernel: to_kernel.clone(),
            rev,
            events,
            seq: AtomicU64::new(0),
            info,
            provider,
            model,
        });

        if let Some(mut unsolicited) = unsolicited {
            let reports = runtime.clone();
            tokio::spawn(async move {
                while let Some(report) = unsolicited.recv().await {
                    reports.dispatch(agent::event_for(report));
                }
            });
        }

        tokio::spawn(async move {
            while let Some(request) = from_loop.recv().await {
                let kernel = kernel.clone();
                let out = kernel_events.clone();
                tokio::spawn(async move {
                    kernel.execute(request, &out).await;
                });
            }
        });

        let reports = runtime.clone();
        tokio::spawn(async move {
            while let Some(report) = kernel_reports.recv().await {
                reports.dispatch(agent::event_for(report));
            }
        });

        runtime
    }

    pub fn id(&self) -> &str {
        &self.info.id
    }

    pub fn info(&self) -> SessionInfo {
        self.info.clone()
    }

    pub fn provider(&self) -> &str {
        &self.provider
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    /// Watch for a change worth re-reading a subscription for.
    pub fn watch_rev(&self) -> watch::Receiver<u64> {
        self.rev.subscribe()
    }

    pub fn rev(&self) -> u64 {
        *self.rev.borrow()
    }

    pub fn status(&self) -> String {
        let state = self.state.lock().expect("session state is never poisoned");
        state
            .state
            .db()
            .get_path(&misa_value::Path::parse("session.status").expect("a literal path"))
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string()
    }

    /// Subscribe to the ephemeral stream.
    ///
    /// Lossy on purpose: a client that falls behind drops deltas and converges on
    /// the next subscription value, which is where the truth is. A lagging client
    /// must never be able to stall a session.
    pub fn subscribe_events(&self) -> broadcast::Receiver<Emission> {
        self.events.subscribe()
    }

    /// Handle one event, then do what its effects asked for.
    ///
    /// Everything that leaves this method has already committed. An effect the
    /// interpreter refused was refused *before* the commit, so a client can never
    /// observe a state that asked for something impossible.
    pub fn dispatch(&self, event: Event) -> Vec<Fault> {
        let outcome = {
            let mut state = self.state.lock().expect("session state is never poisoned");
            state.state.set_clock(now_ms());
            state.state.dispatch(event)
        };
        let faults = outcome.as_faults();
        self.perform(&outcome);
        if outcome.committed() {
            let rev = {
                let state = self.state.lock().expect("session state is never poisoned");
                state.state.rev()
            };
            self.rev.send_replace(rev);
        }
        faults
    }

    fn perform(&self, outcome: &Outcome) {
        for effect in &outcome.effects {
            match effect.kind.as_str() {
                "kernel.provider.call" => {
                    let _ = self.to_kernel.send(Request::ProviderCall {
                        id: fields::text(effect, "id"),
                        provider: fields::text(effect, "provider"),
                        model: fields::text(effect, "model"),
                        messages: fields::value(effect, "messages"),
                        tools: fields::value(effect, "tools"),
                        settings: fields::value(effect, "settings"),
                    });
                }
                "kernel.models.discover" => {
                    let _ = self.to_kernel.send(Request::DiscoverModels {
                        id: fields::text(effect, "id"),
                        provider: fields::text(effect, "provider"),
                    });
                }
                "kernel.tool.run" => {
                    let _ = self.to_kernel.send(Request::ToolRun {
                        id: fields::text(effect, "id"),
                        call_id: fields::text(effect, "call_id"),
                        name: fields::text(effect, "name"),
                        args: fields::value(effect, "args"),
                    });
                }
                "kernel.log.append" => {
                    let _ = self.to_kernel.send(Request::Append {
                        conversation: fields::text(effect, "conversation"),
                        kind: fields::text(effect, "kind"),
                        data: fields::value(effect, "data"),
                    });
                }
                "kernel.attempt.started" => {
                    let conversation = fields::text(effect, "conversation");
                    let _ = self.to_kernel.send(Request::AttemptStarted {
                        id: fields::text(effect, "id"),
                        conversation: (!conversation.is_empty()).then_some(conversation),
                        parent: None,
                        provider: fields::text(effect, "provider"),
                        model: fields::text(effect, "model"),
                        kind: fields::text(effect, "kind"),
                    });
                }
                "kernel.attempt.settled" => {
                    let _ = self.to_kernel.send(Request::AttemptSettled {
                        id: fields::text(effect, "id"),
                        status: fields::text(effect, "status"),
                        input_tokens: fields::int(effect, "input_tokens"),
                        output_tokens: fields::int(effect, "output_tokens"),
                        cost_micros: fields::int(effect, "cost_micros"),
                    });
                }
                "wire.event" => {
                    if let Ok(event) = wire::parse::<SessionEvent>(&fields::value(effect, "event")) {
                        self.emit(event);
                    }
                }
                // The interpreter accepted the effect, so this arm is unreachable;
                // doing nothing is still better than panicking a session.
                _ => {}
            }
        }
    }

    fn emit(&self, event: SessionEvent) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let _ = self.events.send(Emission { seq, event });
    }

    /// Push a line to the clients without touching the transcript.
    pub fn notice(&self, level: misa_proto::wire::Level, text: impl Into<String>) {
        self.emit(SessionEvent::Notice { level, text: text.into() });
    }

    /// Interpret an intent. The only place an intent is understood.
    ///
    /// Every client message arrives here, and each arm answers one question: which
    /// event does this ask for? A client cannot reach past this method, which is why
    /// it cannot invent agent state.
    pub fn intent(&self, intent: Intent) -> Vec<Fault> {
        let event = match intent {
            // A client may name attachments it has already put in the store. The hashes
            // travel as facts; whether the bytes are there is the store's answer, checked
            // when the request is built, so a client cannot make a session believe in an
            // image by naming one.
            Intent::Prompt { text, attachments } => Event::new("intent/prompt")
                .with("text", Value::str(text))
                .with("attachments", encode_blobs(&attachments)),
            Intent::Cancel { target } => Event::new("intent/cancel").with("target", match target {
                Some(target) => Value::str(target),
                None => Value::Null,
            }),
            Intent::Command { name, args } => {
                Event::new("intent/command").with("name", Value::str(name)).with("args", args)
            }
            // Completion is a request with a reply, not a fact about the session, so
            // it is answered by `complete` and never dispatched into the loop.
            Intent::Complete { .. } => {
                return vec![Fault::unsupported(
                    "completion is answered as a request, not dispatched as an intent",
                )];
            }
            Intent::Action { node, action, args, fields: submitted } => Event::new("intent/action")
                .with("node", Value::str(node))
                .with("action", Value::str(action))
                .with("args", args)
                .with("fields", encode_fields(&submitted)),
        };
        self.dispatch(event)
    }

    /// Every query this session answers.
    pub fn queries(&self) -> Vec<String> {
        let state = self.state.lock().expect("session state is never poisoned");
        let mut queries: Vec<String> = state.state.registry().query_ids().map(str::to_string).collect();
        queries.extend(views::queries());
        queries.sort();
        queries.dedup();
        queries
    }

    /// The view a client would see right now, without going through a query.
    pub fn view(&self, capabilities: &Capabilities) -> Result<Node, Fault> {
        match self.read(&Query::new(views::VIEW_QUERY), capabilities)? {
            Reading::View(node) => Ok(node),
            Reading::Data(_) => Err(Fault::new("view", "the view query answered with data")),
        }
    }

    /// Answer an on-demand completion source.
    ///
    /// The counterpart to a resident source's query, and the reason the distinction
    /// exists: this runs only when a client has decided it needs candidates, and
    /// never while somebody is typing into a list the client already holds.
    pub fn complete(
        &self,
        source: &str,
        prefix: &str,
        limit: Option<u32>,
    ) -> Result<(Vec<misa_proto::view::Choice>, bool), Fault> {
        let state = self.state.lock().expect("session state is never poisoned");
        completions::on_demand(
            state.state.db(),
            source,
            prefix,
            limit.unwrap_or(misa_proto::wire::DEFAULT_CANDIDATES),
        )
    }

    /// Read a query. The view is answered here; every other query goes through the
    /// loop's own scope.
    pub fn read(&self, query: &Query, capabilities: &Capabilities) -> Result<Reading, Fault> {
        let mut state = self.state.lock().expect("session state is never poisoned");
        if query.id == views::VIEW_QUERY {
            let rev = state.state.rev();
            let class = capabilities.class.as_str().to_string();
            if let Some((cached_class, cached_rev, node)) = &state.view
                && *cached_class == class
                && *cached_rev == rev
            {
                return Ok(Reading::View(node.clone()));
            }
            let limit = query
                .args
                .first()
                .and_then(Value::as_i64)
                .unwrap_or(views::DEFAULT_WINDOW as i64)
                .max(1) as usize;
            let node = views::document(state.state.db(), capabilities, limit);
            // A tree a client cannot rely on is not sent: the session reports and
            // keeps the last valid view, which is what the previous system learned
            // at its frame boundary.
            if let Err(fault) = validate(&node) {
                return Err(Fault::new("view", fault.to_string()));
            }
            state.view = Some((class, rev, node.clone()));
            return Ok(Reading::View(node));
        }
        let value = state
            .state
            .query(query)
            .map_err(|fault| Fault::new(fault.code.clone(), fault.message.clone()))?;
        Ok(Reading::Data(value))
    }
}

/// The interpreter: which effect kinds this session will run.
///
/// This list is the whole of the session's authority over its kernel. An effect
/// outside it fails the transaction before anything commits, so a policy cannot
/// ask for something the session does not understand and have its state land
/// anyway.
struct OnlyKnownEffects;

impl Interpreter for OnlyKnownEffects {
    fn accepts(&self, effect: &Effect) -> Result<(), String> {
        match effect.kind.as_str() {
            "kernel.provider.call"
            | "kernel.tool.run"
            | "kernel.log.append"
            | "kernel.attempt.started"
            | "kernel.attempt.settled"
            | "kernel.models.discover"
            | "wire.event" => Ok(()),
            other => Err(format!("this session does not know the effect `{other}`")),
        }
    }
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_kernel::{Provider, ScriptedProvider, Turn};
    use std::time::Duration;

    fn runtime() -> Arc<Runtime> {
        let provider: Arc<dyn Provider> = ScriptedProvider::new([
            Turn::call("echo", Value::str("hello"), Turn::say("all done")),
            Turn::say("all done"),
        ]);
        Runtime::start(
            "demo",
            "a demo session",
            Some("c1".into()),
            Arc::new(misa_kernel::LocalKernel::new(provider)),
            "scripted",
            "scripted-1",
            Value::Null,
        )
    }

    fn view(runtime: &Runtime) -> Node {
        match runtime.read(&Query::new(views::VIEW_QUERY), &Capabilities::plain()).unwrap() {
            Reading::View(node) => node,
            Reading::Data(_) => panic!("expected a view"),
        }
    }

    fn transcript(runtime: &Runtime) -> String {
        misa_render::to_plain(&misa_render::render(&view(runtime), &misa_render::Theme::plain(), 100))
    }

    /// Wait until the session is idle again, the way a client waits: by watching
    /// the state it can actually see.
    async fn settle(runtime: &Arc<Runtime>) {
        for _ in 0..400 {
            if runtime.status() == "idle" {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("the session never settled; it is still {}", runtime.status());
    }

    #[tokio::test]
    async fn a_prompt_becomes_a_message_the_view_shows() {
        let runtime = runtime();
        let faults = runtime.intent(Intent::Prompt { text: "write a haiku".into(), attachments: vec![] });
        assert!(faults.is_empty(), "{faults:?}");
        let text = transcript(&runtime);
        assert!(text.contains("write a haiku"), "{text}");
        assert_eq!(runtime.status(), "thinking");
    }

    #[tokio::test]
    async fn a_client_may_name_bytes_it_put_in_the_store_and_only_a_hash_is_a_name() {
        // A client that read a file and uploaded it names a hash, the same way `/attach` does;
        // what it may not do is name something that is not a hash, because no store could ever
        // hold it and a message that carried it would be a message naming a path.
        let runtime = runtime();
        let hash = "a".repeat(64);
        let faults = runtime.intent(Intent::Prompt {
            text: "look at this".into(),
            attachments: vec![
                misa_proto::view::BlobRef { hash: hash.clone(), len: 4, media: Some("image/png".into()) },
                misa_proto::view::BlobRef { hash: "../../etc/shadow".into(), len: 0, media: None },
                // The same blob twice is one attachment: two copies of one hash would be two
                // pictures of the same file.
                misa_proto::view::BlobRef { hash: hash.clone(), len: 4, media: Some("image/png".into()) },
            ],
        });
        assert!(faults.is_empty(), "{faults:?}");

        // A client that can draw is sent the reference; a client that cannot is sent the words.
        let drawn = match runtime.read(&Query::new(views::VIEW_QUERY), &Capabilities::browser()).unwrap() {
            Reading::View(node) => node,
            Reading::Data(_) => panic!("expected a view"),
        };
        let attachment = misa_proto::view::find(&drawn, "attachment.0").expect("the attachment is in the view");
        match &attachment.kind {
            misa_proto::view::Kind::Image { blob, .. } => assert_eq!(blob.hash, hash),
            _ => panic!("an attachment that is not an image is an attachment nothing can show"),
        }
        assert!(misa_proto::view::find(&drawn, "attachment.1").is_none(), "a name that is not a hash became an attachment");

        let plain = transcript(&runtime);
        assert!(plain.contains("image/png"), "a client that cannot draw is not told what is there:\n{plain}");
        assert!(!plain.contains("etc/shadow"), "something that is not a hash reached the transcript:\n{plain}");
    }

    #[tokio::test]
    async fn a_finished_background_command_is_a_line_both_the_person_and_the_model_can_see() {
        // The only report the kernel makes on its own schedule, and the reason it becomes a
        // message rather than a notice: the model has to be able to act on a build that failed
        // while it was answering something else, and the person has to be able to see why it
        // suddenly knew.
        let runtime = runtime();
        let faults = runtime.dispatch(
            Event::new("kernel/process.finished")
                .with("pid", Value::Int(4242))
                .with("command", Value::str("cargo build"))
                .with("exit", Value::Int(0))
                .with("killed", Value::Bool(false))
                .with("seconds", Value::Float(12.4))
                .with("log", Value::str("/tmp/shell/p1-cargo-build.log")),
        );
        assert!(faults.is_empty(), "{faults:?}");

        let text = transcript(&runtime);
        assert!(text.contains("cargo build"), "{text}");
        assert!(text.contains("pid 4242"), "{text}");
        assert!(text.contains("exited 0"), "{text}");
        assert!(text.contains("12.4s"), "{text}");
        assert!(text.contains("/tmp/shell/p1-cargo-build.log"), "{text}");

        // Told to the model as a system message, which is where both providers read a note that
        // is not part of the conversation.
        let messages = match runtime.read(&Query::new("session.conversation"), &Capabilities::plain()).unwrap() {
            Reading::Data(value) => value,
            Reading::View(_) => panic!("expected data"),
        };
        let last = messages.as_list().and_then(|messages| messages.last()).expect("a message");
        assert_eq!(last.get("role").and_then(Value::as_str), Some("system"));
        assert!(last.get("text").and_then(Value::as_str).unwrap().contains("cargo build"));

        // It does not start a turn: a turn is something a person asks for, and answering every
        // finished process with a provider call would spend money behind their back.
        assert_eq!(runtime.status(), "idle");
    }

    #[tokio::test]
    async fn a_tool_call_round_trips_and_the_turn_finishes() {
        let runtime = runtime();
        runtime.intent(Intent::Prompt { text: "use the tool".into(), attachments: vec![] });
        settle(&runtime).await;
        let text = transcript(&runtime);
        assert!(text.contains("use the tool"), "{text}");
        assert!(text.contains("echo"), "the tool call is missing:\n{text}");
        assert!(text.contains("hello"), "the tool result is missing:\n{text}");
        assert!(text.contains("all done"), "the continuation is missing:\n{text}");
    }

    #[tokio::test]
    async fn an_unknown_action_is_a_fault_and_not_a_panic() {
        let runtime = runtime();
        let faults = runtime.intent(Intent::Action {
            node: "m1".into(),
            action: "nope.does.not.exist".into(),
            args: Value::Null,
            fields: Vec::new(),
        });
        assert_eq!(faults.len(), 1);
        assert!(faults[0].message.contains("nope.does.not.exist"), "{:?}", faults[0]);
        // And the session is still usable, which is the point of containment.
        assert!(runtime.intent(Intent::Command { name: "help".into(), args: Value::Null }).is_empty());
    }

    #[tokio::test]
    async fn an_unknown_command_is_reported_as_a_notice_rather_than_refused() {
        let runtime = runtime();
        let faults = runtime.intent(Intent::Command { name: "nonsense".into(), args: Value::Null });
        assert!(faults.is_empty());
        let text = transcript(&runtime);
        assert!(text.contains("nonsense"), "{text}");
    }

    #[tokio::test]
    async fn a_command_can_be_answered_by_an_ephemeral_event() {
        let runtime = runtime();
        let mut events = runtime.subscribe_events();
        runtime.intent(Intent::Command { name: "cost".into(), args: Value::Null });
        let emission = events.try_recv().expect("a notice");
        assert!(matches!(emission.event, SessionEvent::Notice { .. }));
    }

    #[tokio::test]
    async fn the_view_query_is_advertised_and_answers() {
        let runtime = runtime();
        assert!(runtime.queries().iter().any(|query| query == views::VIEW_QUERY));
        assert_eq!(view(&runtime).role, "session");
    }

    #[tokio::test]
    async fn a_data_query_is_answered_as_data() {
        let runtime = runtime();
        match runtime.read(&Query::new("session.status"), &Capabilities::plain()).unwrap() {
            Reading::Data(value) => assert_eq!(value.get("status").and_then(Value::as_str), Some("idle")),
            Reading::View(_) => panic!("expected data"),
        }
    }

    #[tokio::test]
    async fn the_view_is_cached_until_the_database_changes() {
        let runtime = runtime();
        let first = view(&runtime);
        let (class, rev, cached) = runtime.state.lock().unwrap().view.clone().expect("a cached view");
        assert_eq!(first.id, cached.id);
        assert_eq!(class, "plain");
        assert_eq!(rev, runtime.rev());
    }

    #[tokio::test]
    async fn a_client_that_cannot_show_details_is_handed_a_tree_it_can_draw() {
        let runtime = runtime();
        let plain = match runtime.read(&Query::new(views::VIEW_QUERY), &Capabilities::plain()).unwrap() {
            Reading::View(node) => node,
            Reading::Data(_) => panic!("expected a view"),
        };
        let rich = match runtime.read(&Query::new(views::VIEW_QUERY), &Capabilities::browser()).unwrap() {
            Reading::View(node) => node,
            Reading::Data(_) => panic!("expected a view"),
        };
        // Same structure, and the one difference is the *default* a client that has
        // no disclosure widget needs.
        assert_eq!(plain.children.len(), rich.children.len());
    }

    #[tokio::test]
    async fn the_view_window_is_an_argument_not_a_property_of_the_session() {
        let runtime = runtime();
        let narrow = runtime
            .read(&Query::new(views::VIEW_QUERY).arg(Value::Int(1)), &Capabilities::plain())
            .unwrap();
        let wide = runtime
            .read(&Query::new(views::VIEW_QUERY).arg(Value::Int(40)), &Capabilities::plain())
            .unwrap();
        match (narrow, wide) {
            (Reading::View(a), Reading::View(b)) => {
                assert!(misa_proto::view::find(&a, "transcript").is_some());
                assert!(misa_proto::view::find(&b, "transcript").is_some());
            }
            _ => panic!("expected two views"),
        }
    }
}

/// Encoding for the one place a structured value travels inside an effect.
///
/// An effect's data is `Value`, and a session event is a typed thing. Rather than
/// give the loop a second value type, the two cross here, in one pair of functions
/// that a test covers.
mod wire {
    use misa_value::Value;

    pub fn parse<T: serde::Serialize + serde::de::DeserializeOwned>(value: &Value) -> Result<T, String> {
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(value, &mut bytes).map_err(|err| err.to_string())?;
        ciborium::de::from_reader(&bytes[..]).map_err(|err| err.to_string())
    }

    #[allow(dead_code)]
    pub fn render<T: serde::Serialize>(value: &T) -> Value {
        // Only used where a typed thing has to become a value; kept here so the
        // round trip has one home.
        let mut bytes = Vec::new();
        if ciborium::ser::into_writer(value, &mut bytes).is_err() {
            return Value::Null;
        }
        ciborium::de::from_reader(&bytes[..]).unwrap_or(Value::Null)
    }
}

/// The fields a client submitted with an action, as data.
fn encode_fields(fields: &[misa_proto::view::Field]) -> Value {
    Value::list(
        fields
            .iter()
            .map(|field| Value::map([("id", Value::str(&field.id)), ("value", Value::str(&field.value))]))
            .collect::<Vec<_>>(),
    )
}

/// Attachments a client named, as the shape a message carries them in.
///
/// The same fields `/attach` writes, so an attachment is one shape however it arrived and
/// the kernel resolves every one of them the same way: a hash it holds becomes bytes, and a
/// hash it does not becomes a sentence saying so. Nothing here is trusted, because nothing
/// here is anything but a name.
fn encode_blobs(attachments: &[misa_proto::view::BlobRef]) -> Value {
    Value::list(
        attachments
            .iter()
            .map(|blob| {
                let mut entries = std::collections::BTreeMap::new();
                entries.insert("hash".to_string(), Value::str(&blob.hash));
                entries.insert("len".to_string(), Value::Int(blob.len as i64));
                if let Some(media) = &blob.media {
                    entries.insert("media".to_string(), Value::str(media));
                }
                // Where it came from, for the transcript to name: a client that uploaded
                // bytes has no path to offer, and an attachment that arrived from a client
                // says so rather than pretending to be a file on this machine.
                entries.insert("source".to_string(), Value::str("client"));
                Value::Map(Arc::new(entries))
            })
            .collect::<Vec<_>>(),
    )
}
