//! Owner operation lifecycles and caller-bound input requests. An Accepted reply
//! acknowledges admission, not durability: effects wait for their checkpoint.
//! Credential bytes bypass the policy graph and persistent operation metadata.
use crate::{Runtime, State, commands::CommandRegistration};
use misa_kernel::{CredentialAction, Request};
use misa_proto::{
    Fault, Query,
    invocation::{Command, Invocation, OperationRef, Outcome},
    query::{Definition, ResultContract},
    schema::{Field, Literal, Schema},
};
use misa_protocol::invocation::CallContext;
use misa_reframe::{Event, Registry, Tx, read_query};
use misa_value::Value;
use std::{collections::BTreeMap, sync::Arc};

pub const SUMMARY: &str = "operations.summary";
pub const REQUEST: &str = "operation.request";
pub(crate) fn outcome_of_result(record: &Value) -> Option<Outcome> {
    if record.get("terminal").and_then(Value::as_bool) != Some(true) { return None; }
    if let Some(outcome) = record.get("outcome").and_then(|value| crate::wire::parse(value).ok()) { return Some(outcome); }
    Some(match record.get("state").and_then(Value::as_str) {
        Some("succeeded") => Outcome::Completed { value: Value::Null },
        Some("failed" | "cancelled" | "expired") => Outcome::Rejected { fault: Fault::new("operation_failed", "Operation did not complete successfully") },
        _ => Outcome::Indeterminate { fault: Fault::new("interrupted", "Operation interrupted; reconcile durable state before retrying") },
    })
}
const KEY_TTL_MS: i64 = 10 * 60 * 1000;
const MAX_RECORDS: usize = 64;
#[path = "approvals.rs"]
mod approvals;
#[path = "operation_journal.rs"]
mod journal;
#[path = "input_requests.rs"]
mod forms;
pub(crate) use approvals::ToolApprovalPolicy;
pub(crate) use journal::DeferredWork;
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Phase {
    Running,
    Awaiting,
    Submitting,
    Cancelling,
    Succeeded,
    Failed,
    Cancelled,
    Expired,
    Interrupted,
}
impl Phase {
    fn name(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Awaiting => "awaiting_input",
            Self::Submitting => "submitting",
            Self::Cancelling => "cancelling",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
            Self::Interrupted => "interrupted",
        }
    }
    fn terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Expired | Self::Interrupted
        )
    }
}
#[derive(Clone)]
struct Record {
    id: String,
    principal: String,
    provider: String,
    account: String,
    oauth: bool,
    generation: i64,
    phase: Phase,
    expires_ms: Option<i64>,
    challenge: Option<(String, String)>,
}
#[derive(Clone, Default)]
pub(crate) struct Store {
    next: u64,
    records: BTreeMap<String, Record>,
    prompt_owners: BTreeMap<String, String>,
    events: Vec<Event>,
    approvals: BTreeMap<String, approvals::Approval>,
    forms: BTreeMap<String, forms::FormRecord>,
    last_checkpoint: Option<Value>,
}
impl Store {
    pub(crate) fn reconcile(&mut self, db: &Value) {
        approvals::reconcile(self, db);
    }
    fn summary(&self) -> Value {
        Value::list(self.records.values().map(|record| {
            Value::map([
                ("id", Value::str(&record.id)),
                ("kind", Value::str("credentials.authorize")),
                ("provider", Value::str(&record.provider)),
                ("state", Value::str(record.phase.name())),
                ("terminal", Value::Bool(record.phase.terminal())),
                ("generation", Value::Int(record.generation)),
                ("needs_input", Value::Bool(record.phase == Phase::Awaiting)),
                (
                    "expires_ms",
                    record.expires_ms.map(Value::Int).unwrap_or(Value::Null),
                ),
            ])
        }).chain(self.forms.values().map(forms::FormRecord::operation)))
    }
    fn requests(&self) -> Value {
        let approvals = approvals::summaries(self);
        Value::list(approvals.as_list().unwrap_or(&[]).iter().cloned().chain(self.forms.values().map(forms::FormRecord::summary)))
    }
    pub(crate) fn deadline(&self) -> Option<i64> {
        self.records
            .values()
            .filter(|record| record.phase == Phase::Awaiting)
            .filter_map(|record| record.expires_ms)
            .chain(
                self.approvals
                    .values()
                    .filter_map(approvals::Approval::deadline),
            )
            .chain(self.forms.values().filter_map(forms::FormRecord::deadline))
            .min()
    }
    fn owned(
        &mut self,
        context: &CallContext,
        id: &str,
        generation: i64,
    ) -> Result<&mut Record, Fault> {
        let record = self
            .records
            .get_mut(id)
            .filter(|record| record.principal == context.principal)
            .ok_or_else(|| Fault::new("request_unavailable", "Operation is unavailable"))?;
        if record.generation != generation || record.phase.terminal() {
            return Err(Fault::new(
                "stale_request",
                "Operation request is no longer current",
            ));
        }
        Ok(record)
    }
}
fn record(fields: impl IntoIterator<Item = (&'static str, Schema)>) -> Schema {
    Schema::Record {
        fields: fields
            .into_iter()
            .map(|(name, schema)| {
                (
                    name.into(),
                    Field {
                        schema,
                        optional: false,
                    },
                )
            })
            .collect(),
        allow_unknown: false,
    }
}
fn with_outcome(mut schema: Schema) -> Schema {
    if let Schema::Record { fields, .. } = &mut schema { fields.insert("outcome".into(), Field { schema: Schema::Value, optional: true }); }
    schema
}
pub(crate) fn definitions() -> Vec<Definition> {
    vec![
        Definition {
            id: "requests.summary".into(),
            arguments: vec![],
            contract: "requests.summary@1".into(),
            result: ResultContract::Data {
                schema: Schema::List {
                    items: Box::new(Schema::Value),
                },
            },
        },
        Definition {
            id: SUMMARY.into(),
            arguments: vec![],
            contract: "operations.summary@1".into(),
            result: ResultContract::Data {
                schema: Schema::List {
                    items: Box::new(Schema::Value),
                },
            },
        },
        Definition {
            id: "operation.presentation".into(),
            arguments: vec![Schema::String],
            contract: "operation.presentation@1".into(),
            result: ResultContract::Document {},
        },
        Definition {
            id: "operation.output".into(), arguments: vec![Schema::String],
            contract: "operation.output@1".into(),
            result: ResultContract::Data { schema: Schema::List { items: Box::new(Schema::Value) } },
        },
        Definition {id:"operation.usage".into(),arguments:vec![Schema::String],contract:"operation.usage@1".into(),result:ResultContract::Data{schema:Schema::List{items:Box::new(Schema::Value)}}},
        Definition {
            id: "operation.result".into(),
            arguments: vec![Schema::String],
            contract: "operation.result@1".into(),
            result: ResultContract::Data {
                schema: Schema::Nullable {
                    inner: Box::new(with_outcome(record([
                        ("id", Schema::String),
                        ("kind", Schema::String),
                        (
                            "state",
                            Schema::Choice {
                                values: [
                                    "queued",
                                    "running",
                                    "awaiting_input",
                                    "submitting",
                                    "cancelling",
                                    "succeeded",
                                    "failed",
                                    "cancelled",
                                    "expired",
                                    "interrupted",
                                ]
                                .into_iter()
                                .map(|state| Literal::String(state.into()))
                                .collect(),
                            },
                        ),
                        ("terminal", Schema::Bool),
                        ("generation", Schema::Int),
                        (
                            "outputs",
                            Schema::List {
                                items: Box::new(Schema::Int),
                            },
                        ),
                    ]))),
                },
            },
        },
    ]
}
pub(crate) fn restricted_exports() -> Vec<(Definition, crate::observation::RestrictedQuery)> {
    vec![(
        Definition {
            id: REQUEST.into(),
            arguments: vec![Schema::String],
            contract: "operation.request@1".into(),
            result: ResultContract::Data {
                schema: Schema::Value,
            },
        },
        request,
    )]
}
pub(crate) fn registry(registry: Registry) -> Registry {
    forms::registry(registry)
        .subscription(
            "requests.summary",
            read_query(|db, _| {
                db.get("input_requests")
                    .cloned()
                    .unwrap_or_else(|| Value::list([]))
            }),
        )
        .subscription(
            SUMMARY,
            read_query(|db, _| {
                Value::list(
                    db.get("operations")
                        .and_then(Value::as_list)
                        .unwrap_or(&[])
                        .iter()
                        .chain(
                            db.get("prompt_operations")
                                .and_then(Value::as_list)
                                .unwrap_or(&[]),
                        )
                        .chain(db.get(crate::command_operations::ROOT).and_then(Value::as_list).unwrap_or(&[]))
                        .cloned(),
                )
            }),
        )
        .subscription(
            "operation.output",
            read_query(|db, query| {
                let id = query.args.first().and_then(Value::as_str);
                let outputs = db.get("prompt_operations").and_then(Value::as_list).unwrap_or(&[]).iter()
                    .find(|record| record.get("id").and_then(Value::as_str) == id)
                    .and_then(|record| record.get("outputs")).and_then(Value::as_list).unwrap_or(&[]);
                Value::list(db.get("messages").and_then(Value::as_list).unwrap_or(&[]).iter()
                    .filter(|message| message.get("seq").is_some_and(|seq| outputs.contains(seq))).cloned())
            }),
        )
        .subscription("operation.usage",read_query(|db,query| {
            let id=query.args.first().and_then(Value::as_str);
            Value::list(db.get("attempts").and_then(Value::as_list).unwrap_or(&[]).iter().filter(|attempt|attempt.get("operation").and_then(Value::as_str)==id).cloned())
        }))
        .subscription(
            "operation.presentation",
            read_query(|db, query| {
                let id = query.args.first().and_then(Value::as_str);
                let output_ids: std::collections::BTreeSet<i64> = db
                    .get("prompt_operations")
                    .and_then(Value::as_list)
                    .unwrap_or(&[])
                    .iter()
                    .find(|record| record.get("id").and_then(Value::as_str) == id)
                    .and_then(|record| record.get("outputs"))
                    .and_then(Value::as_list)
                    .unwrap_or(&[])
                    .iter()
                    .filter_map(Value::as_i64)
                    .collect();
                let messages = db
                    .get("messages")
                    .and_then(Value::as_list)
                    .unwrap_or(&[])
                    .iter()
                    .filter(|message| {
                        message
                            .get("seq")
                            .and_then(Value::as_i64)
                            .is_some_and(|seq| output_ids.contains(&seq))
                    })
                    .cloned();
                crate::wire::render(&crate::views::transcript(&Value::map([(
                    "messages",
                    Value::list(messages),
                )])))
            }),
        )
        .subscription(
            "operation.result",
            read_query(|db, query| {
                let id = query.args.first().and_then(Value::as_str);
                if let Some(record) = db.get(crate::command_operations::ROOT).and_then(Value::as_list).unwrap_or(&[]).iter().find(|record| record.get("id").and_then(Value::as_str) == id) { return record.clone(); }
                if let Some(record) = db
                    .get("prompt_operations")
                    .and_then(Value::as_list)
                    .unwrap_or(&[])
                    .iter()
                    .find(|record| record.get("id").and_then(Value::as_str) == id)
                {
                    return record.clone();
                }
                let Some(record) = db
                    .get("operations")
                    .and_then(Value::as_list)
                    .unwrap_or(&[])
                    .iter()
                    .find(|record| record.get("id").and_then(Value::as_str) == id)
                else {
                    return Value::Null;
                };
                if record.get("kind").and_then(Value::as_str) == Some("input") { return record.clone(); }
                let phase = record
                    .get("state")
                    .and_then(Value::as_str)
                    .unwrap_or("running");
                Value::map([
                    ("id", record.get("id").cloned().unwrap_or(Value::Null)),
                    ("kind", record.get("kind").cloned().unwrap_or(Value::Null)),
                    ("state", Value::str(phase)),
                    (
                        "terminal",
                        Value::Bool(matches!(
                            phase,
                            "succeeded" | "failed" | "cancelled" | "expired" | "interrupted"
                        )),
                    ),
                    ("outputs", Value::list([])),
                    (
                        "generation",
                        record.get("generation").cloned().unwrap_or(Value::Int(1)),
                    ),
                ])
            }),
        )
        .on_fn(
            "operations/persistence.failed",
            0,
            "operations.persistence.failed",
            |tx, event| {
                crate::command_operations::interrupt(tx)?;
                tx.set(
                    "operations",
                    event
                        .get("summary")
                        .cloned()
                        .unwrap_or_else(|| Value::list([])),
                )?;
                tx.set(
                    "input_requests",
                    event
                        .get("requests")
                        .cloned()
                        .unwrap_or_else(|| Value::list([])),
                )?;
                let records = tx
                    .get("prompt_operations")
                    .and_then(Value::as_list)
                    .unwrap_or(&[])
                    .to_vec();
                for record in records {
                    if record.get("terminal").and_then(Value::as_bool) != Some(true) {
                        prompt_state(
                            tx,
                            &record.get("id").cloned().unwrap_or(Value::Null),
                            "interrupted",
                            None,
                        )?;
                    }
                }
                tx.set("session.status", Value::str("idle"))?;
                tx.delete("session.pending")?;
                tx.delete("session.operation")?;
                tx.set("session.queue", Value::list([]))?;
                Ok(())
            },
        )
        .on_fn(
            "operations/changed",
            0,
            "operations.changed",
            |tx, event| {
                tx.set(
                    "operations",
                    event
                        .get("summary")
                        .cloned()
                        .unwrap_or_else(|| Value::list([])),
                )?;
                tx.set(
                    "input_requests",
                    event
                        .get("requests")
                        .cloned()
                        .unwrap_or_else(|| Value::list([])),
                )?;
                for next in event.get("events").and_then(Value::as_list).unwrap_or(&[]) {
                    if let Some(kind) = next.get("kind").and_then(Value::as_str) {
                        tx.dispatch(Event {
                            kind: kind.into(),
                            data: next.get("data").cloned().unwrap_or(Value::Null),
                        });
                    }
                }
                if let Some(slots) = event.get("slots") {
                    tx.set("session.credentials", slots.clone())?;
                }
                Ok(())
            },
        )
}
/// Installed restricted owner projection. It is intentionally absent from the
/// plugin-callable query graph and shared memoization caches.
pub(crate) fn request(state: &State, query: &Query, context: &CallContext) -> Result<Value, Fault> {
    let id = query
        .args
        .first()
        .and_then(Value::as_str)
        .unwrap_or_default();
    if state.operations.approvals.contains_key(id) {
        return approvals::detail(state, id, context);
    }
    if state.operations.forms.contains_key(id) { return forms::detail(state, id, context); }
    let record = state
        .operations
        .records
        .get(id)
        .filter(|record| !context.principal.is_empty() && record.principal == context.principal)
        .ok_or_else(|| Fault::new("request_unavailable", "Operation request is unavailable"))?;
    if record.phase != Phase::Awaiting {
        return Ok(Value::Null);
    }
    let challenge = record
        .challenge
        .as_ref()
        .map(|(url, code)| Value::map([("url", Value::str(url)), ("code", Value::str(code))]))
        .unwrap_or(Value::Null);
    Ok(Value::map([
        ("id", Value::str(&record.id)),
        ("generation", Value::Int(record.generation)),
        (
            "kind",
            Value::str(if record.oauth {
                "device_authorization"
            } else {
                "credential_value"
            }),
        ),
        ("provider", Value::str(&record.provider)),
        ("challenge", challenge),
        (
            "expires_ms",
            record.expires_ms.map(Value::Int).unwrap_or(Value::Null),
        ),
    ]))
}
pub(crate) fn commands() -> Vec<CommandRegistration> {
    let null = || Schema::Choice {
        values: vec![Literal::Null],
    };
    vec![
        approvals::command(),
        forms::cancel_command(),
        CommandRegistration::new(
            Command {
                preparation: Default::default(),
                id: "credentials.authorize".into(),
                input: Schema::Record {
                    fields: [
                        (
                            "provider".into(),
                            Field {
                                schema: Schema::String,
                                optional: false,
                            },
                        ),
                        (
                            "account".into(),
                            Field {
                                schema: Schema::String,
                                optional: true,
                            },
                        ),
                    ]
                    .into_iter()
                    .collect(),
                    allow_unknown: false,
                },
                result: null(),
            },
            authorize,
        ),
        CommandRegistration::new(
            Command {
                preparation: misa_proto::invocation::Preparation::Request, id: "credentials.resolve".into(),
                input: record([
                    ("request", Schema::String),
                    ("generation", Schema::Int),
                    ("value", Schema::String),
                ]),
                result: null(),
            },
            resolve,
        ),
        CommandRegistration::new(
            Command {
                preparation: Default::default(), id: "operation.cancel".into(),
                input: record([("operation", Schema::String), ("generation", Schema::Int)]),
                result: null(),
            },
            cancel,
        ),
    ]
}
fn text<'a>(input: &'a Value, name: &str) -> &'a str {
    input.get(name).and_then(Value::as_str).unwrap_or_default()
}
fn generation(input: &Value) -> i64 {
    input
        .get("generation")
        .and_then(Value::as_i64)
        .unwrap_or(-1)
}
fn rejected(fault: Fault) -> Outcome {
    Outcome::Rejected { fault }
}
fn authorize(runtime: &Runtime, context: &CallContext, invocation: &Invocation) -> Outcome {
    let provider = text(&invocation.input, "provider").to_owned();
    if context.principal.is_empty()
        || misa_kernel::presets::preset_for_slot(&provider).is_none()
    {
        return rejected(Fault::new(
            "provider_unavailable",
            "Provider is unavailable",
        ));
    }
    let oauth = misa_kernel::presets::oauth(&provider).is_some();
    let scope = runtime.scope();
    runtime.operation_transition_as(crate::kernel_queue::Class::External, None, |store, _db| {
        if store.records.len() >= MAX_RECORDS {
            let oldest = store
                .records
                .values()
                .filter(|record| record.phase.terminal())
                .min_by_key(|record| record.id.clone())
                .map(|record| record.id.clone());
            if let Some(id) = oldest {
                store.records.remove(&id);
            } else {
                return Err(Fault::new("busy", "Too many active credential operations"));
            }
        }
        store.next = store
            .next
            .checked_add(1)
            .ok_or_else(|| Fault::new("exhausted", "Operation identities exhausted"))?;
        let id = format!("credential:{}:{}", scope.incarnation, store.next);
        store.records.insert(
            id.clone(),
            Record {
                id: id.clone(),
                principal: context.principal.clone(),
                provider: provider.clone(),
                account: {
                    let account = text(&invocation.input, "account").trim();
                    if account.is_empty() {
                        "default".into()
                    } else {
                        account.into()
                    }
                },
                oauth,
                generation: 1,
                phase: if oauth {
                    Phase::Running
                } else {
                    Phase::Awaiting
                },
                expires_ms: (!oauth).then(|| crate::now_ms() + KEY_TTL_MS),
                challenge: None,
            },
        );
        let account = text(&invocation.input, "account").trim().to_string();
        let request = oauth.then(|| Request::Credential {
            id: id.clone(),
            action: if account.is_empty() {
                CredentialAction::OAuth { provider }
            } else {
                CredentialAction::OAuthAccount { provider, account }
            },
        });
        Ok((
            Outcome::Accepted {
                operation: OperationRef { scope, id },
            },
            request,
        ))
    })
}
fn resolve(runtime: &Runtime, context: &CallContext, invocation: &Invocation) -> Outcome {
    let value = text(&invocation.input, "value");
    if value.trim().is_empty() || value.len() > 64 * 1024 {
        return rejected(Fault::new(
            "invalid_credential",
            "Credential value is empty or too large",
        ));
    }
    runtime.expire_operations(crate::now_ms());
    let scope = runtime.scope();
    runtime.operation_transition(None, |store, _db| {
        let record = store.owned(
            context,
            text(&invocation.input, "request"),
            generation(&invocation.input),
        )?;
        if record.oauth || record.phase != Phase::Awaiting
            || record.expires_ms.is_some_and(|deadline| deadline <= crate::now_ms())
        {
            return Err(Fault::new(
                "stale_request",
                "Credential request is not accepting a value",
            ));
        }
        record.phase = Phase::Submitting;
        record.generation += 1;
        record.expires_ms = None;
        let request = Request::Credential {
            id: record.id.clone(),
            action: CredentialAction::Set {
                slot: record.provider.clone(),
                account: record.account.clone(),
                value: value.into(),
            },
        };
        Ok((
            Outcome::Accepted {
                operation: OperationRef {
                    scope,
                    id: record.id.clone(),
                },
            },
            Some(request),
        ))
    })
}
fn cancel(runtime: &Runtime, context: &CallContext, invocation: &Invocation) -> Outcome {
    if runtime.state.lock().unwrap().operations.forms.contains_key(text(&invocation.input, "operation")) {
        let mut input = invocation.input.as_map().cloned().unwrap_or_default();
        let request = input.remove("operation").unwrap();
        input.insert("request".into(), request);
        return forms::cancel(runtime, context, &Invocation { input: Value::Map(Arc::new(input)), ..invocation.clone() });
    }
    let scope = runtime.scope();
    runtime.operation_transition(None, |store, db| {
        let id = text(&invocation.input, "operation");
        if let Some(principal) = store.prompt_owners.get(id) {
            let record = db
                .get("prompt_operations")
                .and_then(Value::as_list)
                .unwrap_or(&[])
                .iter()
                .find(|record| record.get("id").and_then(Value::as_str) == Some(id));
            if principal != &context.principal || record.is_none() {
                return Err(Fault::new(
                    "operation_unavailable",
                    "Operation is unavailable",
                ));
            }
            let record = record.unwrap();
            if record.get("generation").and_then(Value::as_i64)
                != Some(generation(&invocation.input))
                || record.get("terminal").and_then(Value::as_bool) == Some(true)
            {
                return Err(Fault::new(
                    "stale_request",
                    "Operation is no longer pending",
                ));
            }
            approvals::cancel_for(store, id);
            store
                .events
                .push(Event::new("operation/prompt.cancel").with("operation", Value::str(id)));
            return Ok((
                Outcome::Accepted {
                    operation: OperationRef {
                        scope,
                        id: id.into(),
                    },
                },
                None,
            ));
        }
        let record = store.owned(
            context,
            text(&invocation.input, "operation"),
            generation(&invocation.input),
        )?;
        if matches!(record.phase, Phase::Submitting | Phase::Cancelling) {
            return Err(Fault::new(
                "already_submitted",
                "Credential storage is already submitted",
            ));
        }
        record.phase = if record.oauth {
            Phase::Cancelling
        } else {
            Phase::Cancelled
        };
        record.generation += 1;
        record.expires_ms = None;
        record.challenge = None;
        let request = record.oauth.then(|| Request::Credential {
            id: format!("credential-cancel:{}", record.id),
            action: CredentialAction::CancelOAuth {
                request: record.id.clone(),
            },
        });
        Ok((
            if record.oauth {
                Outcome::Accepted {
                    operation: OperationRef {
                        scope,
                        id: record.id.clone(),
                    },
                }
            } else {
                Outcome::Completed { value: Value::Null }
            },
            request,
        ))
    })
}
impl Runtime {
    /// Logical parent work for host-composed child operations, independent of UI.
    pub fn work_context(&self) -> (Option<String>,Option<String>) {
        let state = self.state.lock().expect("session state is never poisoned");
        let db = state.state.db();
        let operation = db.get("session").and_then(|session|session.get("operation")).and_then(Value::as_str);
        let attempt=operation.and_then(|operation|db.get("attempts")?.as_list()?.iter().rev()
            .find(|attempt| attempt.get("operation").and_then(Value::as_str)==Some(operation))?
            .get("name")?.as_str().map(str::to_owned));
        (operation.map(str::to_owned),attempt)
    }
    fn operation_transition(
        &self, slots: Option<Value>,
        change: impl FnOnce(&mut Store, &Value) -> Result<(Outcome, Option<Request>), Fault>,
    ) -> Outcome { self.operation_transition_as(crate::kernel_queue::Class::Control, slots, change) }
    fn operation_transition_as(
        &self,
        class: crate::kernel_queue::Class,
        slots: Option<Value>,
        change: impl FnOnce(&mut Store, &Value) -> Result<(Outcome, Option<Request>), Fault>,
    ) -> Outcome {
        if self.is_closed() { return rejected(Fault::new("closed_scope", "Session owner is closed")); }
        let (result, request, outcome, revision) = {
            let mut state = self.state.lock().expect("session state is never poisoned");
            let mut candidate = state.operations.clone();
            if state.deferred.len() >= MAX_RECORDS {
                return rejected(Fault::new("busy", "Operation persistence is busy"));
            }
            let (result, mut request) = match change(&mut candidate, state.state.db()) {
                Ok(value) => value,
                Err(fault) => return rejected(fault),
            };
            let mut event = Event::new("operations/changed")
                .with("summary", candidate.summary())
                .with("requests", candidate.requests());
            if let Some(slots) = slots {
                event = event.with("slots", slots);
            }
            let events = std::mem::take(&mut candidate.events)
                .into_iter()
                .map(|mut event| {
                    if matches!(
                        event.kind.as_str(),
                        "intent/interrupt" | "operation/prompt.cancel"
                    ) {
                        if let Some(seq) = crate::pending_seq(state.state.db()) {
                            for suffix in ["text", "thinking"] {
                                let text = state
                                    .streams
                                    .get(&format!("msg.{seq}.{suffix}"))
                                    .map(|stream| stream.text.clone())
                                    .unwrap_or_default();
                                event = event.with(suffix, Value::str(text));
                            }
                        }
                    }
                    Value::map([("kind", Value::str(event.kind)), ("data", event.data)])
                });
            event = event.with("events", Value::list(events));
            let mut outcome = self.dispatch_admitted(&mut state, event, class);
            if !outcome.committed() {
                return rejected(outcome.as_faults().into_iter().next().unwrap_or_else(|| {
                    Fault::new("operation_failed", "Operation transition was refused")
                }));
            }
            approvals::reconcile(&mut candidate, state.state.db());
            candidate.prompt_owners.retain(|id, _| {
                state
                    .state
                    .db()
                    .get("prompt_operations")
                    .and_then(Value::as_list)
                    .unwrap_or(&[])
                    .iter()
                    .any(|record| record.get("id").and_then(Value::as_str) == Some(id))
            });
            state.operations = candidate;
            self.queue_operation_checkpoint(&mut state, &mut outcome, &mut request);
            let revision = state.state.rev();
            self.operation_deadline
                .send_replace(state.operations.deadline());
            (result, request, outcome, revision)
        };
        self.perform(&outcome);
        self.rev.send_replace(revision);
        if let Some(request) = request {
            if self.deliver_request(request, &outcome.admission).is_err() {
                return Outcome::Indeterminate {
                    fault: Fault::new(
                        "kernel_unavailable",
                        "Operation work could not be delivered; reconcile operation state",
                    ),
                };
            }
        }
        result
    }
    pub(crate) fn credential_event(&self, event: &Event) -> Option<Vec<Fault>> {
        let id = text(&event.data, "id");
        if event.kind == "kernel/credential" && id.starts_with("credential-cancel:") {
            return Some(vec![]);
        }
        if !matches!(
            event.kind.as_str(),
            "kernel/credential" | "kernel/credential.prompt"
        ) || !id.starts_with("credential:")
        {
            return None;
        }
        let slots = (event.kind == "kernel/credential").then(|| {
            event
                .get("slots")
                .cloned()
                .unwrap_or_else(|| Value::list([]))
        });
        let result = self.operation_transition(slots, |store, _db| {
            let Some(record) = store.records.get_mut(id) else {
                return Err(Fault::new(
                    "stale_operation",
                    "Credential operation already retired",
                ));
            };
            if record.phase.terminal() {
                return Err(Fault::new(
                    "stale_operation",
                    "Credential operation already settled",
                ));
            }
            if event.kind == "kernel/credential.prompt" {
                if !record.oauth || record.phase != Phase::Running {
                    return Err(Fault::new(
                        "stale_operation",
                        "Authorization challenge is stale",
                    ));
                }
                let url = event.get("url").and_then(Value::as_str).unwrap_or_default();
                let code = event
                    .get("code")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if url.len() > 16 * 1024 || code.len() > 4096 {
                    return Err(Fault::new(
                        "invalid_challenge",
                        "Authorization challenge is too large",
                    ));
                }
                record.challenge = Some((url.into(), code.into()));
                record.phase = Phase::Awaiting;
            } else {
                record.phase = if event.get("ok").and_then(Value::as_bool) == Some(true) {
                    Phase::Succeeded
                } else if record.phase == Phase::Cancelling
                    && event.get("message").and_then(Value::as_str)
                        == Some("Authorization cancelled")
                {
                    Phase::Cancelled
                } else {
                    Phase::Failed
                };
                record.challenge = None;
                record.expires_ms = None;
            }
            record.generation += 1;
            Ok((Outcome::Completed { value: Value::Null }, None))
        });
        // Late callbacks are expected after cancellation; never forward them into
        // legacy panel handlers or expose provider error text to shared snapshots.
        Some(match result {
            Outcome::Rejected { fault } if fault.code != "stale_operation" => vec![fault],
            _ => vec![],
        })
    }
    pub(crate) fn expire_operations(&self, now: i64) {
        let due = self
            .state
            .lock()
            .unwrap()
            .operations
            .deadline()
            .is_some_and(|deadline| deadline <= now);
        if !due {
            return;
        }
        self.operation_transition(None, |store, _db| {
            approvals::expire(store, now);
            forms::expire(store, now);
            for record in store.records.values_mut() {
                if record.phase == Phase::Awaiting
                    && record.expires_ms.is_some_and(|deadline| deadline <= now)
                {
                    record.phase = Phase::Expired;
                    record.generation += 1;
                    record.challenge = None;
                    record.expires_ms = None;
                }
            }
            Ok((Outcome::Completed { value: Value::Null }, None))
        });
    }
}
pub(crate) fn start_expiry_loop(runtime: &Arc<Runtime>) -> tokio::task::JoinHandle<()> {
    let weak = Arc::downgrade(runtime);
    let mut deadline = runtime.operation_deadline.subscribe();
    let mut closing = runtime.closing.subscribe();
    let mut revision = runtime.watch_rev();
    tokio::spawn(async move {
        loop {
            if *closing.borrow_and_update() { return; }
            let next = *deadline.borrow_and_update();
            if let Some(at) = next {
                tokio::select! {
                    _=closing.changed()=>{return;},
                    changed=deadline.changed()=>{if changed.is_err(){return;}},
                    _=tokio::time::sleep(std::time::Duration::from_millis(at.saturating_sub(crate::now_ms()).max(0) as u64))=>{
                        // Arm before attempting the transition: a checkpoint acknowledgement
                        // may make a refused transition retryable without moving its deadline.
                        revision.borrow_and_update();
                        let Some(runtime)=weak.upgrade() else{return;};
                        runtime.expire_operations(crate::now_ms());
                        drop(runtime);
                        if deadline.borrow().is_some_and(|at| at > crate::now_ms()) { continue; }
                        // A past deadline is not permission to busy-loop after refusal.
                        // Successful transitions notify deadline; refused ones await owner
                        // progress. Closing wakes this even while a client retains the owner.
                        tokio::select! {
                            _=closing.changed()=>{return;},
                            changed=deadline.changed()=>{if changed.is_err(){return;}},
                            changed=revision.changed()=>{if changed.is_err(){return;}},
                        }
                    }
                }
            } else {
                tokio::select! {
                    _=closing.changed()=>{return;},
                    changed=deadline.changed()=>{if changed.is_err(){return;}},
                }
            }
        }
    })
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn refused_expiry_waits_for_owner_progress_and_stops_on_shutdown() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use misa_reframe::FnHandler;
        let refuse = Arc::new(AtomicBool::new(false));
        let attempts = Arc::new(AtomicUsize::new(0));
        let refusal = refuse.clone();
        let counter = attempts.clone();
        let contribution = crate::Contribution::new().with_handler("operations/changed", 100,
            Arc::new(FnHandler::new("expiry-refusal", move |_: &mut misa_reframe::Tx<'_>, _: &Event| {
                if refusal.load(Ordering::SeqCst) {
                    counter.fetch_add(1, Ordering::SeqCst);
                    return Err(misa_reframe::Fault::new("test.refused", "Wait for owner progress"));
                }
                Ok(())
            })));
        let runtime = Runtime::start_with("expiry", "Expiry", None, Arc::new(Sink::default()),
            "scripted", "test", Value::Null, contribution);
        let id = start(&runtime, "alice", "anthropic");
        refuse.store(true, Ordering::SeqCst);
        let due = crate::now_ms() - 1;
        runtime.state.lock().unwrap().operations.records.get_mut(&id).unwrap().expires_ms = Some(due);
        runtime.operation_deadline.send_replace(Some(due));
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while attempts.load(Ordering::SeqCst) == 0 { tokio::task::yield_now().await; }
        }).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert_eq!(attempts.load(Ordering::SeqCst), 1, "an overdue refused transition must not spin");
        assert!(runtime.state.lock().unwrap().operations.records[&id].phase == Phase::Awaiting);
        assert!(matches!(resolve(&runtime, &context("alice"),
            &invocation(&runtime, "credentials.resolve", resolve_input(&id, 1))),
            Outcome::Rejected { fault } if fault.code == "stale_request"),
            "deadline validity is enforced even when publishing expiry was refused");
        refuse.store(false, Ordering::SeqCst);
        runtime.rev.send_replace(runtime.rev());
        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            loop {
                if runtime.state.lock().unwrap().operations.records[&id].phase == Phase::Expired { break; }
                tokio::task::yield_now().await;
            }
        }).await.unwrap();
        let task = start_expiry_loop(&runtime);
        runtime.shutdown_complete().await;
        tokio::time::timeout(std::time::Duration::from_secs(1), task).await.unwrap().unwrap();
        // The retained Arc must not keep the scheduling task alive after closure.
        assert!(runtime.is_closed());
    }
    #[tokio::test]
    async fn private_responses_are_declared_request_preparation_without_hiding_their_schemas() {
        let (runtime, _) = setup();
        for id in ["credentials.resolve", "input.resolve", "input.cancel"] {
            assert_eq!(runtime.command_registry[id].definition.preparation, misa_proto::invocation::Preparation::Request);
            runtime.command_registry[id].definition.validate().unwrap();
        }
        assert_eq!(runtime.command_registry["credentials.authorize"].definition.preparation, misa_proto::invocation::Preparation::Direct);
        runtime.shutdown_complete().await;
    }
    use super::*;
    use misa_kernel::{Kernel, KernelEvent};
    use std::{future::Future, pin::Pin, sync::Mutex};
    #[derive(Default)]
    struct Sink(Mutex<Vec<Request>>);
    impl Kernel for Sink {
        fn execute<'a, 'b, 'f>(
            &'a self,
            request: Request,
            _out: &'b tokio::sync::mpsc::UnboundedSender<KernelEvent>,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'f>>
        where
            'a: 'f,
            'b: 'f,
            Self: 'f,
        {
            Box::pin(async move {
                self.0.lock().unwrap().push(request);
            })
        }
    }
    fn setup() -> (Arc<Runtime>, Arc<Sink>) {
        let sink = Arc::new(Sink::default());
        (
            Runtime::start(
                "ops",
                "ops",
                None,
                sink.clone(),
                "scripted",
                "scripted-1",
                Value::Null,
            ),
            sink,
        )
    }
    fn context(principal: &str) -> CallContext {
        CallContext {
            principal: principal.into(),
            connection: 1,
        }
    }
    fn invocation(runtime: &Runtime, command: &str, input: Value) -> Invocation {
        Invocation {
            id: 1,
            scope: runtime.scope(),
            command: command.into(),
            input,
        }
    }
    fn acknowledge_checkpoints(runtime: &Runtime) {
        for _ in 0..MAX_RECORDS {
            let tokens = runtime.state.lock().unwrap().deferred.tokens();
            if tokens.is_empty() {
                return;
            }
            for token in tokens {
                assert!(
                    runtime
                        .dispatch(
                            Event::new("kernel/log.appended")
                                .with("kind", Value::str("operations.checkpoint"))
                                .with("data", Value::map([("checkpoint", Value::str(token))]))
                        )
                        .is_empty()
                );
            }
        }
        panic!("checkpoint acknowledgment did not settle");
    }
    fn start(runtime: &Runtime, principal: &str, provider: &str) -> String {
        let Outcome::Accepted { operation } = authorize(
            runtime,
            &context(principal),
            &invocation(
                runtime,
                "credentials.authorize",
                Value::map([("provider", Value::str(provider))]),
            ),
        ) else {
            panic!("authorization rejected")
        };
        acknowledge_checkpoints(runtime);
        operation.id
    }
    fn detail(runtime: &Runtime, id: &str, principal: &str) -> Result<Value, Fault> {
        request(
            &runtime.state.lock().unwrap(),
            &Query::new(REQUEST).arg(Value::str(id)),
            &context(principal),
        )
    }
    fn resolve_input(id: &str, generation: i64) -> Value {
        Value::map([
            ("request", Value::str(id)),
            ("generation", Value::Int(generation)),
            ("value", Value::str("SECRET-TOKEN")),
        ])
    }
    #[tokio::test]
    async fn independent_requests_resolve_once_and_credentials_never_enter_shared_state() {
        let (runtime, sink) = setup();
        let first = start(&runtime, "alice", "anthropic");
        let second = start(&runtime, "bob", "openai");
        assert!(detail(&runtime, &first, "alice").is_ok());
        assert!(detail(&runtime, &first, "bob").is_err());
        assert!(matches!(
            resolve(
                &runtime,
                &context("bob"),
                &invocation(&runtime, "credentials.resolve", resolve_input(&first, 1))
            ),
            Outcome::Rejected { .. }
        ));
        assert!(matches!(
            resolve(
                &runtime,
                &context("alice"),
                &invocation(&runtime, "credentials.resolve", resolve_input(&first, 1))
            ),
            Outcome::Accepted { .. }
        ));
        assert!(matches!(
            resolve(
                &runtime,
                &context("alice"),
                &invocation(&runtime, "credentials.resolve", resolve_input(&first, 1))
            ),
            Outcome::Rejected { .. }
        ));
        assert!(detail(&runtime, &second, "bob").is_ok());
        for _ in 0..20 {
            if !sink.0.lock().unwrap().is_empty() {
                break;
            }
            acknowledge_checkpoints(&runtime);
            tokio::task::yield_now().await;
        }
        let requests = sink.0.lock().unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|request| matches!(
                    request,
                    Request::Credential {
                        action: CredentialAction::Set { .. },
                        ..
                    }
                ))
                .count(),
            1
        );
        assert!(
            requests.iter().any(|request| matches!(request,Request::Credential {action:CredentialAction::Set {value,..},..} if value=="SECRET-TOKEN"))
        );
        drop(requests);
        assert!(
            !format!("{:?}", runtime.state.lock().unwrap().state.db()).contains("SECRET-TOKEN")
        );
        runtime.dispatch(
            Event::new("kernel/credential")
                .with("id", Value::str(&first))
                .with("ok", Value::Bool(true))
                .with("message", Value::str("SECRET-TOKEN must not be echoed"))
                .with("slots", Value::list([])),
        );
        let state = runtime.state.lock().unwrap();
        assert!(state.operations.records[&first].phase == Phase::Succeeded);
        assert!(!format!("{:?}", state.state.db()).contains("SECRET-TOKEN"));
    }
    #[tokio::test]
    async fn device_challenge_is_private_and_dismissal_does_not_cancel_work() {
        let (runtime, _) = setup();
        let id = start(&runtime, "alice", "kimi-coding");
        runtime.dispatch(
            Event::new("kernel/credential.prompt")
                .with("id", Value::str(&id))
                .with("url", Value::str("https://example.test/device"))
                .with("code", Value::str("PRIVATE-CODE")),
        );
        assert!(format!("{:?}", detail(&runtime, &id, "alice").unwrap()).contains("PRIVATE-CODE"));
        assert!(detail(&runtime, &id, "bob").is_err());
        assert!(
            !format!("{:?}", runtime.state.lock().unwrap().state.db()).contains("PRIVATE-CODE")
        );
        // Hiding client presentation has no owner event. An unrelated report or
        // local dialog cannot replace this operation's challenge.
        let second = start(&runtime, "alice", "anthropic");
        assert!(detail(&runtime, &id, "alice").is_ok());
        assert!(detail(&runtime, &second, "alice").is_ok());
        assert!(matches!(
            cancel(
                &runtime,
                &context("alice"),
                &invocation(
                    &runtime,
                    "operation.cancel",
                    Value::map([
                        ("operation", Value::str(&id)),
                        ("generation", Value::Int(2))
                    ])
                )
            ),
            Outcome::Accepted { .. }
        ));
        runtime.dispatch(
            Event::new("kernel/credential.prompt")
                .with("id", Value::str(&id))
                .with("code", Value::str("LATE-CODE")),
        );
        assert_eq!(detail(&runtime, &id, "alice").unwrap(), Value::Null);
        assert!(detail(&runtime, &second, "alice").is_ok());
    }
    #[tokio::test]
    async fn expiry_is_a_published_transition_and_stale_responses_are_rejected() {
        let (runtime, _) = setup();
        let id = start(&runtime, "alice", "anthropic");
        let (deadline, before) = {
            let state = runtime.state.lock().unwrap();
            (
                state.operations.records[&id].expires_ms.unwrap(),
                state.publications.position(),
            )
        };
        runtime.expire_operations(deadline);
        let state = runtime.state.lock().unwrap();
        assert!(state.publications.position() > before);
        assert!(state.operations.records[&id].phase == Phase::Expired);
        drop(state);
        assert!(matches!(
            resolve(
                &runtime,
                &context("alice"),
                &invocation(&runtime, "credentials.resolve", resolve_input(&id, 1))
            ),
            Outcome::Rejected { .. }
        ));
        assert_eq!(detail(&runtime, &id, "alice").unwrap(), Value::Null);
    }
    #[tokio::test]
    async fn cancellation_racing_completion_never_hides_a_stored_credential() {
        let (runtime, _) = setup();
        let id = start(&runtime, "alice", "kimi-coding");
        let result = cancel(
            &runtime,
            &context("alice"),
            &invocation(
                &runtime,
                "operation.cancel",
                Value::map([
                    ("operation", Value::str(&id)),
                    ("generation", Value::Int(1)),
                ]),
            ),
        );
        assert!(matches!(result, Outcome::Accepted { .. }));
        assert!(runtime.state.lock().unwrap().operations.records[&id].phase == Phase::Cancelling);
        // Kernel cancellation can discover work already finished before its
        // original completion reaches the session. The receipt cannot settle it.
        runtime.dispatch(
            Event::new("kernel/credential")
                .with("id", Value::str(format!("credential-cancel:{id}")))
                .with("ok", Value::Bool(false))
                .with("message", Value::str("Authorization already finished")),
        );
        assert!(runtime.state.lock().unwrap().operations.records[&id].phase == Phase::Cancelling);
        runtime.dispatch(
            Event::new("kernel/credential")
                .with("id", Value::str(&id))
                .with("ok", Value::Bool(true))
                .with("slots", Value::list([])),
        );
        assert!(runtime.state.lock().unwrap().operations.records[&id].phase == Phase::Succeeded);
        let other = start(&runtime, "alice", "kimi-coding");
        cancel(
            &runtime,
            &context("alice"),
            &invocation(
                &runtime,
                "operation.cancel",
                Value::map([
                    ("operation", Value::str(&other)),
                    ("generation", Value::Int(1)),
                ]),
            ),
        );
        runtime.dispatch(
            Event::new("kernel/credential")
                .with("id", Value::str(&other))
                .with("ok", Value::Bool(false))
                .with("message", Value::str("Authorization cancelled"))
                .with("slots", Value::list([])),
        );
        assert!(runtime.state.lock().unwrap().operations.records[&other].phase == Phase::Cancelled);
    }
    #[tokio::test]
    async fn restricted_query_is_context_bound_and_terminal_transition_clears_challenge() {
        use misa_proto::observation::{Content, Encoding, Handle, Member, Publication, Selection};
        let (runtime, _) = setup();
        let id = start(&runtime, "alice", "kimi-coding");
        runtime.dispatch(
            Event::new("kernel/credential.prompt")
                .with("id", Value::str(&id))
                .with("url", Value::str("https://example.test"))
                .with("code", Value::str("PRIVATE-CODE")),
        );
        let selection = Selection {
            scope: runtime.scope(),
            members: BTreeMap::from([(
                "request".into(),
                Member {
                    query: Query::new(REQUEST).arg(Value::str(&id)),
                    contract: "operation.request@1".into(),
                    encoding: Encoding::Value,
                    optional: false,
                },
            )]),
        };
        assert!(runtime.read_selection(&selection).is_err());
        assert!(
            runtime
                .read_selection_with_context(Some(&context("bob")), &selection)
                .is_err()
        );
        let (mut observer, initial) = runtime
            .observe_with_context(
                Some(context("alice")),
                Handle {
                    id: 1,
                    generation: 1,
                },
                selection,
                None,
            )
            .unwrap();
        assert!(matches!(initial, Publication::Snapshot { .. }));
        runtime.dispatch(
            Event::new("kernel/credential")
                .with("id", Value::str(&id))
                .with("ok", Value::Bool(true))
                .with("slots", Value::list([])),
        );
        let publication = observer.poll().unwrap();
        assert!(!format!("{publication:?}").contains("PRIVATE-CODE"));
        let current = runtime
            .read_selection_with_context(Some(&context("alice")), observer.selection())
            .unwrap();
        assert_eq!(current.members["request"], Content::Value(Value::Null));
    }

    #[tokio::test]
    async fn dropping_owner_discards_unacknowledged_secret_work() {
        let (runtime, sink) = setup();
        let id = start(&runtime, "alice", "anthropic");
        assert!(matches!(
            resolve(
                &runtime,
                &context("alice"),
                &invocation(&runtime, "credentials.resolve", resolve_input(&id, 1))
            ),
            Outcome::Accepted { .. }
        ));
        assert_eq!(runtime.state.lock().unwrap().deferred.len(), 1);
        let weak = Arc::downgrade(&runtime);
        drop(runtime);
        assert!(
            weak.upgrade().is_none(),
            "background report loops must not keep the owner alive"
        );
        tokio::task::yield_now().await;
        assert!(!sink.0.lock().unwrap().iter().any(|request| matches!(
            request,
            Request::Credential {
                action: CredentialAction::Set { .. },
                ..
            }
        )));
    }
    #[tokio::test]
    async fn checkpoint_failure_is_atomic_when_handlers_reject_or_have_pending_writes() {
        use misa_reframe::{FnHandler, Tx};
        use misa_proto::observation::{Selection, Member, Encoding, Handle, Publication};
        for mode in ["accepted", "rejected", "busy"] {
            let contribution = crate::Contribution::new()
                .with_root("failure_seen", Value::Bool(false)).unwrap()
                .with_handler("operations/persistence.failed", 100, Arc::new(FnHandler::new(
                    "test.failure", move |tx: &mut Tx<'_>, _: &Event| {
                        if mode == "rejected" { return Err(misa_reframe::Fault::handler("publication refused")); }
                        tx.set("failure_seen", Value::Bool(true))
                    },
                )))
                .with_handler("test/stage", 0, Arc::new(FnHandler::new(
                    "test.stage", |tx: &mut Tx<'_>, _: &Event| tx.set("failure_seen", Value::Bool(true)),
                )));
            let sink = Arc::new(Sink::default());
            let runtime = Runtime::start_with("failure", "failure", None, sink.clone(), "scripted", "scripted-1", Value::Null, contribution);
            let id = start(&runtime, "alice", "anthropic");
            assert!(matches!(resolve(&runtime, &context("alice"), &invocation(&runtime, "credentials.resolve", resolve_input(&id, 1))), Outcome::Accepted { .. }));
            let token = runtime.state.lock().unwrap().deferred.tokens()[0].clone();
            let selection = Selection { scope: runtime.scope(), members: BTreeMap::from([("operations".into(), Member {
                query: Query::new(SUMMARY), contract: "operations.summary@1".into(), encoding: Encoding::Value, optional: false,
            })]) };
            let (mut observer, _) = runtime.observe(Handle { id: 9, generation: 1 }, selection.clone(), None).unwrap();
            if mode == "busy" { assert!(runtime.dispatch(Event::new("test/stage")).is_empty()); }
            let faults = runtime.dispatch(Event::new("kernel/log.failed")
                .with("kind", Value::str("operations.checkpoint"))
                .with("data", Value::map([("checkpoint", Value::str(&token))])));
            tokio::task::yield_now().await;
            assert!(!sink.0.lock().unwrap().iter().any(|request| matches!(request, Request::Credential { action: CredentialAction::Set { .. }, .. })));
            assert_eq!(runtime.state.lock().unwrap().deferred.len(), 0);
            if mode != "accepted" {
                assert_eq!(faults[0].code, "publication_failed");
                assert!(runtime.is_closed());
                assert!(matches!(observer.poll(), Some(Publication::Closed { .. })));
                assert!(runtime.read_selection(&selection).is_err());
            } else {
                assert_eq!(faults[0].code, "persistence_failed");
                assert!(!runtime.is_closed());
                assert!(matches!(observer.poll(), Some(Publication::Update { .. })));
                assert!(runtime.state.lock().unwrap().operations.records[&id].phase == Phase::Interrupted);
                // The contributed state mutation remains journal-gated, and its
                // effect must survive the failure publication's dispatch.
                let data = sink.0.lock().unwrap().iter().find_map(|request| match request {
                    Request::Append { kind, data, .. } if kind == crate::contribution::PATCH_KIND => Some(data.clone()),
                    _ => None,
                }).expect("failure handler journal effect delivered");
                assert_eq!(runtime.state.lock().unwrap().state.db().get("failure_seen"), Some(&Value::Bool(false)));
                runtime.dispatch(Event::new("kernel/log.appended").with("conversation", Value::str("failure")).with("kind", Value::str(crate::contribution::PATCH_KIND)).with("data", data));
                assert_eq!(runtime.state.lock().unwrap().state.db().get("failure_seen"), Some(&Value::Bool(true)));
            }
            runtime.shutdown_complete().await;
        }
    }
    #[tokio::test]
    async fn checkpoint_failure_discards_staged_prompt_and_credential_effects() {
        for credential in [false, true] {
            let (runtime, sink) = setup();
            let operation = if credential {
                let id = start(&runtime, "alice", "anthropic");
                let outcome = resolve(
                    &runtime,
                    &context("alice"),
                    &invocation(&runtime, "credentials.resolve", resolve_input(&id, 1)),
                );
                assert!(matches!(outcome, Outcome::Accepted { .. }));
                id
            } else {
                let outcome = prompt(
                    &runtime,
                    &context("alice"),
                    &invocation(
                        &runtime,
                        "session.prompt",
                        Value::map([("text", Value::str("hello"))]),
                    ),
                    false,
                );
                let Outcome::Accepted { operation } = outcome else {
                    panic!()
                };
                operation.id
            };
            let tokens = runtime.state.lock().unwrap().deferred.tokens();
            assert_eq!(tokens.len(), 1);
            // Accepted is a receipt: before persistence no message/provider/token effect runs.
            tokio::task::yield_now().await;
            assert!(!sink.0.lock().unwrap().iter().any(|request| match request {
                Request::Credential {
                    action: CredentialAction::Set { .. },
                    ..
                }
                | Request::ProviderCall { .. } => true,
                Request::Append { kind, .. } => kind == "message",
                _ => false,
            }));
            let failed = runtime.dispatch(
                Event::new("kernel/log.failed")
                    .with("kind", Value::str("operations.checkpoint"))
                    .with("data", Value::map([("checkpoint", Value::str(&tokens[0]))])),
            );
            assert_eq!(failed[0].code, "persistence_failed");
            assert_eq!(
                result(&runtime, &operation)
                    .get("state")
                    .and_then(Value::as_str),
                Some("interrupted")
            );
            // A late success cannot release discarded effects after a failure.
            runtime.dispatch(
                Event::new("kernel/log.appended")
                    .with("kind", Value::str("operations.checkpoint"))
                    .with("data", Value::map([("checkpoint", Value::str(&tokens[0]))])),
            );
            tokio::task::yield_now().await;
            assert!(!sink.0.lock().unwrap().iter().any(|request| matches!(
                request,
                Request::Credential {
                    action: CredentialAction::Set { .. },
                    ..
                } | Request::ProviderCall { .. }
            )));
            assert_eq!(runtime.state.lock().unwrap().deferred.len(), 0);
        }
    }
    fn checkpoint_entries(sink: &Sink) -> Vec<Value> {
        sink.0
            .lock()
            .unwrap()
            .iter()
            .filter_map(|request| match request {
                Request::Append { kind, data, .. } if kind == "operations.checkpoint" => {
                    Some((kind.clone(), data.clone()))
                }
                _ => None,
            })
            .enumerate()
            .map(|(seq, (kind, data))| {
                Value::map([
                    ("seq", Value::Int(seq as i64 + 1)),
                    ("kind", Value::str(kind)),
                    ("data", data),
                ])
            })
            .collect()
    }
    #[tokio::test]
    async fn restart_reconciles_logical_operations_without_replaying_effects_or_secrets() {
        let (runtime, sink) = approval_setup();
        let (operation, request) = request_tool(&runtime, 100);
        let credential = start(&runtime, "alice", "anthropic");
        assert!(matches!(
            resolve(
                &runtime,
                &context("alice"),
                &invocation(
                    &runtime,
                    "credentials.resolve",
                    resolve_input(&credential, 1)
                )
            ),
            Outcome::Accepted { .. }
        ));
        acknowledge_checkpoints(&runtime);
        tokio::task::yield_now().await;
        let entries = checkpoint_entries(&sink);
        assert!(!entries.is_empty());
        assert!(!format!("{entries:?}").contains("SECRET-TOKEN"));
        assert!(!format!("{entries:?}").contains("PRIVATE-CODE"));
        let old_scope = runtime.scope();
        let (restarted, after) = approval_setup();
        assert_ne!(old_scope.incarnation, restarted.scope().incarnation);
        assert!(
            restarted
                .dispatch(
                    Event::new("kernel/log.loaded")
                        .with("conversation", Value::str("ops"))
                        .with("entries", Value::list(entries))
                )
                .is_empty()
        );
        for id in [&operation, &credential] {
            let result = result(&restarted, id);
            assert_eq!(
                result.get("state").and_then(Value::as_str),
                Some("interrupted")
            );
            assert_eq!(result.get("terminal"), Some(&Value::Bool(true)));
        }
        assert_eq!(detail(&restarted, &request, "alice").unwrap(), Value::Null);
        assert!(matches!(
            resolve_tool(&restarted, "alice", &request, true),
            Outcome::Rejected { .. }
        ));
        acknowledge_checkpoints(&runtime);
        tokio::task::yield_now().await;
        assert!(!after.0.lock().unwrap().iter().any(|request| matches!(
            request,
            Request::ToolRun { .. } | Request::Credential { .. }
        )));
    }
    #[tokio::test]
    async fn durable_final_message_reconciles_success_without_the_trailing_checkpoint() {
        let (runtime, sink) = setup();
        let operation = submit_prompt(&runtime, 110, false);
        acknowledge_checkpoints(&runtime);
        tokio::task::yield_now().await;
        let mut entries = checkpoint_entries(&sink);
        entries.push(Value::map([
            ("seq", Value::Int(10)),
            ("kind", Value::str("message")),
            (
                "data",
                Value::map([
                    ("seq", Value::Int(2)),
                    ("role", Value::str("assistant")),
                    ("state", Value::str("done")),
                    ("operation", Value::str(&operation)),
                    ("text", Value::str("persisted answer")),
                    ("calls", Value::list([])),
                ]),
            ),
        ]));
        let (restarted, _) = setup();
        assert!(
            restarted
                .dispatch(
                    Event::new("kernel/log.loaded")
                        .with("conversation", Value::str("ops"))
                        .with("entries", Value::list(entries))
                )
                .is_empty()
        );
        let result = result(&restarted, &operation);
        assert_eq!(
            result.get("state").and_then(Value::as_str),
            Some("succeeded")
        );
        assert_eq!(result.get("outputs"), Some(&Value::list([Value::Int(2)])));
        let crate::Reading::Data(output) = restarted
            .read(&Query::new("operation.presentation").arg(Value::str(&operation)))
            .unwrap()
        else {
            panic!()
        };
        assert!(format!("{output:?}").contains("persisted answer"));
    }
    fn approval_setup() -> (Arc<Runtime>, Arc<Sink>) {
        let sink = Arc::new(Sink::default());
        let runtime = Runtime::start(
            "ops",
            "ops",
            None,
            sink.clone(),
            "scripted",
            "scripted-1",
            Value::map([("tool_approval", Value::str("ask"))]),
        );
        (runtime, sink)
    }
    fn request_tool(runtime: &Runtime, operation: u64) -> (String, String) {
        let operation = submit_prompt(runtime, operation, false);
        ack_prompt(runtime, 1, "user", "done", Value::list([]));
        ack_prompt(
            runtime,
            2,
            "assistant",
            "done",
            Value::list([Value::map([
                ("id", Value::str("call-1")),
                ("name", Value::str("shell")),
                ("args", Value::map([("command", Value::str("example"))])),
                ("status", Value::str("pending")),
            ])]),
        );
        let request = runtime
            .state
            .lock()
            .unwrap()
            .operations
            .approvals
            .keys()
            .next()
            .unwrap()
            .clone();
        (operation, request)
    }
    fn resolve_tool(runtime: &Runtime, principal: &str, id: &str, approved: bool) -> Outcome {
        (runtime.command_registry["input.resolve"].handler)(
            runtime,
            &context(principal),
            &invocation(
                runtime,
                "input.resolve",
                Value::map([
                    ("request", Value::str(id)),
                    ("generation", Value::Int(1)),
                    ("approved", Value::Bool(approved)),
                ]),
            ),
        )
    }
    #[tokio::test]
    async fn tool_approval_is_private_and_only_first_valid_response_executes() {
        let (runtime, sink) = approval_setup();
        let (_, request) = request_tool(&runtime, 70);
        acknowledge_checkpoints(&runtime);
        tokio::task::yield_now().await;
        assert!(
            !sink
                .0
                .lock()
                .unwrap()
                .iter()
                .any(|request| matches!(request, Request::ToolRun { .. }))
        );
        assert!(detail(&runtime, &request, "bob").is_err());
        assert_eq!(
            detail(&runtime, &request, "alice")
                .unwrap()
                .get("kind")
                .and_then(Value::as_str),
            Some("tool_approval")
        );
        assert!(matches!(
            resolve_tool(&runtime, "bob", &request, true),
            Outcome::Rejected { .. }
        ));
        assert!(matches!(
            resolve_tool(&runtime, "alice", &request, true),
            Outcome::Completed { .. }
        ));
        assert!(matches!(
            resolve_tool(&runtime, "alice", &request, true),
            Outcome::Rejected { .. }
        ));
        acknowledge_checkpoints(&runtime);
        tokio::task::yield_now().await;
        assert_eq!(
            sink.0
                .lock()
                .unwrap()
                .iter()
                .filter(|request| matches!(request, Request::ToolRun { .. }))
                .count(),
            1
        );
        assert_eq!(detail(&runtime, &request, "alice").unwrap(), Value::Null);
    }
    #[tokio::test]
    async fn interrupt_before_approved_checkpoint_ack_prevents_tool_execution() {
        let (runtime, sink) = approval_setup();
        let (_, request) = request_tool(&runtime, 72);
        assert!(matches!(resolve_tool(&runtime, "alice", &request, true), Outcome::Completed { .. }));
        assert!(runtime.state.lock().unwrap().deferred.len() > 0);
        submit_prompt(&runtime, 73, true);
        tokio::task::yield_now().await;
        let requests = sink.0.lock().unwrap();
        assert!(!requests.iter().any(|request| matches!(request, Request::ToolRun { .. })));
        assert!(requests.iter().any(|request| matches!(request, Request::Append { kind, data, .. } if kind == "tool_result" && data.get("ok") == Some(&Value::Bool(false)))));
    }
    #[tokio::test]
    async fn denial_expiry_and_operation_cancellation_never_execute_a_tool() {
        for disposition in ["deny", "expire", "cancel", "interrupt"] {
            let (runtime, sink) = approval_setup();
            let (operation, request) = request_tool(&runtime, 80);
            match disposition {
                "deny" => {
                    assert!(matches!(
                        resolve_tool(&runtime, "alice", &request, false),
                        Outcome::Completed { .. }
                    ));
                }
                "expire" => runtime.expire_operations(crate::now_ms() + KEY_TTL_MS + 1),
                "cancel" => {
                    let input = Value::map([
                        ("operation", Value::str(&operation)),
                        ("generation", Value::Int(1)),
                    ]);
                    assert!(matches!(
                        cancel(
                            &runtime,
                            &context("bob"),
                            &invocation(&runtime, "operation.cancel", input.clone())
                        ),
                        Outcome::Rejected { .. }
                    ));
                    assert!(matches!(
                        cancel(
                            &runtime,
                            &context("alice"),
                            &invocation(&runtime, "operation.cancel", input)
                        ),
                        Outcome::Accepted { .. }
                    ));
                }
                _ => {
                    submit_prompt(&runtime, 81, true);
                }
            }
            assert!(
                matches!(
                    resolve_tool(&runtime, "alice", &request, true),
                    Outcome::Rejected { .. }
                ),
                "{disposition}"
            );
            assert_eq!(
                detail(&runtime, &request, "alice").unwrap(),
                Value::Null,
                "{disposition}"
            );
            acknowledge_checkpoints(&runtime);
            tokio::task::yield_now().await;
            let requests = sink.0.lock().unwrap();
            assert!(
                !requests
                    .iter()
                    .any(|request| matches!(request, Request::ToolRun { .. })),
                "{disposition}"
            );
            assert!(requests.iter().any(|request| matches!(request, Request::Append { kind, data, .. } if kind == "tool_result" && data.get("ok") == Some(&Value::Bool(false)))), "{disposition}");
        }
    }
    #[tokio::test]
    async fn targeted_cancel_does_not_cancel_other_prompts_or_accept_duplicate_generation() {
        let (runtime, _) = setup();
        let active = submit_prompt(&runtime, 90, false);
        let queued = submit_prompt(&runtime, 91, false);
        let input =
            |id: &str| Value::map([("operation", Value::str(id)), ("generation", Value::Int(1))]);
        assert!(matches!(
            cancel(
                &runtime,
                &context("alice"),
                &invocation(&runtime, "operation.cancel", input(&queued))
            ),
            Outcome::Accepted { .. }
        ));
        assert_eq!(
            result(&runtime, &active).get("terminal"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            result(&runtime, &queued)
                .get("state")
                .and_then(Value::as_str),
            Some("cancelled")
        );
        ack_prompt(&runtime, 1, "user", "done", Value::list([]));
        assert!(matches!(
            cancel(
                &runtime,
                &context("alice"),
                &invocation(&runtime, "operation.cancel", input(&active))
            ),
            Outcome::Accepted { .. }
        ));
        assert!(matches!(
            cancel(
                &runtime,
                &context("alice"),
                &invocation(&runtime, "operation.cancel", input(&active))
            ),
            Outcome::Rejected { .. }
        ));
        assert_eq!(
            result(&runtime, &active).get("terminal"),
            Some(&Value::Bool(false))
        );
        ack_prompt(&runtime, 2, "assistant", "cancelled", Value::list([]));
        assert_eq!(
            result(&runtime, &active)
                .get("state")
                .and_then(Value::as_str),
            Some("cancelled")
        );
    }
    fn submit_prompt(runtime: &Runtime, id: u64, interrupt: bool) -> String {
        let mut invocation = invocation(
            runtime,
            "session.prompt",
            Value::map([("text", Value::str("hello"))]),
        );
        invocation.id = id;
        let Outcome::Accepted { operation } =
            prompt(runtime, &context("alice"), &invocation, interrupt)
        else {
            panic!("prompt rejected")
        };
        acknowledge_checkpoints(runtime);
        operation.id
    }
    fn result(runtime: &Runtime, id: &str) -> Value {
        let crate::Reading::Data(value) = runtime
            .read(&Query::new("operation.result").arg(Value::str(id)))
            .unwrap()
        else {
            panic!()
        };
        value
    }
    fn ack_prompt(runtime: &Runtime, seq: i64, role: &str, state: &str, calls: Value) {
        let faults = runtime.dispatch(
            Event::new("kernel/log.appended")
                .with("conversation", Value::str("ops"))
                .with("seq", Value::Int(seq))
                .with("kind", Value::str("message"))
                .with(
                    "data",
                    Value::map([
                        ("seq", Value::Int(seq)),
                        ("role", Value::str(role)),
                        ("state", Value::str(state)),
                        ("text", Value::str("answer")),
                        ("calls", calls),
                    ]),
                ),
        );
        assert!(faults.is_empty(), "{faults:?}");
        acknowledge_checkpoints(runtime);
    }
    #[tokio::test]
    async fn prompt_completion_is_journaled_and_correlated_while_next_prompt_runs() {
        let (runtime, _) = setup();
        let first = submit_prompt(&runtime, 10, false);
        let second = submit_prompt(&runtime, 11, false);
        assert_eq!(
            result(&runtime, &second)
                .get("state")
                .and_then(Value::as_str),
            Some("queued")
        );
        ack_prompt(&runtime, 1, "user", "done", Value::list([]));
        runtime.dispatch(
            Event::new("kernel/provider.finished")
                .with("id", Value::str("r1"))
                .with("ok", Value::Bool(true))
                .with("text", Value::str("answer")),
        );
        assert_eq!(
            result(&runtime, &first).get("terminal"),
            Some(&Value::Bool(false)),
            "provider completion precedes persistence"
        );
        ack_prompt(&runtime, 2, "assistant", "done", Value::list([]));
        let done = result(&runtime, &first);
        assert_eq!(done.get("state").and_then(Value::as_str), Some("succeeded"));
        assert_eq!(done.get("outputs"), Some(&Value::list([Value::Int(2)])));
        assert_eq!(
            result(&runtime, &second)
                .get("state")
                .and_then(Value::as_str),
            Some("running")
        );
        let crate::Reading::Data(value) = runtime
            .read(&Query::new("operation.presentation").arg(Value::str(&first)))
            .unwrap()
        else {
            panic!()
        };
        let output: misa_proto::Node = crate::wire::parse(&value).unwrap();
        assert!(misa_proto::view::find(&output, "msg.2").is_some());
        assert!(
            misa_proto::view::find(&output, "msg.1").is_none(),
            "the prompt itself is not answer output"
        );
        assert!(
            misa_proto::view::find(&output, "msg.3").is_none(),
            "the next operation cannot leak into output"
        );
        assert_ne!(
            runtime
                .state
                .lock()
                .unwrap()
                .state
                .db()
                .get("session")
                .unwrap()
                .get("status")
                .and_then(Value::as_str),
            Some("idle")
        );
    }
    #[tokio::test]
    async fn prompt_tool_round_remains_pending_and_removed_queue_is_cancelled() {
        let (runtime, _) = setup();
        let first = submit_prompt(&runtime, 20, false);
        let removed = submit_prompt(&runtime, 21, false);
        assert!(
            runtime
                .dispatch(Event::new("intent/queue.clear"))
                .is_empty()
        );
        assert_eq!(
            result(&runtime, &removed)
                .get("state")
                .and_then(Value::as_str),
            Some("cancelled")
        );
        ack_prompt(&runtime, 1, "user", "done", Value::list([]));
        ack_prompt(
            &runtime,
            2,
            "assistant",
            "done",
            Value::list([Value::map([
                ("id", Value::str("call-1")),
                ("name", Value::str("tool")),
                ("status", Value::str("pending")),
            ])]),
        );
        assert_eq!(
            result(&runtime, &first).get("terminal"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            result(&runtime, &first).get("outputs"),
            Some(&Value::list([Value::Int(2)]))
        );
        runtime.dispatch(
            Event::new("kernel/failed")
                .with("id", Value::str("unrelated-discovery"))
                .with("message", Value::str("unavailable")),
        );
        assert_eq!(
            result(&runtime, &first).get("terminal"),
            Some(&Value::Bool(false))
        );
        runtime.dispatch(
            Event::new("kernel/failed")
                .with("id", Value::str("ops"))
                .with("message", Value::str("failed")),
        );
        assert_eq!(
            result(&runtime, &first)
                .get("state")
                .and_then(Value::as_str),
            Some("failed")
        );
    }
    #[tokio::test]
    async fn interrupt_cancels_only_prior_operation_after_acknowledgement() {
        let (runtime, _) = setup();
        let first = submit_prompt(&runtime, 30, false);
        ack_prompt(&runtime, 1, "user", "done", Value::list([]));
        let urgent = submit_prompt(&runtime, 31, true);
        assert_eq!(
            result(&runtime, &first).get("terminal"),
            Some(&Value::Bool(false))
        );
        ack_prompt(&runtime, 2, "assistant", "cancelled", Value::list([]));
        assert_eq!(
            result(&runtime, &first)
                .get("state")
                .and_then(Value::as_str),
            Some("cancelled")
        );
        assert_eq!(
            result(&runtime, &urgent)
                .get("state")
                .and_then(Value::as_str),
            Some("running")
        );
        ack_prompt(&runtime, 3, "user", "done", Value::list([]));
        ack_prompt(&runtime, 4, "assistant", "failed", Value::list([]));
        assert_eq!(
            result(&runtime, &urgent)
                .get("state")
                .and_then(Value::as_str),
            Some("failed")
        );
    }
}

/// Prompt admission creates the operation in the same transaction as its queue entry.
/// Completion is recorded by the journal acknowledgment, never by a client idle timer.
pub(crate) fn prompt(
    runtime: &Runtime,
    context: &CallContext,
    invocation: &Invocation,
    interrupt: bool,
) -> Outcome {
    let scope = runtime.scope();
    let id = format!(
        "prompt:{}:{}:{}",
        scope.incarnation, context.connection, invocation.id
    );
    let event = Event::new(if interrupt {
        "intent/interrupt"
    } else {
        "intent/prompt"
    })
    .with(
        if interrupt { "prompt" } else { "text" },
        invocation.input.get("text").cloned().unwrap_or(Value::Null),
    )
    .with(
        "attachments",
        invocation
            .input
            .get("attachments")
            .cloned()
            .unwrap_or_else(|| Value::list([])),
    )
    .with("operation", Value::str(&id));
    runtime.operation_transition_as(crate::kernel_queue::Class::External, None, |store, db| {
        // Retention follows the public operation records; identities are owner-only.
        store.prompt_owners.retain(|key, _| {
            db.get("prompt_operations")
                .and_then(Value::as_list)
                .unwrap_or(&[])
                .iter()
                .any(|record| record.get("id").and_then(Value::as_str) == Some(key))
        });
        if context.principal.is_empty() {
            return Err(Fault::new(
                "unauthorized",
                "Prompt requires an authenticated caller",
            ));
        }
        store
            .prompt_owners
            .insert(id.clone(), context.principal.clone());
        store.events.push(event);
        Ok((
            Outcome::Accepted {
                operation: OperationRef { scope, id },
            },
            None,
        ))
    })
}
pub(crate) fn admit_prompt(tx: &mut Tx<'_>, event: &Event) -> Result<Value, misa_reframe::Fault> {
    let operation = event.get("operation").cloned().unwrap_or(Value::Null);
    if operation.as_str().is_none() {
        return Ok(operation);
    }
    let mut records = tx
        .get("prompt_operations")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .to_vec();
    if records.iter().any(|r| r.get("id") == Some(&operation)) {
        return Err(misa_reframe::Fault::handler(
            "Prompt operation already exists",
        ));
    }
    if records.len() >= MAX_RECORDS {
        let Some(index) = records
            .iter()
            .position(|r| r.get("terminal").and_then(Value::as_bool) == Some(true))
        else {
            return Err(misa_reframe::Fault::handler(
                "Too many pending prompt operations",
            ));
        };
        records.remove(index);
    }
    records.push(Value::map([
        ("id", operation.clone()),
        ("kind", Value::str("session.prompt")),
        ("state", Value::str("queued")),
        ("generation", Value::Int(1)),
        ("terminal", Value::Bool(false)),
        ("outputs", Value::list([])),
    ]));
    tx.set("prompt_operations", Value::list(records))?;
    Ok(operation)
}
pub(crate) fn prompt_state(
    tx: &mut Tx<'_>,
    id: &Value,
    state: &str,
    output: Option<i64>,
) -> Result<(), misa_reframe::Fault> {
    if id.as_str().is_none() {
        return Ok(());
    }
    let Some(index) = tx
        .get("prompt_operations")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .iter()
        .position(|r| {
            r.get("id") == Some(id) && r.get("terminal").and_then(Value::as_bool) != Some(true)
        })
    else {
        return Ok(());
    };
    let base = format!("prompt_operations[{index}]");
    tx.set(&format!("{base}.state"), Value::str(state))?;
    tx.set(
        &format!("{base}.terminal"),
        Value::Bool(matches!(
            state,
            "succeeded" | "failed" | "cancelled" | "interrupted"
        )),
    )?;
    if let Some(seq) = output {
        tx.push(&format!("{base}.outputs"), Value::Int(seq))?;
    }
    Ok(())
}
pub(crate) fn settle_prompt(
    tx: &mut Tx<'_>,
    state: &str,
    output: Option<i64>,
) -> Result<(), misa_reframe::Fault> {
    let id = tx.get("session.operation").cloned().unwrap_or(Value::Null);
    prompt_state(tx, &id, state, output)?;
    if matches!(state, "succeeded" | "failed" | "cancelled" | "interrupted") {
        tx.delete("session.operation")?;
    }
    Ok(())
}

/// Cancelling an owner operation also resolves its unsubmitted tool requests.
/// Tool outcomes are journalled; no denied tool reaches the kernel executor.
pub(crate) fn cancel_prompt_inputs(
    tx: &mut Tx<'_>,
    operation: &Value,
) -> Result<(), misa_reframe::Fault> {
    if operation.as_str().is_none() {
        return Ok(());
    }
    let mut summaries = tx
        .get("input_requests")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .to_vec();
    let mut changed = false;
    for summary in &mut summaries {
        if summary.get("operation") != Some(operation)
            || summary.get("kind").and_then(Value::as_str) != Some("tool_approval")
            || summary.get("state").and_then(Value::as_str) != Some("awaiting_input")
        {
            continue;
        }
        let call = summary.get("call").cloned().unwrap_or(Value::Null);
        let generation = summary
            .get("generation")
            .and_then(Value::as_i64)
            .unwrap_or(1)
            + 1;
        if let Value::Map(fields) = summary {
            let fields = Arc::make_mut(fields);
            fields.insert("state".into(), Value::str("cancelled"));
            fields.insert("generation".into(), Value::Int(generation));
        }
        tx.fx(misa_reframe::Effect::new("kernel.log.append")
            .with("conversation", Value::str(tx.text("session.conversation")))
            .with("kind", Value::str("tool_result"))
            .with(
                "data",
                Value::map([
                    ("call", call),
                    ("ok", Value::Bool(false)),
                    ("text", Value::str("Tool approval cancelled")),
                ]),
            ));
        changed = true;
    }
    if changed {
        tx.set("input_requests", Value::list(summaries))?;
    }
    Ok(())
}

pub(crate) fn restore_event(event: Event) -> (Event, Option<Store>) {
    journal::restore(event)
}

impl Runtime {
    /// Resolve the host-bound responder for this exact currently-running tool.
    /// A tool request never supplies its own caller identity.
    pub(crate) fn trusted_tool_context(&self, request: &Request) -> Option<(CallContext, String)> {
        if self.is_closed() { return None; }
        let Request::ToolRun { id, call_id, .. } = request else { return None; };
        if id != call_id { return None; }
        let state = self.state.lock().ok()?;
        let session = state.state.db().get("session")?;
        if !session.get("running_tools")?.as_list()?.iter().any(|value| value.as_str() == Some(call_id)) { return None; }
        let operation = session.get("operation")?.as_str()?;
        let principal = state.operations.prompt_owners.get(operation)?.clone();
        Some((CallContext { principal, connection: 0 }, operation.to_owned()))
    }
}
