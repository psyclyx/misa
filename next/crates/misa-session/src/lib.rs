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
//! Clients invoke installed, schema-checked commands and read or observe coherent
//! selections of exported queries. Scope incarnations fence owner lifetimes;
//! trusted caller context gates private input requests. Tests can inject local
//! domain inputs; network clients use the installed command registry.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use misa_proto::view::Node;
use misa_proto::{Fault, Query};
use misa_reframe::fields;
use misa_reframe::{Effect, Event, Interpreter, Loop};
use misa_value::Value;
use tokio::sync::{broadcast, mpsc, watch};

use misa_kernel::{CredentialAction, Kernel, KernelEvent, Request};

pub use contribution::Contribution;
#[cfg(test)]
mod intent;
#[cfg(test)]
pub(crate) use intent::Intent;
mod events;
pub use events::{Level, SessionEvent};

pub mod agent;
mod canonical;
/// What a session declares: its commands, their arguments, and where a value can
/// come from.
pub mod catalog;
/// Answering a request for candidates.
pub mod completions;
/// What a composition adds to the loop: handlers, subscriptions, and their state.
pub mod contribution;
pub mod indicators;
mod journal;
mod kernel_queue;
mod protocol;
mod publication;
mod reports;
pub mod usage;
pub mod views;
use kernel_queue::Outcome;
mod command_operations;
pub mod commands;
pub mod observation;
#[cfg(test)]
mod observation_tests;
pub(crate) mod operations;
mod tool_bindings;

#[derive(Clone, Debug)]
pub enum Reading {
    View(Node),
    Data(Value),
}

/// Internal domain notifications. Scoped transport publishes owner transactions.
#[derive(Clone, Debug)]
pub struct Emission {
    pub event: SessionEvent,
}

/// Owner metadata, independent of transport greetings and installed catalogs.
#[derive(Clone, Debug)]
pub struct Metadata {
    pub id: String,
    pub title: String,
    pub conversation: Option<String>,
    pub created_ms: i64,
    pub policy: Vec<String>,
}

/// A running agent session.
pub struct Runtime {
    reports: Mutex<reports::Pending>,
    state: Mutex<State>,
    /// What compositions contributed to every view this session builds.
    sections: Vec<views::Section>,
    to_kernel: kernel_queue::Queue,
    kernel_budget: kernel_queue::Budget,
    rev: watch::Sender<u64>,
    events: broadcast::Sender<Emission>,
    metadata: Metadata,
    next_call: AtomicU64,
    provider: String,
    model: String,
    incarnation: String,
    parent_attempt: Option<String>,
    exports: std::collections::BTreeMap<String, misa_proto::query::Definition>,
    restricted_exports: std::collections::BTreeMap<String, observation::RestrictedQuery>,
    command_registry: std::collections::BTreeMap<String, commands::CommandRegistration>,
    tool_bindings: std::collections::BTreeMap<String, misa_proto::tool::Binding>,
    tool_invocations: Mutex<
        std::collections::BTreeMap<
            u64,
            (
                Request,
                String,
                Option<misa_proto::invocation::OperationRef>,
            ),
        >,
    >,
    operation_deadline: watch::Sender<Option<i64>>,
    tool_approval: operations::ToolApprovalPolicy,
    closed: AtomicBool,
    closed_reason: std::sync::OnceLock<Fault>,
    started: AtomicBool,
    closing: watch::Sender<bool>,
    stopped: watch::Receiver<bool>,
}

struct State {
    operations: operations::Store,
    deferred: operations::DeferredWork,
    state: Loop,
    streams: std::collections::BTreeMap<String, misa_proto::sync::Stream>,
    stream_bytes: u64,
    view: canonical::Canonical,
    publications: publication::Publications,
}

impl Runtime {
    /// The session state guard.
    ///
    /// A panic while the lock is held poisons it. Taking the guard anyway is
    /// deliberate: one broken operation must not make this session — and every
    /// future connection to it — permanently unreachable. What a panic left
    /// behind is visible in the session's views and recoverable; a dead scope
    /// is not.
    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

/// The composition a session runs. Named so a client can display it and a
/// diagnostic can print it.
pub const POLICY: &[&str] = &["agent.loop", "agent.tools"];
const DEFAULT_MODEL_REFRESH_INTERVAL_MS: i64 = 15 * 60 * 1_000;

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
        Runtime::start_with(
            id,
            title,
            conversation,
            kernel,
            provider,
            model,
            config,
            Contribution::default(),
        )
    }

    /// A session whose loop is the shipped one *plus* what a composition added.
    ///
    /// The contribution is registered the way the shipped handlers are — through the loop's own
    /// registry — so a plugin is not a special case anywhere in this crate: its handlers are
    /// handlers, its subscriptions are subscriptions, and the effects it asks for are held to
    /// the accepted-effects list like every other effect in the system.
    #[allow(clippy::too_many_arguments)]
    pub fn start_with(
        id: impl Into<String>,
        title: impl Into<String>,
        conversation: Option<String>,
        kernel: Arc<dyn Kernel>,
        provider: impl Into<String>,
        model: impl Into<String>,
        config: Value,
        contribution: Contribution,
    ) -> Arc<Runtime> {
        let runtime = Self::prepare_with(
            id,
            title,
            conversation,
            kernel,
            provider,
            model,
            config,
            contribution,
        );
        runtime.activate();
        runtime
    }

    /// Compose an owner without admitting startup effects. The daemon activates
    /// it only after desired membership has committed.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare_with(
        id: impl Into<String>,
        title: impl Into<String>,
        conversation: Option<String>,
        kernel: Arc<dyn Kernel>,
        provider: impl Into<String>,
        model: impl Into<String>,
        config: Value,
        contribution: Contribution,
    ) -> Arc<Runtime> {
        let tool_approval = operations::ToolApprovalPolicy::from_config(&config);
        // Model lists and prices are provider facts, not immutable session config.
        // The interval is configurable for hosts that want a different freshness
        // policy; zero disables the background refresh while startup and login
        // refreshes remain intact.
        let model_refresh_interval_ms = config
            .get("model_refresh_interval_ms")
            .and_then(Value::as_i64)
            .unwrap_or(DEFAULT_MODEL_REFRESH_INTERVAL_MS)
            .max(0);
        let parent_attempt = config
            .get("parent_attempt")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let id = id.into();
        let provider = provider.into();
        let model = model.into();
        let created_ms = now_ms();
        let command_registry =
            commands::install(&contribution.commands).expect("invalid command composition");
        let tool_bindings = tool_bindings::install(&contribution.tools, &command_registry)
            .expect("invalid tool composition");
        let tool_schemas = tool_bindings::schemas(&tool_bindings, &command_registry);
        let mut config = config.as_map().cloned().unwrap_or_default();
        config.insert("installed_tools".into(), tool_schemas);
        let config = Value::Map(Arc::new(config));
        let command_catalogs = commands::catalogs(&command_registry, &contribution.bindings)
            .expect("invalid action binding composition");

        let registry = contribution.registry(command_operations::registry(operations::registry(
            indicators::subscriptions(agent::registry()),
        )));
        let mut indicators = indicators::builtins();
        for indicator in &contribution.indicators {
            indicators
                .register(indicator.clone())
                .expect("contribution validates indicator names");
        }
        let mut presentations = vec![misa_proto::presentation::Presentation {
            id: "conversation".into(),
            title: "Conversation".into(),
            variants: vec![misa_proto::presentation::Variant {
                id: "semantic".into(),
                requirements: vec![],
                member: misa_proto::observation::Member {
                    query: misa_proto::Query::new(observation::CONVERSATION),
                    contract: "conversation.presentation@1".into(),
                    encoding: misa_proto::observation::Encoding::Document,
                    optional: false,
                },
            }],
        }];
        presentations.push(misa_proto::presentation::Presentation {
            id: "status".into(),
            title: "Status".into(),
            variants: vec![misa_proto::presentation::Variant {
                id: "semantic".into(),
                requirements: vec![],
                member: indicators::definition()
                    .member(vec![])
                    .expect("status contract"),
            }],
        });
        presentations.extend(contribution.presentations.iter().cloned());
        let catalog = wire::render(&presentations);
        let catalog_export = misa_proto::presentation::definition();
        if let misa_proto::query::ResultContract::Data { schema } = &catalog_export.result {
            schema
                .validate(&catalog)
                .expect("invalid presentation catalog data");
        }
        let mut registry = registry
            .subscription(indicators::MODEL_QUERY, indicators.subscription())
            .subscription(indicators::DOCUMENT_QUERY, indicators.document())
            .subscription(
                misa_proto::presentation::CATALOG,
                misa_reframe::derived_query([], move |_| catalog.clone()),
            );
        for (definition, value) in &command_catalogs {
            let value = value.clone();
            registry = registry.subscription(
                &definition.id,
                misa_reframe::derived_query([], move |_| value.clone()),
            );
        }
        let mut exports = std::collections::BTreeMap::new();
        let mut restricted_exports = std::collections::BTreeMap::new();
        for (definition, project) in operations::restricted_exports() {
            definition
                .check()
                .expect("invalid restricted query contract");
            assert!(
                !registry.query_ids().any(|id| id == definition.id),
                "restricted query cannot be installed in the shared graph"
            );
            assert!(
                restricted_exports
                    .insert(definition.id.clone(), project)
                    .is_none(),
                "duplicate restricted query"
            );
            exports.insert(definition.id.clone(), definition);
        }
        for definition in observation::builtins()
            .into_iter()
            .chain(completions::exports())
            .chain(usage::exports())
            .chain(operations::definitions())
            .chain([
                catalog_export,
                misa_proto::query::catalog_definition(),
                indicators::definition(),
            ])
            .chain(
                command_catalogs
                    .into_iter()
                    .map(|(definition, _)| definition),
            )
            .chain(contribution.query_exports.iter().cloned())
        {
            definition.check().expect("invalid exported query contract");
            assert!(
                definition.id == observation::CONVERSATION
                    || definition.id == misa_proto::query::CATALOG
                    || registry.query_ids().any(|id| id == definition.id),
                "export refers to a missing query"
            );
            assert!(
                exports.insert(definition.id.clone(), definition).is_none(),
                "duplicate exported query"
            );
        }
        let query_catalog = wire::render(&exports.values().cloned().collect::<Vec<_>>());
        let registry = Arc::new(registry.subscription(
            misa_proto::query::CATALOG,
            misa_reframe::derived_query([], move |_| query_catalog.clone()),
        ));
        registry
            .validate()
            .expect("invalid owner query composition");
        for registration in command_registry.values() {
            if let Some(event) = registration.event_kind() {
                assert!(
                    !registry.handlers_for(event).is_empty(),
                    "command refers to an unhandled event"
                );
            }
        }
        misa_proto::presentation::validate_catalog(&presentations, &exports)
            .expect("invalid installed presentation catalog");
        // Two channels, not one: a request goes out to the kernel and a report comes
        // back. The loop never awaits, and a kernel report re-enters it as an
        // ordinary event rather than as a callback from inside a transaction.
        let (to_kernel, from_loop) = kernel_queue::Queue::new();
        let kernel_budget = kernel_queue::Budget::default();
        let (kernel_events, mut kernel_reports) = mpsc::unbounded_channel::<KernelEvent>();
        let (events, _) = broadcast::channel::<Emission>(512);
        let (rev, _) = watch::channel(0u64);
        let (closing, shutdown) = watch::channel(false);
        let (stopped, stopped_rx) = watch::channel(false);
        // A kernel that reports on its own schedule — a background command finishing — needs a
        // reader that is not a request. There is exactly one such stream, so there is one task.
        let unsolicited = kernel.events();

        let mut epoch = [0u8; 16];
        getrandom::fill(&mut epoch).expect("session incarnation requires system randomness");
        let epoch = epoch
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut initial = contribution.initial_state(&id, &provider, &model, created_ms);
        initial = misa_value::apply_one(
            &initial,
            &misa_value::Path::parse("session.incarnation").unwrap(),
            &misa_value::Op::Set(Value::str(&epoch)),
        )
        .unwrap();
        if let Some(conversation) = &conversation {
            initial = misa_value::apply_one(
                &initial,
                &misa_value::Path::parse("session.conversation").unwrap(),
                &misa_value::Op::Set(Value::str(conversation)),
            )
            .unwrap();
            initial = misa_value::apply_one(
                &initial,
                &misa_value::Path::parse("session.status").unwrap(),
                &misa_value::Op::Set(Value::str("loading")),
            )
            .unwrap();
        }
        let mut state = Loop::new(registry.clone(), Arc::new(AcceptedEffects), initial);
        state.set_clock(created_ms);
        state.set_config(config);
        // The transport pushes a snapshot to a client that asks for it; the view is
        // what a client draws before it has sent anything.
        state.watch(Query::new(misa_proto::VIEW_QUERY));

        let metadata = Metadata {
            id: id.clone(),
            title: title.into(),
            conversation,
            created_ms,
            policy: POLICY.iter().map(|name| name.to_string()).collect(),
        };

        let view = canonical::Canonical::new(state.db(), &contribution.sections, epoch.clone());
        let runtime = Arc::new(Runtime {
            reports: Mutex::new(Default::default()),
            state: Mutex::new(State {
                operations: Default::default(),
                deferred: Default::default(),
                state,
                view,
                streams: Default::default(),
                stream_bytes: 0,
                publications: Default::default(),
            }),
            sections: contribution.sections,
            to_kernel: to_kernel.clone(),
            kernel_budget,
            rev,
            events,
            metadata,
            next_call: AtomicU64::new(1),
            provider,
            model,
            incarnation: epoch,
            parent_attempt,
            exports,
            restricted_exports,
            command_registry,
            tool_bindings,
            tool_invocations: Mutex::new(Default::default()),
            operation_deadline: watch::channel(None).0,
            tool_approval,
            closed: AtomicBool::new(false),
            closed_reason: std::sync::OnceLock::new(),
            started: AtomicBool::new(false),
            closing,
            stopped: stopped_rx,
        });

        if let Some(mut unsolicited) = unsolicited {
            let reports = Arc::downgrade(&runtime);
            tokio::spawn(async move {
                while let Some(report) = unsolicited.recv().await {
                    let Some(reports) = reports.upgrade() else {
                        break;
                    };
                    reports.dispatch(agent::event_for(report));
                }
            });
        }

        tokio::spawn(from_loop.run(kernel, kernel_events, shutdown, stopped));

        let reports = Arc::downgrade(&runtime);
        tokio::spawn(async move {
            while let Some(report) = kernel_reports.recv().await {
                let Some(reports) = reports.upgrade() else {
                    break;
                };
                reports.dispatch(agent::event_for(report));
            }
        });

        let weak = Arc::downgrade(&runtime);
        let wake = runtime.kernel_budget.wake.clone();
        let mut closing = runtime.closing.subscribe();
        tokio::spawn(async move {
            loop {
                tokio::select! { biased; _ = closing.changed() => break, _ = wake.notified() => {} }
                let Some(runtime) = weak.upgrade() else {
                    break;
                };
                runtime.drain_reports();
            }
        });
        operations::start_expiry_loop(&runtime);
        if model_refresh_interval_ms > 0 {
            let weak = Arc::downgrade(&runtime);
            let mut closing = runtime.closing.subscribe();
            tokio::spawn(async move {
                let mut refresh = tokio::time::interval(std::time::Duration::from_millis(
                    model_refresh_interval_ms as u64,
                ));
                // Startup already asks for the credential inventory. Do not send
                // the same request a second time on the interval's immediate tick.
                refresh.tick().await;
                loop {
                    tokio::select! {
                        _ = closing.changed() => break,
                        _ = refresh.tick() => {
                            let Some(runtime) = weak.upgrade() else { break; };
                            if runtime.dispatch(misa_reframe::Event::new(
                                "discovery/credentials.refresh",
                            )).iter().any(|fault| fault.code == "closed_scope") {
                                break;
                            }
                        }
                    }
                }
            });
        }
        runtime
    }

    pub fn is_started(&self) -> bool {
        self.started.load(Ordering::Acquire)
    }
    pub fn activate(&self) {
        if self.is_closed() || self.started.swap(true, Ordering::AcqRel) {
            return;
        }
        self.dispatch(Event::new("session/started"));
        if let Some(conversation) = &self.metadata.conversation {
            let admission = self
                .kernel_budget
                .reserve(kernel_queue::Class::Control, 0)
                .expect("startup capacity");
            let _ = self.to_kernel.send(
                Request::Load {
                    conversation: conversation.clone(),
                    after: 0,
                    limit: i64::MAX as usize,
                },
                &admission,
            );
        }
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Closing the owner invalidates existing observations even while clients
    /// still hold Arcs. Submitted external effects cannot be rolled back.
    pub fn shutdown(&self) {
        self.shutdown_with_fault(Fault::new("closed_scope", "Session owner is closed"));
    }
    pub(crate) fn shutdown_with_fault(&self, fault: Fault) {
        if self.closed_reason.set(fault).is_err() {
            return;
        }
        self.closed.store(true, Ordering::Release);
        self.closing.send_replace(true);
        self.operation_deadline.send_replace(None);
        self.reports.lock().expect("report queue poisoned").clear();
        self.tool_invocations
            .lock()
            .expect("tool correlations poisoned")
            .clear();
        let mut state = self.state();
        self.interrupt_operation_checkpoints(&mut state);
        state.publications.commit(vec![]);
        self.rev.send_replace(state.state.rev());
    }
    pub(crate) fn closure_fault(&self) -> Fault {
        self.closed_reason
            .get()
            .cloned()
            .unwrap_or_else(|| Fault::new("closed_scope", "Session owner is closed"))
    }

    /// Wait until this owner's tracked kernel futures have been dropped.
    /// Already committed external effects remain authoritative.
    pub async fn shutdown_complete(&self) {
        self.shutdown();
        let mut stopped = self.stopped.clone();
        while !*stopped.borrow_and_update() {
            if stopped.changed().await.is_err() {
                break;
            }
        }
    }

    pub fn id(&self) -> &str {
        &self.metadata.id
    }

    pub fn metadata(&self) -> Metadata {
        self.metadata.clone()
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
        let state = self.state();
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
    fn dispatch_once(&self, event: Event) -> Vec<Fault> {
        if self.is_closed() {
            return vec![Fault::new("closed_scope", "Session owner is closed")];
        }
        if !self.is_started() {
            return vec![Fault::new(
                "not_ready",
                "Session owner has not been activated",
            )];
        }
        if let Some(faults) = self.operation_checkpoint_event(&event) {
            return faults;
        }
        if let Some(faults) = self.input_event(&event) {
            return faults;
        }
        let event = if event.kind == "kernel/log.failed"
            && event.get("kind").and_then(Value::as_str) != Some(contribution::PATCH_KIND)
        {
            Event::new("kernel/failed")
                .with(
                    "id",
                    event.get("conversation").cloned().unwrap_or(Value::Null),
                )
                .with(
                    "message",
                    event.get("message").cloned().unwrap_or(Value::Null),
                )
        } else {
            event
        };
        if let Some(mut faults) = self.credential_event(&event) {
            // Credential callbacks first settle their private operation record. The
            // resulting public capability fact still belongs to the normal agent
            // reducer: model discovery and usage refresh must follow a newly stored
            // credential without making clients issue a second command.
            if event.kind == "kernel/credential"
                && !event
                    .get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| id.starts_with("credential-cancel:"))
            {
                // The kernel's completion message is private provider text.  Give the
                // public reducer only the capability facts it needs; in particular, never
                // let an API error or echoed token become a notice in the shared graph.
                let public = Event::new("kernel/credential")
                    .with("id", event.get("id").cloned().unwrap_or(Value::Null))
                    .with("ok", event.get("ok").cloned().unwrap_or(Value::Bool(false)))
                    .with("slot", event.get("slot").cloned().unwrap_or(Value::Null))
                    .with(
                        "slots",
                        event
                            .get("slots")
                            .cloned()
                            .unwrap_or_else(|| Value::list([])),
                    );
                faults.extend(self.dispatch_state_event(public));
            }
            return faults;
        }
        self.dispatch_state_event(event)
    }

    fn dispatch_state_event(&self, event: Event) -> Vec<Fault> {
        if matches!(
            event.kind.as_str(),
            "kernel/provider.delta" | "kernel/provider.thinking"
        ) {
            self.append_stream(&event);
            return Vec::new();
        }
        let outcome = {
            let mut state = self.state();
            let (event, restored) = operations::restore_event(event);
            let mut outcome = self.dispatch_locked(&mut state, event);
            if outcome.committed() {
                if let Some(restored) = restored {
                    state.operations = restored;
                    state.deferred.clear();
                }
                self.queue_operation_checkpoint(&mut state, &mut outcome, &mut None);
            }
            outcome
        };
        let faults = outcome.as_faults();
        self.perform(&outcome);
        if outcome.committed() {
            let rev = {
                let state = self.state();
                state.state.rev()
            };
            self.rev.send_replace(rev);
        }
        faults
    }

    /// Caller holds the owner lock; publication and stream retirement share the commit.
    fn dispatch_locked(&self, state: &mut State, event: Event) -> Outcome {
        let class = if event.kind.starts_with("kernel/") || event.kind.starts_with("owner/") {
            kernel_queue::Class::Control
        } else {
            kernel_queue::Class::External
        };
        self.dispatch_admitted(state, event, class)
    }
    fn dispatch_admitted(
        &self,
        state: &mut State,
        mut event: Event,
        class: kernel_queue::Class,
    ) -> Outcome {
        if matches!(event.kind.as_str(), "intent/cancel" | "intent/interrupt") {
            if let Some(seq) = pending_seq(state.state.db()) {
                for (field, suffix) in [("text", "text"), ("thinking", "thinking")] {
                    let text = state
                        .streams
                        .get(&format!("msg.{seq}.{suffix}"))
                        .map(|stream| stream.text.clone())
                        .unwrap_or_default();
                    event = event.with(field, Value::str(text));
                }
            }
        }
        state.state.set_clock(now_ms());
        let previous = pending_seq(state.state.db());
        let mut admission = kernel_queue::Admission::default();
        let inner = state.state.dispatch_checked(event, |outcome| {
            if self.is_closed() {
                return Err(misa_reframe::Fault::new(
                    "closed_scope",
                    "Session owner is closed",
                ));
            }
            admission = self.kernel_budget.reserve(class, outcome.effects.len())?;
            Ok(())
        });
        let outcome = Outcome { inner, admission };
        if outcome.committed() {
            state.operations.reconcile(state.state.db());
            self.operation_deadline
                .send_replace(state.operations.deadline());
            let State {
                state: loop_, view, ..
            } = state;
            view.advance(loop_.db(), &outcome.changes, &self.sections, loop_.rev());
        }
        let current = pending_seq(state.state.db());
        let mut stream_updates = Vec::new();
        if previous != current {
            if let Some(seq) = previous {
                for suffix in ["text", "thinking"] {
                    let id = format!("msg.{seq}.{suffix}");
                    state.streams.remove(&id);
                    let update = misa_proto::sync::StreamUpdate::End { id };
                    stream_updates.push(update.clone());
                    self.emit(SessionEvent::Stream { update });
                }
            }
            if let Some(seq) = current {
                for (suffix, role) in [
                    ("text", "message.assistant"),
                    ("thinking", "message.assistant.thinking"),
                ] {
                    let stream = misa_proto::sync::Stream {
                        id: format!("msg.{seq}.{suffix}"),
                        role: role.into(),
                        text: String::new(),
                    };
                    state.streams.insert(stream.id.clone(), stream.clone());
                    let update = misa_proto::sync::StreamUpdate::Current { stream };
                    stream_updates.push(update.clone());
                    self.emit(SessionEvent::Stream { update });
                }
            }
        }
        if outcome.committed() || !stream_updates.is_empty() {
            state.publications.commit(stream_updates);
        }
        outcome
    }

    fn append_stream(&self, event: &Event) {
        let mut state = self.state();
        let pending = state
            .state
            .db()
            .get("session")
            .and_then(|session| session.get("pending"));
        if pending
            .and_then(|pending| pending.get("request"))
            .and_then(Value::as_str)
            != event.get("id").and_then(Value::as_str)
        {
            return;
        }
        let Some(seq) = pending_seq(state.state.db()) else {
            return;
        };
        let suffix = if event.kind.ends_with("thinking") {
            "thinking"
        } else {
            "text"
        };
        let id = format!("msg.{seq}.{suffix}");
        let text = event
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let Some(stream) = state.streams.get_mut(&id) else {
            return;
        };
        let offset = stream.text.len();
        stream.text.push_str(text);
        state.stream_bytes += text.len() as u64;
        let update = misa_proto::sync::StreamUpdate::Append {
            id,
            offset,
            text: text.into(),
        };
        state.publications.commit(vec![update.clone()]);
        self.emit(SessionEvent::Stream { update });
    }

    pub fn streams(&self) -> Vec<misa_proto::sync::Stream> {
        self.state().streams.values().cloned().collect()
    }

    fn perform(&self, outcome: &Outcome) {
        let send = |request| self.to_kernel.send(request, &outcome.admission);
        for effect in &outcome.effects {
            match effect.kind.as_str() {
                "owner.input.continue" => self.continue_input(effect),
                "kernel.provider.call" => {
                    let _ = send(Request::ProviderCall {
                        id: fields::text(effect, "id"),
                        provider: fields::text(effect, "provider"),
                        model: fields::text(effect, "model"),
                        messages: fields::value(effect, "messages"),
                        tools: fields::value(effect, "tools"),
                        settings: fields::value(effect, "settings"),
                    });
                }
                "kernel.usage" => {
                    let _ = send(Request::Usage {
                        id: fields::text(effect, "id"),
                        provider: fields::text(effect, "provider"),
                    });
                }
                "kernel.models.discover" => {
                    let _ = send(Request::DiscoverModels {
                        id: fields::text(effect, "id"),
                        provider: fields::text(effect, "provider"),
                    });
                }
                "kernel.tool.run" => {
                    self.approve_tool(effect, &outcome.admission);
                }
                "kernel.log.append" => {
                    let _ = send(Request::Append {
                        conversation: fields::text(effect, "conversation"),
                        kind: fields::text(effect, "kind"),
                        data: fields::value(effect, "data"),
                    });
                }
                // Reading the log back, which is what `/resume` is: a listing when no
                // conversation was named, and the entries themselves when one was.
                "kernel.log.list" => {
                    let _ = send(Request::Conversations {
                        id: fields::text(effect, "id"),
                    });
                }
                "kernel.log.load" => {
                    let _ = send(Request::Load {
                        conversation: fields::text(effect, "conversation"),
                        after: fields::int(effect, "after"),
                        limit: fields::int(effect, "limit").max(1) as usize,
                    });
                }
                // A file read where the daemon is, which is the only place a path means
                // anything. `/attach <path>` is the whole of this.
                "kernel.blob.file" => {
                    let _ = send(Request::BlobFile {
                        id: fields::text(effect, "id"),
                        path: fields::text(effect, "path"),
                    });
                }
                "kernel.attempt.started" => {
                    let conversation = fields::text(effect, "conversation");
                    let _ = send(Request::AttemptStarted {
                        id: format!("{}:{}", self.incarnation, fields::text(effect, "id")),
                        conversation: (!conversation.is_empty()).then_some(conversation),
                        parent: self.parent_attempt.clone(),
                        provider: fields::text(effect, "provider"),
                        model: fields::text(effect, "model"),
                        kind: fields::text(effect, "kind"),
                    });
                }
                // A credential is a slot and, one action at a time, what to do with it. The
                // bytes of a key travel this way once — from a client's field, through the
                // session, into the daemon — and never come back.
                "kernel.credential" => {
                    let action = match fields::text(effect, "action").as_str() {
                        "set" => CredentialAction::Set {
                            slot: fields::text(effect, "slot"),
                            account: fields::text(effect, "account"),
                            value: fields::text(effect, "value"),
                        },
                        "delete" => CredentialAction::Delete {
                            slot: fields::text(effect, "slot"),
                        },
                        "delete_account" => CredentialAction::DeleteAccount {
                            slot: fields::text(effect, "slot"),
                            account: fields::text(effect, "account"),
                        },
                        "select" => CredentialAction::Select {
                            slot: fields::text(effect, "slot"),
                            account: fields::text(effect, "account"),
                        },
                        // A device code rather than a value: the provider names the flow, and
                        // the daemon is the only side that knows how to run one.
                        "oauth" => CredentialAction::OAuth {
                            provider: fields::text(effect, "provider"),
                        },
                        "oauth_account" => CredentialAction::OAuthAccount {
                            provider: fields::text(effect, "provider"),
                            account: fields::text(effect, "account"),
                        },
                        "cancel_oauth" => CredentialAction::CancelOAuth {
                            request: fields::text(effect, "request"),
                        },
                        // A listing asks for nothing and stores nothing, which makes it the
                        // safe reading of an action a handler spelled wrong.
                        _ => CredentialAction::List,
                    };
                    let _ = send(Request::Credential {
                        id: fields::text(effect, "id"),
                        action,
                    });
                }
                "kernel.attempt.settled" => {
                    let _ = send(Request::AttemptSettled {
                        id: format!("{}:{}", self.incarnation, fields::text(effect, "id")),
                        status: fields::text(effect, "status"),
                        input_tokens: fields::int(effect, "input_tokens"),
                        output_tokens: fields::int(effect, "output_tokens"),
                        cost_micros: fields::int(effect, "cost_micros"),
                    });
                }
                "wire.event" => {
                    if let Ok(event) = wire::parse::<SessionEvent>(&fields::value(effect, "event"))
                    {
                        self.emit(event);
                    }
                }
                "owner.checkpoint.failed" => {
                    self.dispatch(
                        Event::new("kernel/log.failed")
                            .with("kind", Value::str("operations.checkpoint"))
                            .with(
                                "data",
                                Value::map([("checkpoint", fields::value(effect, "checkpoint"))]),
                            ),
                    );
                }
                // The interpreter accepted the effect, so this arm is unreachable;
                // doing nothing is still better than panicking a session.
                _ => {}
            }
        }
    }

    fn emit(&self, event: SessionEvent) {
        let _ = self.events.send(Emission { event });
    }

    /// Publish an owner diagnostic through ordinary observable domain state.
    pub fn notice(&self, level: crate::Level, text: impl Into<String>) {
        let faults = self.dispatch(
            Event::new("owner/notice")
                .with("level", Value::str(level.as_str()))
                .with("text", Value::str(text.into())),
        );
        if let Some(fault) = faults.into_iter().next() {
            self.shutdown_with_fault(fault);
        }
    }

    /// Interpret an intent. The only place an intent is understood.
    ///
    /// Local domain adapter. Network callers enter through the installed command
    /// registry, which checks authority and arguments before choosing a transition.
    #[cfg(test)]
    pub(crate) fn intent(&self, intent: Intent) -> Vec<Fault> {
        let event = match intent {
            Intent::Interrupt { text, attachments } => Event::new("intent/interrupt")
                .with("prompt", Value::str(text))
                .with("attachments", encode_blobs(&attachments)),
            // A client may name attachments it has already put in the store. The hashes
            // travel as facts; whether the bytes are there is the store's answer, checked
            // when the request is built, so a client cannot make a session believe in an
            // image by naming one.
            Intent::Prompt { text, attachments } => Event::new("intent/prompt")
                .with("text", Value::str(text))
                .with("attachments", encode_blobs(&attachments)),
            Intent::Command { name, args } => Event::new("intent/command")
                .with("name", Value::str(name))
                .with("args", args),
            Intent::Action {
                node,
                action,
                args,
                fields: submitted,
            } => Event::new("intent/action")
                .with("node", Value::str(node))
                .with("action", Value::str(action))
                .with("args", args)
                .with("fields", encode_fields(&submitted)),
        };
        self.dispatch(event)
    }

    /// Every query this session answers.
    pub fn queries(&self) -> Vec<String> {
        let state = self.state();
        let mut queries: Vec<String> = state
            .state
            .registry()
            .query_ids()
            .map(str::to_string)
            .collect();
        queries.extend(views::queries());
        queries.sort();
        queries.dedup();
        queries
    }

    /// The view a client would see right now, without going through a query.
    pub fn view(&self) -> Result<Node, Fault> {
        match self.read(&Query::new(misa_proto::VIEW_QUERY))? {
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
        let state = self.state();
        completions::on_demand(
            state.state.db(),
            source,
            prefix,
            limit.unwrap_or(misa_proto::preparation::DEFAULT_CANDIDATES),
        )
    }

    /// Read a query. The view is answered here; every other query goes through the
    /// loop's own scope.
    pub fn read(&self, query: &Query) -> Result<Reading, Fault> {
        let mut state = self.state();
        if query.id == misa_proto::VIEW_QUERY {
            return Ok(Reading::View(state.view.tree.snapshot()));
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
/// This list is the whole of the session's authority over its kernel. An effect outside it
/// fails the transaction before anything commits, so a policy cannot ask for something the
/// session does not understand and have its state land anyway.
///
/// Public because it is also the *composition's* question: a caller that loaded a plugin has to
/// know whether this session can run what the plugin asks for, and the answer belongs here
/// rather than in a second list somewhere else that would drift from this one.
pub struct AcceptedEffects;

impl Interpreter for AcceptedEffects {
    fn accepts(&self, effect: &Effect) -> Result<(), String> {
        match effect.kind.as_str() {
            "kernel.provider.call"
            | "kernel.tool.run"
            | "kernel.log.append"
            | "kernel.log.list"
            | "kernel.log.load"
            | "kernel.usage"
            | "kernel.blob.file"
            | "kernel.attempt.started"
            | "kernel.attempt.settled"
            | "kernel.models.discover"
            | "kernel.credential"
            | "wire.event" => Ok(()),
            "owner.input.continue" => Ok(()),
            other => Err(format!("this session does not know the effect `{other}`")),
        }
    }
}

fn pending_seq(db: &Value) -> Option<i64> {
    db.get("session")?.get("pending")?.get("seq")?.as_i64()
}

pub fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use misa_kernel::{Provider, ScriptedProvider, Turn};
    use std::time::Duration;

    fn runtime() -> Arc<Runtime> {
        let provider: Arc<dyn Provider> = ScriptedProvider::new([
            Turn::call("echo", Value::str("hello"), Turn::say("all done")),
            Turn::say("all done"),
        ])
        .named("claude");
        Runtime::start(
            "demo",
            "a demo session",
            Some("demo".into()),
            Arc::new(misa_kernel::LocalKernel::new(provider)),
            "claude",
            "claude-sonnet-5",
            Value::Null,
        )
    }

    pub(crate) fn view(runtime: &Runtime) -> Node {
        match runtime.read(&Query::new(misa_proto::VIEW_QUERY)).unwrap() {
            Reading::View(node) => node,
            Reading::Data(_) => panic!("expected a view"),
        }
    }

    pub(crate) fn transcript(runtime: &Runtime) -> String {
        misa_lines::to_plain(&misa_lines::render(
            &view(runtime),
            &misa_render::Theme::plain(),
            100,
        ))
    }

    /// Wait until the view says something, the way a client waits: by reading what it can
    /// actually see.
    async fn wait_for(runtime: &Arc<Runtime>, want: impl Fn(&str) -> bool) -> String {
        for _ in 0..500 {
            let text = transcript(runtime);
            if want(&text) {
                return text;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!(
            "the view never said it:
{}",
            transcript(runtime)
        );
    }

    /// Wait for a notice that says a particular thing.
    ///
    /// An outcome that is not part of the transcript — a token stored, an authorization
    /// refused — arrives as a notice, which is a thing a client sees and a log does not keep.
    /// It takes what the notice should say because a session's own notices arrive on the same
    /// stream, and the one that matters is usually not the first.
    pub(crate) async fn wait_notice(
        events: &mut tokio::sync::broadcast::Receiver<Emission>,
        want: &str,
    ) -> String {
        for _ in 0..500 {
            if let Ok(emission) = events.try_recv()
                && let SessionEvent::Notice { text, .. } = emission.event
                && text.contains(want)
            {
                return text;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("no notice said `{want}`");
    }

    /// A device-authorization server this machine runs: one request per connection, the
    /// answers scripted in order.
    ///
    /// The shipped flows name services on the internet, which is why the flows a daemon knows
    /// are a composition decision at all: without a way to point one at a local address, the
    /// only test of a device flow would be a test that needs an account.
    async fn device_server() -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let address = listener.local_addr().expect("an address");
        tokio::spawn(async move {
            let script = [
                (
                    "200 OK",
                    r#"{"device_code":"dev","user_code":"AAAA-BBBB","verification_uri":"https://example.invalid/device","interval":1,"expires_in":30}"#,
                ),
                (
                    "200 OK",
                    r#"{"access_token":"an-access","refresh_token":"a-refresh","expires_in":3600,"account_id":"acct-9"}"#,
                ),
            ];
            for (status, body) in script {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                // Read the request to the end, so the client is never left writing into a
                // socket nobody is reading.
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                loop {
                    let Ok(read) = socket.read(&mut buffer).await else {
                        break;
                    };
                    if read == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..read]);
                    if let Some(end) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&request[..end]).to_lowercase();
                        let length: usize = head
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .and_then(|value| value.trim().parse().ok())
                            .unwrap_or(0);
                        if request.len() >= end + 4 + length {
                            break;
                        }
                    }
                }
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.flush().await;
            }
        });
        format!("http://{address}")
    }

    /// A session whose daemon can authorize `kimi-coding` against `base`.
    fn flow_runtime(base: &str) -> Arc<Runtime> {
        let provider: Arc<dyn Provider> = ScriptedProvider::always("no turns here");
        let kernel = misa_kernel::LocalKernel::new(provider).with_flow(
            "kimi-coding",
            misa_kernel::oauth::Flow {
                kind: misa_kernel::oauth::Kind::Rfc8628,
                client_id: "a-client",
                authorization_url: Box::leak(format!("{base}/device").into_boxed_str()),
                token_url: Box::leak(format!("{base}/token").into_boxed_str()),
                verification_url: "https://example.invalid/device",
            },
        );
        Runtime::start(
            "demo",
            "a demo session",
            None,
            Arc::new(kernel),
            "scripted",
            "scripted-1",
            Value::Null,
        )
    }

    /// Wait until the session is idle again, the way a client waits: by watching
    /// the state it can actually see.
    pub(crate) async fn settle(runtime: &Arc<Runtime>) {
        for _ in 0..400 {
            if runtime.status() == "idle" {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!(
            "the session never settled; it is still {}",
            runtime.status()
        );
    }

    #[tokio::test]
    async fn a_prompt_becomes_a_message_the_view_shows() {
        let runtime = runtime();
        let faults = runtime.intent(Intent::Prompt {
            text: "write a haiku".into(),
            attachments: vec![],
        });
        assert!(faults.is_empty(), "{faults:?}");
        let text = wait_for(&runtime, |text| text.contains("write a haiku")).await;
        assert!(text.contains("write a haiku"), "{text}");
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
                misa_proto::view::BlobRef {
                    hash: hash.clone(),
                    len: 4,
                    media: Some("image/png".into()),
                },
                misa_proto::view::BlobRef {
                    hash: "../../etc/shadow".into(),
                    len: 0,
                    media: None,
                },
                // The same blob twice is one attachment: two copies of one hash would be two
                // pictures of the same file.
                misa_proto::view::BlobRef {
                    hash: hash.clone(),
                    len: 4,
                    media: Some("image/png".into()),
                },
            ],
        });
        assert!(faults.is_empty(), "{faults:?}");

        // Every client receives the image reference after the log acknowledges it.
        settle(&runtime).await;
        let drawn = match runtime.read(&Query::new(misa_proto::VIEW_QUERY)).unwrap() {
            Reading::View(node) => node,
            Reading::Data(_) => panic!("expected a view"),
        };
        let attachment = misa_proto::view::find(&drawn, &format!("msg.1.attachment.{hash}"))
            .expect("the attachment is in the view");
        match &attachment.kind {
            misa_proto::view::Kind::Image { blob, .. } => assert_eq!(blob.hash, hash),
            _ => panic!("an attachment that is not an image is an attachment nothing can show"),
        }
        assert!(
            misa_proto::view::find(&drawn, "attachment.1").is_none(),
            "a name that is not a hash became an attachment"
        );

        let plain = transcript(&runtime);
        assert!(
            plain.contains("image/png"),
            "a client that cannot draw is not told what is there:\n{plain}"
        );
        assert!(
            !plain.contains("etc/shadow"),
            "something that is not a hash reached the transcript:\n{plain}"
        );
    }

    #[tokio::test]
    async fn usage_refresh_coalesces_and_ignores_stale_completions() {
        let runtime = runtime();
        let command = || Intent::Command {
            name: "usage".into(),
            args: Value::Null,
        };
        runtime.intent(command());
        runtime.intent(command());
        runtime.intent(command());
        let response = |id: &str, label: &str| {
            Event::new("kernel/usage").with("id", Value::str(id)).with(
                "facts",
                Value::map([
                    ("unavailable", Value::Bool(false)),
                    (
                        "windows",
                        Value::list([Value::map([
                            ("label", Value::str(label)),
                            ("remaining", Value::Int(75)),
                        ])]),
                    ),
                ]),
            )
        };
        assert!(
            runtime
                .dispatch(response("usage.1", "Current quota"))
                .is_empty()
        );
        assert!(transcript(&runtime).contains("Current quota"));
        runtime.dispatch(response("usage.1", "Stale quota"));
        assert!(!transcript(&runtime).contains("Stale quota"));
        runtime.dispatch(
            Event::new("kernel/usage")
                .with("id", Value::str("usage.2"))
                .with("facts", Value::map([("unavailable", Value::Bool(true))])),
        );
        let report = transcript(&runtime);
        assert!(report.contains("Unavailable"));
        assert!(!report.contains("Current quota"));
        runtime.dispatch(response("usage.2", "Duplicate quota"));
        assert!(!transcript(&runtime).contains("Duplicate quota"));
    }

    #[tokio::test]
    async fn device_panel_cancel_stops_authorization_and_late_prompts_do_not_reopen_it() {
        let base = device_server().await;
        let runtime = flow_runtime(&base);
        let mut events = runtime.subscribe_events();
        assert!(
            runtime
                .intent(Intent::Command {
                    name: "login".into(),
                    args: Value::str("kimi-coding")
                })
                .is_empty()
        );
        wait_for(&runtime, |text| text.contains("AAAA-BBBB")).await;
        let tree = view(&runtime);
        let panel = misa_proto::view::find(&tree, "authorize").unwrap();
        assert!(
            panel
                .actions
                .iter()
                .any(|action| action.id == "credential.cancel")
        );
        assert!(
            runtime
                .intent(Intent::Action {
                    node: "authorize".into(),
                    action: "credential.cancel".into(),
                    args: Value::Null,
                    fields: vec![]
                })
                .is_empty()
        );
        wait_notice(&mut events, "Authorization cancelled").await;
        assert!(!transcript(&runtime).contains("AAAA-BBBB"));
        runtime.dispatch(
            Event::new("kernel/credential.prompt")
                .with("id", Value::str("oauth:demo:1"))
                .with("code", Value::str("LATE-CODE")),
        );
        assert!(!transcript(&runtime).contains("LATE-CODE"));
    }

    /// Something a command can be run with, for the test below.
    fn argument_for(command: &str) -> Value {
        match command {
            "model" => Value::str("claude-sonnet-5"),
            "effort" => Value::str("medium"),
            "login" | "logout" => Value::str("openai"),
            "status" => Value::str("openai"),
            "account" => Value::map([
                ("provider", Value::str("openai")),
                ("account", Value::str("default")),
            ]),
            "attach" | "image" => Value::str("/tmp/there-is-no-file-here"),
            "resume" => Value::str("c1"),
            _ => Value::Null,
        }
    }

    #[tokio::test]
    async fn every_declared_command_is_one_the_loop_can_actually_run() {
        // The promise a declaration makes is that a client may send it. `/status`, `/usage`,
        // `/login`, `/logout`, and `/attach` were handled by the loop and declared to nobody,
        // so a palette could not offer them and a client that sent one anyway was told there
        // was no such command: a frontend could not do what the previous terminal could.
        let runtime = runtime();
        let declared = crate::catalog::commands();
        assert!(
            declared.len() >= 10,
            "the declarations are the whole of what a client sees"
        );
        for command in declared {
            let faults = runtime.intent(Intent::Command {
                name: command.id.clone(),
                args: argument_for(&command.id),
            });
            assert!(
                faults.is_empty(),
                "`/{}` is declared and cannot run: {faults:?}",
                command.id
            );
        }
    }

    #[tokio::test]
    async fn resume_reads_a_conversation_back_out_of_the_log() {
        // `/resume <id>` asked the daemon for a conversation and then dropped the answer: the
        // `Loaded` event was mapped to a no-op, so the command could not have loaded anything
        // even once its effect was accepted. The whole path is here — the two journalled
        // entries, the request, the answer, and the transcript rebuilt from it.
        let runtime = runtime();
        let mut events = runtime.subscribe_events();
        runtime.intent(Intent::Prompt {
            text: "remember this".into(),
            attachments: vec![],
        });
        settle(&runtime).await;

        let faults = runtime.intent(Intent::Command {
            name: "resume".into(),
            args: Value::str("demo"),
        });
        assert!(faults.is_empty(), "{faults:?}");
        let notice = wait_notice(&mut events, "reloaded").await;
        assert!(
            notice.contains("reloaded 3 messages"),
            "the log was not read back: {notice}"
        );

        let text = transcript(&runtime);
        assert!(text.contains("remember this"), "{text}");
        assert!(
            text.contains("all done"),
            "the answer came back with it:\n{text}"
        );
    }

    #[tokio::test]
    async fn attach_reads_a_file_where_the_daemon_is() {
        // `/attach <path>` is a capability, not a client's upload: the path means something
        // only on the machine the daemon runs on. Its effect was refused too, so the command
        // was declared and unreachable.
        let path = std::env::temp_dir().join("misa-session-attach-test.txt");
        std::fs::write(&path, b"a note").expect("a file");
        let runtime = runtime();
        let mut events = runtime.subscribe_events();
        let faults = runtime.intent(Intent::Command {
            name: "attach".into(),
            args: Value::str(path.to_string_lossy().to_string()),
        });
        assert!(faults.is_empty(), "{faults:?}");
        let notice = wait_notice(&mut events, "attached").await;
        assert!(notice.contains("6 bytes"), "{notice}");
        let _ = std::fs::remove_file(&path);
    }

    #[tokio::test]
    async fn a_client_can_start_a_device_flow_and_the_code_arrives_as_a_panel() {
        // The item this closes: a subscription used to be authorizable only by
        // `misa-daemon login <provider>`, on the daemon's own terminal, because a client had
        // nothing to paste. Now the client asks, the daemon starts the flow, and the code
        // comes back as a panel — which is a thing every frontend already draws.
        let base = device_server().await;
        let runtime = flow_runtime(&base);
        let mut events = runtime.subscribe_events();
        let faults = runtime.intent(Intent::Command {
            name: "login".into(),
            args: Value::str("kimi-coding"),
        });
        assert!(faults.is_empty(), "{faults:?}");
        // Not the secret panel: a service that hands out tokens has no value for anybody to
        // type, and a text box would be a lie.
        assert!(
            misa_proto::view::find(&view(&runtime), "login").is_none(),
            "a subscription is not a key"
        );

        // The code, and where to type it.
        let shown = wait_for(&runtime, |text| text.contains("AAAA-BBBB")).await;
        assert!(shown.contains("Authorize `kimi-coding`"), "{shown}");
        assert!(shown.contains("https://example.invalid/device"), "{shown}");

        // Then the verdict, and the panel goes away with it: a code that has been approved
        // must not sit on a screen looking like something still to do.
        let notice = wait_notice(&mut events, "stored a token").await;
        assert!(
            notice.contains("stored a token for `kimi-coding`"),
            "{notice}"
        );
        assert!(notice.contains("acct-9"), "{notice}");
        let after = wait_for(&runtime, |text| !text.contains("AAAA-BBBB")).await;
        assert!(
            after.contains("kimi-coding"),
            "the session still works:
{after}"
        );
    }

    #[tokio::test]
    async fn a_key_is_a_panel_with_a_field_and_the_action_that_stores_it() {
        // The other half of `/login`: a service that takes a key has a form, and the field is
        // a secret, which is a shape a client can draw without knowing what it is for.
        let runtime = runtime();
        let faults = runtime.intent(Intent::Command {
            name: "login".into(),
            args: Value::str("anthropic"),
        });
        assert!(faults.is_empty(), "{faults:?}");
        let node = view(&runtime);
        let panel = misa_proto::view::find(&node, "login").expect("a panel");
        assert_eq!(panel.label.as_deref(), Some("Credential for `anthropic`"));
        let input = misa_proto::view::find(&node, "panel.input").expect("a form");
        assert_eq!(input.actions.len(), 1);
        assert_eq!(input.actions[0].id, "panel.submit");
        assert_eq!(input.actions[0].on, misa_proto::view::ActionOn::Submit);
        match &input.kind {
            misa_proto::view::Kind::Fields { fields } => {
                assert_eq!(fields.len(), 1);
                assert!(fields[0].secret);
            }
            other => panic!("expected a field, got {other:?}"),
        }
        // And it can be got rid of by the action the session offered, which is the only way
        // anything in this system is got rid of.
        assert!(
            panel
                .actions
                .iter()
                .any(|action| action.id == "panel.close")
        );
    }

    #[tokio::test]
    async fn a_report_is_in_the_view_and_not_only_in_the_sessions_own_state() {
        // `/status` wrote a panel into the session's state that no frontend could see: the
        // panel existed for nobody. It is in the tree now, and a row is a fact rather than an
        // input, so a surface that gives every field a text box does not offer an edit that
        // could never be saved.
        let runtime = runtime();
        runtime.intent(Intent::Command {
            name: "status".into(),
            args: Value::str("openai"),
        });
        let node = view(&runtime);
        let panel = misa_proto::view::find(&node, "status").expect("a panel");
        assert_eq!(panel.label.as_deref(), Some("Status for `openai`"));
        let rows = misa_proto::view::find(&node, "panel.rows").expect("rows");
        assert_eq!(
            rows.children.len(),
            1,
            "status reports the selected provider"
        );
        match &rows.children[0].kind {
            misa_proto::view::Kind::Fields { fields } => assert!(
                fields[0].read_only,
                "a row a client could type into is a row nobody can save"
            ),
            other => panic!("expected a row, got {other:?}"),
        }
        // The same tree is what every frontend renders, so the transcript says it too.
        let text = transcript(&runtime);
        assert!(text.contains("Status for `openai`"), "{text}");
        assert!(text.contains("logged out"), "{text}");
    }

    #[tokio::test]
    async fn changing_a_credential_reaches_the_daemon_and_its_answer_comes_back() {
        // The bug this covers: `kernel.credential` was not an effect the session's interpreter
        // accepted, so the transaction failed before it committed and `/logout` — and the
        // login panel's Store button — did nothing at all but report a fault.
        let runtime = runtime();
        let mut events = runtime.subscribe_events();
        let faults = runtime.intent(Intent::Command {
            name: "logout".into(),
            args: Value::str("anthropic"),
        });
        assert!(faults.is_empty(), "the effect was refused: {faults:?}");
        let notice = wait_notice(&mut events, "no credential").await;
        assert!(
            notice.contains("no credential for `anthropic`"),
            "the daemon did not answer: {notice}"
        );
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

        let text = wait_for(&runtime, |text| text.contains("cargo build")).await;
        assert!(text.contains("cargo build"), "{text}");
        assert!(text.contains("pid 4242"), "{text}");
        assert!(text.contains("exited 0"), "{text}");
        assert!(text.contains("12.4s"), "{text}");
        assert!(text.contains("/tmp/shell/p1-cargo-build.log"), "{text}");

        // Told to the model as a system message, which is where both providers read a note that
        // is not part of the conversation.
        let messages = match runtime.read(&Query::new("session.conversation")).unwrap() {
            Reading::Data(value) => value,
            Reading::View(_) => panic!("expected data"),
        };
        let last = messages
            .as_list()
            .and_then(|messages| messages.last())
            .expect("a message");
        assert_eq!(last.get("role").and_then(Value::as_str), Some("system"));
        assert!(
            last.get("text")
                .and_then(Value::as_str)
                .unwrap()
                .contains("cargo build")
        );

        // It does not start a turn: a turn is something a person asks for, and answering every
        // finished process with a provider call would spend money behind their back.
        assert_eq!(runtime.status(), "idle");
    }

    #[tokio::test]
    async fn a_tool_call_round_trips_and_the_turn_finishes() {
        let runtime = runtime();
        runtime.intent(Intent::Prompt {
            text: "use the tool".into(),
            attachments: vec![],
        });
        settle(&runtime).await;
        let text = transcript(&runtime);
        assert!(text.contains("use the tool"), "{text}");
        assert!(text.contains("echo"), "the tool call is missing:\n{text}");
        assert!(
            text.contains("hello"),
            "the tool result is missing:\n{text}"
        );
        assert!(
            text.contains("all done"),
            "the continuation is missing:\n{text}"
        );
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
        assert!(
            faults[0].message.contains("nope.does.not.exist"),
            "{:?}",
            faults[0]
        );
        // And the session is still usable, which is the point of containment.
        assert!(
            runtime
                .intent(Intent::Command {
                    name: "help".into(),
                    args: Value::Null
                })
                .is_empty()
        );
    }

    #[tokio::test]
    async fn an_unknown_command_is_reported_as_a_notice_rather_than_refused() {
        let runtime = runtime();
        let faults = runtime.intent(Intent::Command {
            name: "nonsense".into(),
            args: Value::Null,
        });
        assert!(faults.is_empty());
        let text = transcript(&runtime);
        assert!(text.contains("nonsense"), "{text}");
    }

    #[tokio::test]
    async fn a_command_can_be_answered_by_an_ephemeral_event() {
        let runtime = runtime();
        let mut events = runtime.subscribe_events();
        runtime.intent(Intent::Command {
            name: "cost".into(),
            args: Value::Null,
        });
        let emission = events.try_recv().expect("a notice");
        assert!(matches!(emission.event, SessionEvent::Notice { .. }));
    }

    #[tokio::test]
    async fn the_view_query_is_advertised_and_answers() {
        let runtime = runtime();
        assert!(
            runtime
                .queries()
                .iter()
                .any(|query| query == misa_proto::VIEW_QUERY)
        );
        assert_eq!(view(&runtime).role, "session");
    }

    #[tokio::test]
    async fn a_data_query_is_answered_as_data() {
        let runtime = runtime();
        settle(&runtime).await;
        match runtime.read(&Query::new("session.status")).unwrap() {
            Reading::Data(value) => {
                assert_eq!(value.get("status").and_then(Value::as_str), Some("idle"))
            }
            Reading::View(_) => panic!("expected data"),
        }
    }

    #[tokio::test]
    async fn the_view_is_cached_until_the_database_changes() {
        let runtime = runtime();
        let first = view(&runtime);
        let state = runtime.state.lock().unwrap();
        assert_eq!(first, state.view.tree.snapshot());
        assert_eq!(state.view.version.rev, state.state.rev());
    }

    #[tokio::test]
    async fn repeated_reads_return_the_same_tree() {
        let runtime = runtime();
        let plain = match runtime.read(&Query::new(misa_proto::VIEW_QUERY)).unwrap() {
            Reading::View(node) => node,
            Reading::Data(_) => panic!("expected a view"),
        };
        let rich = match runtime.read(&Query::new(misa_proto::VIEW_QUERY)).unwrap() {
            Reading::View(node) => node,
            Reading::Data(_) => panic!("expected a view"),
        };
        assert_eq!(plain, rich);
    }

    #[tokio::test]
    async fn legacy_query_arguments_cannot_truncate_the_canonical_document() {
        let runtime = runtime();
        let narrow = runtime
            .read(&Query::new(misa_proto::VIEW_QUERY).arg(Value::Int(1)))
            .unwrap();
        let wide = runtime
            .read(&Query::new(misa_proto::VIEW_QUERY).arg(Value::Int(40)))
            .unwrap();
        match (narrow, wide) {
            (Reading::View(a), Reading::View(b)) => {
                assert_eq!(a, b);
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

    pub fn parse<T: serde::Serialize + serde::de::DeserializeOwned>(
        value: &Value,
    ) -> Result<T, String> {
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
#[cfg(test)]
fn encode_fields(fields: &[misa_proto::view::Field]) -> Value {
    Value::list(
        fields
            .iter()
            .map(|field| {
                Value::map([
                    ("id", Value::str(&field.id)),
                    ("value", Value::str(&field.value)),
                ])
            })
            .collect::<Vec<_>>(),
    )
}

/// Attachments a client named, as the shape a message carries them in.
///
/// The same fields `/attach` writes, so an attachment is one shape however it arrived and
/// the kernel resolves every one of them the same way: a hash it holds becomes bytes, and a
/// hash it does not becomes a sentence saying so. Nothing here is trusted, because nothing
/// here is anything but a name.
#[cfg(test)]
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

#[cfg(test)]
mod contribution_tests {
    use super::*;
    use crate::tests::{settle, transcript, view, wait_notice};

    use std::sync::Arc;

    use misa_kernel::{Provider, ScriptedProvider, Store};
    use misa_proto::view::{Kind, Node};
    use misa_reframe::{Event, FnHandler, Subscription, Tx};
    use misa_value::Value;

    /// A handler a composition brought: it writes into the root the contribution declared.
    fn adopting() -> Arc<dyn misa_reframe::Handler> {
        Arc::new(FnHandler::new(
            "test.contribution",
            |tx: &mut Tx<'_>, event: &Event| {
                tx.set("guest.seen", Value::str(event.kind.as_str()))?;
                Ok(())
            },
        ))
    }

    /// A subscription over the root that handler writes, so a test can read it the way a client
    /// would: as a query.
    fn reading() -> Subscription {
        Subscription::Read {
            read: Arc::new(|db, _query, _previous| {
                Ok({
                    db.get("guest")
                        .and_then(|guest| guest.get("seen"))
                        .cloned()
                        .unwrap_or(Value::Null)
                })
            }),
        }
    }

    fn contribution() -> Contribution {
        Contribution::new()
            .with_root("guest", Value::map([]))
            .expect("a root the session does not own")
            .with_handler("intent/prompt", 10, adopting())
            .with_subscription("guest.seen", reading())
    }

    fn session(contribution: Contribution) -> Arc<Runtime> {
        let provider: Arc<dyn Provider> = ScriptedProvider::always("an answer");
        Runtime::start_with(
            "demo",
            "a demo session",
            None,
            Arc::new(misa_kernel::LocalKernel::new(provider)),
            "scripted",
            "scripted-1",
            Value::Null,
            contribution,
        )
    }

    #[test]
    fn a_contribution_may_not_claim_a_root_the_session_already_owns() {
        // The manifest is what says which names are taken, and this is the whole of what stops
        // a contribution from overwriting what the loop decided: `session.status` is not a
        // plugin's to write.
        let fault = Contribution::new()
            .with_root("session", Value::Null)
            .unwrap_err();
        assert_eq!(fault.code, "composition.root");
        assert!(fault.message.contains("session"), "{}", fault.message);
        // A root is one top-level name, not a path.
        assert!(
            Contribution::new()
                .with_root("guest.turns", Value::Null)
                .is_err()
        );
        assert!(Contribution::new().with_root("", Value::Null).is_err());
        // And the same name twice is a mistake rather than two roots.
        assert!(
            Contribution::new()
                .with_root("guest", Value::Null)
                .expect("a root")
                .with_root("guest", Value::Null)
                .is_err()
        );
        assert!(Contribution::new().with_root("guest", Value::Null).is_ok());
    }

    /// A section a composition contributed, as a test builds one.
    fn presenting(plugin: &'static str) -> views::Section {
        views::Section {
            inputs: vec![misa_value::Path::root()],
            namespace: format!("plugin.{plugin}"),
            build: std::sync::Arc::new(|db| {
                let mut tree = Node::section("test.widget").id("widget");
                tree.label = Some(format!(
                    "{} bytes of state",
                    misa_lines::to_plain(&[]).len()
                ));
                tree.children.push(
                    Node::new(
                        "test.note",
                        Kind::Status {
                            text: format!("seen {}", db.get("session").is_some()),
                        },
                    )
                    .id("note"),
                );
                Ok(tree)
            }),
        }
    }

    #[test]
    fn a_contribution_may_not_claim_an_affordance_the_session_has() {
        // An action is what the session's own vocabulary is made of. A contribution that declared
        // `panel.close` would have the session's handler do its work under a name it did not
        // choose, which is the shadowing this check exists to make impossible.
        let fault = Contribution::new().with_action("panel.close").unwrap_err();
        assert_eq!(fault.code, "composition.action");
        assert!(fault.message.contains("panel.close"), "{}", fault.message);
        assert!(
            Contribution::new()
                .with_action("refresh")
                .unwrap()
                .with_action("refresh")
                .is_err(),
            "the same action twice is a mistake"
        );
    }

    #[tokio::test]
    async fn every_action_the_session_handles_is_one_it_knows_by_name() {
        // The list a composition is kept out of is the list this handler answers. If the two drift,
        // a contribution could claim something the session handles and nobody would notice.
        let runtime = session(Contribution::new());
        for action in crate::agent::ACTIONS {
            let faults = runtime.intent(Intent::Action {
                node: "session".into(),
                action: action.to_string(),
                args: Value::Null,
                fields: Vec::new(),
            });
            assert!(
                faults
                    .iter()
                    .all(|fault| !fault.message.contains("no action named")),
                "`{action}` is in the list and not handled: {faults:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_contribution_can_present_a_tree_the_session_places() {
        let runtime = session(contribution().with_section(presenting("test.plugin")));
        let node = view(&runtime);
        let section = misa_proto::view::find(&node, "plugin.test.plugin")
            .expect("the section is in the document");
        assert_eq!(section.role, "plugin.test.plugin");
        // The plain transcript render is what every frontend gets for words; the structure is what
        // each of them draws in its own idiom.
        let text = transcript(&runtime);
        assert!(text.contains("seen true"), "{text}");
        // The plugin's own ids are inside the session's namespace, so nothing can collide with a
        // node the session wrote — and a client's memory of which nodes it opened still follows the
        // plugin's own identity.
        let widget = misa_proto::view::find(&node, "plugin.test.plugin.widget")
            .expect("the namespaced root");
        // The section's namespace and the plugin's own id, in that order: the plugin promises its
        // ids are unique inside its own tree, and the session promises the namespace is.
        assert_eq!(widget.children[0].id, "plugin.test.plugin.note");
    }

    #[tokio::test]
    async fn a_contribution_that_cannot_present_is_a_sentence_and_not_a_broken_view() {
        // A fault is data: a plugin that cannot draw is a line in the document, which is the
        // difference between a widget that is missing and a session whose view nobody can draw.
        let broken = views::Section {
            inputs: vec![misa_value::Path::root()],
            namespace: "plugin.test.broken".to_string(),
            build: std::sync::Arc::new(|_db| Err("nothing to draw".to_string())),
        };
        let runtime = session(Contribution::new().with_section(broken));
        let node = view(&runtime);
        misa_proto::view::validate(&node).expect("a tree a client may be sent");
        let text = transcript(&runtime);
        assert!(text.contains("test.broken"), "{text}");
        assert!(text.contains("nothing to draw"), "{text}");
        // And the rest of the document is still there, which is the point.
        assert!(misa_proto::view::find(&node, "composer").is_some());
    }

    #[tokio::test]
    async fn an_action_a_composition_declared_reaches_its_handler_and_is_not_a_fault() {
        // The whole of how a plugin is acted on, with no router anywhere: the action arrives as the
        // loop's own `intent/action` event, the session's handler does not know it (and does not
        // fault, because a composition declared it), and the handler that declared the event kind
        // does the work.
        let acted = Arc::new(misa_reframe::FnHandler::new(
            "test.acted",
            |tx: &mut Tx<'_>, event: &Event| {
                tx.set("guest.acted", Value::str(event.field("action")))?;
                Ok(())
            },
        ));
        let contribution = Contribution::new()
            .with_root("guest", Value::map([]))
            .expect("a root")
            .with_action("refresh")
            .expect("an affordance the session does not have")
            .with_handler("intent/action", 10, acted)
            .with_subscription("guest.acted", reading_acted());
        let runtime = session(contribution);

        let faults = runtime.intent(Intent::Action {
            node: "plugin.test.plugin.widget".into(),
            action: "refresh".into(),
            args: Value::Null,
            fields: Vec::new(),
        });
        assert!(faults.is_empty(), "{faults:?}");
        wait_for_value(&runtime, "guest.acted", &Value::str("refresh")).await;
        match runtime.read(&Query::new("guest.acted")).expect("a value") {
            Reading::Data(value) => assert_eq!(value.as_str(), Some("refresh")),
            Reading::View(_) => panic!("expected data"),
        }

        // And one nobody declared is still the fault it was: a client finds out that an affordance
        // it kept from an older view does not exist any more.
        let faults = runtime.intent(Intent::Action {
            node: "plugin.test.plugin.widget".into(),
            action: "stale".into(),
            args: Value::Null,
            fields: Vec::new(),
        });
        assert_eq!(faults.len(), 1, "{faults:?}");
        assert!(faults[0].message.contains("stale"), "{faults:?}");
    }

    /// A subscription over the root the acted handler writes, so a test can read it as a client
    /// would.
    fn reading_acted() -> Subscription {
        Subscription::Read {
            read: Arc::new(|db, _query, _previous| {
                Ok({
                    db.get("guest")
                        .and_then(|guest| guest.get("acted"))
                        .cloned()
                        .unwrap_or(Value::Null)
                })
            }),
        }
    }

    /// The whole of a composition's root, as a client would read it.
    ///
    /// `reading` answers one key inside the root, which is what most of these tests want; this
    /// answers the root itself, which is the only way to see a patch whose path *is* the root.
    fn reading_all() -> Subscription {
        Subscription::Read {
            read: Arc::new(|db, _query, _previous| {
                Ok(db.get("guest").cloned().unwrap_or(Value::Null))
            }),
        }
    }

    /// What a composition's subscription says right now, as a client would read it.
    fn value(runtime: &Runtime, query: &str) -> Value {
        match runtime.read(&Query::new(query)).expect("a value") {
            Reading::Data(value) => value,
            Reading::View(_) => panic!("a composition's subscription answered with a view"),
        }
    }

    /// Wait for a value, the way a client waits: by looking.
    async fn wait_for_value(runtime: &Arc<Runtime>, query: &str, want: &Value) -> Value {
        for _ in 0..200 {
            let value = value(runtime, query);
            if &value == want {
                return value;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!(
            "`{query}` never became {want:?}; it is {:?}",
            value(runtime, query)
        );
    }

    #[tokio::test]
    async fn what_a_composition_wrote_is_rebuilt_from_the_log_when_a_session_resumes() {
        // The durability question, answered the way everything else here answers it: the log is the
        // truth. Events are ephemeral, so nothing but the patches themselves can reproduce what a
        // plugin decided — so the session records them into the conversation, and a resumed session
        // folds them back. No file, no format, and nothing the plugin has to cooperate with.
        let store = Arc::new(misa_kernel::MemoryStore::new());
        let provider: Arc<dyn Provider> = ScriptedProvider::always("an answer");
        let first = Runtime::start_with(
            "demo",
            "a demo session",
            Some("demo".into()),
            Arc::new(misa_kernel::LocalKernel::new(provider.clone()).with_store(store.clone())),
            "scripted",
            "scripted-1",
            Value::Null,
            contribution().with_subscription("guest.all", reading_all()),
        );
        let faults = first.intent(Intent::Prompt {
            text: "hello".into(),
            attachments: vec![],
        });
        assert!(faults.is_empty(), "{faults:?}");
        wait_for_value(&first, "guest.seen", &Value::str("intent/prompt")).await;
        // The turn is allowed to finish, because an append is an effect and effects reach the
        // daemon after the transaction that asked for them.
        settle(&first).await;

        // The log has what it wrote, as its own kind of entry, in order.
        let recorded = store
            .load("demo", 0, 10_000)
            .expect("the log")
            .into_iter()
            .filter(|entry| entry.kind == contribution::PATCH_KIND)
            .collect::<Vec<_>>();
        assert_eq!(recorded.len(), 1, "{recorded:?}");
        assert_eq!(
            recorded[0].data.get("path").and_then(Value::as_str),
            Some("guest.seen")
        );

        // One entry for a root this composition is not running, and one that is not a patch at all:
        // a log can be older than the composition, and history with nowhere to go is skipped rather
        // than reported — while a patch that cannot be applied is reported, because state going
        // missing in silence is worse.
        store
            .append(
                "demo",
                contribution::PATCH_KIND,
                &Value::map([
                    ("path", Value::str("nobody.declared")),
                    ("patch", Value::str("delete")),
                ]),
                0,
            )
            .expect("an entry for a composition that is not running");
        store
            .append(
                "demo",
                contribution::PATCH_KIND,
                &Value::map([
                    ("path", Value::str("a..b")),
                    ("patch", Value::str("delete")),
                ]),
                0,
            )
            .expect("an entry that is not a patch");
        // And one whose path is the root itself, which is a patch like any other: a path is
        // relative to the database, so `guest` is a key the replay writes exactly as the loop
        // would, and the fold is not a second rule about what a root is.
        store
            .append(
                "demo",
                contribution::PATCH_KIND,
                &Value::map([
                    ("path", Value::str("guest")),
                    (
                        "patch",
                        Value::map([("merge", Value::map([("whole", Value::str("root"))]))]),
                    ),
                ]),
                0,
            )
            .expect("an entry on the root itself");

        // A second session over the same store: its root is empty until the conversation is read
        // back.
        let second = Runtime::start_with(
            "demo",
            "a demo session",
            Some("demo".into()),
            Arc::new(misa_kernel::LocalKernel::new(provider).with_store(store.clone())),
            "scripted",
            "scripted-1",
            Value::Null,
            contribution().with_subscription("guest.all", reading_all()),
        );
        assert_eq!(
            value(&second, "guest.seen"),
            Value::Null,
            "a fresh session starts empty"
        );
        let mut events = second.subscribe_events();
        let faults = second.intent(Intent::Command {
            name: "resume".into(),
            args: Value::str("demo"),
        });
        assert!(faults.is_empty(), "{faults:?}");
        let replayed = wait_for_value(&second, "guest.seen", &Value::str("intent/prompt")).await;
        assert_eq!(replayed.as_str(), Some("intent/prompt"));
        let whole = value(&second, "guest.all");
        assert_eq!(
            whole.get("whole").and_then(Value::as_str),
            Some("root"),
            "a patch on the root itself applies: {whole:?}"
        );
        assert_eq!(
            whole.get("seen").and_then(Value::as_str),
            Some("intent/prompt"),
            "and it does not replace what was there: {whole:?}"
        );

        // And the two things the replay has to say are said: what it put back, and what it could
        // not.
        let notice = wait_notice(&mut events, "replayed").await;
        assert!(notice.contains("replayed 2"), "{notice}");
        let warning = wait_notice(&mut events, "could not be replayed").await;
        assert!(warning.contains("1 recorded patch"), "{warning}");
    }

    #[tokio::test]
    async fn a_contributions_handlers_subscriptions_and_roots_are_part_of_the_session() {
        // The shipped loop still runs — a prompt is still a turn — and the contribution saw the
        // same event, wrote into the root it declared, and is readable as a query.
        let runtime = session(contribution());
        let faults = runtime.intent(Intent::Prompt {
            text: "hello".into(),
            attachments: vec![],
        });
        assert!(faults.is_empty(), "{faults:?}");

        wait_for_value(&runtime, "guest.seen", &Value::str("intent/prompt")).await;
        let reading = runtime.read(&Query::new("guest.seen")).expect("a value");
        match reading {
            Reading::Data(value) => assert_eq!(value.as_str(), Some("intent/prompt")),
            Reading::View(_) => panic!("a composition's subscription answered with a view"),
        }
        tests::settle(&runtime).await;
        // And a client can see the turn the shipped loop ran, which is what says the two
        // registrations live in one loop rather than two.
        let text = match runtime.read(&Query::new(misa_proto::VIEW_QUERY)).unwrap() {
            Reading::View(node) => misa_lines::to_plain(&misa_lines::render(
                &node,
                &misa_render::Theme::plain(),
                100,
            )),
            Reading::Data(_) => panic!("expected a view"),
        };
        assert!(text.contains("hello"), "{text}");
    }

    #[tokio::test]
    async fn a_contribution_that_writes_a_root_nobody_declared_is_a_fault_not_a_silence() {
        // Two rules, and a composition that breaks either one hears about it rather than finding out
        // later that its state never existed. The first is the composition's own: it may only write
        // into the roots it declared, so a contribution with no roots at all cannot write
        // `guest.seen` however much it wants to.
        let runtime = session(Contribution::new().with_handler("intent/prompt", 10, adopting()));
        let faults = runtime.intent(Intent::Prompt {
            text: "hello".into(),
            attachments: vec![],
        });
        assert_eq!(faults.len(), 1, "{faults:?}");
        assert_eq!(faults[0].code, "composition.root");
        assert!(
            faults[0].message.contains("guest.seen"),
            "{}",
            faults[0].message
        );

        // The second is the database's: a patch may only create the *last* key of its path, so a
        // write that would have to invent a container on the way is refused even inside a root the
        // composition does own. Two mistakes, and a client can tell them apart by the code.
        let deep = Arc::new(FnHandler::new(
            "test.deep",
            |tx: &mut Tx<'_>, _event: &Event| {
                tx.set("guest.turns[0].seen", Value::str("first"))?;
                Ok(())
            },
        ));
        let declared = session(
            Contribution::new()
                .with_root("guest", Value::map([]))
                .expect("a root the session does not own")
                .with_handler("intent/prompt", 10, deep),
        );
        let faults = declared.intent(Intent::Prompt {
            text: "hello".into(),
            attachments: vec![],
        });
        assert_eq!(faults.len(), 1, "{faults:?}");
        assert_eq!(faults[0].code, "patch");
        assert!(
            faults[0].message.contains("guest.turns"),
            "{}",
            faults[0].message
        );
    }

    #[tokio::test]
    async fn a_handler_cannot_write_outside_the_roots_its_composition_declared() {
        // The declaration is where a composition says what its state is, and this is where that is
        // enforced. A handler that reaches for `messages` — the transcript the loop decides — is a
        // fault rather than a quiet rewrite, and because one dispatch is one transaction, the fault
        // takes everything else in it too: the write it was allowed to make, and the message the
        // loop had already recorded before the plugin ran.
        let intruder = Arc::new(FnHandler::new(
            "test.intruder",
            |tx: &mut Tx<'_>, _event: &Event| {
                tx.set("guest.seen", Value::str("mine"))?;
                tx.set("messages", Value::list([]))?;
                Ok(())
            },
        ));
        let runtime = session(
            Contribution::new()
                .with_root("guest", Value::map([]))
                .expect("a root the session does not own")
                .with_handler("intent/prompt", 10, intruder)
                .with_subscription("guest.seen", reading()),
        );
        let faults = runtime.intent(Intent::Prompt {
            text: "hello".into(),
            attachments: vec![],
        });
        assert_eq!(faults.len(), 1, "{faults:?}");
        assert_eq!(faults[0].code, "composition.root");
        assert!(
            faults[0].message.contains("messages"),
            "{}",
            faults[0].message
        );
        // Nothing landed: not the root it did declare, and not the transcript.
        assert_eq!(
            value(&runtime, "guest.seen"),
            Value::Null,
            "the write it was allowed to make is gone too"
        );
        assert!(
            !transcript(&runtime).contains("hello"),
            "{}",
            transcript(&runtime)
        );
    }
}

#[cfg(test)]
mod stream_contract_tests {
    use super::*;
    use misa_proto::sync::StreamUpdate;

    fn runtime() -> Arc<Runtime> {
        Runtime::start(
            "stream-test",
            "stream test",
            None,
            Arc::new(misa_kernel::LocalKernel::new(
                misa_kernel::ScriptedProvider::always("unused"),
            )),
            "scripted",
            "scripted-1",
            Value::Null,
        )
    }
    fn ack(runtime: &Runtime, seq: i64, data: Value) {
        assert!(
            runtime
                .dispatch(
                    Event::new("kernel/log.appended")
                        .with("conversation", Value::str("stream-test"))
                        .with("seq", Value::Int(seq))
                        .with("kind", Value::str("message"))
                        .with("data", data)
                )
                .is_empty()
        );
    }
    /// The answer's journal write and a person's cancel can cross in flight.
    /// The message settles once — and a turn somebody stopped does not start its
    /// tools when the record's log reply lands.
    #[tokio::test]
    async fn a_cancel_crossing_the_answers_journal_write_stops_the_turn() {
        let runtime = runtime();
        runtime.intent(Intent::Prompt {
            text: "go".into(),
            attachments: vec![],
        });
        ack(
            &runtime,
            1,
            Value::map([
                ("seq", Value::Int(1)),
                ("role", Value::str("user")),
                ("text", Value::str("go")),
            ]),
        );
        let calls = Value::list([Value::map([
            ("id", Value::str("toolu_crossing")),
            ("name", Value::str("echo")),
            ("args", Value::str("hi")),
        ])]);
        // The answer for the pending message arrives, tool call and all.
        assert!(
            runtime
                .dispatch(
                    Event::new("kernel/provider.finished")
                        .with("id", Value::str("r1"))
                        .with("ok", Value::Bool(true))
                        .with("text", Value::str("running it"))
                        .with("thinking", Value::str(""))
                        .with("tool_calls", calls.clone())
                        .with("provider_state", Value::list([]))
                        .with("input_tokens", Value::Int(1))
                        .with("output_tokens", Value::Int(1))
                        .with("error", Value::str(""))
                )
                .is_empty()
        );
        // The person stops the turn while the answer's record is in flight.
        assert!(
            runtime
                .intent(Intent::Action {
                    node: "composer".into(),
                    action: "turn.cancel".into(),
                    args: Value::Null,
                    fields: Vec::new(),
                })
                .is_empty()
        );
        // Its log reply lands afterwards, and settles the message it promised...
        ack(
            &runtime,
            2,
            Value::map([
                ("seq", Value::Int(2)),
                ("role", Value::str("assistant")),
                ("text", Value::str("running it")),
                ("state", Value::str("done")),
                ("calls", calls),
            ]),
        );
        let state = runtime.state.lock().unwrap();
        let session = state.state.db().get("session").unwrap();
        assert_eq!(
            state
                .state
                .db()
                .get("messages")
                .unwrap()
                .as_list()
                .unwrap()
                .len(),
            2
        );
        // ...without restarting a turn somebody stopped.
        assert_eq!(session.get("status").unwrap().as_str(), Some("idle"));
        assert_eq!(
            session
                .get("running_tools")
                .unwrap()
                .as_list()
                .unwrap()
                .len(),
            0
        );
        assert_eq!(session.get("requests").unwrap().as_i64(), Some(1));
    }
    #[tokio::test]
    async fn repeated_interrupts_do_not_cancel_the_new_priority_turn() {
        let runtime = runtime();
        runtime.intent(Intent::Prompt {
            text: "original".into(),
            attachments: vec![],
        });
        runtime.intent(Intent::Prompt {
            text: "waiting".into(),
            attachments: vec![],
        });
        for text in ["urgent1", "urgent2"] {
            runtime.intent(Intent::Interrupt {
                text: text.into(),
                attachments: vec![],
            });
        }
        for (seq, text) in [(1, "original"), (2, "urgent2")] {
            ack(
                &runtime,
                seq,
                Value::map([
                    ("seq", Value::Int(seq)),
                    ("role", Value::str("user")),
                    ("text", Value::str(text)),
                ]),
            );
        }
        let state = runtime.state.lock().unwrap();
        let session = state.state.db().get("session").unwrap();
        assert_eq!(session.get("status").unwrap().as_str(), Some("thinking"));
        assert!(session.get("pending").is_some());
        let queue = session.get("queue").unwrap().as_list().unwrap();
        assert_eq!(queue.len(), 2);
        assert_eq!(queue[0].get("text").unwrap().as_str(), Some("urgent1"));
        assert_eq!(queue[1].get("text").unwrap().as_str(), Some("waiting"));
        assert!(queue.iter().all(|item| item.get("interrupt").is_none()));
    }
    #[tokio::test]
    async fn empty_interrupt_preserves_active_turn_and_queue() {
        let runtime = runtime();
        runtime.intent(Intent::Prompt {
            text: "original".into(),
            attachments: vec![],
        });
        runtime.intent(Intent::Prompt {
            text: "waiting".into(),
            attachments: vec![],
        });
        let before = runtime.state.lock().unwrap().state.db().clone();
        assert!(
            !runtime
                .intent(Intent::Interrupt {
                    text: "  ".into(),
                    attachments: vec![]
                })
                .is_empty()
        );
        assert_eq!(&before, runtime.state.lock().unwrap().state.db());
    }
    #[tokio::test]
    async fn interrupt_settles_tool_results_before_priority_prompt_without_another_generation() {
        for recording in [false, true] {
            let runtime = runtime();
            runtime.intent(Intent::Prompt {
                text: "original".into(),
                attachments: vec![],
            });
            ack(
                &runtime,
                1,
                Value::map([
                    ("seq", Value::Int(1)),
                    ("role", Value::str("user")),
                    ("text", Value::str("original")),
                ]),
            );
            let calls = Value::list([Value::map([
                ("id", Value::str("tool-1")),
                ("name", Value::str("echo")),
                ("args", Value::map([])),
                ("status", Value::str("pending")),
            ])]);
            assert!(
                runtime
                    .dispatch(
                        Event::new("kernel/provider.finished")
                            .with("id", Value::str("r1"))
                            .with("ok", Value::Bool(true))
                            .with("tool_calls", calls.clone())
                    )
                    .is_empty()
            );
            let assistant = Value::map([
                ("seq", Value::Int(2)),
                ("role", Value::str("assistant")),
                ("calls", calls),
            ]);
            if !recording {
                ack(&runtime, 2, assistant.clone());
            }
            runtime.intent(Intent::Prompt {
                text: "waiting".into(),
                attachments: vec![],
            });
            runtime.intent(Intent::Interrupt {
                text: "urgent".into(),
                attachments: vec![],
            });
            if recording {
                let mut state = runtime.state.lock().unwrap();
                let outcome = state.state.dispatch(
                    Event::new("kernel/log.appended")
                        .with("conversation", Value::str("stream-test"))
                        .with("seq", Value::Int(2))
                        .with("kind", Value::str("message"))
                        .with("data", assistant),
                );
                assert!(outcome.committed());
                let State {
                    state: loop_, view, ..
                } = &mut *state;
                view.advance(loop_.db(), &outcome.changes, &runtime.sections, loop_.rev());
                assert!(
                    !outcome
                        .effects
                        .iter()
                        .any(|effect| effect.kind == "kernel.tool.run")
                );
                let records: Vec<_> = outcome
                    .effects
                    .iter()
                    .filter(|effect| effect.kind == "kernel.log.append")
                    .collect();
                assert_eq!(records.len(), 1);
                assert_eq!(
                    records[0]
                        .get("data")
                        .unwrap()
                        .get("text")
                        .unwrap()
                        .as_str(),
                    Some("cancelled before starting")
                );
                let repeated = state.state.dispatch(Event::new("agent/tools"));
                assert!(
                    repeated.effects.is_empty(),
                    "cancellation must be journalled once"
                );
            }
            {
                let state = runtime.state.lock().unwrap();
                let session = state.state.db().get("session").unwrap();
                assert_eq!(session.get("status").unwrap().as_str(), Some("tools"));
                assert_eq!(session.get("queue").unwrap().as_list().unwrap().len(), 2);
            }
            runtime.dispatch(
                Event::new("kernel/log.appended")
                    .with("conversation", Value::str("stream-test"))
                    .with("seq", Value::Int(3))
                    .with("kind", Value::str("tool_result"))
                    .with(
                        "data",
                        Value::map([
                            ("call", Value::str("tool-1")),
                            ("ok", Value::Bool(true)),
                            ("text", Value::str("result")),
                        ]),
                    ),
            );
            let state = runtime.state.lock().unwrap();
            let session = state.state.db().get("session").unwrap();
            assert_eq!(session.get("status").unwrap().as_str(), Some("recording"));
            assert_eq!(session.get("requests").unwrap().as_i64(), Some(1));
            let queue = session.get("queue").unwrap().as_list().unwrap();
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].get("text").unwrap().as_str(), Some("waiting"));
        }
    }
    #[tokio::test]
    async fn interrupt_prioritizes_the_draft_during_recording_and_generation() {
        for generating in [false, true] {
            let runtime = runtime();
            let user = Value::map([
                ("seq", Value::Int(1)),
                ("role", Value::str("user")),
                ("text", Value::str("original")),
                ("state", Value::str("done")),
                ("attachments", Value::list([])),
            ]);
            assert!(
                runtime
                    .intent(Intent::Prompt {
                        text: "original".into(),
                        attachments: vec![]
                    })
                    .is_empty()
            );
            if generating {
                ack(&runtime, 1, user.clone());
            }
            assert!(
                runtime
                    .intent(Intent::Prompt {
                        text: "waiting".into(),
                        attachments: vec![]
                    })
                    .is_empty()
            );
            assert!(
                runtime
                    .intent(Intent::Interrupt {
                        text: "urgent".into(),
                        attachments: vec![]
                    })
                    .is_empty()
            );
            {
                let state = runtime.state.lock().unwrap();
                let session = state.state.db().get("session").unwrap();
                let queue = session.get("queue").unwrap().as_list().unwrap();
                assert_eq!(queue[0].get("text").unwrap().as_str(), Some("urgent"));
                assert_eq!(queue[1].get("text").unwrap().as_str(), Some("waiting"));
                assert!(session.get("pending").is_none());
            }
            if generating {
                ack(
                    &runtime,
                    2,
                    Value::map([
                        ("seq", Value::Int(2)),
                        ("role", Value::str("assistant")),
                        ("text", Value::str("partial")),
                        ("state", Value::str("cancelled")),
                        ("calls", Value::list([])),
                        ("attachments", Value::list([])),
                    ]),
                );
            } else {
                ack(&runtime, 1, user);
            }
            let state = runtime.state.lock().unwrap();
            let session = state.state.db().get("session").unwrap();
            let queue = session.get("queue").unwrap().as_list().unwrap();
            assert_eq!(queue.len(), 1);
            assert_eq!(queue[0].get("text").unwrap().as_str(), Some("waiting"));
            assert_eq!(session.get("status").unwrap().as_str(), Some("recording"));
        }
    }
    #[tokio::test]
    async fn image_only_prompts_can_submit_or_interrupt() {
        let image = misa_proto::view::BlobRef {
            hash: "a".repeat(64),
            media: Some("image/png".into()),
            len: 4,
        };
        for intent in [
            Intent::Prompt {
                text: String::new(),
                attachments: vec![image.clone()],
            },
            Intent::Interrupt {
                text: String::new(),
                attachments: vec![image],
            },
        ] {
            let runtime = runtime();
            assert!(runtime.intent(intent).is_empty());
            let state = runtime.state.lock().unwrap();
            assert_eq!(
                state
                    .state
                    .db()
                    .get("session")
                    .unwrap()
                    .get("status")
                    .unwrap()
                    .as_str(),
                Some("recording")
            );
        }
    }
    #[tokio::test]
    async fn attaching_the_same_blob_twice_keeps_one_stable_draft_identity() {
        let runtime = runtime();
        let hash = "a".repeat(64);
        let event = Event::new("kernel/blob")
            .with("ok", Value::Bool(true))
            .with("hash", Value::str(&hash))
            .with("media", Value::str("text/plain"))
            .with("len", Value::Int(3))
            .with("id", Value::str("note.txt"));
        assert!(runtime.dispatch(event.clone()).is_empty());
        assert!(runtime.dispatch(event).is_empty());
        let view = runtime.view().unwrap();
        misa_proto::view::validate(&view).unwrap();
        let attachments = misa_proto::view::find(&view, "attachments").unwrap();
        assert_eq!(attachments.children.len(), 2, "count plus one attachment");
        assert!(misa_proto::view::find(&view, &format!("attachment.{hash}")).is_some());
    }
    // No await: the kernel tasks cannot run until these controlled acknowledgments finish.
    #[tokio::test]
    async fn streaming_work_is_linear_and_canonical_state_waits_for_the_log() {
        for tokens in [64, 128] {
            let mut runs = Vec::new();
            for _ in 0..6 {
                let runtime = runtime();
                runtime.intent(Intent::Prompt {
                    text: "prompt".into(),
                    attachments: vec![],
                });
                assert!(
                    runtime
                        .state
                        .lock()
                        .unwrap()
                        .state
                        .db()
                        .get("messages")
                        .unwrap()
                        .as_list()
                        .unwrap()
                        .is_empty()
                );
                ack(
                    &runtime,
                    1,
                    Value::map([
                        ("seq", Value::Int(1)),
                        ("role", Value::str("user")),
                        ("text", Value::str("prompt")),
                        ("state", Value::str("done")),
                        ("attachments", Value::list([])),
                    ]),
                );
                let (request, before, version, work) = {
                    let state = runtime.state.lock().unwrap();
                    (
                        state
                            .state
                            .db()
                            .get("session")
                            .unwrap()
                            .get("pending")
                            .unwrap()
                            .get("request")
                            .unwrap()
                            .as_str()
                            .unwrap()
                            .to_owned(),
                        state.state.db().clone(),
                        state.view.version.clone(),
                        state.view.work.clone(),
                    )
                };
                let mut events = runtime.subscribe_events();
                let chunk = "é".repeat(32);
                let mut wire_bytes = 0;
                for index in 0..tokens {
                    assert!(
                        runtime
                            .dispatch(
                                Event::new("kernel/provider.delta")
                                    .with("id", Value::str(&request))
                                    .with("text", Value::str(&chunk))
                            )
                            .is_empty()
                    );
                    let emission = events.try_recv().expect("each successful token is emitted");
                    match &emission.event {
                        SessionEvent::Stream {
                            update: StreamUpdate::Append { offset, text, .. },
                        } => {
                            assert_eq!(*offset, index * 64);
                            assert_eq!(text, &chunk);
                        }
                        other => panic!("expected append, got {other:?}"),
                    }
                    wire_bytes += misa_proto::encode(&emission.event).unwrap().len();
                }
                {
                    let state = runtime.state.lock().unwrap();
                    assert!(
                        state.state.db().same(&before),
                        "tokens must not patch the database"
                    );
                    assert_eq!(state.view.version, version);
                    assert_eq!(
                        state.view.work, work,
                        "tokens must not build or encode tree operations"
                    );
                    assert_eq!(state.stream_bytes, (tokens * 64) as u64);
                    assert_eq!(state.streams["msg.2.text"].text, chunk.repeat(tokens));
                    runs.push((state.stream_bytes, wire_bytes));
                }
                let settled = Value::map([
                    ("seq", Value::Int(2)),
                    ("role", Value::str("assistant")),
                    ("text", Value::str(chunk.repeat(tokens))),
                    ("state", Value::str("done")),
                    ("calls", Value::list([])),
                    ("attachments", Value::list([])),
                ]);
                ack(&runtime, 2, settled.clone());
                let state = runtime.state.lock().unwrap();
                assert!(state.streams.is_empty());
                assert_eq!(
                    state.state.db().get("messages").unwrap().as_list().unwrap()[1],
                    settled
                );
            }
            assert!(runs.iter().all(|run| run == &runs[0]));
            eprintln!(
                "tokens={tokens} repetitions=6 append_bytes={} wire_bytes={} token_db_patches=0 token_view_ops=0",
                runs[0].0, runs[0].1
            );
        }
    }
}
