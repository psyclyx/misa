//! Authoritative daemon session lifecycle. Composition stays in the host factory.
use crate::directory::Directory;
use misa_proto::{
    Fault,
    invocation::{Command, Invocation, Outcome},
    observation::{Scope, ScopeId},
    schema::{Field, Schema},
};
use misa_protocol::invocation::CallContext;
use misa_session::Runtime;
use misa_value::Value;
use std::{future::Future, pin::Pin, sync::Arc};

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SessionSpec {
    pub id: String,
    pub title: String,
    pub conversation: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub parent_attempt: Option<String>,
    #[serde(skip)]
    pub recovering: bool,
}
/// Host composition returns `Runtime::prepare_with`, with no startup effects.
/// The directory commits desired membership before calling `activate`.
pub type Factory = dyn Fn(
        CallContext,
        SessionSpec,
    ) -> Pin<Box<dyn Future<Output = Result<Arc<Runtime>, Fault>> + Send>>
    + Send
    + Sync;
fn record(fields: impl IntoIterator<Item = (&'static str, bool)>) -> Schema {
    Schema::Record {
        fields: fields
            .into_iter()
            .map(|(name, optional)| {
                (
                    name.into(),
                    Field {
                        schema: Schema::String,
                        optional,
                    },
                )
            })
            .collect(),
        allow_unknown: false,
    }
}
pub fn commands() -> Vec<Command> {
    let result = record([("id", false), ("incarnation", false)]);
    let mut commands = vec![
        Command {
            preparation: Default::default(),
            id: "daemon.session.create".into(),
            input: record([
                ("id", false),
                ("title", true),
                ("provider", true),
                ("model", true),
            ]),
            result: result.clone(),
        },
        Command {
            preparation: Default::default(),
            id: "daemon.session.resume".into(),
            input: record([
                ("id", false),
                ("conversation", false),
                ("title", true),
                ("provider", true),
                ("model", true),
            ]),
            result,
        },
        Command {
            preparation: Default::default(),
            id: "daemon.session.close".into(),
            input: record([("id", false), ("incarnation", false)]),
            result: Schema::Value,
        },
        Command {
            preparation: Default::default(),
            id: "operation.cancel".into(),
            input: Schema::Record {
                fields: [
                    (
                        "operation".into(),
                        Field {
                            schema: Schema::String,
                            optional: false,
                        },
                    ),
                    (
                        "generation".into(),
                        Field {
                            schema: Schema::Int,
                            optional: false,
                        },
                    ),
                ]
                .into_iter()
                .collect(),
                allow_unknown: false,
            },
            result: Schema::Value,
        },
    ];
    let mut forget = commands.last().unwrap().clone();
    forget.id = "daemon.work.forget".into();
    commands.push(forget);
    commands
}
fn text(input: &Value, field: &str) -> String {
    input
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn optional(input: &Value, field: &str) -> Option<String> {
    input.get(field).and_then(Value::as_str).map(str::to_owned)
}
pub(crate) async fn execute(
    directory: &Directory,
    context: &CallContext,
    invocation: Invocation,
) -> Outcome {
    if matches!(
        invocation.command.as_str(),
        "operation.cancel" | "daemon.work.forget"
    ) && invocation.scope == directory.scope_value()
    {
        let definition = commands()
            .into_iter()
            .find(|c| c.id == invocation.command)
            .unwrap();
        if let Err(fault) = invocation.validate(&definition, Default::default()) {
            return Outcome::Rejected { fault };
        }
        return if invocation.command == "operation.cancel" {
            directory.cancel_work(context, &invocation)
        } else {
            directory.forget_work(context, &invocation).await
        };
    }
    let result = async {
        if context.principal.is_empty() {
            return Err(Fault::new(
                "unauthorized",
                "Lifecycle requires an authenticated caller",
            ));
        }
        if invocation.scope != directory.scope_value() {
            return Err(Fault::new(
                "stale_scope",
                "Daemon incarnation does not match",
            ));
        }
        let definition = commands()
            .into_iter()
            .find(|command| command.id == invocation.command)
            .ok_or_else(|| Fault::unsupported("Lifecycle command is not installed"))?;
        invocation.validate(&definition, Default::default())?;
        let id = text(&invocation.input, "id");
        if id.is_empty() || id.len() > 128 || id.chars().any(|ch| ch.is_control()) {
            return Err(Fault::new("invalid_session", "Session identity is invalid"));
        }
        if invocation.command == "daemon.session.close" {
            let scope = Scope {
                id: ScopeId::Session { id },
                incarnation: text(&invocation.input, "incarnation"),
            };
            if !directory.close(&scope).await? {
                return Err(Fault::new(
                    "unavailable_scope",
                    "Session incarnation is unavailable",
                ));
            }
            return Ok(Value::Null);
        }
        let runtime = directory
            .open(
                context,
                SessionSpec {
                    title: optional(&invocation.input, "title").unwrap_or_else(|| id.clone()),
                    id,
                    conversation: if invocation.command == "daemon.session.resume" {
                        Some(text(&invocation.input, "conversation"))
                    } else {
                        None
                    },
                    provider: optional(&invocation.input, "provider"),
                    model: optional(&invocation.input, "model"),
                    parent_attempt: None,
                    recovering: false,
                },
            )
            .await?;
        Ok(resource(&runtime.scope()))
    }
    .await;
    match result {
        Ok(value) => Outcome::Completed { value },
        Err(fault) => Outcome::Rejected { fault },
    }
}
pub(crate) fn resource(scope: &Scope) -> Value {
    let ScopeId::Session { id } = &scope.id else {
        unreachable!()
    };
    Value::map([
        ("id", Value::str(id)),
        ("incarnation", Value::str(&scope.incarnation)),
    ])
}
pub(crate) fn encode<T: serde::Serialize>(value: &T) -> Value {
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(value, &mut bytes).expect("installed schema serializes");
    ciborium::de::from_reader(&bytes[..]).expect("installed schema is a value")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::membership::{MembershipStore, Memory};
    fn context() -> CallContext {
        CallContext {
            principal: "owner".into(),
            connection: 1,
        }
    }
    fn spec(id: &str, conversation: Option<&str>) -> SessionSpec {
        SessionSpec {
            id: id.into(),
            title: id.into(),
            conversation: conversation.map(str::to_owned),
            provider: None,
            model: None,
            parent_attempt: None,
            recovering: false,
        }
    }
    fn directory() -> Arc<Directory> {
        let directory = Directory::fresh().unwrap();
        directory
            .install_factory(Arc::new(|_, spec| {
                Box::pin(async move {
                    Ok(Runtime::prepare_with(
                        spec.id,
                        spec.title,
                        spec.conversation,
                        Arc::new(misa_kernel::LocalKernel::new(
                            misa_kernel::ScriptedProvider::always("done"),
                        )),
                        "scripted",
                        "test",
                        Value::Null,
                        misa_session::Contribution::default(),
                    ))
                })
            }))
            .unwrap();
        directory
    }
    #[tokio::test]
    async fn membership_restores_new_incarnation_and_close_removes_desire() {
        let storage = Arc::new(Memory::default());
        let first = directory();
        first.install_membership(storage.clone()).await.unwrap();
        let runtime = first.open(&context(), spec("one", None)).await.unwrap();
        let old = runtime.scope();
        assert_eq!(
            first
                .open(&context(), spec("alias", Some("one")))
                .await
                .err()
                .unwrap()
                .code,
            "conversation_busy"
        );
        first.shutdown_complete().await;
        assert!(runtime.is_closed());
        assert_eq!(storage.load().unwrap().len(), 1);
        let second = directory();
        second.install_membership(storage.clone()).await.unwrap();
        assert!(second.restore(&context()).await.is_empty());
        let restored = second.sessions().pop().unwrap();
        assert_ne!(old, restored.scope());
        assert!(!second.close(&old).await.unwrap());
        assert!(second.close(&restored.scope()).await.unwrap());
        assert!(restored.is_closed());
        assert!(storage.load().unwrap().is_empty());
        assert!(second.sessions().is_empty());
    }
    struct Failing;
    impl MembershipStore for Failing {
        fn load(&self) -> Result<Vec<SessionSpec>, Fault> {
            Ok(vec![])
        }
        fn save(&self, _: &[SessionSpec]) -> Result<(), Fault> {
            Err(Fault::new("storage", "failed"))
        }
    }
    #[derive(Default)]
    struct ClosingKernel {
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
        closed: std::sync::atomic::AtomicBool,
    }
    impl misa_kernel::Kernel for ClosingKernel {
        fn execute<'a, 'b, 'f>(
            &'a self,
            _: misa_kernel::Request,
            _: &'b tokio::sync::mpsc::UnboundedSender<misa_kernel::KernelEvent>,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'f>>
        where
            'a: 'f,
            'b: 'f,
            Self: 'f,
        {
            Box::pin(async {})
        }
        fn close<'a, 'b, 'f>(
            &'a self,
            _: &'b tokio::sync::mpsc::UnboundedSender<misa_kernel::KernelEvent>,
        ) -> Pin<Box<dyn Future<Output = ()> + Send + 'f>>
        where
            'a: 'f,
            'b: 'f,
            Self: 'f,
        {
            Box::pin(async move {
                self.entered.notify_one();
                self.release.notified().await;
                self.closed
                    .store(true, std::sync::atomic::Ordering::Release);
            })
        }
    }
    #[tokio::test]
    async fn failed_creation_and_shutdown_await_kernel_release() {
        for failure in [true, false] {
            let directory = Directory::fresh().unwrap();
            let kernel = Arc::new(ClosingKernel::default());
            let installed = kernel.clone();
            directory
                .install_factory(Arc::new(move |_, spec| {
                    let kernel = installed.clone();
                    Box::pin(async move {
                        Ok(Runtime::prepare_with(
                            spec.id,
                            spec.title,
                            spec.conversation,
                            kernel,
                            "scripted",
                            "test",
                            Value::Null,
                            misa_session::Contribution::default(),
                        ))
                    })
                }))
                .unwrap();
            if failure {
                directory
                    .install_membership(Arc::new(Failing))
                    .await
                    .unwrap();
            } else {
                directory.open(&context(), spec("one", None)).await.unwrap();
            }
            let task_owner = directory.clone();
            let task = tokio::spawn(async move {
                if failure {
                    assert!(
                        task_owner
                            .open(&context(), spec("one", None))
                            .await
                            .is_err()
                    );
                } else {
                    task_owner.shutdown_complete().await;
                }
            });
            tokio::time::timeout(std::time::Duration::from_secs(1), kernel.entered.notified())
                .await
                .unwrap();
            assert!(
                !task.is_finished(),
                "lifecycle cannot finish before kernel capabilities are released"
            );
            kernel.release.notify_one();
            tokio::time::timeout(std::time::Duration::from_secs(1), task)
                .await
                .unwrap()
                .unwrap();
            assert!(kernel.closed.load(std::sync::atomic::Ordering::Acquire));
            assert!(directory.sessions().is_empty());
            directory.shutdown_complete().await;
        }
    }
    #[tokio::test]
    async fn failed_membership_commit_does_not_publish_session() {
        let directory = directory();
        directory
            .install_membership(Arc::new(Failing))
            .await
            .unwrap();
        assert!(directory.open(&context(), spec("one", None)).await.is_err());
        assert!(directory.sessions().is_empty());
    }
    #[tokio::test]
    async fn shutdown_fences_and_cancels_pending_composition() {
        let directory = Directory::fresh().unwrap();
        let entered = Arc::new(tokio::sync::Notify::new());
        let signal = entered.clone();
        directory
            .install_factory(Arc::new(move |_, _| {
                let signal = signal.clone();
                Box::pin(async move {
                    signal.notify_one();
                    std::future::pending::<Result<Arc<Runtime>, Fault>>().await
                })
            }))
            .unwrap();
        let opening = directory.clone();
        let task = tokio::spawn(async move { opening.open(&context(), spec("one", None)).await });
        entered.notified().await;
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            directory.shutdown_complete(),
        )
        .await
        .expect("shutdown cancels pending factory");
        assert_eq!(task.await.unwrap().err().unwrap().code, "closed_scope");
        assert!(directory.sessions().is_empty());
        assert_eq!(
            directory
                .open(&context(), spec("late", None))
                .await
                .err()
                .unwrap()
                .code,
            "closed_scope"
        );
    }
    #[tokio::test]
    async fn concurrent_sessions_have_distinct_attempt_identity_in_shared_store() {
        use misa_protocol::invocation::CommandOwner;
        let kernel = Arc::new(misa_kernel::LocalKernel::new(
            misa_kernel::ScriptedProvider::new([
                misa_kernel::Turn::say("one"),
                misa_kernel::Turn::say("two"),
            ]),
        ));
        let store = kernel.store().clone();
        let first = Runtime::start(
            "one",
            "one",
            None,
            kernel.clone(),
            "scripted",
            "test",
            Value::Null,
        );
        let second = Runtime::start("two", "two", None, kernel, "scripted", "test", Value::Null);
        for runtime in [&first, &second] {
            assert!(matches!(
                runtime
                    .execute(
                        &context(),
                        Invocation {
                            id: 1,
                            scope: runtime.scope(),
                            command: "session.prompt".into(),
                            input: Value::map([("text", Value::str("go"))])
                        }
                    )
                    .await,
                Outcome::Accepted { .. }
            ));
        }
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let attempts = store.attempts(None).unwrap();
                if attempts.len() == 2 && attempts.iter().all(|a| a.status == "ok") {
                    assert_ne!(attempts[0].name, attempts[1].name);
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        first.shutdown_complete().await;
        second.shutdown_complete().await;
    }
}
