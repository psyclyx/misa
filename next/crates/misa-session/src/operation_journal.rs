//! Sanitized owner checkpoints. Recovery reconciles facts and never replays work.
use super::*;
const KIND: &str = "operations.checkpoint";
fn phase(value: &str) -> Phase {
    match value {
        "succeeded" => Phase::Succeeded,
        "failed" => Phase::Failed,
        "cancelled" => Phase::Cancelled,
        "expired" => Phase::Expired,
        _ => Phase::Interrupted,
    }
}
impl Store {
    fn checkpoint(&self, db: &Value) -> Value {
        let credentials = Value::list(self.records.values().map(|record| {
            Value::map([
                ("id", Value::str(&record.id)),
                ("principal", Value::str(&record.principal)),
                ("provider", Value::str(&record.provider)),
                ("oauth", Value::Bool(record.oauth)),
                ("state", Value::str(record.phase.name())),
                ("generation", Value::Int(record.generation)),
            ])
        }));
        Value::map([
            ("version", Value::Int(1)),
            ("next", Value::Int(self.next as i64)),
            ("credentials", credentials),
            (
                "owners",
                Value::Map(Arc::new(
                    self.prompt_owners
                        .iter()
                        .map(|(id, principal)| (id.clone(), Value::str(principal)))
                        .collect(),
                )),
            ),
            (
                "prompts",
                db.get("prompt_operations")
                    .cloned()
                    .unwrap_or_else(|| Value::list([])),
            ),
            ("requests", approvals::checkpoint(self)),
            ("commands", db.get(crate::command_operations::ROOT).cloned().unwrap_or_else(|| Value::list([]))),
        ])
    }
}

#[derive(Default)]
pub(crate) struct DeferredWork {
    next: u64,
    pending: BTreeMap<String, Deferred>,
}
struct Deferred {
    effects: Vec<misa_reframe::Effect>,
    request: Option<Request>,
    operation: Option<String>,
}
impl DeferredWork {
    pub(crate) fn len(&self) -> usize {
        self.pending.len()
    }
    pub(crate) fn clear(&mut self) {
        self.pending.clear();
    }
    #[cfg(test)]
    pub(super) fn tokens(&self) -> Vec<String> {
        self.pending.keys().cloned().collect()
    }
}
impl Runtime {
    /// Side effects wait for the precise checkpoint acknowledgment. Staged input
    /// credentials remain transient Rust values and never enter a graph or log.
    pub(crate) fn queue_operation_checkpoint(
        &self,
        state: &mut State,
        effects: &mut Vec<misa_reframe::Effect>,
        request: &mut Option<Request>,
    ) {
        let checkpoint = state.operations.checkpoint(state.state.db());
        if state.operations.last_checkpoint.as_ref() == Some(&checkpoint) {
            return;
        }
        if state.operations.last_checkpoint.is_none()
            && state.operations.records.is_empty()
            && state.operations.prompt_owners.is_empty()
            && state.operations.approvals.is_empty()
            && state.state.db().get(crate::command_operations::ROOT).and_then(Value::as_list).is_none_or(|records| records.is_empty())
        {
            return;
        }
        state.deferred.next = state
            .deferred
            .next
            .checked_add(1)
            .expect("checkpoint sequence exhausted");
        let token = format!("{}:{}", state.view.version.epoch, state.deferred.next);
        let conversation = state
            .state
            .db()
            .get("session")
            .and_then(|session| session.get("conversation"))
            .and_then(Value::as_str)
            .unwrap_or(self.id())
            .to_owned();
        let operation = state
            .state
            .db()
            .get("session")
            .and_then(|session| session.get("operation"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut fields = checkpoint.as_map().unwrap().clone();
        fields.insert("checkpoint".into(), Value::str(&token));
        state.deferred.pending.insert(
            token.clone(),
            Deferred {
                effects: std::mem::take(effects),
                request: request.take(),
                operation,
            },
        );
        state.operations.last_checkpoint = Some(checkpoint);
        // A failed local delivery is handled just like a negative disk acknowledgment.
        if self
            .to_kernel
            .send(Request::Append {
                conversation,
                kind: KIND.into(),
                data: Value::Map(Arc::new(fields)),
            })
            .is_err()
        {
            self.fail_operation_checkpoints(state);
        }
    }
    pub(crate) fn fail_operation_checkpoints(&self, state: &mut State) {
        state.deferred.pending.clear();
        for record in state
            .operations
            .records
            .values_mut()
            .filter(|record| !record.phase.terminal())
        {
            record.phase = Phase::Interrupted;
            record.generation += 1;
            record.challenge = None;
            record.expires_ms = None;
        }
        approvals::interrupt(&mut state.operations);
        let event = Event::new("operations/persistence.failed")
            .with("summary", state.operations.summary())
            .with("requests", approvals::summaries(&state.operations));
        self.dispatch_locked(state, event);
        self.rev.send_replace(state.state.rev());
    }
    pub(crate) fn operation_checkpoint_event(&self, event: &Event) -> Option<Vec<Fault>> {
        if !matches!(
            event.kind.as_str(),
            "kernel/log.appended" | "kernel/log.failed"
        ) || event.get("kind").and_then(Value::as_str) != Some(KIND)
        {
            return None;
        }
        let token = event
            .get("data")
            .and_then(|data| data.get("checkpoint"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        let deferred = {
            let mut state = self.state.lock().expect("session state is never poisoned");
            let Some(mut deferred) = state.deferred.pending.remove(token) else {
                return Some(vec![]);
            };
            if event.kind == "kernel/log.failed" {
                self.fail_operation_checkpoints(&mut state);
                return Some(vec![Fault::new(
                    "persistence_failed",
                    "Operation checkpoint failed; work was not replayed",
                )]);
            }
            let active = state
                .state
                .db()
                .get("session")
                .and_then(|session| session.get("operation"))
                .and_then(Value::as_str);
            if deferred
                .operation
                .as_deref()
                .is_some_and(|operation| active != Some(operation))
            {
                deferred.effects.retain(|effect| {
                    !matches!(
                        effect.kind.as_str(),
                        "kernel.provider.call" | "kernel.tool.run"
                    )
                });
                if matches!(deferred.request, Some(Request::ToolRun { .. })) {
                    deferred.request = None;
                }
            }
            let pending = state.state.db().get("session").and_then(|session| session.get("pending"))
                .and_then(|pending| pending.get("request")).and_then(Value::as_str);
            deferred.effects.retain(|effect| !matches!(effect.kind.as_str(), "kernel.provider.call" | "kernel.attempt.started")
                || effect.data.get("id").and_then(Value::as_str) == pending);
            let interrupted = state.state.db().get("session").and_then(|session| session.get("queue"))
                .and_then(Value::as_list).and_then(|queue| queue.first()).and_then(|entry| entry.get("interrupt"))
                .and_then(Value::as_bool) == Some(true);
            if interrupted && matches!(deferred.request, Some(Request::ToolRun { .. })) {
                if let Some(Request::ToolRun { call_id, .. }) = deferred.request.take() {
                    let conversation = state.state.db().get("session").and_then(|session| session.get("conversation")).cloned().unwrap_or(Value::Null);
                    deferred.effects.push(misa_reframe::Effect::new("kernel.log.append")
                        .with("conversation", conversation).with("kind", Value::str("tool_result"))
                        .with("data", Value::map([("call", Value::str(call_id)), ("ok", Value::Bool(false)), ("text", Value::str("Tool cancelled before execution"))])));
                }
            }
            if let Some(Request::Credential { id, action }) = &deferred.request {
                let original = match action {
                    CredentialAction::CancelOAuth { request } => request.as_str(),
                    _ => id.as_str(),
                };
                let phase = state
                    .operations
                    .records
                    .get(original)
                    .map(|record| record.phase);
                let permitted = match action {
                    CredentialAction::CancelOAuth { .. } => phase == Some(Phase::Cancelling),
                    CredentialAction::Set { .. } => phase == Some(Phase::Submitting),
                    _ => matches!(phase, Some(Phase::Running | Phase::Awaiting)),
                };
                if !permitted {
                    deferred.request = None;
                }
            }
            deferred
        };
        self.perform(&misa_reframe::Outcome {
            effects: deferred.effects,
            ..Default::default()
        });
        if let Some(request) = deferred.request {
            let _ = self.deliver_request(request);
        }
        Some(vec![])
    }
}
pub(super) fn restore(event: Event) -> (Event, Option<Store>) {
    if event.kind != "kernel/log.loaded" {
        return (event, None);
    }
    let entries = event.get("entries").and_then(Value::as_list).unwrap_or(&[]);
    let checkpoint = entries
        .iter()
        .rev()
        .find(|entry| entry.get("kind").and_then(Value::as_str) == Some(KIND))
        .and_then(|entry| entry.get("data"))
        .filter(|data| data.get("version").and_then(Value::as_i64) == Some(1));
    let mut store = Store::default();
    let mut prompts = Vec::new();
    if let Some(checkpoint) = checkpoint {
        store.next = checkpoint
            .get("next")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .max(0) as u64;
        for value in checkpoint
            .get("credentials")
            .and_then(Value::as_list)
            .unwrap_or(&[])
            .iter()
            .take(MAX_RECORDS)
        {
            let id = text(value, "id").to_owned();
            if id.is_empty() {
                continue;
            }
            let recovered = phase(text(value, "state"));
            store.records.insert(
                id.clone(),
                Record {
                    id,
                    principal: text(value, "principal").into(),
                    provider: text(value, "provider").into(),
                    oauth: value.get("oauth").and_then(Value::as_bool).unwrap_or(false),
                    generation: value.get("generation").and_then(Value::as_i64).unwrap_or(1)
                        + i64::from(recovered == Phase::Interrupted),
                    phase: recovered,
                    expires_ms: None,
                    challenge: None,
                },
            );
        }
        if let Some(owners) = checkpoint.get("owners").and_then(Value::as_map) {
            store
                .prompt_owners
                .extend(
                    owners
                        .iter()
                        .take(MAX_RECORDS)
                        .filter_map(|(id, principal)| {
                            principal
                                .as_str()
                                .map(|principal| (id.clone(), principal.into()))
                        }),
                );
        }
        prompts.extend(
            checkpoint
                .get("prompts")
                .and_then(Value::as_list)
                .unwrap_or(&[])
                .iter()
                .take(MAX_RECORDS)
                .cloned(),
        );
        approvals::restore(
            &mut store,
            checkpoint
                .get("requests")
                .and_then(Value::as_list)
                .unwrap_or(&[]),
        );
    }
    // A persisted final assistant fact wins even if its trailing checkpoint did
    // not reach storage. Sequence IDs, not a client idle state, identify output.
    for entry in entries {
        if entry.get("kind").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(message) = entry.get("data") else {
            continue;
        };
        let Some(id) = message.get("operation").and_then(Value::as_str) else {
            continue;
        };
        let index = match prompts
            .iter()
            .position(|record| record.get("id").and_then(Value::as_str) == Some(id))
        {
            Some(index) => index,
            None => {
                if prompts.len() >= MAX_RECORDS {
                    prompts.remove(0);
                }
                prompts.push(Value::map([
                    ("id", Value::str(id)),
                    ("kind", Value::str("session.prompt")),
                    ("state", Value::str("running")),
                    ("generation", Value::Int(1)),
                    ("terminal", Value::Bool(false)),
                    ("outputs", Value::list([])),
                ]));
                prompts.len() - 1
            }
        };
        if message.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Value::Map(fields) = &mut prompts[index] else {
            continue;
        };
        let fields = Arc::make_mut(fields);
        let mut outputs = fields
            .get("outputs")
            .and_then(Value::as_list)
            .unwrap_or(&[])
            .to_vec();
        if let Some(seq) = message.get("seq").and_then(Value::as_i64) {
            if !outputs.contains(&Value::Int(seq)) {
                outputs.push(Value::Int(seq));
            }
        }
        fields.insert("outputs".into(), Value::list(outputs));
        if message
            .get("calls")
            .and_then(Value::as_list)
            .is_some_and(|calls| !calls.is_empty())
        {
            continue;
        }
        let state = match message.get("state").and_then(Value::as_str) {
            Some("cancelled") => "cancelled",
            Some("failed") => "failed",
            _ => "succeeded",
        };
        fields.insert("state".into(), Value::str(state));
        fields.insert("terminal".into(), Value::Bool(true));
    }
    for prompt in &mut prompts {
        if prompt.get("terminal").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        if let Value::Map(fields) = prompt {
            let fields = Arc::make_mut(fields);
            let generation = fields
                .get("generation")
                .and_then(Value::as_i64)
                .unwrap_or(1)
                + 1;
            fields.insert("generation".into(), Value::Int(generation));
            fields.insert("state".into(), Value::str("interrupted"));
            fields.insert("terminal".into(), Value::Bool(true));
        }
    }
    let restored = Value::map([
        ("prompts", Value::list(prompts)),
        ("operations", store.summary()),
        ("requests", approvals::summaries(&store)),
    ]);
    (event.with("restored_operations", restored), Some(store))
}
