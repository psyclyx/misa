//! Owner-side tool admission. Renderers consume ordinary input request contracts.
use super::*;
use misa_reframe::Effect;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolApprovalPolicy {
    Allow,
    Ask,
    Deny,
}
impl ToolApprovalPolicy {
    pub(crate) fn from_config(config: &Value) -> Self {
        match config.get("tool_approval").and_then(Value::as_str) {
            None | Some("allow") => Self::Allow,
            Some("ask") => Self::Ask,
            // A misspelled security policy cannot silently permit execution.
            _ => Self::Deny,
        }
    }
}
#[derive(Clone)]
pub(super) struct Approval {
    id: String,
    operation: String,
    principal: String,
    generation: i64,
    state: &'static str,
    expires_ms: i64,
    request: Request,
}
impl Approval {
    pub(super) fn deadline(&self) -> Option<i64> {
        (self.state == "awaiting_input").then_some(self.expires_ms)
    }
    fn summary(&self) -> Value {
        Value::map([
            ("id", Value::str(&self.id)),
            ("operation", Value::str(&self.operation)),
            ("kind", Value::str("tool_approval")),
            ("state", Value::str(self.state)),
            ("terminal", Value::Bool(self.state != "awaiting_input")),
            (
                "call",
                Value::str(match &self.request {
                    Request::ToolRun { call_id, .. } => call_id,
                    _ => unreachable!(),
                }),
            ),
            ("generation", Value::Int(self.generation)),
            ("expires_ms", Value::Int(self.expires_ms)),
        ])
    }
    fn refuse(&self, reason: &str) -> Event {
        let Request::ToolRun { call_id, .. } = &self.request else {
            unreachable!()
        };
        Event::new("kernel/tool.finished")
            .with("call_id", Value::str(call_id))
            .with("ok", Value::Bool(false))
            .with("text", Value::str(reason))
    }
}
pub(super) fn summaries(store: &Store) -> Value {
    let mut values = store
        .approvals
        .values()
        .map(Approval::summary)
        .collect::<Vec<_>>();
    values.extend(
        store
            .records
            .values()
            .filter(|record| record.phase == Phase::Awaiting)
            .map(|record| {
                Value::map([
                    ("id", Value::str(&record.id)),
                    ("operation", Value::str(&record.id)),
                    (
                        "kind",
                        Value::str(if record.oauth {
                            "device_authorization"
                        } else {
                            "credential_value"
                        }),
                    ),
                    ("state", Value::str(record.phase.name())),
                    ("generation", Value::Int(record.generation)),
                    (
                        "expires_ms",
                        record.expires_ms.map(Value::Int).unwrap_or(Value::Null),
                    ),
                ])
            }),
    );
    Value::list(values)
}
pub(super) fn detail(state: &State, id: &str, context: &CallContext) -> Result<Value, Fault> {
    let record = state
        .operations
        .approvals
        .get(id)
        .filter(|record| !context.principal.is_empty() && record.principal == context.principal)
        .ok_or_else(|| Fault::new("request_unavailable", "Input request is unavailable"))?;
    if record.state != "awaiting_input" {
        return Ok(Value::Null);
    }
    let Request::ToolRun { name, args, .. } = &record.request else {
        unreachable!()
    };
    Ok(Value::map([
        ("id", Value::str(id)),
        ("operation", Value::str(&record.operation)),
        ("kind", Value::str("tool_approval")),
        ("generation", Value::Int(record.generation)),
        ("expires_ms", Value::Int(record.expires_ms)),
        (
            "tool",
            Value::map([("name", Value::str(name)), ("arguments", args.clone())]),
        ),
    ]))
}
pub(super) fn command() -> CommandRegistration {
    CommandRegistration::new(
        Command {
            preparation: misa_proto::invocation::Preparation::Request,
            id: "input.resolve".into(),
            input: {
                let mut schema = record([("request", Schema::String), ("generation", Schema::Int)]);
                if let Schema::Record { fields, .. } = &mut schema {
                    fields.insert(
                        "approved".into(),
                        Field {
                            schema: Schema::Bool,
                            optional: true,
                        },
                    );
                    fields.insert(
                        "value".into(),
                        Field {
                            schema: Schema::Value,
                            optional: true,
                        },
                    );
                }
                schema
            },
            result: Schema::Choice {
                values: vec![Literal::Null],
            },
        },
        resolve,
    )
}
fn resolve(runtime: &Runtime, context: &CallContext, invocation: &Invocation) -> Outcome {
    if runtime
        .state
        .lock()
        .unwrap()
        .operations
        .forms
        .contains_key(text(&invocation.input, "request"))
    {
        return forms::resolve(runtime, context, invocation);
    }
    if invocation
        .input
        .get("approved")
        .and_then(Value::as_bool)
        .is_none()
        || invocation.input.get("value").is_some()
    {
        return rejected(Fault::new(
            "invalid_input",
            "Tool approval requires one approved boolean",
        ));
    }
    runtime.operation_transition(None, |store, db| {
        let id = text(&invocation.input, "request");
        let record = store
            .approvals
            .get_mut(id)
            .filter(|record| record.principal == context.principal && !context.principal.is_empty())
            .ok_or_else(|| Fault::new("request_unavailable", "Input request is unavailable"))?;
        let active = db
            .get("session")
            .and_then(|session| session.get("operation"))
            .and_then(Value::as_str)
            == Some(&record.operation);
        if record.generation != generation(&invocation.input)
            || record.state != "awaiting_input"
            || !active
            || record.expires_ms <= crate::now_ms()
        {
            return Err(Fault::new(
                "stale_request",
                "Input request is no longer pending",
            ));
        }
        let approved = invocation.input.get("approved").and_then(Value::as_bool) == Some(true);
        record.generation += 1;
        record.state = if approved { "approved" } else { "denied" };
        let request = if approved {
            Some(record.request.clone())
        } else {
            store.events.push(record.refuse("Tool execution denied"));
            None
        };
        Ok((Outcome::Completed { value: Value::Null }, request))
    })
}
pub(super) fn cancel_for(store: &mut Store, operation: &str) {
    for record in store
        .approvals
        .values_mut()
        .filter(|record| record.operation == operation && record.state == "awaiting_input")
    {
        record.state = "cancelled";
        record.generation += 1;
        store.events.push(record.refuse("Tool approval cancelled"));
    }
}
pub(super) fn expire(store: &mut Store, now: i64) {
    for record in store
        .approvals
        .values_mut()
        .filter(|record| record.state == "awaiting_input" && record.expires_ms <= now)
    {
        record.state = "expired";
        record.generation += 1;
        store.events.push(record.refuse("Tool approval expired"));
    }
}
impl Runtime {
    pub(crate) fn approve_tool(&self, effect: &Effect, admission: &crate::kernel_queue::Admission) {
        let request = Request::ToolRun {
            id: crate::fields::text(effect, "id"),
            call_id: crate::fields::text(effect, "call_id"),
            name: crate::fields::text(effect, "name"),
            args: crate::fields::value(effect, "args"),
        };
        if self.tool_approval == ToolApprovalPolicy::Allow {
            let _ = self.deliver_request(request, admission);
            return;
        }
        let operation = crate::fields::text(effect, "operation");
        let outcome = self.operation_transition(None, |store, db| {
            let principal = store.prompt_owners.get(&operation).cloned();
            let active = db
                .get("session")
                .and_then(|session| session.get("operation"))
                .and_then(Value::as_str)
                == Some(&operation);
            let Request::ToolRun { call_id, .. } = &request else {
                unreachable!()
            };
            if self.tool_approval == ToolApprovalPolicy::Deny || principal.is_none() || !active {
                store.events.push(
                    Event::new("kernel/tool.finished")
                        .with("call_id", Value::str(call_id))
                        .with("ok", Value::Bool(false))
                        .with("text", Value::str("Tool execution denied by owner policy")),
                );
                return Ok((Outcome::Completed { value: Value::Null }, None));
            }
            if store.approvals.len() >= MAX_RECORDS {
                if let Some(id) = store
                    .approvals
                    .iter()
                    .find(|(_, record)| record.state != "awaiting_input")
                    .map(|(id, _)| id.clone())
                {
                    store.approvals.remove(&id);
                } else {
                    store.events.push(
                        Event::new("kernel/tool.finished")
                            .with("call_id", Value::str(call_id))
                            .with("ok", Value::Bool(false))
                            .with("text", Value::str("Too many pending tool approvals")),
                    );
                    return Ok((Outcome::Completed { value: Value::Null }, None));
                }
            }
            store.next += 1;
            let id = format!("input:{}", store.next);
            store.approvals.insert(
                id.clone(),
                Approval {
                    id,
                    operation,
                    principal: principal.unwrap(),
                    generation: 1,
                    state: "awaiting_input",
                    expires_ms: crate::now_ms().saturating_add(KEY_TTL_MS),
                    request,
                },
            );
            Ok((Outcome::Completed { value: Value::Null }, None))
        });
        if let Outcome::Rejected { fault } | Outcome::Indeterminate { fault } = outcome {
            self.notice(crate::Level::Error, fault.message);
        }
    }
}

/// Public request transitions are committed by the session transaction; private
/// responder capabilities follow that same committed state before publication.
pub(super) fn reconcile(store: &mut Store, db: &Value) {
    for summary in db
        .get("input_requests")
        .and_then(Value::as_list)
        .unwrap_or(&[])
    {
        let Some(record) = summary
            .get("id")
            .and_then(Value::as_str)
            .and_then(|id| store.approvals.get_mut(id))
        else {
            continue;
        };
        if record.state == "awaiting_input"
            && summary.get("state").and_then(Value::as_str) == Some("cancelled")
        {
            record.state = "cancelled";
            record.generation = summary
                .get("generation")
                .and_then(Value::as_i64)
                .unwrap_or(record.generation + 1);
        }
    }
}

/// Never journal tool arguments or credential values in operation metadata.
pub(super) fn checkpoint(store: &Store) -> Value {
    Value::list(store.approvals.values().map(|record| {
        Value::map([
            ("id", Value::str(&record.id)),
            ("operation", Value::str(&record.operation)),
            ("principal", Value::str(&record.principal)),
            ("generation", Value::Int(record.generation)),
            ("state", Value::str(record.state)),
        ])
    }))
}
pub(super) fn restore(store: &mut Store, records: &[Value]) {
    for value in records.iter().take(MAX_RECORDS) {
        let id = text(value, "id").to_owned();
        if id.is_empty() {
            continue;
        }
        let state = match text(value, "state") {
            "denied" => "denied",
            "cancelled" => "cancelled",
            "expired" => "expired",
            // Approved means submitted, not that the external effect completed.
            _ => "interrupted",
        };
        store.approvals.insert(
            id.clone(),
            Approval {
                id,
                operation: text(value, "operation").into(),
                principal: text(value, "principal").into(),
                generation: value.get("generation").and_then(Value::as_i64).unwrap_or(1) + 1,
                state,
                expires_ms: 0,
                request: Request::ToolRun {
                    id: String::new(),
                    call_id: String::new(),
                    name: String::new(),
                    args: Value::Null,
                },
            },
        );
    }
}

pub(super) fn interrupt(store: &mut Store) {
    for record in store
        .approvals
        .values_mut()
        .filter(|record| matches!(record.state, "awaiting_input" | "approved"))
    {
        record.state = "interrupted";
        record.generation += 1;
    }
}
