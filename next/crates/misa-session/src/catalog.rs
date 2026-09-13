//! What a session declares about itself.
//!
//! This is the previous system's sealed catalogs, and it is deliberately the same
//! idea: a session is a *composition*, and a composition is data. What changed is
//! that the declarations now cross a wire, so they have to say enough for a client
//! it has never met to be useful with no round trip:
//!
//! - a **command** says what it does and which arguments it needs;
//! - an **argument** says where its values come from, by naming a **source**;
//! - a **source** says whether a client should hold it (`Resident`) or ask for it
//!   (`OnDemand`).
//!
//! That is all a client needs to know that `/model` should open a picker, that
//! models are worth holding, and that a conversation search is worth asking for.
//! Nothing here says how a picker looks, what keys drive it, or when it opens —
//! those differ on every platform and none of them is the session's business.
//!
//! # Why a model's price lives here
//!
//! Because cost is a *fact about a request* and a price is a *fact about a model*,
//! and neither is presentation. The session settles an attempt with a cost in
//! micros; whether that becomes `$1.24` or `1,24 €` is the client's, which is why
//! `value.money` exists in the view vocabulary.

use misa_proto::view::Choice;
use misa_proto::wire::{Arg, Command, Source};
use misa_value::Value;

/// The models to offer, from what the service said and what the catalog knows.
///
/// The service is the *list* and the catalog is the *facts*, which is the previous system's
/// arrangement and the only one that survives a model being renamed by the people who serve it:
/// a row nobody has heard of is still offered, with whatever can honestly be said about it and
/// no invented price. Before the first answer from a service the catalog is the list, because a
/// session with no models to offer is a picker with nothing in it.
pub fn choices(discovered: &Value, provider: &str, current: &str) -> Vec<Choice> {
    let listed = discovered.as_list().unwrap_or(&[]);
    if listed.is_empty() {
        return MODELS.iter().map(|model| choice_of(model, current)).collect();
    }
    listed
        .iter()
        .filter_map(|row| {
            let id = row.get("id").and_then(Value::as_str)?;
            Some(match model(id) {
                Some(known) => choice_of(&known, current),
                None => Choice {
                    value: id.to_string(),
                    label: row.get("label").and_then(Value::as_str).unwrap_or(id).to_string(),
                    // Said plainly, because it is the truth about a model nobody priced: the
                    // service serves it, and nothing here pretends to know more.
                    detail: Some(format!("{provider} · the service lists it")),
                },
            })
        })
        .collect()
}

/// One model, as a candidate a picker can show.
fn choice_of(model: &Model, current: &str) -> Choice {
    let detail = format!(
        "{}{} · {}k context{}",
        model.provider,
        if model.id == current { " · current" } else { "" },
        model.context_window / 1_000,
        if model.effort { " · effort" } else { "" },
    );
    Choice { value: model.id.to_string(), label: model.label.to_string(), detail: Some(detail) }
}

/// One model a session can be talking to.
#[derive(Clone, Copy, Debug)]
pub struct Model {
    pub id: &'static str,
    pub label: &'static str,
    pub provider: &'static str,
    /// The wire protocol the provider speaks. `scripted` is the shipped one.
    pub api: &'static str,
    pub context_window: i64,
    /// Whether the model accepts a reasoning-effort setting.
    pub effort: bool,
    /// Micros per thousand input tokens.
    pub input_micros: i64,
    /// Micros per thousand output tokens.
    pub output_micros: i64,
}

/// The shipped catalog.
///
/// Small, and every entry is one the shipped kernel can actually reach. The
/// previous system shipped a dozen real providers; they belong here, and they are
/// deliberately absent until an adapter exists, because a model a session cannot
/// call is a model a picker should not offer.
///
/// Adding one is a row plus a `Provider` implementation in the kernel. That is the
/// whole of what "a provider is a plugin" means here.
pub const MODELS: &[Model] = &[
    Model {
        id: "scripted-1",
        label: "Scripted (one tool call, then an answer)",
        provider: "scripted",
        api: "scripted",
        context_window: 8_192,
        effort: false,
        input_micros: 0,
        output_micros: 0,
    },
    Model {
        id: "scripted-chatty",
        label: "Scripted (three answers)",
        provider: "scripted",
        api: "scripted",
        context_window: 32_768,
        effort: false,
        input_micros: 0,
        output_micros: 0,
    },
    // The shape a real one takes, kept here so the fields are not a mystery:
    //
    // Model {
    //     id: "claude-sonnet-5",
    //     label: "Claude Sonnet 5",
    //     provider: "claude",
    //     api: "anthropic.messages",
    //     context_window: 200_000,
    //     effort: true,
    //     input_micros: 3_000,
    //     output_micros: 15_000,
    // },
];

/// Reasoning efforts, weakest first. Only offered by a model that takes one.
pub const EFFORTS: &[&str] = &["low", "medium", "high"];

/// The model a session starts on.
pub const DEFAULT_MODEL: &str = "scripted-1";

pub fn model(id: &str) -> Option<Model> {
    MODELS.iter().copied().find(|model| model.id == id)
}

pub fn default_model() -> Model {
    model(DEFAULT_MODEL).unwrap_or(MODELS[0])
}

/// What a settled attempt cost, from the model's own prices.
///
/// Rounded to a micro, which is the smallest unit anything here records. A model
/// that is not in the catalog costs nothing, which is the honest answer for a
/// scripted one: it is not that the cost is unknown, it is that there is none.
pub fn cost_micros(model_id: &str, input_tokens: i64, output_tokens: i64) -> i64 {
    let Some(model) = model(model_id) else {
        return 0;
    };
    let input = input_tokens.max(0) * model.input_micros / 1_000;
    let output = output_tokens.max(0) * model.output_micros / 1_000;
    input + output
}

/// The sources a session declares.
pub fn sources() -> Vec<Source> {
    vec![
        Source::resident("models", "Models"),
        Source::resident("effort", "Reasoning effort"),
        Source::resident("commands", "Commands"),
        Source::on_demand(
            "conversations",
            "Conversations",
            "searched by the session, because a log outgrows what a client should hold",
        ),
    ]
}

/// A source a client should hold, and the query it is read from.
pub fn resident_query(source: &str) -> Option<String> {
    let source = sources().into_iter().find(|entry| entry.id == source)?;
    match source.kind {
        misa_proto::wire::SourceKind::Resident => Some(source.query()),
        misa_proto::wire::SourceKind::OnDemand => None,
    }
}

/// The commands a session declares.
///
/// Five, and each one is an operation on the *session*: nothing here is a report or
/// a piece of interface. `/usage` is deliberately absent — a usage report is a
/// presentation of facts the session already publishes, so it is a client's own
/// action, and a client that does not have one is still correct.
pub fn commands() -> Vec<Command> {
    vec![
        Command::new("clear", "Clear", "forget this branch and start again")
            .arg(Arg::new("reason", "Reason")),
        Command::new("compact", "Compact", "summarise earlier messages and replace history"),
        Command::new("model", "Model", "choose the model for the turn after next")
            .arg(Arg::new("model", "Model").required().from("models")),
        Command::new("effort", "Effort", "choose how hard the model should think")
            .arg(Arg::new("level", "Level").required().from("effort")),
        // A command rather than something the picker does on its own: "which models does this
        // service serve" is a request to a service, and a client opening a list should not cost
        // one. A person asking is a person who wants to know.
        Command::new("models", "Models", "ask the provider which models it has"),
        Command::new("resume", "Resume", "load a stored conversation")
            .arg(Arg::new("conversation", "Conversation").from("conversations")),
    ]
}

/// What a tool looks like to a model.
#[derive(Clone, Debug)]
pub struct ToolDecl {
    pub name: &'static str,
    pub description: &'static str,
    /// The JSON Schema the model is given. A tool without one cannot be called
    /// reliably, so every declared tool has one.
    pub schema: &'static str,
}

/// The tools the shipped composition declares to a model.
///
/// Each name must have an implementation in the kernel. `Shipped` is checked by a
/// test against `misa_kernel::tools::shipped`, because a declaration a kernel
/// cannot honour is a model being told about a tool that will always fail.
pub const TOOLS: &[ToolDecl] = &[
    ToolDecl {
        name: "read_file",
        description: "Read a text file, optionally a window of its numbered lines.",
        schema: r#"{"type":"object","properties":{"path":{"type":"string"},"start_line":{"type":"integer"},"max_lines":{"type":"integer"}},"required":["path"]}"#,
    },
    ToolDecl {
        name: "write_file",
        description: "Write a text file, creating its directory if it does not exist.",
        schema: r#"{"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}"#,
    },
    ToolDecl {
        name: "list_directory",
        description: "List a directory, sorted, marking directories with a trailing slash.",
        schema: r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}"#,
    },
    ToolDecl {
        name: "shell",
        description: "Run a command with `sh -lc`. Its output goes to a log file, and the answer \
                 names that file. If the command is still running after `wait_ms` — or if \
                 `background` is true, which is what to use for anything long — the answer comes \
                 back at once with its pid and its log, and the session reports when it finishes. \
                 It is only stopped at `timeout_ms`, which is ten minutes by default. Read or tail \
                 the log to follow a command that is still going; do not start it again.",
        schema: r#"{"type":"object","properties":{"command":{"type":"string"},"background":{"type":"boolean"},"wait_ms":{"type":"integer"},"timeout_ms":{"type":"integer"}},"required":["command"]}"#,
    },
    ToolDecl {
        name: "echo",
        description: "Print the arguments back, unchanged. The smallest tool there is.",
        schema: r#"{"type":"object","properties":{}}"#,
    },
];

/// The tools, as the value a provider request carries.
pub fn tool_schemas() -> misa_value::Value {
    misa_value::Value::list(TOOLS.iter().map(|tool| {
        misa_value::Value::map([
            ("name", misa_value::Value::str(tool.name)),
            ("description", misa_value::Value::str(tool.description)),
            ("input_schema", misa_value::Value::str(tool.schema)),
        ])
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_declared_model_is_reachable_by_id() {
        for entry in MODELS {
            assert!(model(entry.id).is_some());
            assert!(!entry.label.is_empty(), "`{}` has no label", entry.id);
            assert!(entry.context_window > 0);
        }
        assert_eq!(default_model().id, DEFAULT_MODEL);
    }

    #[test]
    fn a_cost_is_a_function_of_tokens_and_the_models_own_prices() {
        // The shipped models are free, so the arithmetic is checked against the
        // shape a real one takes rather than against a scripted row.
        let real = Model {
            id: "x",
            label: "x",
            provider: "p",
            api: "a",
            context_window: 1,
            effort: false,
            input_micros: 3_000,
            output_micros: 15_000,
        };
        assert_eq!(real.input_micros, 3_000);
        assert_eq!(cost_micros("scripted-1", 1_000_000, 1_000_000), 0);
        assert_eq!(cost_micros("no-such-model", 1_000, 1_000), 0);
    }

    #[test]
    fn a_command_that_cannot_run_without_an_argument_says_so() {
        let model = commands()
            .into_iter()
            .find(|command| command.id == "model")
            .expect("a model command");
        let required = model.first_required().expect("a required argument");
        assert_eq!(required.name, "model");
        assert_eq!(required.source.as_deref(), Some("models"));
        // And the source it names is one this session declares.
        assert!(resident_query("models").is_some());
    }

    #[test]
    fn an_optional_argument_is_not_required_and_may_have_no_source() {
        let clear = commands()
            .into_iter()
            .find(|command| command.id == "clear")
            .expect("a clear command");
        let reason = clear.args.first().expect("an argument");
        assert!(!reason.required);
        assert!(reason.source.is_none());
        assert!(clear.first_required().is_none());
    }

    #[test]
    fn a_resident_source_has_a_query_and_an_on_demand_one_does_not() {
        assert_eq!(resident_query("models").as_deref(), Some("completion.models"));
        assert_eq!(resident_query("commands").as_deref(), Some("completion.commands"));
        assert!(
            resident_query("conversations").is_none(),
            "an on-demand source was offered as something to hold"
        );
    }

    #[test]
    fn every_declared_tool_has_a_schema_a_model_could_validate_against() {
        for tool in TOOLS {
            assert!(tool.schema.starts_with('{'), "`{}` has no schema", tool.name);
            assert!(tool.schema.contains("type"), "`{}`'s schema says nothing", tool.name);
            assert!(!tool.description.is_empty());
        }
        let schemas = tool_schemas();
        assert_eq!(schemas.as_list().map(<[misa_value::Value]>::len), Some(TOOLS.len()));
    }

    #[test]
    fn every_declared_tool_is_one_the_shipped_kernel_can_run() {
        // A declared tool the kernel cannot honour is a model being told about
        // something that will always fail.
        // The tools, as the kernel builds them: the shell needs somewhere to put its logs and
        // somewhere to report a command that outlives its call, and neither is the session's.
        let (events, _reports) = tokio::sync::mpsc::unbounded_channel();
        let shipped = misa_kernel::tools::shipped(&misa_kernel::tools::default_shell_dir(), &events);
        let runnable: Vec<&str> = shipped.iter().map(|tool| tool.name()).collect();
        for tool in TOOLS {
            assert!(
                runnable.contains(&tool.name),
                "`{}` is declared to a model but the kernel has no implementation",
                tool.name
            );
        }
    }

    #[test]
    fn the_effort_levels_are_ordered_and_named() {
        assert_eq!(EFFORTS.first(), Some(&"low"));
        assert_eq!(EFFORTS.last(), Some(&"high"));
    }
}
