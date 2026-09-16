//! Caller-bound typed input, with a server-installed command continuation.
//! Responses are non-secret values. Credential material uses the separate kernel sink.
use super::*;
use misa_proto::{input::Form, invocation::ActionBinding};
#[derive(Clone, serde::Serialize, serde::Deserialize)]
enum Continuation {
    Command(ActionBinding),
    Transaction {
        command: String,
        event: String,
        input: Value,
    },
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct FormRecord {
    id: String,
    principal: String,
    form: Form,
    continuation: Continuation,
    generation: i64,
    phase: Phase,
    expires_ms: i64,
    child: Option<OperationRef>,
    outcome: Option<Outcome>,
    #[serde(default)]
    pending_outcome: Option<Outcome>,
}
impl FormRecord {
    pub(super) fn deadline(&self) -> Option<i64> {
        (self.phase == Phase::Awaiting).then_some(self.expires_ms)
    }
    pub(super) fn operation(&self) -> Value {
        let mut fields = BTreeMap::from([
            ("id".into(), Value::str(&self.id)),
            ("kind".into(), Value::str("input")),
            ("state".into(), Value::str(self.phase.name())),
            ("terminal".into(), Value::Bool(self.phase.terminal())),
            ("generation".into(), Value::Int(self.generation)),
            ("outputs".into(), Value::list([])),
        ]);
        if let Some(outcome) = &self.outcome {
            fields.insert("outcome".into(), crate::wire::render(outcome));
        }
        Value::Map(Arc::new(fields))
    }
    pub(super) fn summary(&self) -> Value {
        Value::map([
            ("id", Value::str(&self.id)),
            ("operation", Value::str(&self.id)),
            ("kind", Value::str("form")),
            ("state", Value::str(self.phase.name())),
            ("terminal", Value::Bool(self.phase != Phase::Awaiting)),
            ("generation", Value::Int(self.generation)),
            ("expires_ms", Value::Int(self.expires_ms)),
        ])
    }
}
pub(super) fn detail(state: &State, id: &str, context: &CallContext) -> Result<Value, Fault> {
    let record = state
        .operations
        .forms
        .get(id)
        .filter(|record| !context.principal.is_empty() && record.principal == context.principal)
        .ok_or_else(|| Fault::new("request_unavailable", "Input request is unavailable"))?;
    if record.phase != Phase::Awaiting {
        return Ok(Value::Null);
    }
    let bound = BTreeMap::from([
        ("request".into(), Value::str(id)),
        ("generation".into(), Value::Int(record.generation)),
    ]);
    Ok(Value::map([
        ("id", Value::str(id)),
        ("operation", Value::str(id)),
        ("generation", Value::Int(record.generation)),
        ("kind", Value::str("form")),
        ("title", Value::str(&record.form.title)),
        ("input", crate::wire::render(&record.form.input)),
        ("fields", crate::wire::render(&record.form.fields)),
        (
            "resolve",
            crate::wire::render(&ActionBinding {
                command: "input.resolve".into(),
                bound: bound.clone(),
                inputs: BTreeMap::from([("value".into(), "value".into())]),
            }),
        ),
        (
            "cancel",
            crate::wire::render(&ActionBinding {
                command: "input.cancel".into(),
                bound,
                inputs: BTreeMap::new(),
            }),
        ),
    ]))
}
fn owned<'a>(
    store: &'a mut Store,
    context: &CallContext,
    input: &Value,
) -> Result<&'a mut FormRecord, Fault> {
    let record = store
        .forms
        .get_mut(text(input, "request"))
        .filter(|record| !context.principal.is_empty() && record.principal == context.principal)
        .ok_or_else(|| Fault::new("request_unavailable", "Input request is unavailable"))?;
    if record.phase != Phase::Awaiting
        || record.generation != generation(input)
        || record.expires_ms <= crate::now_ms()
    {
        return Err(Fault::new(
            "stale_request",
            "Input request is no longer pending",
        ));
    }
    Ok(record)
}
pub(super) fn resolve(
    runtime: &Runtime,
    context: &CallContext,
    invocation: &Invocation,
) -> Outcome {
    runtime.operation_transition(None, |store, _| {
        let record = owned(store, context, &invocation.input)?;
        let value = invocation
            .input
            .get("value")
            .ok_or_else(|| Fault::new("invalid_input", "Input response needs a value"))?;
        record
            .form
            .input
            .validate(value)
            .map_err(|error| Fault::new("invalid_input", error.to_string()))?;
        let input = match &record.continuation {
            Continuation::Command(binding) => {
                let input = binding.prepare(&BTreeMap::from([("value".into(), value.clone())]))?;
                let definition = &runtime
                    .command_registry
                    .get(&binding.command)
                    .ok_or_else(|| {
                        Fault::new("invalid_command", "Input continuation is unavailable")
                    })?
                    .definition;
                definition
                    .input
                    .validate(&input)
                    .map_err(|error| Fault::new("invalid_input", error.to_string()))?;
                input
            }
            Continuation::Transaction { input, .. } => {
                Value::map([("arguments", input.clone()), ("value", value.clone())])
            }
        };
        record.phase = Phase::Submitting;
        record.generation += 1;
        let id = record.id.clone();
        let effect = Event::new("input/continue")
            .with("operation", Value::str(&id))
            .with("input", input);
        store.events.push(effect);
        Ok((
            Outcome::Accepted {
                operation: OperationRef {
                    scope: runtime.scope(),
                    id,
                },
            },
            None,
        ))
    })
}
pub(super) fn cancel(runtime: &Runtime, context: &CallContext, invocation: &Invocation) -> Outcome {
    runtime.operation_transition(None, |store, _| {
        let record = owned(store, context, &invocation.input)?;
        record.phase = Phase::Cancelling;
        record.generation += 1;
        record.pending_outcome = Some(Outcome::Rejected {
            fault: Fault::new("cancelled", "Input request cancelled"),
        });
        Ok((
            Outcome::Accepted {
                operation: OperationRef {
                    scope: runtime.scope(),
                    id: record.id.clone(),
                },
            },
            None,
        ))
    })
}
pub(super) fn cancel_command() -> CommandRegistration {
    CommandRegistration::new(
        Command {
            preparation: misa_proto::invocation::Preparation::Request, id: "input.cancel".into(),
            input: record([("request", Schema::String), ("generation", Schema::Int)]),
            result: Schema::Choice {
                values: vec![Literal::Null],
            },
        },
        cancel,
    )
}
pub(super) fn registry(registry: Registry) -> Registry {
    registry.on_fn("input/continue", 0, "input.continue", |tx, event| {
        tx.fx(misa_reframe::Effect::new("owner.input.continue").with_value(event.data.clone()));
        Ok(())
    })
}
pub(super) fn expire(store: &mut Store, now: i64) {
    for record in store.forms.values_mut() {
        if record.deadline().is_some_and(|deadline| deadline <= now) {
            record.phase = Phase::Cancelling;
            record.generation += 1;
            record.pending_outcome = Some(Outcome::Rejected {
                fault: Fault::new("expired", "Input request expired"),
            });
        }
    }
}
pub(super) fn interrupt(store: &mut Store) {
    for record in store.forms.values_mut() {
        if !record.phase.terminal() {
            record.phase = Phase::Interrupted;
            record.generation += 1;
            record.pending_outcome = None;
            record.outcome = Some(Outcome::Indeterminate {
                fault: Fault::new(
                    "interrupted",
                    "Input continuation interrupted; reconcile effects before retrying",
                ),
            });
        }
    }
}
fn publish_outcome(record: &mut FormRecord, outcome: Outcome) {
    record.phase = match &outcome {
        Outcome::Completed { .. } => Phase::Succeeded,
        Outcome::Rejected { fault } if fault.code == "cancelled" => Phase::Cancelled,
        Outcome::Rejected { fault } if fault.code == "expired" => Phase::Expired,
        Outcome::Rejected { .. } => Phase::Failed,
        _ => Phase::Interrupted,
    };
    record.pending_outcome = None;
    record.outcome = Some(outcome);
}
pub(super) fn acknowledged(store: &mut Store, data: &Value) -> bool {
    let Some(saved) = data.get("forms").and_then(Value::as_map) else {
        return false;
    };
    let mut changed = false;
    for (id, record) in &mut store.forms {
        let Some(pending) = record.pending_outcome.clone() else {
            continue;
        };
        let Some(written) = saved
            .get(id)
            .and_then(|value| crate::wire::parse::<FormRecord>(value).ok())
        else {
            continue;
        };
        if written.pending_outcome.as_ref() == Some(&pending) {
            publish_outcome(record, pending);
            changed = true;
        }
    }
    changed
}
pub(super) fn restore(store: &mut Store) {
    for record in store.forms.values_mut() {
        if let Some(outcome) = record.pending_outcome.take() {
            publish_outcome(record, outcome);
        }
    }
    interrupt(store);
}
impl Runtime {
    /// The owner installs the continuation; the client supplies only one typed value.
    /// Forms must not contain credentials or other secret fields.
    pub fn request_input(
        &self,
        context: &CallContext,
        form: Form,
        continuation: ActionBinding,
        ttl_ms: i64,
    ) -> Outcome {
        if let Err(fault) = form.validate() {
            return rejected(fault);
        }
        let Some(command) = self.command_registry.get(&continuation.command) else {
            return rejected(Fault::new(
                "invalid_command",
                "Input continuation is not installed",
            ));
        };
        if let Err(fault) = continuation.validate_for(&command.definition) {
            return rejected(fault);
        }
        if continuation
            .inputs
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>()
            != ["value"]
            || !(1..=24 * 60 * 60 * 1000).contains(&ttl_ms)
            || context.principal.is_empty()
        {
            return rejected(Fault::new(
                "invalid_request",
                "Input request needs one value binding, caller and bounded expiry",
            ));
        }
        self.start_input(context, form, Continuation::Command(continuation), ttl_ms)
    }
    pub(crate) fn request_transaction_input(
        &self,
        context: &CallContext,
        invocation: &Invocation,
        form: Form,
        event: &str,
    ) -> Outcome {
        self.start_input(
            context,
            form,
            Continuation::Transaction {
                command: invocation.command.clone(),
                event: event.into(),
                input: invocation.input.clone(),
            },
            KEY_TTL_MS,
        )
    }
    fn start_input(
        &self,
        context: &CallContext,
        form: Form,
        continuation: Continuation,
        ttl_ms: i64,
    ) -> Outcome {
        if let Err(fault) = form.validate() {
            return rejected(fault);
        }
        if context.principal.is_empty() {
            return rejected(Fault::new(
                "request_unavailable",
                "Input request requires an authenticated caller",
            ));
        }
        self.operation_transition(None, |store, _| {
            if store.forms.len() >= MAX_RECORDS {
                if let Some(id) = store
                    .forms
                    .iter()
                    .find(|(_, record)| record.phase.terminal())
                    .map(|(id, _)| id.clone())
                {
                    store.forms.remove(&id);
                } else {
                    return Err(Fault::new("busy", "Input request capacity reached"));
                }
            }
            store.next = store
                .next
                .checked_add(1)
                .ok_or_else(|| Fault::new("capacity", "Operation identifiers exhausted"))?;
            let id = format!("{}:input:{}", self.scope().incarnation, store.next);
            store.forms.insert(
                id.clone(),
                FormRecord {
                    id: id.clone(),
                    principal: context.principal.clone(),
                    form,
                    continuation,
                    generation: 1,
                    phase: Phase::Awaiting,
                    expires_ms: crate::now_ms() + ttl_ms,
                    child: None,
                    outcome: None,
                    pending_outcome: None,
                },
            );
            Ok((
                Outcome::Accepted {
                    operation: OperationRef {
                        scope: self.scope(),
                        id,
                    },
                },
                None,
            ))
        })
    }
    pub(crate) fn continue_input(&self, effect: &misa_reframe::Effect) {
        let id = text(&effect.data, "operation").to_owned();
        let work = {
            let state = self.state.lock().unwrap();
            state
                .operations
                .forms
                .get(&id)
                .filter(|record| record.phase == Phase::Submitting && record.child.is_none())
                .map(|record| {
                    (
                        CallContext {
                            principal: record.principal.clone(),
                            connection: 0,
                        },
                        record.continuation.clone(),
                    )
                })
        };
        let Some((context, continuation)) = work else {
            return;
        };
        let input = effect.get("input").cloned().unwrap_or(Value::Null);
        let invocation = |command| Invocation {
            id: self.next_call.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            scope: self.scope(),
            command,
            input: input.clone(),
        };
        let outcome = match continuation {
            Continuation::Command(binding) => {
                self.execute_command(&context, invocation(binding.command))
            }
            Continuation::Transaction { command, event, .. } => {
                self.execute_event_transaction(&context, &invocation(command), &event)
            }
        };
        self.input_outcome(&id, outcome);
    }
    fn input_outcome(&self, id: &str, outcome: Outcome) {
        self.operation_transition(None,|store,_|{
            let record=store.forms.get_mut(id).filter(|record|record.phase==Phase::Submitting).ok_or_else(||Fault::new("stale_request","Input operation already settled"))?;
            match &outcome {
                Outcome::Accepted{operation} if operation.scope==self.scope()=>{record.child=Some(operation.clone());},
                Outcome::Accepted{operation}=>{record.pending_outcome=Some(Outcome::Indeterminate{fault:Fault::new("foreign_operation","Input continuation returned a foreign operation; reconcile it directly").with(Value::map([("operation",crate::wire::render(operation))]))});},
                _=>{record.pending_outcome=Some(outcome);},
            }
            Ok((Outcome::Completed{value:Value::Null},None))
        });
        self.settle_input_continuations();
    }
    pub(crate) fn settle_input_continuations(&self) {
        let completed = {
            let mut state = self.state.lock().unwrap();
            let pending = state
                .operations
                .forms
                .values()
                .filter(|record| {
                    record.phase == Phase::Submitting && record.pending_outcome.is_none()
                })
                .filter_map(|record| {
                    record
                        .child
                        .as_ref()
                        .map(|child| (record.id.clone(), child.id.clone()))
                })
                .collect::<Vec<_>>();
            pending
                .into_iter()
                .filter_map(|(id, child)| {
                    let result = state
                        .state
                        .query(&Query::new("operation.result").arg(Value::str(child)))
                        .ok()?;
                    let outcome = outcome_of_result(&result)?;
                    Some((id, outcome))
                })
                .collect::<Vec<_>>()
        };
        for (id, outcome) in completed {
            self.input_outcome(&id, outcome);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context(name: &str) -> CallContext {
        CallContext {
            principal: name.into(),
            connection: 1,
        }
    }
    fn form() -> Form {
        Form {
            title: "Name the pet".into(),
            input: record([("name", Schema::String)]),
            fields: BTreeMap::from([(
                "name".into(),
                misa_proto::input::Field {
                    label: "Pet name".into(),
                },
            )]),
        }
    }
    fn setup() -> Arc<Runtime> {
        let contribution = crate::Contribution::default()
            .with_root("pet", Value::str("unknown"))
            .unwrap()
            .with_handler(
                "pet/name",
                0,
                Arc::new(misa_reframe::FnHandler::new(
                    "pet.name",
                    |tx: &mut Tx<'_>, event: &Event| {
                        tx.set(
                            "pet",
                            event
                                .get("input")
                                .and_then(|input| input.get("value"))
                                .and_then(|value| value.get("name"))
                                .cloned()
                                .unwrap_or(Value::Null),
                        )
                    },
                )),
            )
            .with_command(CommandRegistration::event(
                "pet.name",
                record([("value", Schema::Value)]),
                "pet/name",
            ));
        Runtime::start_with(
            "forms",
            "Forms",
            None,
            Arc::new(misa_kernel::LocalKernel::new(
                misa_kernel::ScriptedProvider::always("unused"),
            )),
            "scripted",
            "test",
            Value::Null,
            contribution,
        )
    }
    fn begin(runtime: &Runtime) -> String {
        let Outcome::Accepted { operation } = runtime.request_input(
            &context("alice"),
            form(),
            ActionBinding {
                command: "pet.name".into(),
                bound: BTreeMap::new(),
                inputs: BTreeMap::from([("value".into(), "value".into())]),
            },
            60_000,
        ) else {
            panic!()
        };
        operation.id
    }
    fn invocation(runtime: &Runtime, id: &str, value: Value) -> Invocation {
        Invocation {
            id: 99,
            scope: runtime.scope(),
            command: "input.resolve".into(),
            input: Value::map([
                ("request", Value::str(id)),
                ("generation", Value::Int(1)),
                ("value", value),
            ]),
        }
    }
    #[tokio::test]
    async fn private_schema_form_resolves_once_and_waits_for_durable_continuation() {
        let runtime = setup();
        let id = begin(&runtime);
        let query = Query::new(REQUEST).arg(Value::str(&id));
        assert!(request(&runtime.state.lock().unwrap(), &query, &context("bob")).is_err());
        let detail = request(&runtime.state.lock().unwrap(), &query, &context("alice")).unwrap();
        assert_eq!(detail.get("kind").and_then(Value::as_str), Some("form"));
        let invalid = invocation(&runtime, &id, Value::map([("name", Value::Int(7))]));
        assert!(matches!(
            runtime.execute_command(&context("alice"), invalid),
            Outcome::Rejected { .. }
        ));
        let valid = invocation(&runtime, &id, Value::map([("name", Value::str("Miso"))]));
        assert!(matches!(
            runtime.execute_command(&context("bob"), valid.clone()),
            Outcome::Rejected { .. }
        ));
        assert!(matches!(
            runtime.execute_command(&context("alice"), valid.clone()),
            Outcome::Accepted { .. }
        ));
        assert!(matches!(
            runtime.execute_command(&context("alice"), valid),
            Outcome::Rejected { .. }
        ));
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let state = runtime.state.lock().unwrap();
                let result = state.operations.forms[&id].operation();
                drop(state);
                if result.get("terminal") == Some(&Value::Bool(true)) {
                    assert_eq!(
                        result.get("state").and_then(Value::as_str),
                        Some("succeeded")
                    );
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            runtime
                .state
                .lock()
                .unwrap()
                .state
                .db()
                .get("pet")
                .and_then(Value::as_str),
            Some("Miso")
        );
        assert_eq!(
            request(&runtime.state.lock().unwrap(), &query, &context("alice")).unwrap(),
            Value::Null
        );
        runtime.shutdown_complete().await;
    }
    #[tokio::test]
    async fn cancellation_expiry_and_recovery_do_not_run_a_continuation() {
        for expire in [true, false] {
            let runtime = setup();
            let id = begin(&runtime);
            if expire {
                runtime.expire_operations(crate::now_ms() + 61_000);
            } else {
                let cancel = Invocation {
                    id: 2,
                    scope: runtime.scope(),
                    command: "input.cancel".into(),
                    input: Value::map([
                        ("request", Value::str(&id)),
                        ("generation", Value::Int(1)),
                    ]),
                };
                assert!(matches!(
                    runtime.execute_command(&context("alice"), cancel),
                    Outcome::Accepted { .. }
                ));
            }
            assert_eq!(
                runtime
                    .state
                    .lock()
                    .unwrap()
                    .state
                    .db()
                    .get("pet")
                    .and_then(Value::as_str),
                Some("unknown")
            );
            assert!(matches!(
                runtime.execute_command(
                    &context("alice"),
                    invocation(&runtime, &id, Value::map([("name", Value::str("late"))]))
                ),
                Outcome::Rejected { .. }
            ));
            runtime.shutdown_complete().await;
        }
        let runtime = setup();
        let id = begin(&runtime);
        let mut recovered = runtime.state.lock().unwrap().operations.clone();
        restore(&mut recovered);
        assert_eq!(
            recovered.forms[&id]
                .operation()
                .get("state")
                .and_then(Value::as_str),
            Some("interrupted")
        );
        runtime.shutdown_complete().await;
    }
}

#[cfg(test)]
mod persistence_tests {
    use super::*;
    fn new_store() -> Store {
        let mut store = Store::default();
        store.forms.insert(
            "form".into(),
            FormRecord {
                id: "form".into(),
                principal: "alice".into(),
                form: Form {
                    title: "A form".into(),
                    input: record([("choice", Schema::String)]),
                    fields: BTreeMap::new(),
                },
                continuation: Continuation::Command(ActionBinding {
                    command: "test.run".into(),
                    bound: BTreeMap::new(),
                    inputs: BTreeMap::from([("value".into(), "value".into())]),
                }),
                generation: 2,
                phase: Phase::Submitting,
                expires_ms: i64::MAX,
                child: None,
                outcome: None,
                pending_outcome: Some(Outcome::Completed {
                    value: Value::str("typed answer"),
                }),
            },
        );
        store
    }
    #[test]
    fn terminal_results_require_exact_checkpoint_and_recover_without_replay() {
        let mut store = new_store();
        assert!(!store.forms["form"].phase.terminal());
        assert!(!acknowledged(&mut store, &Value::map([])));
        let checkpoint = Value::map([("forms", crate::wire::render(&store.forms))]);
        let mut recovered = store.clone();
        restore(&mut recovered);
        assert_eq!(
            recovered.forms["form"]
                .operation()
                .get("state")
                .and_then(Value::as_str),
            Some("succeeded")
        );
        assert!(acknowledged(&mut store, &checkpoint));
        assert!(!acknowledged(&mut store, &checkpoint));
        let outcome = outcome_of_result(&store.forms["form"].operation()).unwrap();
        assert_eq!(
            outcome,
            Outcome::Completed {
                value: Value::str("typed answer")
            }
        );
        let mut failed = new_store();
        interrupt(&mut failed);
        assert_eq!(
            failed.forms["form"]
                .operation()
                .get("state")
                .and_then(Value::as_str),
            Some("interrupted")
        );
    }
}
