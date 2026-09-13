//! Parsing a line into the small set of things a client may ask for.
//!
//! A frontend has a line somebody typed and a declaration from the session. This
//! turns the first into an [`Intent`] using the second, which is the last piece that
//! makes "the client knows what the session can do without asking" useful: a
//! command's arguments are separated by the declaration, not guessed.
//!
//! What is *not* here: what any of it means. A submission is an intent, and the
//! session's answer is whatever its own policy says.

use misa_proto::wire::{Command, Intent};
use misa_value::Value;

/// What a line amounts to.
#[derive(Clone, Debug, PartialEq)]
pub enum Parsed {
    /// Ordinary text, to be submitted as a prompt.
    Prompt(String),
    /// A command, with its arguments named by the declaration.
    Command { name: String, args: Value },
    /// A command this session does not have.
    Unknown { name: String },
    /// A command that needs an argument before it can run.
    Needs {
        command: String,
        argument: String,
        source: Option<String>,
        /// What has been given so far, so a frontend can keep the partial line.
        given: Vec<(String, String)>,
    },
    /// Nothing to do.
    Empty,
}

/// Parse a line against the declarations.
///
/// The first line only: a message with a newline in it is a prompt, because a
/// command is one line by definition and a person pasting a code block did not mean
/// to run the first line of it.
pub fn parse(line: &str, commands: &[Command]) -> Parsed {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return Parsed::Empty;
    }
    let Some(rest) = trimmed.strip_prefix('/') else {
        return Parsed::Prompt(line.to_string());
    };
    if rest.contains('\n') {
        return Parsed::Prompt(line.to_string());
    }
    let mut words = rest.split_whitespace();
    let name = words.next().unwrap_or_default().to_string();
    let values: Vec<&str> = words.collect();
    let Some(declared) = commands.iter().find(|command| command.id == name) else {
        return Parsed::Unknown { name };
    };

    // Positional arguments in the order they are declared, which is the order a
    // person types them, so no frontend has to invent a syntax for naming one.
    let mut given = Vec::new();
    for (index, arg) in declared.args.iter().enumerate() {
        if let Some(value) = values.get(index)
            && !value.is_empty()
        {
            given.push((arg.name.clone(), (*value).to_string()));
        }
    }
    let mut args = std::collections::BTreeMap::new();
    for (name, value) in &given {
        args.insert(name.clone(), Value::str(value));
    }

    if let Some(missing) = declared.args.iter().find(|arg| {
        arg.required && !given.iter().any(|(name, _)| name == &arg.name)
    }) {
        return Parsed::Needs {
            command: name,
            argument: missing.name.clone(),
            source: missing.source.clone(),
            given,
        };
    }
    Parsed::Command { name, args: Value::Map(std::sync::Arc::new(args)) }
}

/// The intent a parsed line becomes, when it is ready.
pub fn intent(parsed: &Parsed) -> Option<Intent> {
    match parsed {
        Parsed::Prompt(text) => Some(Intent::Prompt { text: text.clone(), attachments: Vec::new() }),
        Parsed::Command { name, args } => Some(Intent::Command {
            name: name.clone(),
            args: args.clone(),
        }),
        // A line that is not ready is not sent: the client opens the picker its
        // declaration pointed at, which is the whole reason this returns nothing.
        Parsed::Needs { .. } | Parsed::Unknown { .. } | Parsed::Empty => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::wire::Arg;

    fn commands() -> Vec<Command> {
        vec![
            Command::new("clear", "Clear", "forget this branch"),
            Command::new("model", "Model", "choose a model")
                .arg(Arg::new("model", "Model").required().from("models")),
            Command::new("resume", "Resume", "load a conversation")
                .arg(Arg::new("conversation", "Conversation").from("conversations")),
        ]
    }

    #[test]
    fn ordinary_text_is_a_prompt() {
        assert_eq!(parse("hello there", &commands()), Parsed::Prompt("hello there".into()));
    }

    #[test]
    fn a_pasted_block_that_starts_with_a_slash_is_still_a_prompt() {
        // A command is one line; somebody pasting code did not mean to run its first
        // line.
        let line = "/not a command\nbecause there are two lines";
        assert_eq!(parse(line, &commands()), Parsed::Prompt(line.into()));
    }

    #[test]
    fn a_command_without_arguments_parses_from_its_name_alone() {
        match parse("/clear", &commands()) {
            Parsed::Command { name, .. } => assert_eq!(name, "clear"),
            other => panic!("expected a command, got {other:?}"),
        }
    }

    #[test]
    fn a_command_that_needs_an_argument_says_so_and_where_to_find_it() {
        match parse("/model", &commands()) {
            Parsed::Needs { command, argument, source, given } => {
                assert_eq!(command, "model");
                assert_eq!(argument, "model");
                assert_eq!(source.as_deref(), Some("models"));
                assert!(given.is_empty());
            }
            other => panic!("expected a required argument, got {other:?}"),
        }
        assert!(intent(&parse("/model", &commands())).is_none(), "an unready line was sent");
    }

    #[test]
    fn an_argument_is_taken_positionally_and_named_by_the_declaration() {
        match parse("/model scripted-1", &commands()) {
            Parsed::Command { name, args } => {
                assert_eq!(name, "model");
                assert_eq!(args.get("model").and_then(Value::as_str), Some("scripted-1"));
            }
            other => panic!("expected a command, got {other:?}"),
        }
    }

    #[test]
    fn an_optional_argument_may_be_left_out_and_the_command_still_runs() {
        match parse("/resume", &commands()) {
            Parsed::Command { name, args } => {
                assert_eq!(name, "resume");
                assert!(args.as_map().expect("a map").is_empty());
            }
            other => panic!("expected a command, got {other:?}"),
        }
    }

    #[test]
    fn a_command_the_session_does_not_have_is_reported_rather_than_sent() {
        match parse("/nonsense x", &commands()) {
            Parsed::Unknown { name } => assert_eq!(name, "nonsense"),
            other => panic!("expected an unknown command, got {other:?}"),
        }
        assert!(intent(&parse("/nonsense", &commands())).is_none());
    }

    #[test]
    fn an_empty_line_asks_for_nothing() {
        assert_eq!(parse("   ", &commands()), Parsed::Empty);
        assert!(intent(&Parsed::Empty).is_none());
    }

    #[test]
    fn a_ready_command_becomes_the_intent_it_declared() {
        match intent(&parse("/model gpt-5", &commands())).expect("an intent") {
            Intent::Command { name, args } => {
                assert_eq!(name, "model");
                assert_eq!(args.get("model").and_then(Value::as_str), Some("gpt-5"));
            }
            other => panic!("expected a command, got {other:?}"),
        }
    }
}
