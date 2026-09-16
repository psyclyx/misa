//! Durable event transactions use the ordinary operation queries. The journal
//! decision contains both the patches and their terminal command outcome.
use crate::Runtime;
use misa_proto::{
    Fault,
    invocation::{Invocation, OperationRef, Outcome},
};
use misa_protocol::invocation::CallContext;
use misa_reframe::{Event, Registry, Tx};
use misa_value::{Op, Path, Value};
use std::sync::Arc;

pub(crate) const ROOT: &str = "command_operations";
const MAX: usize = 64;
fn records(db: &Value) -> &[Value] {
    db.get(ROOT).and_then(Value::as_list).unwrap_or(&[])
}
fn terminal(record: &Value, state: &str) -> Value {
    let mut fields = record.as_map().unwrap().clone();
    fields.insert("state".into(), Value::str(state));
    fields.insert("terminal".into(), Value::Bool(true));
    Value::Map(Arc::new(fields))
}
pub(crate) fn registry(registry: Registry) -> Registry {
    registry.on_fn(
        "command/transaction",
        0,
        "command.transaction",
        |tx, event| {
            let record = event
                .get("record")
                .cloned()
                .ok_or_else(|| misa_reframe::Fault::new("command", "Missing operation"))?;
            let mut records = records(tx.db()).to_vec();
            if records.len() >= MAX {
                if let Some(index) = records.iter().position(|record| {
                    record.get("terminal").and_then(Value::as_bool) == Some(true)
                }) {
                    records.remove(index);
                } else {
                    return Err(misa_reframe::Fault::new(
                        "busy",
                        "Command operation capacity reached",
                    ));
                }
            }
            records.push(record.clone());
            tx.set(ROOT, Value::list(records))?;
            tx.set("session.command_transaction", record)?;
            tx.dispatch(
                Event::new(
                    event
                        .get("event")
                        .and_then(Value::as_str)
                        .unwrap_or_default(),
                )
                .with_value(event.get("data").cloned().unwrap_or(Value::Null)),
            );
            Ok(())
        },
    )
}
/// Finalizer hook; only the host wrapper can install the protected marker.
pub(crate) fn staged(candidate: &Value) -> Option<Value> {
    candidate
        .get("session")?
        .get("command_transaction")
        .filter(|value| !matches!(value, Value::Null))
        .cloned()
}
pub(crate) fn add_completion(data: Value, command: &Value) -> Value {
    let mut fields = data.as_map().unwrap().clone();
    fields.insert("operation".into(), terminal(command, "succeeded"));
    Value::Map(Arc::new(fields))
}
pub(crate) fn clear_marker(outcome: &mut misa_reframe::Outcome) {
    outcome.changes.last_mut().unwrap().patches.push((
        Path::parse("session.command_transaction").unwrap(),
        Op::Set(Value::Null),
    ));
}
pub(crate) fn settle(
    tx: &mut Tx<'_>,
    data: &Value,
    state: &str,
) -> Result<(), misa_reframe::Fault> {
    let Some(record) = data.get("operation") else {
        return Ok(());
    };
    let Some(index) = records(tx.db()).iter().position(|existing| {
        existing.get("id") == record.get("id")
            && existing.get("terminal").and_then(Value::as_bool) != Some(true)
    }) else {
        return Ok(());
    };
    tx.set(&format!("{ROOT}[{index}]"), terminal(record, state))
}
pub(crate) fn restore(entries: &[Value]) -> Value {
    let mut records = Vec::new();
    for entry in entries {
        if entry.get("kind").and_then(Value::as_str) == Some("operations.checkpoint") {
            if let Some(saved) = entry
                .get("data")
                .and_then(|data| data.get("commands"))
                .and_then(Value::as_list)
            {
                records = saved
                    .iter()
                    .take(MAX)
                    .map(|record| {
                        if record.get("terminal").and_then(Value::as_bool) == Some(true) {
                            record.clone()
                        } else {
                            terminal(record, "interrupted")
                        }
                    })
                    .collect();
            }
        }
        if entry.get("kind").and_then(Value::as_str) != Some(crate::contribution::PATCH_KIND) {
            continue;
        }
        if let Some(record) = entry.get("data").and_then(|data| data.get("operation")) {
            records.retain(|previous| previous.get("id") != record.get("id"));
            if records.len() == MAX {
                records.remove(0);
            }
            records.push(record.clone());
        }
    }
    Value::list(records)
}
pub(crate) fn interrupt(tx: &mut Tx<'_>) -> Result<(), misa_reframe::Fault> {
    let interrupted = records(tx.db())
        .iter()
        .map(|record| {
            if record.get("terminal").and_then(Value::as_bool) == Some(true) {
                record.clone()
            } else {
                terminal(record, "interrupted")
            }
        })
        .collect::<Vec<_>>();
    tx.set(ROOT, Value::list(interrupted))?;
    if tx
        .get("session.plugin_write")
        .and_then(|data| data.get("operation"))
        .is_some()
    {
        tx.set("session.plugin_write", Value::Null)?;
    }
    Ok(())
}
impl Runtime {
    pub(crate) fn settle_transaction_tools(&self) {
        let completed = {
            let mut state = self.state.lock().expect("session state poisoned");
            self.tool_invocations.lock().expect("tool correlations poisoned").iter().filter_map(|(id, (_, _, operation))| {
                let operation = operation.as_ref()?;
                if operation.scope != self.scope() { return None; }
                let record = state.state.query(&misa_proto::Query::new("operation.result").arg(Value::str(&operation.id))).ok()?;
                let outcome = crate::operations::outcome_of_result(&record)?;
                Some((*id, outcome))
            }).collect::<Vec<_>>()
        };
        for (id, outcome) in completed {
            self.complete_tool_invocation(id, outcome);
        }
    }
    pub(crate) fn execute_event_transaction(
        &self,
        context: &CallContext,
        invocation: &Invocation,
        kind: &str,
    ) -> Outcome {
        let id = format!(
            "{}:command:{}",
            self.scope().incarnation,
            self.next_call.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let record = Value::map([
            ("id", Value::str(&id)),
            ("kind", Value::str(&invocation.command)),
            ("state", Value::str("running")),
            ("terminal", Value::Bool(false)),
            ("generation", Value::Int(1)),
            ("outputs", Value::list([])),
        ]);
        let event = Event::new(kind)
            .with("command", Value::str(&invocation.command))
            .with("input", invocation.input.clone())
            .with("request", Value::str(invocation.id.to_string()))
            .with("principal", Value::str(&context.principal))
            .with("connection", Value::str(context.connection.to_string()));
        let outcome = {
            let mut state = self.state.lock().expect("session state poisoned");
            if self.is_closed() {
                return Outcome::Rejected {
                    fault: Fault::new("closed_scope", "Session closed"),
                };
            }
            let mut outcome = self.dispatch_locked(
                &mut state,
                Event::new("command/transaction")
                    .with("record", record)
                    .with("event", Value::str(kind))
                    .with("data", event.data),
            );
            if outcome.committed() {
                self.queue_operation_checkpoint(&mut state, &mut outcome.effects, &mut None);
            }
            outcome
        };
        if let Some(fault) = outcome.as_faults().into_iter().next() {
            return Outcome::Rejected { fault };
        }
        self.perform(&outcome);
        self.rev.send_replace(outcome.rev);
        Outcome::Accepted {
            operation: OperationRef {
                scope: self.scope(),
                id,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Contribution, commands::CommandRegistration};
    use misa_kernel::{Kernel, KernelEvent, Request};
    use misa_proto::{Query, schema::Schema};
    use std::{future::Future, pin::Pin, sync::Mutex};
    #[derive(Default)]
    struct Sink(Mutex<Vec<Request>>);
    impl Kernel for Sink {
        fn execute<'a, 'b, 'f>(
            &'a self,
            request: Request,
            _: &'b tokio::sync::mpsc::UnboundedSender<KernelEvent>,
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
    fn setup(effect: bool) -> (Arc<Runtime>, Arc<Sink>) {
        let sink = Arc::new(Sink::default());
        let contribution = Contribution::default()
            .with_root("counter", Value::Int(0))
            .unwrap()
            .with_handler(
                "counter/increment",
                0,
                Arc::new(misa_reframe::FnHandler::new(
                    "counter.increment",
                    move |tx: &mut Tx<'_>, _: &Event| {
                        tx.set("counter", Value::Int(tx.int("counter") + 1))?;
                        if effect {
                            tx.fx(misa_reframe::Effect::new("kernel.log.list"));
                        }
                        Ok(())
                    },
                )),
            )
            .with_command(CommandRegistration::event(
                "counter.increment",
                Schema::Choice {
                    values: vec![misa_proto::schema::Literal::Null],
                },
                "counter/increment",
            ));
        (
            Runtime::start_with(
                "commands",
                "Commands",
                None,
                sink.clone(),
                "scripted",
                "test",
                Value::Null,
                contribution,
            ),
            sink,
        )
    }
    fn invoke(runtime: &Runtime, id: u64) -> Outcome {
        runtime.execute_command(
            &CallContext {
                principal: "alice".into(),
                connection: 1,
            },
            Invocation {
                id,
                scope: runtime.scope(),
                command: "counter.increment".into(),
                input: Value::Null,
            },
        )
    }
    fn result(runtime: &Runtime, id: &str) -> Value {
        runtime
            .state
            .lock()
            .unwrap()
            .state
            .query(&Query::new("operation.result").arg(Value::str(id)))
            .unwrap()
    }
    async fn next_append(sink: &Sink, kind: &str) -> (String, Value) {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let found = {
                    let mut requests = sink.0.lock().unwrap();
                    requests
                        .iter()
                        .position(
                            |request| matches!(request,Request::Append{kind:got,..} if got==kind),
                        )
                        .map(|index| requests.remove(index))
                };
                if let Some(Request::Append {
                    conversation, data, ..
                }) = found
                {
                    return (conversation, data);
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap()
    }
    fn reply(runtime: &Runtime, kind: &str, conversation: &str, data: Value, failed: bool) {
        runtime.dispatch(
            Event::new(if failed {
                "kernel/log.failed"
            } else {
                "kernel/log.appended"
            })
            .with("conversation", Value::str(conversation))
            .with("kind", Value::str(kind))
            .with("data", data)
            .with("seq", Value::Int(1))
            .with("message", Value::str("disk failure")),
        );
    }
    #[tokio::test]
    async fn transaction_success_waits_for_exact_journal_ack_and_recovery_keeps_the_fact() {
        let (runtime, sink) = setup(false);
        let Outcome::Accepted { operation } = invoke(&runtime, 1) else {
            panic!("transaction must return a receipt")
        };
        assert_eq!(
            result(&runtime, &operation.id).get("terminal"),
            Some(&Value::Bool(false))
        );
        assert_eq!(
            runtime.state.lock().unwrap().state.db().get("counter"),
            Some(&Value::Int(0))
        );
        let (conversation, checkpoint) = next_append(&sink, "operations.checkpoint").await;
        assert!(!sink.0.lock().unwrap().iter().any(|request|matches!(request,Request::Append{kind,..} if kind==crate::contribution::PATCH_KIND)));
        reply(
            &runtime,
            "operations.checkpoint",
            &conversation,
            checkpoint.clone(),
            false,
        );
        let (_, patch) = next_append(&sink, crate::contribution::PATCH_KIND).await;
        let mut stale = patch.as_map().unwrap().clone();
        stale.insert("write".into(), Value::Int(999));
        reply(
            &runtime,
            crate::contribution::PATCH_KIND,
            &conversation,
            Value::Map(Arc::new(stale)),
            false,
        );
        assert_eq!(
            result(&runtime, &operation.id).get("terminal"),
            Some(&Value::Bool(false))
        );
        reply(
            &runtime,
            crate::contribution::PATCH_KIND,
            &conversation,
            patch.clone(),
            false,
        );
        assert_eq!(
            result(&runtime, &operation.id)
                .get("state")
                .and_then(Value::as_str),
            Some("succeeded")
        );
        assert_eq!(
            runtime.state.lock().unwrap().state.db().get("counter"),
            Some(&Value::Int(1))
        );
        let entries = vec![
            Value::map([
                ("kind", Value::str("operations.checkpoint")),
                ("data", checkpoint),
            ]),
            Value::map([
                ("kind", Value::str(crate::contribution::PATCH_KIND)),
                ("data", patch),
            ]),
        ];
        assert_eq!(
            restore(&entries).as_list().unwrap()[0]
                .get("state")
                .and_then(Value::as_str),
            Some("succeeded")
        );
        assert_eq!(
            restore(&entries[..1]).as_list().unwrap()[0]
                .get("state")
                .and_then(Value::as_str),
            Some("interrupted")
        );
        runtime.shutdown_complete().await;
    }
    #[tokio::test]
    async fn failed_checkpoint_or_patch_cannot_publish_mutation_or_success() {
        for failed_checkpoint in [true, false] {
            let (runtime, sink) = setup(false);
            let Outcome::Accepted { operation } = invoke(&runtime, 1) else {
                panic!()
            };
            let (conversation, checkpoint) = next_append(&sink, "operations.checkpoint").await;
            reply(
                &runtime,
                "operations.checkpoint",
                &conversation,
                checkpoint,
                failed_checkpoint,
            );
            if !failed_checkpoint {
                let (_, patch) = next_append(&sink, crate::contribution::PATCH_KIND).await;
                reply(
                    &runtime,
                    crate::contribution::PATCH_KIND,
                    &conversation,
                    patch,
                    true,
                );
            }
            assert_eq!(
                runtime.state.lock().unwrap().state.db().get("counter"),
                Some(&Value::Int(0))
            );
            assert_eq!(
                result(&runtime, &operation.id)
                    .get("state")
                    .and_then(Value::as_str),
                Some(if failed_checkpoint {
                    "interrupted"
                } else {
                    "failed"
                })
            );
            runtime.shutdown_complete().await;
        }
    }
    #[tokio::test]
    async fn ambiguous_external_effect_command_is_rejected_atomically() {
        let (runtime, _) = setup(true);
        assert!(
            matches!(invoke(&runtime,1),Outcome::Rejected{fault} if fault.code=="command.effects")
        );
        assert_eq!(
            runtime.state.lock().unwrap().state.db().get("counter"),
            Some(&Value::Int(0))
        );
        assert!(records(runtime.state.lock().unwrap().state.db()).is_empty());
        runtime.shutdown_complete().await;
    }
}

#[cfg(test)]
mod tool_test {
    use super::*;
    use misa_kernel::{Kernel, KernelEvent, Request};
    use std::{future::Future, pin::Pin, sync::Mutex};
    struct Gate {
        kernel: misa_kernel::LocalKernel,
        held: Mutex<Option<(Request, tokio::sync::mpsc::UnboundedSender<KernelEvent>)>>,
    }
    impl Kernel for Gate {
        fn execute<'a, 'b, 'f>(
            &'a self,
            request: Request,
            out: &'b tokio::sync::mpsc::UnboundedSender<KernelEvent>,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'f>>
        where
            'a: 'f,
            'b: 'f,
            Self: 'f,
        {
            Box::pin(async move {
                if matches!(&request,Request::Append{kind,..} if kind==crate::contribution::PATCH_KIND)
                {
                    *self.held.lock().unwrap() = Some((request, out.clone()));
                } else {
                    self.kernel.execute(request, out).await;
                }
            })
        }
    }
    #[tokio::test]
    async fn model_tool_cannot_continue_before_its_transaction_is_durable() {
        let kernel = Arc::new(Gate {
            kernel: misa_kernel::LocalKernel::new(misa_kernel::ScriptedProvider::new([
                misa_kernel::Turn::call(
                    "counter_increment",
                    Value::map([]),
                    misa_kernel::Turn::say("continued"),
                ),
            ])),
            held: Mutex::new(None),
        });
        let contribution = crate::Contribution::default()
            .with_root("counter", Value::Int(0))
            .unwrap()
            .with_handler(
                "counter/increment",
                0,
                Arc::new(misa_reframe::FnHandler::new(
                    "counter.increment",
                    |tx: &mut Tx<'_>, _: &Event| tx.set("counter", Value::Int(1)),
                )),
            )
            .with_command(crate::commands::CommandRegistration::event(
                "counter.increment",
                misa_proto::schema::Schema::Record {
                    fields: Default::default(),
                    allow_unknown: false,
                },
                "counter/increment",
            ))
            .with_tool(misa_proto::tool::Binding {
                name: "counter_increment".into(),
                description: "Increment".into(),
                command: "counter.increment".into(),
            });
        let runtime = Runtime::start_with(
            "tool-transaction",
            "Tool",
            None,
            kernel.clone(),
            "scripted",
            "test",
            Value::Null,
            contribution,
        );
        let result = runtime.execute_command(
            &CallContext {
                principal: "alice".into(),
                connection: 1,
            },
            Invocation {
                id: 99,
                scope: runtime.scope(),
                command: "session.prompt".into(),
                input: Value::map([("text", Value::str("increment"))]),
            },
        );
        assert!(matches!(result, Outcome::Accepted { .. }));
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while kernel.held.lock().unwrap().is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            runtime.state.lock().unwrap().state.db().get("counter"),
            Some(&Value::Int(0))
        );
        assert!(
            !kernel
                .kernel
                .entries()
                .iter()
                .any(|entry| entry.kind == "tool_result")
        );
        assert_eq!(runtime.tool_invocations.lock().unwrap().len(), 1);
        let (request, out) = kernel.held.lock().unwrap().take().unwrap();
        kernel.kernel.execute(request, &out).await;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if kernel
                    .kernel
                    .entries()
                    .iter()
                    .any(|entry| entry.kind == "tool_result")
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(
            runtime.state.lock().unwrap().state.db().get("counter"),
            Some(&Value::Int(1))
        );
        assert!(runtime.tool_invocations.lock().unwrap().is_empty());
        runtime.shutdown_complete().await;
    }
}
