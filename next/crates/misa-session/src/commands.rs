//! Installed command contracts and owner-side handlers. Bindings carry no authority.
use crate::Runtime;
use misa_reframe::Event;
use misa_proto::{
    Fault,
    invocation::{Command, Invocation, Outcome},
    schema::{Field, Literal, Schema},
};
use misa_protocol::invocation::{CallContext, CommandOwner, Execution};
use misa_value::Value;
use std::{collections::BTreeMap, sync::Arc};

pub type Handler = Arc<dyn Fn(&Runtime, &CallContext, &Invocation) -> Outcome + Send + Sync>;
#[derive(Clone)]
pub struct CommandRegistration {
    pub definition: Command,
    pub handler: Handler,
    event: Option<String>,
}
impl CommandRegistration {
    pub fn new(
        definition: Command,
        handler: impl Fn(&Runtime, &CallContext, &Invocation) -> Outcome + Send + Sync + 'static,
    ) -> Self {
        Self {
            definition,
            handler: Arc::new(handler),
            event: None,
        }
    }
    /// Installation binds one declared event. No caller can select the event name.
    /// The accepted operation completes when its state transaction is journaled.
    /// External effects require an explicit operation-aware handler instead.
    pub fn event(id: impl Into<String>, input: Schema, event: impl Into<String>) -> Self {
        let event = event.into();
        let event_name = event.clone();
        let mut registration = Self::new(
            Command {
                preparation: Default::default(), id: id.into(),
                input,
                result: admitted(),
            },
            move |runtime, context, invocation| {
                runtime.execute_event_transaction(context, invocation, &event)
            },
        );
        registration.event = Some(event_name);
        registration
    }
    pub fn event_kind(&self) -> Option<&str> {
        self.event.as_deref()
    }
    /// A declared non-secret form precedes the same durable event transaction.
    pub fn input_event(id: impl Into<String>, input: Schema, event: impl Into<String>, form: misa_proto::input::Form) -> Result<Self, Fault> {
        form.validate()?;
        let event = event.into();
        let event_name = event.clone();
        let mut registration = Self::new(Command { preparation: Default::default(), id: id.into(), input, result: admitted() }, move |runtime, context, invocation| runtime.request_transaction_input(context, invocation, form.clone(), &event));
        registration.event = Some(event_name);
        Ok(registration)
    }
}
pub fn install(
    contributed: &[CommandRegistration],
) -> Result<BTreeMap<String, CommandRegistration>, Fault> {
    let mut installed = BTreeMap::new();
    for registration in builtins().into_iter().chain(crate::operations::commands()).chain(contributed.iter().cloned()) {
        registration.definition.validate()?;
        if installed
            .insert(registration.definition.id.clone(), registration)
            .is_some()
        {
            return Err(Fault::protocol("Duplicate installed command"));
        }
    }
    Ok(installed)
}

pub fn catalogs(
    installed: &BTreeMap<String, CommandRegistration>,
    bindings: &[misa_proto::invocation::Binding],
) -> Result<Vec<(misa_proto::query::Definition, Value)>, Fault> {
    let mut ids = std::collections::BTreeSet::new();
    for declaration in bindings {
        if declaration.id.is_empty() || !ids.insert(&declaration.id) {
            return Err(Fault::protocol("Duplicate or empty action binding"));
        }
        let binding = &declaration.binding;
        binding.validate()?;
        let command = installed
            .get(&binding.command)
            .ok_or_else(|| Fault::protocol("Binding names uninstalled command"))?;
        binding.validate_for(&command.definition)?;
    }
    let commands = installed
        .values()
        .map(|entry| entry.definition.clone())
        .collect::<Vec<_>>();
    let binding_schema = record([
        ("id", Schema::String, false),
        ("binding", Schema::Value, false),
    ]);
    let shortcuts = shortcuts(installed);
    let candidates: Vec<misa_proto::view::Choice> = shortcuts.iter().map(|shortcut| {
        let args = shortcut.args.iter().map(|arg| arg.label.as_str()).collect::<Vec<_>>().join(" ");
        misa_proto::view::Choice {
            value: format!("/{}", shortcut.id), label: format!("/{}", shortcut.id),
            detail: Some(if args.is_empty() { shortcut.description.clone() } else { format!("{} — {args}", shortcut.description) }),
        }
    }).collect();
    Ok(vec![
        (crate::completions::command_definition(), crate::wire::render(&candidates)),
        (
            misa_proto::query::Definition {
                id: misa_proto::preparation::SHORTCUTS.into(), arguments: vec![],
                contract: "commands.shortcuts@1".into(),
                result: misa_proto::query::ResultContract::Data { schema: Schema::List { items: Box::new(Schema::Value) } },
            },
            crate::wire::render(&shortcuts),
        ),
        (
            misa_proto::invocation::catalog_definition(),
            crate::wire::render(&commands),
        ),
        (
            misa_proto::query::Definition {
                id: misa_proto::invocation::BINDING_CATALOG.into(),
                arguments: vec![],
                contract: "actions.catalog@1".into(),
                result: misa_proto::query::ResultContract::Data {
                    schema: Schema::List {
                        items: Box::new(binding_schema),
                    },
                },
            },
            crate::wire::render(&bindings),
        ),
    ])
}
fn shortcuts(installed: &BTreeMap<String, CommandRegistration>) -> Vec<misa_proto::preparation::Shortcut> {
    use misa_proto::{preparation::{Shortcut, Target}, observation::{Member, Encoding}, Query};
    crate::catalog::commands().into_iter().filter_map(|mut entry| {
        let target = match entry.id.as_str() {
            "status" | "usage" => {
                let (id, encoding) = if entry.id == "status" { ("session.status-report", Encoding::Value) } else { ("usage.presentation", Encoding::Document) };
                Target::Read { member: Member { query: Query::new(id), contract: format!("{id}@1"), encoding, optional: false } }
            }
            other => {
                let command = match other {
                    "model" => "session.model.select", "effort" => "session.effort.select",
                    "clear" => "session.clear", "compact" => "session.compact",
                    "attach" | "image" => "session.attachment.add",
                    "models" => "session.models.refresh",
                    "login" => "credentials.authorize",
                    _ => return None,
                };
                if !installed.contains_key(command) { return None; }
                if other == "effort" { entry.args[0].name = "effort".into(); }
                Target::Command { command: command.into() }
            }
        };
        Some(Shortcut { id: entry.id, label: entry.label, description: entry.description, args: entry.args, target })
    }).collect()
}
fn admitted() -> Schema {
    Schema::Choice {
        values: vec![Literal::Null],
    }
}
fn record(fields: impl IntoIterator<Item = (&'static str, Schema, bool)>) -> Schema {
    Schema::Record {
        fields: fields
            .into_iter()
            .map(|(name, schema, optional)| (name.into(), Field { schema, optional }))
            .collect(),
        allow_unknown: false,
    }
}
fn outcome(faults: Vec<Fault>) -> Outcome {
    match faults.into_iter().next() {
        Some(fault) => Outcome::Rejected { fault },
        None => Outcome::Completed { value: Value::Null },
    }
}
fn text(input: &Value, name: &str) -> String {
    input
        .get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}
fn transition_registration(
    id: &str,
    input: Schema,
    prepare: impl Fn(&Value) -> Event + Send + Sync + 'static,
) -> CommandRegistration {
    CommandRegistration::new(
        Command {
            preparation: Default::default(), id: id.into(),
            input,
            result: admitted(),
        },
        move |runtime, _, invocation| outcome(runtime.dispatch(prepare(&invocation.input))),
    )
}
pub fn builtins() -> Vec<CommandRegistration> {
    let prompt = || {
        record([
            ("text", Schema::String, false),
            (
                "attachments",
                Schema::List {
                    items: Box::new(record([
                        ("hash", Schema::String, false),
                        ("len", Schema::Int, false),
                        (
                            "media",
                            Schema::Nullable {
                                inner: Box::new(Schema::String),
                            },
                            true,
                        ),
                    ])),
                },
                true,
            ),
        ])
    };
    let mut registrations = vec![
        CommandRegistration::new(Command {
            preparation: Default::default(), id: "session.attachment.resolve".into(),
            input: record([("node", Schema::String, false)]),
            result: record([("hash", Schema::String, false), ("len", Schema::Int, false), ("media", Schema::Nullable { inner: Box::new(Schema::String) }, true)]),
        }, |runtime, _, invocation| {
            let state = runtime.state.lock().expect("session state is never poisoned");
            let node = text(&invocation.input, "node");
            let Some(target) = state.view.tree.node(&node).filter(|node| node.actions.iter().any(|action| action.id == "attachment.save")) else {
                return Outcome::Rejected { fault: Fault::unsupported("This node does not offer an attachment to save") };
            };
            let misa_proto::view::Kind::Image { blob, .. } = &target.kind else {
                return Outcome::Rejected { fault: Fault::unsupported("This node is not an attachment") };
            };
            Outcome::Completed { value: crate::wire::render(blob) }
        }),
        CommandRegistration::new(Command { preparation: Default::default(), id: "session.prompt".into(), input: prompt(), result: admitted() },
            |runtime, context, invocation| crate::operations::prompt(runtime, context, invocation, false)),
        CommandRegistration::new(Command { preparation: Default::default(), id: "session.interrupt".into(), input: prompt(), result: admitted() },
            |runtime, context, invocation| crate::operations::prompt(runtime, context, invocation, true)),
        transition_registration(
            "session.cancel",
            record([(
                "target",
                Schema::Nullable {
                    inner: Box::new(Schema::String),
                },
                true,
            )]),
            |input| {
                Event::new("intent/cancel").with("target", input.get("target").cloned().unwrap_or(Value::Null))
            },
        ),
        transition_registration(
            "session.model.select",
            record([("model", Schema::String, false)]),
            |input| {
                Event::new("intent/command").with("name", Value::str("model")).with("args", Value::str(text(input, "model")))
            },
        ),
        transition_registration(
            "session.effort.select",
            record([("effort", Schema::String, false)]),
            |input| {
                Event::new("intent/command").with("name", Value::str("effort")).with("args", Value::str(text(input, "effort")))
            },
        ),
    ];
    for (id, name, argument, required) in [
        ("session.clear", "clear", "reason", false),
        ("session.compact", "compact", "", false),
        ("session.attachment.add", "attach", "path", true),
    ] {
        let input = if argument.is_empty() { record([]) } else { record([(argument, Schema::String, !required)]) };
        registrations.push(transition_registration(id, input, move |input|
            Event::new("intent/command").with("name", Value::str(name)).with("args", Value::str(text(input, argument)))
        ));
    }
    for (id, event) in [
        ("session.models.refresh", "discovery/models.refresh"),
        ("session.usage.refresh", "discovery/usage.refresh"),
        ("session.conversations.refresh", "discovery/conversations.refresh"),
    ] {
        registrations.push(CommandRegistration::new(Command { preparation: Default::default(), id: id.into(), input: record([]), result: admitted() },
            move |runtime, _, _| outcome(runtime.dispatch(misa_reframe::Event::new(event)))));
    }
    registrations
}

impl CommandOwner for Runtime {
    fn scope(&self) -> misa_proto::observation::Scope {
        Runtime::scope(self)
    }
    fn command(&self, _context: &CallContext, id: &str) -> Option<Command> {
        self.command_registry
            .get(id)
            .map(|registration| registration.definition.clone())
    }
    fn execute<'a>(&'a self, context: &'a CallContext, invocation: Invocation) -> Execution<'a> {
        Box::pin(async move { self.execute_command(context, invocation) })
    }
}
impl Runtime {
    pub(crate) fn execute_command(&self, context:&CallContext, invocation:Invocation)->Outcome {

            if self.is_closed() { return Outcome::Rejected { fault: Fault::new("closed_scope", "Session owner is closed") }; }
            if !self.is_started() {return Outcome::Rejected{fault:Fault::new("not_ready","Session owner has not been activated")};}
            if invocation.scope != self.scope() {
                return Outcome::Rejected {
                    fault: Fault::new(
                        "invalid_command",
                        "Command targets an old owner incarnation",
                    ),
                };
            }
            let Some(registration) = self.command_registry.get(&invocation.command) else {
                return Outcome::Rejected {
                    fault: Fault::new("invalid_command", "Command is not exported"),
                };
            };
            if let Err(fault) = invocation.validate(&registration.definition, Default::default()) {
                return Outcome::Rejected { fault };
            }
            let result = (registration.handler)(self, context, &invocation);
            let valid = match &result {
                Outcome::Completed { value } => {
                    registration.definition.result.validate(value).is_ok()
                }
                Outcome::Accepted { operation } => {
                    !operation.id.is_empty() && operation.scope.validate().is_ok()
                }
                _ => true,
            };
            if valid {
                result
            } else {
                Outcome::Indeterminate {
                    fault: Fault::new(
                        "invalid_result",
                        "Command result is invalid; execution may have occurred",
                    ),
                }
            }

    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_protocol::invocation::Dispatcher;
    use std::sync::Arc;

    #[tokio::test]
    async fn conversation_resume_is_owned_only_by_daemon_lifecycle() {
        let runtime = Runtime::start("resume-boundary", "Resume", None,
            Arc::new(misa_kernel::LocalKernel::new(misa_kernel::ScriptedProvider::always("unused"))),
            "scripted", "test", Value::Null);
        assert!(!runtime.command_registry.contains_key("session.conversation.resume"));
        assert!(!shortcuts(&runtime.command_registry).iter().any(|shortcut| shortcut.id == "resume"));
        let dispatcher = Dispatcher::new(CallContext { principal: "trusted".into(), connection: 1 }, 4, Default::default(), Default::default());
        let result = dispatcher.dispatch(runtime.as_ref(), Invocation { id: 1, scope: runtime.scope(),
            command: "session.conversation.resume".into(), input: Value::map([("conversation", Value::str("other"))]) }).await;
        assert!(matches!(result.outcome, Outcome::Rejected { .. }));
        runtime.shutdown_complete().await;
    }

    #[tokio::test]
    async fn contributed_handlers_have_typed_results_and_preserve_uncertain_outcomes() {
        let contribution = crate::Contribution::new()
            .with_command(CommandRegistration::new(
                Command {
                    preparation: Default::default(), id: "test.increment".into(),
                    input: Schema::Int,
                    result: Schema::Int,
                },
                |_, context, invocation| {
                    assert_eq!(context.principal, "trusted");
                    Outcome::Completed {
                        value: Value::Int(invocation.input.as_i64().unwrap() + 1),
                    }
                },
            ))
            .with_command(CommandRegistration::new(
                Command {
                    preparation: Default::default(), id: "test.invalid-result".into(),
                    input: Schema::Int,
                    result: Schema::Int,
                },
                |_, _, _| Outcome::Completed {
                    value: Value::str("wrong contract"),
                },
            ));
        let runtime = Runtime::start_with(
            "typed",
            "Typed",
            None,
            Arc::new(misa_kernel::LocalKernel::new(
                misa_kernel::ScriptedProvider::always("unused"),
            )),
            "scripted",
            "test",
            Value::Null,
            contribution,
        );
        let dispatcher = Dispatcher::new(
            CallContext {
                principal: "trusted".into(),
                connection: 7,
            },
            4,
            Default::default(),
            Default::default(),
        );
        let mut invocation = Invocation {
            id: 1,
            scope: runtime.scope(),
            command: "test.increment".into(),
            input: Value::Int(4),
        };
        assert!(matches!(
            dispatcher
                .dispatch(runtime.as_ref(), invocation.clone())
                .await
                .outcome,
            Outcome::Completed {
                value: Value::Int(5)
            }
        ));
        invocation.command = "test.invalid-result".into();
        assert!(matches!(
            dispatcher
                .dispatch(runtime.as_ref(), invocation)
                .await
                .outcome,
            Outcome::Indeterminate { .. }
        ));
        let definition = misa_proto::query::catalog_definition();
        let selected = misa_proto::observation::Selection {
            scope: runtime.scope(),
            members: BTreeMap::from([(
                "catalog".into(),
                misa_proto::observation::Member {
                    query: misa_proto::Query::new(&definition.id),
                    contract: definition.contract,
                    encoding: definition.result.encoding(),
                    optional: false,
                },
            )]),
        };
        let snapshot = runtime.read_selection(&selected).unwrap();
        let misa_proto::observation::Content::Value(value) = &snapshot.members["catalog"] else {
            panic!()
        };
        let definitions: Vec<misa_proto::query::Definition> = crate::wire::parse(value).unwrap();
        assert!(
            definitions
                .iter()
                .any(|definition| definition.id == misa_proto::query::CATALOG)
        );
        assert!(
            definitions
                .iter()
                .any(|definition| definition.id == misa_proto::invocation::CATALOG)
        );
    }

    #[test]
    fn duplicate_commands_and_invalid_bindings_fail_installation() {
        assert!(install(&[builtins().remove(0)]).is_err());
        let installed = install(&[]).unwrap();
        let mut binding = misa_proto::invocation::Binding {
            id: "test".into(),
            binding: misa_proto::invocation::ActionBinding {
                command: "session.model.select".into(),
                bound: BTreeMap::from([("model".into(), Value::Bool(true))]),
                inputs: BTreeMap::new(),
            },
        };
        assert!(catalogs(&installed, &[binding.clone()]).is_err());
        binding
            .binding
            .bound
            .insert("model".into(), Value::str("test"));
        assert!(catalogs(&installed, &[binding.clone()]).is_ok());
        binding.binding.command = "undeclared".into();
        assert!(catalogs(&installed, &[binding]).is_err());
    }

    #[tokio::test]
    async fn invalid_inputs_and_old_incarnations_never_start_a_turn() {
        let runtime = Runtime::start(
            "commands",
            "Commands",
            None,
            Arc::new(misa_kernel::LocalKernel::new(
                misa_kernel::ScriptedProvider::always("unused"),
            )),
            "scripted",
            "test",
            Value::Null,
        );
        let context = CallContext {
            principal: "paired-user".into(),
            connection: 1,
        };
        let dispatcher = Dispatcher::new(context, 4, Default::default(), Default::default());
        let mut invocation = Invocation {
            id: 1,
            scope: runtime.scope(),
            command: "session.prompt".into(),
            input: Value::map([("text", Value::Int(4))]),
        };
        assert!(matches!(
            dispatcher
                .dispatch(runtime.as_ref(), invocation.clone())
                .await
                .outcome,
            Outcome::Rejected { .. }
        ));
        assert_eq!(runtime.status(), "idle");
        invocation.input = Value::map([("text", Value::str("hello"))]);
        invocation.scope.incarnation = "previous".into();
        assert!(matches!(
            dispatcher
                .dispatch(runtime.as_ref(), invocation.clone())
                .await
                .outcome,
            Outcome::Rejected { .. }
        ));
        assert_eq!(runtime.status(), "idle");
        invocation.scope = runtime.scope();
        assert!(matches!(
            dispatcher
                .dispatch(runtime.as_ref(), invocation)
                .await
                .outcome,
            Outcome::Accepted { .. }
        ));
        assert_ne!(runtime.status(), "idle");
    }
}
