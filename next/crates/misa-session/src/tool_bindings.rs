//! Explicit model entry points. Final dispatch runs after approval and checkpoint admission.
use crate::{KernelEvent, Request, Runtime, commands::CommandRegistration};
use misa_proto::{
    Fault,
    invocation::{Invocation, Outcome},
    schema::{Literal, Schema},
    tool::Binding,
};
use misa_value::Value;
use std::{collections::BTreeMap, sync::atomic::Ordering};

pub(crate) fn install(
    bindings: &[Binding],
    commands: &BTreeMap<String, CommandRegistration>,
) -> Result<BTreeMap<String, Binding>, Fault> {
    let mut installed = BTreeMap::new();
    for binding in bindings {
        if binding.name.is_empty()
            || binding.name.len() > 64
            || !binding
                .name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"_-".contains(&byte))
            || binding.description.is_empty()
            || crate::catalog::TOOLS
                .iter()
                .any(|tool| tool.name == binding.name)
            || installed.contains_key(&binding.name)
        {
            return Err(Fault::protocol(
                "Duplicate, builtin or empty contributed tool",
            ));
        }
        let command = commands
            .get(&binding.command)
            .ok_or_else(|| Fault::protocol("Tool names uninstalled command"))?;
        json_schema(&command.definition.input)?;
        installed.insert(binding.name.clone(), binding.clone());
    }
    Ok(installed)
}
pub(crate) fn schemas(
    bindings: &BTreeMap<String, Binding>,
    commands: &BTreeMap<String, CommandRegistration>,
) -> Value {
    let mut tools = crate::catalog::tool_schemas().as_list().unwrap().to_vec();
    tools.extend(bindings.values().map(|binding| {
        Value::map([
            ("name", Value::str(&binding.name)),
            ("description", Value::str(&binding.description)),
            (
                "input_schema",
                Value::str(
                    json_schema(&commands[&binding.command].definition.input)
                        .expect("installed tool schema")
                        .to_string(),
                ),
            ),
        ])
    }));
    Value::list(tools)
}
fn json_schema(schema: &Schema) -> Result<serde_json::Value, Fault> {
    use serde_json::{Value as Json, json};
    Ok(match schema {
        Schema::Value => json!({}),
        Schema::Bool => json!({"type":"boolean"}),
        Schema::Int => json!({"type":"integer"}),
        Schema::Number => json!({"type":"number"}),
        Schema::String => json!({"type":"string"}),
        Schema::Bytes => {
            return Err(Fault::protocol(
                "Byte inputs cannot be exposed as JSON model tools",
            ));
        }
        Schema::Nullable { inner } => json!({"anyOf":[json_schema(inner)?,{"type":"null"}]}),
        Schema::List { items } => json!({"type":"array","items":json_schema(items)?}),
        Schema::Record {
            fields,
            allow_unknown,
        } => {
            let properties = fields
                .iter()
                .map(|(id, field)| Ok((id.clone(), json_schema(&field.schema)?)))
                .collect::<Result<serde_json::Map<String, Json>, Fault>>()?;
            json!({"type":"object","properties":properties,"required":fields.iter().filter(|(_,field)|!field.optional).map(|(id,_)|id).collect::<Vec<_>>(),"additionalProperties":allow_unknown})
        }
        Schema::Choice { values } => {
            json!({"enum":values.iter().map(|literal|match literal{Literal::Null=>Json::Null,Literal::Bool(value)=>json!(value),Literal::Int(value)=>json!(value),Literal::String(value)=>json!(value)}).collect::<Vec<_>>()})
        }
    })
}
impl Runtime {
    pub(crate) fn deliver_request(&self, request: Request) -> Result<(), Fault> {
        let Request::ToolRun { name, args, .. } = &request else {
            return self
                .to_kernel
                .send(request)
                .map_err(|_| Fault::new("closed_scope", "Kernel dispatcher closed"));
        };
        let Some(binding) = self.tool_bindings.get(name) else {
            return self
                .to_kernel
                .send(request)
                .map_err(|_| Fault::new("closed_scope", "Kernel dispatcher closed"));
        };
        let Some((context, operation)) = self.trusted_tool_context(&request) else {
            return Err(Fault::new(
                "stale_tool",
                "Tool no longer belongs to active work",
            ));
        };
        let id = self.seq.fetch_add(1, Ordering::Relaxed);
        let invocation = Invocation {
            id,
            scope: self.scope(),
            command: binding.command.clone(),
            input: args.clone(),
        };
        {
            let mut pending = self
                .tool_invocations
                .lock()
                .expect("tool correlations poisoned");
            if pending.len() >= 64 {
                drop(pending);
                self.finish_bound_tool(
                    request,
                    operation,
                    Outcome::Rejected {
                        fault: Fault::new("capacity", "Too many pending tool commands"),
                    },
                );
                return Err(Fault::new("capacity", "Too many pending tool commands"));
            }
            pending.insert(id, (request, operation));
        }
        let outcome = self.execute_command(&context, invocation);
        if !matches!(outcome, Outcome::Accepted { .. }) {
            self.complete_tool_invocation(id, outcome);
        }
        Ok(())
    }
    /// Supervisors resolve accepted command work through its host invocation identity.
    /// A terminal callback from obsolete work cannot advance the current turn.
    pub fn complete_tool_invocation(&self, id: u64, outcome: Outcome) {
        if matches!(outcome, Outcome::Accepted { .. }) {
            return;
        }
        let request = self
            .tool_invocations
            .lock()
            .expect("tool correlations poisoned")
            .remove(&id);
        if let Some((request, operation)) = request
            && self
                .trusted_tool_context(&request)
                .is_some_and(|(_, current)| current == operation)
        {
            let outcome = match (&request, &outcome) {
                (Request::ToolRun { name, .. }, Outcome::Completed { value })
                    if self
                        .tool_bindings
                        .get(name)
                        .and_then(|binding| self.command_registry.get(&binding.command))
                        .is_none_or(|command| {
                            command.definition.result.validate(value).is_err()
                        }) =>
                {
                    Outcome::Indeterminate {
                        fault: Fault::new(
                            "invalid_result",
                            "Supervised command returned a result outside its installed contract",
                        ),
                    }
                }
                _ => outcome,
            };
            self.finish_bound_tool(request, operation, outcome);
        }
    }
    fn finish_bound_tool(&self, request: Request, operation: String, outcome: Outcome) {
        let Request::ToolRun { id, call_id, .. } = request else {
            return;
        };
        let (ok, text) = match outcome {
            Outcome::Completed { value } => match serde_json::to_string(&value) {
                Ok(text) => (true, text),
                Err(error) => (
                    false,
                    format!("Outcome uncertain; result could not be encoded: {error}"),
                ),
            },
            Outcome::Rejected { fault } => (false, format!("{}: {}", fault.code, fault.message)),
            Outcome::Indeterminate { fault } => (
                false,
                format!(
                    "Outcome uncertain; do not retry automatically: {}",
                    fault.message
                ),
            ),
            Outcome::Accepted { .. } => return,
        };
        self.dispatch(
            crate::agent::event_for(KernelEvent::ToolFinished {
                id,
                call_id,
                ok,
                text,
            })
            .with("operation", Value::str(operation)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_protocol::invocation::{CallContext, CommandOwner};
    use std::sync::Arc;
    #[test]
    fn tools_require_unique_installed_commands_and_json_inputs() {
        let binding = Binding {
            name: "test_run".into(),
            description: "Run".into(),
            command: "test.run".into(),
        };
        assert!(install(&[binding.clone()], &BTreeMap::new()).is_err());
        let command = CommandRegistration::new(
            misa_proto::invocation::Command {
                id: binding.command.clone(),
                input: Schema::Bytes,
                result: Schema::String,
            },
            |_, _, _| Outcome::Completed {
                value: Value::str("done"),
            },
        );
        let mut commands = BTreeMap::from([(binding.command.clone(), command)]);
        assert!(install(&[binding.clone()], &commands).is_err());
        commands.get_mut(&binding.command).unwrap().definition.input = Schema::Record {
            fields: BTreeMap::new(),
            allow_unknown: false,
        };
        assert!(install(&[binding.clone()], &commands).is_ok());
        assert!(install(&[binding.clone(), binding], &commands).is_err());
    }
    #[tokio::test]
    async fn accepted_tool_work_waits_for_its_authoritative_terminal_callback() {
        for (result, expected_ok) in [
            (Value::str("supervisor result"), true),
            (Value::Int(7), false),
        ] {
            let (started, mut requests) = tokio::sync::mpsc::unbounded_channel();
            let contribution = crate::Contribution::new()
                .with_command(CommandRegistration::new(
                    misa_proto::invocation::Command {
                        id: "test.defer".into(),
                        input: Schema::Record {
                            fields: BTreeMap::new(),
                            allow_unknown: false,
                        },
                        result: Schema::String,
                    },
                    move |runtime, _, invocation| {
                        started.send(invocation.id).unwrap();
                        Outcome::Accepted {
                            operation: misa_proto::invocation::OperationRef {
                                scope: runtime.scope(),
                                id: "supervised-work".into(),
                            },
                        }
                    },
                ))
                .with_tool(Binding {
                    name: "test_defer".into(),
                    description: "Deferred work".into(),
                    command: "test.defer".into(),
                });
            let kernel = Arc::new(misa_kernel::LocalKernel::new(
                misa_kernel::ScriptedProvider::new([misa_kernel::Turn::call(
                    "test_defer",
                    Value::map([]),
                    misa_kernel::Turn::say("finished"),
                )]),
            ));
            let runtime = Runtime::start_with(
                "tool",
                "Tool",
                None,
                kernel.clone(),
                "scripted",
                "scripted-1",
                Value::Null,
                contribution,
            );
            let context = CallContext {
                principal: "owner".into(),
                connection: 1,
            };
            let Outcome::Accepted { operation } = runtime
                .execute(
                    &context,
                    Invocation {
                        id: 1,
                        scope: runtime.scope(),
                        command: "session.prompt".into(),
                        input: Value::map([
                            ("text", Value::str("delegate")),
                            ("attachments", Value::list([])),
                        ]),
                    },
                )
                .await
            else {
                panic!()
            };
            let invocation =
                tokio::time::timeout(std::time::Duration::from_secs(5), requests.recv())
                    .await
                    .unwrap()
                    .unwrap();
            let terminal = || match runtime
                .read(&misa_proto::Query {
                    id: "operation.result".into(),
                    args: vec![Value::str(&operation.id)],
                })
                .unwrap()
            {
                crate::Reading::Data(value) => value
                    .get("terminal")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                _ => panic!(),
            };
            assert!(
                !terminal(),
                "Accepted tool work must not continue the model"
            );
            assert_eq!(runtime.tool_invocations.lock().unwrap().len(), 1);
            runtime.complete_tool_invocation(invocation, Outcome::Completed { value: result });
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while !terminal() {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(runtime.tool_invocations.lock().unwrap().is_empty());
            assert_eq!(
                kernel
                    .entries()
                    .iter()
                    .filter(|entry| entry.kind == "tool_result")
                    .last()
                    .and_then(|entry| entry.data.get("ok"))
                    .and_then(Value::as_bool),
                Some(expected_ok)
            );
            runtime.complete_tool_invocation(
                invocation,
                Outcome::Completed {
                    value: Value::str("duplicate ignored"),
                },
            );
            runtime.shutdown();
        }
    }
}
