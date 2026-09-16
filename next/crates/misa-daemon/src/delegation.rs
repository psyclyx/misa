//! Child work is a daemon operation. A tool waits for its terminal result;
//! directory summaries may lag the child's independently committed publications.
//! At most 256 work records are retained. `daemon.work.forget` explicitly removes
//! a terminal record and releases its deduplication key; kernel conversation and
//! attempt facts remain durable. Inclusive overview usage covers retained links.
use crate::{directory::Directory, lifecycle::SessionSpec};

// The model-facing child result is a bounded command result. The full output
// remains in the child's conversation log and is never silently truncated.
const RESULT_BYTES: usize = 256 * 1024;
use misa_proto::{
    Fault, Query,
    invocation::{Command, Invocation, OperationRef, Outcome},
    observation::Scope,
    schema::{Field, Schema},
};
use misa_protocol::invocation::{CallContext, CommandOwner};
use misa_session::{Contribution, Reading, Runtime, commands::CommandRegistration};
use misa_value::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Weak},
};

#[derive(Default, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Work {
    next: u64,
    records: BTreeMap<String, Record>,
}
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Record {
    #[serde(default)]
    conversation: Option<String>,
    #[serde(default)]
    usage: BTreeMap<String, Usage>,
    #[serde(default = "parent_lifetime")]
    parent_bound: bool,
    parent: Scope,
    #[serde(default)]
    parent_operation: Option<String>,
    #[serde(default)]
    parent_attempt: Option<String>,
    #[serde(default)]
    child_operation: Option<String>,
    principal: String,
    key: String,
    #[serde(default)]
    input_hash: String,
    child: Option<Scope>,
    state: String,
    result: Value,
}
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
struct Usage {
    input_tokens: i64,
    output_tokens: i64,
    cost_micros: i64,
}
impl Usage {
    fn read(value: &Value) -> Self {
        let number = |key: &str| value.get(key).and_then(Value::as_i64).unwrap_or(0).max(0);
        Self {
            input_tokens: number("input_tokens"),
            output_tokens: number("output_tokens"),
            cost_micros: number("cost_micros"),
        }
    }
    fn add(&mut self, other: &Self) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cost_micros = self.cost_micros.saturating_add(other.cost_micros);
    }
}
fn parent_lifetime() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    fn pending_record() -> Record {
        Record {
            parent: Scope {
                id: misa_proto::observation::ScopeId::Session {
                    id: "parent".into(),
                },
                incarnation: "run".into(),
            },
            parent_bound: true,
            parent_operation: None,
            parent_attempt: None,
            child_operation: None,
            conversation: Some("child-log".into()),
            principal: "owner".into(),
            key: "one".into(),
            input_hash: String::new(),
            child: None,
            state: "running".into(),
            result: Value::Null,
            usage: BTreeMap::new(),
        }
    }
    async fn until(directory: &Directory, condition: impl Fn() -> bool) {
        let mut changed = directory.watch_work();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !condition() {
                changed.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
    }
    fn overview_row(directory: &Directory, scope: &Scope) -> Value {
        use misa_proto::observation::{Content, Encoding, Member, Selection};
        let selection = Selection {
            scope: directory.scope_value(),
            members: BTreeMap::from([(
                "overview".into(),
                Member {
                    query: Query::new("daemon.overview"),
                    contract: "daemon.overview@1".into(),
                    encoding: Encoding::Value,
                    optional: false,
                },
            )]),
        };
        let snapshot = misa_protocol::owner::Owner::read(
            directory,
            &CallContext {
                principal: "owner".into(),
                connection: 1,
            },
            &selection,
        )
        .unwrap();
        let Content::Value(rows) = &snapshot.members["overview"] else {
            panic!()
        };
        rows.as_list()
            .unwrap()
            .iter()
            .find(|row| row.get("scope") == Some(&crate::lifecycle::encode(scope)))
            .cloned()
            .unwrap_or(Value::Null)
    }
    async fn approve(child: &Runtime, context: &CallContext) -> Outcome {
        let Reading::Data(requests) = child.read(&Query::new("requests.summary")).unwrap() else {
            panic!()
        };
        let request = requests.as_list().unwrap().first().unwrap();
        child
            .execute(
                context,
                Invocation {
                    id: 100,
                    scope: child.scope(),
                    command: "input.resolve".into(),
                    input: Value::map([
                        ("request", request.get("id").unwrap().clone()),
                        ("generation", request.get("generation").unwrap().clone()),
                        ("approved", Value::Bool(true)),
                    ]),
                },
            )
            .await
    }
    #[tokio::test]
    async fn overview_tracks_working_approval_and_independent_children_together() {
        let directory = Directory::fresh().unwrap();
        let owner = Arc::downgrade(&directory);
        directory
            .install_factory(Arc::new(move |_, spec| {
                let owner = owner.clone();
                Box::pin(async move {
                    let busy = spec.model.as_deref() == Some("busy");
                    let turn = if busy {
                        misa_kernel::Turn::call(
                            "shell",
                            Value::map([
                                ("command", Value::str("sleep 30")),
                                ("wait_ms", Value::Int(30000)),
                            ]),
                            misa_kernel::Turn::say("busy done"),
                        )
                    } else {
                        misa_kernel::Turn::call(
                            "echo",
                            Value::str("approved"),
                            misa_kernel::Turn::say("done"),
                        )
                    };
                    Ok(Runtime::prepare_with(
                        spec.id,
                        spec.title,
                        spec.conversation,
                        Arc::new(misa_kernel::LocalKernel::new(
                            misa_kernel::ScriptedProvider::new([turn]),
                        )),
                        "scripted",
                        spec.model.unwrap_or_else(|| "test".into()),
                        Value::map([(
                            "tool_approval",
                            Value::str(if busy { "allow" } else { "ask" }),
                        )]),
                        install(Contribution::default(), owner),
                    ))
                })
            }))
            .unwrap();
        let context = CallContext {
            principal: "owner".into(),
            connection: 1,
        };
        let parent = directory
            .open(
                &context,
                SessionSpec {
                    id: "parent".into(),
                    title: "parent".into(),
                    conversation: None,
                    provider: None,
                    model: None,
                    parent_attempt: None,
                    recovering: false,
                },
            )
            .await
            .unwrap();
        let mut ids = Vec::new();
        for (index, (key, command, model)) in [
            ("busy", "work.delegate", "busy"),
            ("approval", "work.delegate", "ask"),
            ("independent", "work.start", "ask"),
        ]
        .into_iter()
        .enumerate()
        {
            let outcome = parent
                .execute(
                    &context,
                    Invocation {
                        id: index as u64 + 1,
                        scope: parent.scope(),
                        command: command.into(),
                        input: Value::map([
                            ("key", Value::str(key)),
                            ("task", Value::str("execute")),
                            ("model", Value::str(model)),
                        ]),
                    },
                )
                .await;
            ids.push(match outcome {
                Outcome::Accepted { operation } => operation.id,
                Outcome::Completed { value } => {
                    value.get("id").and_then(Value::as_str).unwrap().into()
                }
                other => panic!("{other:?}"),
            });
        }
        until(&directory, || {
            let row = overview_row(&directory, &parent.scope());
            row.get("working") == Some(&Value::Bool(true))
                && row.get("attention") == Some(&Value::Int(2))
        })
        .await;
        let row = overview_row(&directory, &parent.scope());
        assert_eq!(
            row.get("blocking").and_then(Value::as_list).unwrap().len(),
            2
        );
        let child = |index: usize| {
            let scope = directory.work.lock().unwrap().records[&ids[index]]
                .child
                .clone()
                .unwrap();
            directory.session(&scope).unwrap()
        };
        let approval = child(1);
        let independent = child(2);
        assert!(matches!(
            approve(
                &approval,
                &CallContext {
                    principal: "owner".into(),
                    connection: 2
                }
            )
            .await,
            Outcome::Completed { .. } | Outcome::Accepted { .. }
        ));
        until(&directory, || {
            let done = directory.work.lock().unwrap().records[&ids[1]].state == "succeeded";
            done && overview_row(&directory, &parent.scope()).get("attention")
                == Some(&Value::Int(1))
        })
        .await;
        directory.close(&parent.scope()).await.unwrap();
        until(&directory, || {
            directory.work.lock().unwrap().records[&ids[0]].state == "cancelled"
        })
        .await;
        assert!(!independent.is_closed());
        assert!(matches!(
            approve(&independent, &context).await,
            Outcome::Completed { .. } | Outcome::Accepted { .. }
        ));
        until(&directory, || {
            directory.work.lock().unwrap().records[&ids[2]].state == "succeeded"
        })
        .await;
        directory.shutdown_complete().await;
        assert!(directory.supervisors.lock().unwrap().is_empty());
    }
    #[tokio::test]
    async fn failed_terminal_checkpoint_publishes_only_interrupted_and_forget_is_authorized() {
        use misa_protocol::owner::Owner;
        let directory = Directory::fresh().unwrap();
        directory
            .edit_work()
            .records
            .insert("one".into(), pending_record());
        let context = CallContext {
            principal: "owner".into(),
            connection: 1,
        };
        let before = directory
            .read(&context, &directory.selection())
            .unwrap()
            .position;
        let path = std::env::temp_dir().join(format!(
            "misa-terminal-failure-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"not a directory").unwrap();
        *directory.work_path.lock().unwrap() = Some(path.join("work.cbor"));
        assert!(matches!(
            directory
                .settle_work(
                    "one",
                    Outcome::Completed {
                        value: Value::str("effect completed")
                    }
                )
                .await,
            Outcome::Indeterminate { .. }
        ));
        assert_eq!(
            directory
                .read(&context, &directory.selection())
                .unwrap()
                .position,
            before + 1,
            "Never publish provisional succeeded before persistence fails"
        );
        assert_eq!(
            directory.work.lock().unwrap().records["one"].state,
            "interrupted"
        );
        let forget = Invocation {
            id: 3,
            scope: directory.scope_value(),
            command: "daemon.work.forget".into(),
            input: Value::map([
                ("operation", Value::str("one")),
                ("generation", Value::Int(1)),
            ]),
        };
        assert!(matches!(
            directory
                .execute(
                    &CallContext {
                        principal: "other".into(),
                        connection: 2
                    },
                    forget.clone()
                )
                .await,
            Outcome::Rejected { .. }
        ));
        assert!(
            matches!(
                directory.execute(&context, forget.clone()).await,
                Outcome::Rejected { .. }
            ),
            "Failed durable removal keeps result available"
        );
        assert!(directory.work.lock().unwrap().records.contains_key("one"));
        let saved = path.with_extension("cbor");
        *directory.work_path.lock().unwrap() = Some(saved.clone());
        assert!(matches!(
            directory.execute(&context, forget).await,
            Outcome::Completed { .. }
        ));
        assert!(!directory.work.lock().unwrap().records.contains_key("one"));
        let reopened = Directory::fresh().unwrap();
        reopened.install_work_log(saved.clone()).await.unwrap();
        assert!(reopened.work.lock().unwrap().records.is_empty());
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(saved).unwrap();
    }
    #[tokio::test]
    async fn child_executes_and_terminal_result_continues_parent_tool_turn() {
        let directory = Directory::fresh().unwrap();
        let store: Arc<dyn misa_kernel::Store> = Arc::new(misa_kernel::MemoryStore::default());
        let factory_store = store.clone();
        let owner = Arc::downgrade(&directory);
        directory
            .install_factory(Arc::new(move |_, spec| {
                let owner = owner.clone();
                let store = factory_store.clone();
                Box::pin(async move {
                    let provider = if spec.id == "parent" {
                        misa_kernel::ScriptedProvider::new([misa_kernel::Turn::call(
                            "delegate",
                            Value::map([
                                ("key", Value::str("child-one")),
                                ("task", Value::str("compute child answer")),
                            ]),
                            misa_kernel::Turn::say("parent continued"),
                        )])
                    } else {
                        misa_kernel::ScriptedProvider::always("child completed")
                    };
                    Ok(Runtime::prepare_with(
                        spec.id,
                        spec.title,
                        spec.conversation,
                        Arc::new(misa_kernel::LocalKernel::new(provider).with_store(store)),
                        "scripted",
                        "test",
                        Value::map([(
                            "parent_attempt",
                            spec.parent_attempt
                                .as_ref()
                                .map(Value::str)
                                .unwrap_or(Value::Null),
                        )]),
                        install(Contribution::default(), owner),
                    ))
                })
            }))
            .unwrap();
        let context = CallContext {
            principal: "owner".into(),
            connection: 1,
        };
        let parent = directory
            .open(
                &context,
                SessionSpec {
                    id: "parent".into(),
                    title: "parent".into(),
                    conversation: None,
                    provider: None,
                    model: None,
                    parent_attempt: None,
                    recovering: false,
                },
            )
            .await
            .unwrap();
        let mut changed = parent.watch_rev();
        let Outcome::Accepted { operation } = parent
            .execute(
                &context,
                Invocation {
                    id: 1,
                    scope: parent.scope(),
                    command: "session.prompt".into(),
                    input: Value::map([("text", Value::str("delegate work"))]),
                },
            )
            .await
        else {
            panic!()
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(Reading::Data(value)) =
                    parent.read(&Query::new("operation.result").arg(Value::str(&operation.id)))
                {
                    if value.get("terminal").and_then(Value::as_bool) == Some(true) {
                        assert_eq!(
                            value.get("state").and_then(Value::as_str),
                            Some("succeeded")
                        );
                        break;
                    }
                }
                changed.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        let work = directory.work.lock().unwrap();
        assert_eq!(work.records.len(), 1);
        let record = work.records.values().next().unwrap();
        assert_eq!(record.state, "succeeded");
        let encoded = format!("{:?}", record.result);
        assert!(encoded.contains("child completed"), "{encoded}");
        drop(work);
        let output = parent
            .read(&Query::new("operation.output").arg(Value::str(&operation.id)))
            .unwrap();
        assert!(format!("{output:?}").contains("parent continued"));
        assert_eq!(
            directory.sessions().len(),
            1,
            "bound child closes after result"
        );
        let attempts = store.attempts(None).unwrap();
        let child = attempts
            .iter()
            .find(|attempt| attempt.parent.is_some())
            .expect("child attempt has explicit ancestry");
        assert!(
            attempts
                .iter()
                .any(|attempt| Some(&attempt.name) == child.parent.as_ref())
        );
        directory.shutdown();
    }
    #[tokio::test]
    async fn authenticated_cancel_closes_bound_child_waiting_for_approval() {
        let directory = Directory::fresh().unwrap();
        let owner = Arc::downgrade(&directory);
        directory
            .install_factory(Arc::new(move |_, spec| {
                let owner = owner.clone();
                Box::pin(async move {
                    Ok(Runtime::prepare_with(
                        spec.id,
                        spec.title,
                        spec.conversation,
                        Arc::new(misa_kernel::LocalKernel::new(
                            misa_kernel::ScriptedProvider::new([misa_kernel::Turn::call(
                                "echo",
                                Value::str("must await approval"),
                                misa_kernel::Turn::say("done"),
                            )]),
                        )),
                        "scripted",
                        "test",
                        Value::map([("tool_approval", Value::str("ask"))]),
                        install(Contribution::default(), owner),
                    ))
                })
            }))
            .unwrap();
        let context = CallContext {
            principal: "owner".into(),
            connection: 1,
        };
        let parent = directory
            .open(
                &context,
                SessionSpec {
                    id: "parent".into(),
                    title: "parent".into(),
                    conversation: None,
                    provider: None,
                    model: None,
                    parent_attempt: None,
                    recovering: false,
                },
            )
            .await
            .unwrap();
        let mut changed = directory.watch_work();
        let Outcome::Accepted { operation } = parent
            .execute(
                &context,
                Invocation {
                    id: 2,
                    scope: parent.scope(),
                    command: "work.delegate".into(),
                    input: Value::map([("key", Value::str("wait")), ("task", Value::str("run"))]),
                },
            )
            .await
        else {
            panic!()
        };
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if directory.work.lock().unwrap().records[&operation.id]
                    .child
                    .is_some()
                {
                    break;
                }
                changed.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        let cancel = Invocation {
            id: 3,
            scope: directory.scope_value(),
            command: "operation.cancel".into(),
            input: Value::map([
                ("operation", Value::str(&operation.id)),
                ("generation", Value::Int(1)),
            ]),
        };
        assert!(matches!(
            directory
                .execute(
                    &CallContext {
                        principal: "intruder".into(),
                        connection: 2
                    },
                    cancel.clone()
                )
                .await,
            Outcome::Rejected { .. }
        ));
        assert!(matches!(
            directory.execute(&context, cancel).await,
            Outcome::Accepted { .. }
        ));
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if directory.work.lock().unwrap().records[&operation.id].state == "cancelled" {
                    break;
                }
                changed.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(directory.sessions().len(), 1);
        directory.shutdown();
    }
    #[tokio::test]
    async fn restart_retains_relationship_but_never_replays_pending_child() {
        let path = std::env::temp_dir().join(format!(
            "misa-work-{}-{}.cbor",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let first = Directory::fresh().unwrap();
        first.install_work_log(path.clone()).await.unwrap();
        first.edit_work().records.insert(
            "logical-work".into(),
            Record {
                conversation: None,
                usage: BTreeMap::new(),
                parent_bound: true,
                parent_operation: None,
                parent_attempt: None,
                child_operation: None,
                parent: misa_proto::observation::Scope {
                    id: misa_proto::observation::ScopeId::Session {
                        id: "parent".into(),
                    },
                    incarnation: "old".into(),
                },
                principal: "owner".into(),
                key: "one".into(),
                input_hash: String::new(),
                child: None,
                state: "running".into(),
                result: Value::Null,
            },
        );
        first.persist_work().await.unwrap();
        let second = Directory::fresh().unwrap();
        second.install_work_log(path.clone()).await.unwrap();
        let work = second.work.lock().unwrap();
        assert_eq!(work.records["logical-work"].state, "interrupted");
        assert_eq!(work.records["logical-work"].parent.incarnation, "old");
        assert!(second.sessions().is_empty());
        drop(work);
        std::fs::remove_file(path).unwrap();
    }
    #[tokio::test]
    async fn failed_work_checkpoint_prevents_child_factory_execution() {
        let directory = Directory::fresh().unwrap();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = calls.clone();
        directory
            .install_factory(Arc::new(move |_, _| {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                Box::pin(async { Err(Fault::new("unexpected", "factory must not execute")) })
            }))
            .unwrap();
        let parent = Runtime::start_with(
            "parent",
            "parent",
            None,
            Arc::new(misa_kernel::LocalKernel::new(
                misa_kernel::ScriptedProvider::always("unused"),
            )),
            "scripted",
            "test",
            Value::Null,
            install(Contribution::default(), Arc::downgrade(&directory)),
        );
        directory.insert(parent.clone()).unwrap();
        let path = std::env::temp_dir().join(format!(
            "misa-work-failure-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::write(&path, b"not a directory").unwrap();
        *directory.work_path.lock().unwrap() = Some(path.join("work.cbor"));
        let mut changed = directory.watch_work();
        let context = CallContext {
            principal: "owner".into(),
            connection: 1,
        };
        let Outcome::Accepted { operation } = parent
            .execute(
                &context,
                Invocation {
                    id: 1,
                    scope: parent.scope(),
                    command: "work.delegate".into(),
                    input: Value::map([
                        ("key", Value::str("one")),
                        ("task", Value::str("never execute")),
                    ]),
                },
            )
            .await
        else {
            panic!()
        };
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if directory.work.lock().unwrap().records[&operation.id].state == "interrupted" {
                    break;
                }
                changed.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 0);
        assert_eq!(directory.sessions().len(), 1);
        directory.shutdown();
        std::fs::remove_file(path).unwrap();
    }
    #[tokio::test]
    async fn independent_child_survives_parent_close_and_finishes_after_approval() {
        let directory = Directory::fresh().unwrap();
        let owner = Arc::downgrade(&directory);
        directory
            .install_factory(Arc::new(move |_, spec| {
                let owner = owner.clone();
                Box::pin(async move {
                    Ok(Runtime::prepare_with(
                        spec.id,
                        spec.title,
                        spec.conversation,
                        Arc::new(misa_kernel::LocalKernel::new(
                            misa_kernel::ScriptedProvider::new([misa_kernel::Turn::call(
                                "echo",
                                Value::str("approved child"),
                                misa_kernel::Turn::say("child finished"),
                            )]),
                        )),
                        "scripted",
                        "test",
                        Value::map([("tool_approval", Value::str("ask"))]),
                        install(Contribution::default(), owner),
                    ))
                })
            }))
            .unwrap();
        let context = CallContext {
            principal: "owner".into(),
            connection: 1,
        };
        let parent = directory
            .open(
                &context,
                SessionSpec {
                    id: "parent".into(),
                    title: "parent".into(),
                    conversation: None,
                    provider: None,
                    model: None,
                    parent_attempt: None,
                    recovering: false,
                },
            )
            .await
            .unwrap();
        let mut changed = directory.watch_work();
        let Outcome::Completed { value } = parent
            .execute(
                &context,
                Invocation {
                    id: 2,
                    scope: parent.scope(),
                    command: "work.start".into(),
                    input: Value::map([
                        ("key", Value::str("independent")),
                        ("task", Value::str("run")),
                    ]),
                },
            )
            .await
        else {
            panic!()
        };
        let id = value.get("id").and_then(Value::as_str).unwrap().to_owned();
        let child = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let scope = directory.work.lock().unwrap().records[&id].child.clone();
                if let Some(scope) = scope {
                    break directory.session(&scope).unwrap();
                }
                changed.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        let mut child_changed = child.watch_rev();
        let request = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Ok(Reading::Data(value)) = child.read(&Query::new("requests.summary")) {
                    if let Some(request) = value.as_list().and_then(|list| list.first()) {
                        break request.clone();
                    }
                }
                child_changed.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert!(directory.close(&parent.scope()).await.unwrap());
        assert!(
            !child.is_closed(),
            "Independent lifetime must not follow parent close"
        );
        assert!(matches!(
            child
                .execute(
                    &context,
                    Invocation {
                        id: 3,
                        scope: child.scope(),
                        command: "input.resolve".into(),
                        input: Value::map([
                            ("request", request.get("id").unwrap().clone()),
                            ("generation", request.get("generation").unwrap().clone()),
                            ("approved", Value::Bool(true)),
                        ])
                    }
                )
                .await,
            Outcome::Completed { .. } | Outcome::Accepted { .. }
        ));
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if directory.work.lock().unwrap().records[&id].state == "succeeded" {
                    break;
                }
                changed.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert!(child.is_closed());
        assert!(directory.sessions().is_empty());
        directory.shutdown();
    }
    #[test]
    fn overview_deduplicates_attention_and_preserves_cycles_and_unavailability() {
        let scope = |id: &str| Scope {
            id: misa_proto::observation::ScopeId::Session { id: id.into() },
            incarnation: "run".into(),
        };
        let edge = |parent: Scope, child: Scope| Record {
            conversation: None,
            usage: BTreeMap::new(),
            parent,
            parent_bound: true,
            parent_operation: None,
            parent_attempt: None,
            child_operation: None,
            principal: "owner".into(),
            key: "key".into(),
            input_hash: String::new(),
            child: Some(child),
            state: "running".into(),
            result: Value::Null,
        };
        let mut work = Work::default();
        work.records
            .insert("one".into(), edge(scope("parent"), scope("child")));
        work.records
            .insert("duplicate".into(), edge(scope("parent"), scope("child")));
        let entry = |id: &str, working: bool, requests: Vec<Value>| misa_proto::directory::Entry {
            id: id.into(),
            incarnation: "run".into(),
            title: id.into(),
            availability: misa_proto::directory::Availability::Current,
            source_position: 3,
            summary: Value::map([
                ("working", Value::Bool(working)),
                ("attention", Value::Int(requests.len() as i64)),
                ("requests", Value::list(requests)),
                (
                    "usage",
                    crate::lifecycle::encode(&Usage {
                        cost_micros: if id == "parent" { 10 } else { 20 },
                        ..Default::default()
                    }),
                ),
            ]),
        };
        let parent = entry("parent", false, vec![]);
        let child = entry(
            "child",
            true,
            vec![Value::map([
                ("id", Value::str("approve")),
                ("generation", Value::Int(1)),
            ])],
        );
        let rows = crate::lifecycle::encode(&vec![parent.clone(), child]);
        let overview = work.overview(&rows);
        let row = &overview.as_list().unwrap()[0];
        assert_eq!(row.get("attention"), Some(&Value::Int(1)));
        assert_eq!(row.get("working"), Some(&Value::Bool(true)));
        assert_eq!(row.get("cyclic"), Some(&Value::Bool(false)));
        assert_eq!(
            row.get("direct_usage")
                .and_then(|value| value.get("cost_micros")),
            Some(&Value::Int(10))
        );
        assert_eq!(
            row.get("inclusive_usage")
                .and_then(|value| value.get("cost_micros")),
            Some(&Value::Int(30))
        );
        for record in work.records.values_mut() {
            record.state = "succeeded".into();
            record.usage.insert(
                "same-attempt".into(),
                Usage {
                    cost_micros: 20,
                    ..Default::default()
                },
            );
        }
        let completed = work.overview(&crate::lifecycle::encode(&vec![parent.clone()]));
        assert_eq!(
            completed.as_list().unwrap()[0]
                .get("inclusive_usage")
                .and_then(|value| value.get("cost_micros")),
            Some(&Value::Int(30)),
            "Duplicate relationship must not count one attempt twice"
        );
        for record in work.records.values_mut() {
            record.state = "running".into();
        }
        work.records
            .insert("cycle".into(), edge(scope("child"), scope("parent")));
        assert_eq!(
            work.overview(&rows).as_list().unwrap()[0].get("cyclic"),
            Some(&Value::Bool(true))
        );
        let missing = work.overview(&crate::lifecycle::encode(&vec![parent]));
        assert_eq!(
            missing.as_list().unwrap()[0].get("availability"),
            Some(&Value::str("stale"))
        );
        assert_eq!(
            missing.as_list().unwrap()[0]
                .get("unavailable")
                .and_then(Value::as_list)
                .unwrap()
                .len(),
            1
        );
    }
}
fn record(fields: impl IntoIterator<Item = (&'static str, Schema)>) -> Schema {
    Schema::Record {
        fields: fields
            .into_iter()
            .map(|(key, schema)| {
                (
                    key.into(),
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
fn work_input() -> Schema {
    let Schema::Record {
        mut fields,
        allow_unknown,
    } = record([("key", Schema::String), ("task", Schema::String)])
    else {
        unreachable!()
    };
    for name in ["context", "provider", "model"] {
        fields.insert(
            name.into(),
            Field {
                schema: Schema::String,
                optional: true,
            },
        );
    }
    Schema::Record {
        fields,
        allow_unknown,
    }
}
pub(crate) fn definitions() -> Vec<misa_proto::query::Definition> {
    vec![
        misa_proto::query::Definition {
            id: "daemon.overview".into(),
            arguments: vec![],
            contract: "daemon.overview@1".into(),
            result: misa_proto::query::ResultContract::Data {
                schema: Schema::List {
                    items: Box::new(Schema::Value),
                },
            },
        },
        misa_proto::query::Definition {
            id: "daemon.work".into(),
            arguments: vec![],
            contract: "daemon.work@1".into(),
            result: misa_proto::query::ResultContract::Data {
                schema: Schema::List {
                    items: Box::new(Schema::Value),
                },
            },
        },
        misa_proto::query::Definition {
            id: "operation.result".into(),
            arguments: vec![Schema::String],
            contract: "daemon.operation.result@1".into(),
            result: misa_proto::query::ResultContract::Data {
                schema: Schema::Nullable {
                    inner: Box::new(record([
                        ("id", Schema::String),
                        ("kind", Schema::String),
                        ("state", Schema::String),
                        ("generation", Schema::Int),
                        ("terminal", Schema::Bool),
                        ("result", Schema::Value),
                        ("reconciliation", Schema::Value),
                    ])),
                },
            },
        },
    ]
}
impl Work {
    /// Cross-owner facts retain their source freshness. The traversal only uses
    /// installed cheap summaries and relationships; it never reads a transcript.
    pub(crate) fn overview(&self, rows: &Value) -> Value {
        use std::collections::BTreeSet;
        let entries = misa_proto::directory::entries(rows).unwrap_or_default();
        let sources = entries
            .iter()
            .map(|entry| (entry.scope(), entry))
            .collect::<BTreeMap<_, _>>();
        Value::list(entries.iter().map(|root| {
            let root_scope = root.scope();
            let mut pending = vec![(root_scope.clone(), BTreeSet::<Scope>::new(), true)];
            let mut seen = BTreeSet::new();
            let mut requests = BTreeMap::new();
            let mut unavailable = BTreeSet::new();
            let mut working = false;
            let mut stale = false;
            let mut cyclic = false;
            let mut blocking = Vec::new();
            let mut usage = Usage::default();
            let mut completed_attempts = BTreeMap::new();
            while let Some((scope, mut ancestors, required)) = pending.pop() {
                if !ancestors.insert(scope.clone()) {
                    cyclic = true;
                    continue;
                }
                if !seen.insert(scope.clone()) {
                    continue;
                }
                if let Some(entry) = sources.get(&scope) {
                    usage.add(&Usage::read(
                        entry.summary.get("usage").unwrap_or(&Value::Null),
                    ));
                    stale |= entry.availability != misa_proto::directory::Availability::Current;
                    working |= entry.summary.get("working").and_then(Value::as_bool) == Some(true);
                    for request in entry
                        .summary
                        .get("requests")
                        .and_then(Value::as_list)
                        .unwrap_or(&[])
                    {
                        if let (Some(id), Some(generation)) = (
                            request.get("id").and_then(Value::as_str),
                            request.get("generation").and_then(Value::as_i64),
                        ) {
                            requests
                                .entry((scope.clone(), id.to_owned(), generation))
                                .or_insert_with(|| {
                                    Value::map([
                                        ("scope", crate::lifecycle::encode(&scope)),
                                        (
                                            "source_position",
                                            Value::Int(entry.source_position as i64),
                                        ),
                                        (
                                            "availability",
                                            crate::lifecycle::encode(&entry.availability),
                                        ),
                                        ("request", request.clone()),
                                    ])
                                });
                        }
                    }
                } else if required {
                    unavailable.insert(scope.clone());
                }
                for (id, record) in self
                    .records
                    .iter()
                    .filter(|(_, record)| record.parent == scope)
                {
                    let active =
                        matches!(record.state.as_str(), "starting" | "running" | "cancelling");
                    if scope == root_scope && record.parent_bound && active {
                        blocking.push(Value::str(id));
                    }
                    if let Some(child) = &record.child {
                        if !sources.contains_key(child) {
                            completed_attempts.extend(
                                record
                                    .usage
                                    .iter()
                                    .map(|(id, usage)| (id.clone(), usage.clone())),
                            );
                        }
                        pending.push((child.clone(), ancestors.clone(), active));
                    }
                }
            }
            for attempt in completed_attempts.values() {
                usage.add(attempt);
            }
            Value::map([
                (
                    "direct_usage",
                    root.summary
                        .get("usage")
                        .cloned()
                        .unwrap_or_else(|| crate::lifecycle::encode(&Usage::default())),
                ),
                ("inclusive_usage", crate::lifecycle::encode(&usage)),
                ("scope", crate::lifecycle::encode(&root_scope)),
                ("source_position", Value::Int(root.source_position as i64)),
                (
                    "availability",
                    Value::str(if stale || !unavailable.is_empty() {
                        "stale"
                    } else {
                        "current"
                    }),
                ),
                (
                    "direct_working",
                    root.summary
                        .get("working")
                        .cloned()
                        .unwrap_or(Value::Bool(false)),
                ),
                ("working", Value::Bool(working)),
                (
                    "direct_attention",
                    root.summary
                        .get("attention")
                        .cloned()
                        .unwrap_or(Value::Int(0)),
                ),
                ("attention", Value::Int(requests.len() as i64)),
                ("requests", Value::list(requests.into_values())),
                ("blocking", Value::list(blocking)),
                (
                    "unavailable",
                    Value::list(unavailable.iter().map(crate::lifecycle::encode)),
                ),
                ("cyclic", Value::Bool(cyclic)),
            ])
        }))
    }
    pub(crate) fn result(&self, id: &str) -> Value {
        self.records
            .get(id)
            .map(|record| {
                Value::map([
                    ("id", Value::str(id)),
                    ("kind", Value::str("delegation")),
                    ("state", Value::str(&record.state)),
                    ("generation", Value::Int(1)),
                    (
                        "terminal",
                        Value::Bool(!matches!(
                            record.state.as_str(),
                            "starting" | "running" | "cancelling"
                        )),
                    ),
                    ("result", record.result.clone()),
                    (
                        "reconciliation",
                        Value::map([
                            (
                                "scope",
                                record
                                    .child
                                    .as_ref()
                                    .map(crate::lifecycle::encode)
                                    .unwrap_or(Value::Null),
                            ),
                            (
                                "conversation",
                                record
                                    .conversation
                                    .as_ref()
                                    .map(Value::str)
                                    .unwrap_or(Value::Null),
                            ),
                            (
                                "operation",
                                record
                                    .child_operation
                                    .as_ref()
                                    .map(Value::str)
                                    .unwrap_or(Value::Null),
                            ),
                        ]),
                    ),
                ])
            })
            .unwrap_or(Value::Null)
    }
    pub(crate) fn value(&self) -> Value {
        Value::list(self.records.iter().map(|(id, r)| {
            Value::map([
                ("id", Value::str(id)),
                ("parent", crate::lifecycle::encode(&r.parent)),
                (
                    "conversation",
                    r.conversation
                        .as_ref()
                        .map(Value::str)
                        .unwrap_or(Value::Null),
                ),
                (
                    "lifetime",
                    Value::str(if r.parent_bound {
                        "parent"
                    } else {
                        "independent"
                    }),
                ),
                (
                    "blocking",
                    Value::Bool(
                        r.parent_bound
                            && matches!(r.state.as_str(), "starting" | "running" | "cancelling"),
                    ),
                ),
                (
                    "parent_operation",
                    r.parent_operation
                        .as_ref()
                        .map(Value::str)
                        .unwrap_or(Value::Null),
                ),
                (
                    "parent_attempt",
                    r.parent_attempt
                        .as_ref()
                        .map(Value::str)
                        .unwrap_or(Value::Null),
                ),
                (
                    "child_operation",
                    r.child_operation
                        .as_ref()
                        .map(Value::str)
                        .unwrap_or(Value::Null),
                ),
                (
                    "child",
                    r.child
                        .as_ref()
                        .map(crate::lifecycle::encode)
                        .unwrap_or(Value::Null),
                ),
                ("state", Value::str(&r.state)),
                (
                    "usage",
                    crate::lifecycle::encode(&r.usage.values().fold(
                        Usage::default(),
                        |mut total, usage| {
                            total.add(usage);
                            total
                        },
                    )),
                ),
            ])
        }))
    }
}
/// The host explicitly installs delegation as a command and model tool.
pub fn install(mut contribution: Contribution, directory: Weak<Directory>) -> Contribution {
    for (command, name, parent_bound, description) in [
        (
            "work.delegate",
            "delegate",
            true,
            "Delegate a task and wait for its final result. It is cancelled with its parent work.",
        ),
        (
            "work.start",
            "start_work",
            false,
            "Start independent child work. Returns an operation reference immediately; observe it for completion.",
        ),
    ] {
        let directory = directory.clone();
        contribution = contribution
            .with_command(CommandRegistration::new(
                Command {
                    id: command.into(),
                    input: work_input(),
                    result: Schema::Value,
                },
                move |runtime, context, invocation| {
                    let Some(directory) = directory.upgrade() else {
                        return rejected("unavailable", "Daemon is unavailable");
                    };
                    directory.delegate(runtime, context, invocation, parent_bound)
                },
            ))
            .with_tool(misa_proto::tool::Binding {
                name: name.into(),
                description: description.into(),
                command: command.into(),
            });
    }
    contribution
}
fn rejected(code: &str, message: &str) -> Outcome {
    Outcome::Rejected {
        fault: Fault::new(code, message),
    }
}
fn decoded_outcome(value: &Value) -> Outcome {
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(value, &mut bytes).expect("operation outcome serializes");
    ciborium::de::from_reader(&bytes[..]).unwrap_or_else(|_| Outcome::Indeterminate {
        fault: Fault::new("invalid_result", "Stored work outcome is unavailable"),
    })
}
fn parent_stopped(parent: &Weak<Runtime>, operation: Option<&str>) -> bool {
    parent.upgrade().is_none_or(|runtime|runtime.is_closed() || operation.is_some_and(|id| {
        matches!(runtime.read(&Query::new("operation.result").arg(Value::str(id))), Ok(Reading::Data(value)) if value.get("terminal").and_then(Value::as_bool)==Some(true))
    }))
}
impl Directory {
    /// Explicit disposal releases retained results and their deduplication key.
    /// Conversation facts and the attempt ledger remain in the kernel store.
    pub(crate) async fn forget_work(
        &self,
        context: &CallContext,
        invocation: &Invocation,
    ) -> Outcome {
        let id = invocation
            .input
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let _guard = self.work_writes.lock().await;
        let mut candidate = self.work.lock().unwrap().clone();
        let Some(record) = candidate.records.get(id) else {
            return rejected("unavailable_operation", "Work record is unavailable");
        };
        if record.principal != context.principal {
            return rejected("unauthorized", "Work belongs to another caller");
        }
        if invocation.input.get("generation").and_then(Value::as_i64) != Some(1)
            || matches!(record.state.as_str(), "starting" | "running" | "cancelling")
        {
            return rejected("stale_request", "Only terminal work can be forgotten");
        }
        if record.child.as_ref().is_some_and(|child| {
            candidate
                .records
                .values()
                .any(|descendant| &descendant.parent == child)
        }) {
            return rejected(
                "dependent_work",
                "Forget descendant records before their parent relationship",
            );
        }
        candidate.records.remove(id);
        let path = self.work_path.lock().unwrap().clone();
        if let Some(path) = path {
            let result = tokio::task::spawn_blocking(move || {
                crate::membership::write_atomic(&path, &candidate)
            })
            .await;
            match result {
                Ok(Ok(())) => {}
                Ok(Err(fault)) => return Outcome::Rejected { fault },
                Err(error) => return rejected("work_storage", &error.to_string()),
            }
        }
        self.edit_work().records.remove(id);
        Outcome::Completed { value: Value::Null }
    }
    pub(crate) fn cancel_work(&self, context: &CallContext, invocation: &Invocation) -> Outcome {
        let id = invocation
            .input
            .get("operation")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let generation = invocation.input.get("generation").and_then(Value::as_i64);
        {
            let mut work = self.edit_work();
            let Some(record) = work.records.get_mut(id) else {
                return rejected("unavailable_operation", "Operation is unavailable");
            };
            if record.principal != context.principal {
                return rejected("unauthorized", "Operation belongs to another caller");
            }
            if generation != Some(1) || !matches!(record.state.as_str(), "starting" | "running") {
                return rejected(
                    "stale_request",
                    "Operation generation is no longer cancellable",
                );
            }
            record.state = "cancelling".into();
        }
        Outcome::Accepted {
            operation: OperationRef {
                scope: self.scope_value(),
                id: id.into(),
            },
        }
    }
    pub async fn install_work_log(&self, path: std::path::PathBuf) -> Result<(), Fault> {
        let source = path.clone();
        let mut work: Work = tokio::task::spawn_blocking(move || {
            let file = match std::fs::File::open(source) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    return Ok(Work::default());
                }
                Err(error) => return Err(Fault::new("work_storage", error.to_string())),
            };
            if file
                .metadata()
                .map_err(|e| Fault::new("work_storage", e.to_string()))?
                .len()
                > 128 * 1024 * 1024
            {
                return Err(Fault::new("work_storage", "Work log exceeds limit"));
            }
            let work: Work = ciborium::de::from_reader(file)
                .map_err(|e| Fault::new("work_storage", e.to_string()))?;
            if work.records.len() > 256 {
                return Err(Fault::new("work_storage", "Work record capacity exceeded"));
            }
            Ok(work)
        })
        .await
        .map_err(|e| Fault::new("work_storage", e.to_string()))??;
        let mut children = Vec::new();
        for (id, record) in work.records.iter_mut() {
            if matches!(record.state.as_str(), "starting" | "running" | "cancelling") {
                if let Some(Scope {
                    id: misa_proto::observation::ScopeId::Session { id },
                    ..
                }) = &record.child
                {
                    if record.parent_bound {
                        children.push(id.clone());
                    }
                } else {
                    // Membership may have committed immediately before the
                    // relationship checkpoint; the child identity is allocated
                    // from the durable work identity, never guessed from UI state.
                    if record.parent_bound {
                        children.push(id.replace(':', "-"));
                    }
                }
                record.state = "interrupted".into();
                record.result = crate::lifecycle::encode(&Outcome::Indeterminate {
                    fault: Fault::new(
                        "interrupted",
                        "Daemon restarted; child work is not replayed",
                    ),
                });
            }
        }
        self.forget_bound_children(&children).await?;
        *self.edit_work() = work;
        *self.work_path.lock().unwrap() = Some(path);
        self.persist_work().await?;
        Ok(())
    }
    async fn persist_work(&self) -> Result<(), Fault> {
        let _guard = self.work_writes.lock().await;
        let Some(path) = self.work_path.lock().unwrap().clone() else {
            return Ok(());
        };
        let work = self.work.lock().unwrap().clone();
        tokio::task::spawn_blocking(move || crate::membership::write_atomic(&path, &work))
            .await
            .map_err(|e| Fault::new("work_storage", e.to_string()))?
    }
    async fn settle_work(&self, id: &str, outcome: Outcome) -> Outcome {
        let _guard = self.work_writes.lock().await;
        let mut candidate = self.work.lock().unwrap().clone();
        let Some(record) = candidate.records.get_mut(id) else {
            return Outcome::Indeterminate {
                fault: Fault::new("unavailable_operation", "Work record is unavailable"),
            };
        };
        if !matches!(record.state.as_str(), "starting" | "running" | "cancelling") {
            return decoded_outcome(&record.result);
        }
        record.state = match &outcome {
            Outcome::Completed { .. } => "succeeded",
            Outcome::Indeterminate { fault } if fault.code == "cancelled" => "cancelled",
            _ => "interrupted",
        }
        .into();
        record.result = crate::lifecycle::encode(&outcome);
        let terminal = record.clone();
        let path = self.work_path.lock().unwrap().clone();
        let saved = match path {
            Some(path) => tokio::task::spawn_blocking(move || {
                crate::membership::write_atomic(&path, &candidate)
            })
            .await
            .map_err(|error| Fault::new("work_storage", error.to_string()))
            .and_then(|result| result),
            None => Ok(()),
        };
        match saved {
            Ok(()) => {
                self.edit_work().records.insert(id.into(), terminal);
                outcome
            }
            Err(fault) => {
                let outcome = Outcome::Indeterminate { fault };
                if let Some(record) = self.edit_work().records.get_mut(id) {
                    record.state = "interrupted".into();
                    record.result = crate::lifecycle::encode(&outcome);
                }
                outcome
            }
        }
    }
    async fn stop_child(&self, child: &Runtime) -> Result<(), Fault> {
        match self.close(&child.scope()).await {
            Ok(_) => Ok(()),
            Err(fault) => {
                child.shutdown_complete().await;
                Err(fault)
            }
        }
    }
    fn delegate(
        self: &Arc<Self>,
        runtime: &Runtime,
        context: &CallContext,
        invocation: &Invocation,
        parent_bound: bool,
    ) -> Outcome {
        let mut supervisors = self.supervisors.lock().unwrap();
        if self.is_closed() {
            return rejected("closed_scope", "Daemon is shutting down");
        }
        supervisors.retain(|task| !task.is_finished());
        let mut task = invocation
            .input
            .get("task")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let input_context = invocation
            .input
            .get("context")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if task.trim().is_empty() {
            return rejected("invalid_work", "Work requires a nonempty task");
        }
        if !input_context.is_empty() {
            task = format!("Context:\n{input_context}\n\nTask:\n{task}");
        }
        let provider = invocation
            .input
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or(runtime.provider())
            .to_owned();
        let model = invocation
            .input
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(runtime.model())
            .to_owned();
        if provider.len() > 256 || model.len() > 256 {
            return rejected(
                "invalid_work",
                "Provider or model identifier exceeds its bound",
            );
        }
        let mut encoded = Vec::new();
        ciborium::ser::into_writer(&invocation.input, &mut encoded)
            .expect("validated arguments serialize");
        let input_hash = blake3::hash(&encoded).to_hex().to_string();
        let key = invocation
            .input
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if task.is_empty() || task.len() > 1024 * 1024 || key.is_empty() || key.len() > 128 {
            return rejected(
                "invalid_work",
                "Work requires a bounded key and nonempty task",
            );
        }
        if context.principal.is_empty() {
            return rejected("unauthorized", "Work requires an authenticated caller");
        }
        let parent = runtime.scope();
        let (parent_operation, parent_attempt) = runtime.work_context();
        let Some(parent_runtime) = self.session(&parent) else {
            return rejected("closed_scope", "Parent session is unavailable");
        };
        let id = {
            let mut work = self.edit_work();
            if let Some((id, r)) = work.records.iter().find(|(_, r)| {
                r.parent == parent && r.principal == context.principal && r.key == key
            }) {
                if r.input_hash != input_hash || r.parent_bound != parent_bound {
                    return rejected(
                        "work_conflict",
                        "Work key already names different input or lifetime",
                    );
                }
                if !parent_bound {
                    return Outcome::Completed {
                        value: crate::lifecycle::encode(&OperationRef {
                            scope: self.scope_value(),
                            id: id.clone(),
                        }),
                    };
                }
                return if matches!(r.state.as_str(), "running" | "starting" | "cancelling") {
                    rejected("work_pending", &format!("Work {id} is already pending"))
                } else {
                    decoded_outcome(&r.result)
                };
            }
            // New children cannot introduce an edge to an existing ancestor. Bound
            // depth additionally prevents recursive delegation from exhausting owners.
            let mut cursor = parent.clone();
            let mut depth = 0;
            while let Some(r) = work
                .records
                .values()
                .find(|r| r.child.as_ref() == Some(&cursor))
            {
                depth += 1;
                cursor = r.parent.clone();
                if depth >= 8 {
                    return rejected("depth", "Delegation depth exceeded");
                }
            }
            if work.records.len() >= 256 {
                return rejected("busy", "Delegation capacity reached");
            }
            let Some(next) = work.next.checked_add(1) else {
                return rejected("exhausted", "Work identities exhausted");
            };
            work.next = next;
            let id = format!("work:{}:{}", self.scope_value().incarnation, work.next);
            work.records.insert(
                id.clone(),
                Record {
                    conversation: None,
                    usage: BTreeMap::new(),
                    parent,
                    parent_bound,
                    parent_operation: parent_operation.clone(),
                    parent_attempt: parent_attempt.clone(),
                    child_operation: None,
                    principal: context.principal.clone(),
                    key,
                    input_hash,
                    child: None,
                    state: "starting".into(),
                    result: Value::Null,
                },
            );
            id
        };
        let directory = Arc::downgrade(self);
        let parent = Arc::downgrade(&parent_runtime);
        let context = context.clone();
        let invocation_id = invocation.id;
        let work_id = id.clone();
        let supervisor = tokio::spawn(async move {
            let Some(directory) = directory.upgrade() else {
                return;
            };
            let result = match directory.persist_work().await {
                Ok(()) => {
                    directory
                        .run_child(
                            &work_id,
                            &parent,
                            parent_operation,
                            parent_attempt,
                            parent_bound,
                            provider,
                            model,
                            &context,
                            task,
                        )
                        .await
                }
                Err(fault) => Err(fault),
            };
            let outcome = match result {
                Ok(value) => Outcome::Completed { value },
                Err(fault) => Outcome::Indeterminate { fault },
            };
            let outcome = directory.settle_work(&work_id, outcome).await;
            if let Some(parent) = parent.upgrade().filter(|_| parent_bound) {
                parent.complete_tool_invocation(invocation_id, outcome);
            }
        });
        supervisors.push(supervisor);
        let operation = OperationRef {
            scope: self.scope_value(),
            id,
        };
        if parent_bound {
            Outcome::Accepted { operation }
        } else {
            Outcome::Completed {
                value: crate::lifecycle::encode(&operation),
            }
        }
    }
    async fn run_child(
        &self,
        id: &str,
        parent: &Weak<Runtime>,
        parent_operation: Option<String>,
        parent_attempt: Option<String>,
        parent_bound: bool,
        provider: String,
        model: String,
        context: &CallContext,
        task: String,
    ) -> Result<Value, Fault> {
        if self
            .work
            .lock()
            .unwrap()
            .records
            .get(id)
            .is_some_and(|r| r.state == "cancelling")
        {
            return Err(Fault::new(
                "cancelled",
                "Delegation cancelled before child admission",
            ));
        }
        let mut parent_changed = parent.upgrade().map(|runtime| runtime.watch_rev());
        let child = self
            .open(
                context,
                SessionSpec {
                    id: id.replace(':', "-"),
                    title: "Delegated work".into(),
                    conversation: None,
                    provider: Some(provider),
                    model: Some(model),
                    parent_attempt,
                    recovering: false,
                },
            )
            .await?;
        {
            let mut work = self.edit_work();
            let record = work.records.get_mut(id).unwrap();
            record.child = Some(child.scope());
            record.conversation = Some(
                child
                    .info()
                    .conversation
                    .unwrap_or_else(|| child.id().into()),
            );
            if record.state != "cancelling" {
                record.state = "running".into();
            }
        }
        if let Err(fault) = self.persist_work().await {
            self.stop_child(&child).await?;
            return Err(fault);
        }
        let mut changed = child.watch_rev();
        let mut work_changed = self.watch_work();
        if parent_bound && parent_stopped(parent, parent_operation.as_deref()) {
            self.stop_child(&child).await?;
            return Err(Fault::new(
                "cancelled",
                "Parent stopped before child prompt admission",
            ));
        }
        if self
            .work
            .lock()
            .unwrap()
            .records
            .get(id)
            .is_some_and(|r| r.state == "cancelling")
        {
            self.stop_child(&child).await?;
            return Err(Fault::new(
                "cancelled",
                "Delegation cancelled before prompt admission",
            ));
        }
        let Outcome::Accepted { operation } = child
            .execute(
                context,
                Invocation {
                    id: 1,
                    scope: child.scope(),
                    command: "session.prompt".into(),
                    input: Value::map([("text", Value::str(task))]),
                },
            )
            .await
        else {
            self.stop_child(&child).await?;
            return Err(Fault::new("child_rejected", "Child prompt was rejected"));
        };
        self.edit_work()
            .records
            .get_mut(id)
            .unwrap()
            .child_operation = Some(operation.id.clone());
        if let Err(fault) = self.persist_work().await {
            self.stop_child(&child).await?;
            return Err(fault);
        }
        loop {
            let parent_ended = parent_bound && parent_stopped(parent, parent_operation.as_deref());
            let cancelled = self
                .work
                .lock()
                .unwrap()
                .records
                .get(id)
                .is_some_and(|r| r.state == "cancelling");
            if parent_ended || child.is_closed() || cancelled {
                self.stop_child(&child).await?;
                return Err(Fault::new(
                    if cancelled || parent_ended {
                        "cancelled"
                    } else {
                        "interrupted"
                    },
                    "Delegation owner stopped work",
                ));
            }
            let selection = misa_proto::observation::Selection {
                scope: child.scope(),
                members: [
                    ("result", "operation.result", "operation.result@1"),
                    ("output", "operation.output", "operation.output@1"),
                    ("usage", "operation.usage", "operation.usage@1"),
                ]
                .into_iter()
                .map(|(name, query, contract)| {
                    (
                        name.into(),
                        misa_proto::observation::Member {
                            query: Query::new(query).arg(Value::str(&operation.id)),
                            contract: contract.into(),
                            encoding: misa_proto::observation::Encoding::Value,
                            optional: false,
                        },
                    )
                })
                .collect(),
            };
            if let Ok(snapshot) = child.read_selection(&selection) {
                let Some(misa_proto::observation::Content::Value(result)) =
                    snapshot.members.get("result")
                else {
                    return Err(Fault::new("invalid_result", "Child result missing"));
                };
                if result.get("terminal").and_then(Value::as_bool) == Some(true) {
                    if let Some(misa_proto::observation::Content::Value(attempts)) =
                        snapshot.members.get("usage")
                    {
                        self.edit_work().records.get_mut(id).unwrap().usage = attempts
                            .as_list()
                            .unwrap_or(&[])
                            .iter()
                            .filter_map(|attempt| {
                                Some((
                                    attempt.get("name")?.as_str()?.to_owned(),
                                    Usage::read(attempt),
                                ))
                            })
                            .collect();
                    }
                    let succeeded =
                        result.get("state").and_then(Value::as_str) == Some("succeeded");
                    let value = Value::map([
                        ("operation", crate::lifecycle::encode(&operation)),
                        (
                            "conversation",
                            Value::str(
                                child
                                    .info()
                                    .conversation
                                    .unwrap_or_else(|| child.id().into()),
                            ),
                        ),
                        ("result", result.clone()),
                        (
                            "source_position",
                            Value::Int(i64::try_from(snapshot.position).unwrap_or(i64::MAX)),
                        ),
                        (
                            "output",
                            match snapshot.members.get("output") {
                                Some(misa_proto::observation::Content::Value(value)) => {
                                    value.clone()
                                }
                                _ => Value::Null,
                            },
                        ),
                    ]);
                    self.stop_child(&child).await?;
                    if Schema::Value
                        .validate_with(
                            &value,
                            misa_proto::schema::Limits {
                                bytes: RESULT_BYTES,
                                ..Default::default()
                            },
                        )
                        .is_err()
                    {
                        return Err(Fault::new(
                            "result_limit",
                            "Child completed, but its result exceeds the model result budget; read its persisted conversation",
                        ));
                    }
                    return if succeeded {
                        Ok(value)
                    } else {
                        Err(Fault::new("child_failed", "Child work did not succeed"))
                    };
                }
            }
            tokio::select! { _=changed.changed()=>{}, _=async {match parent_changed.as_mut() {Some(changed) if parent_bound=>{let _=changed.changed().await;},_=>std::future::pending::<()>().await}}=>{},_=work_changed.changed()=>{} }
        }
    }
}
