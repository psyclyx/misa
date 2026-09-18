//! Local composer parsing and presentation declarations.
//!
//! Shortcut metadata names arguments and completion sources. Parsing only prepares
//! local input; the installed command catalog and owner validate invocations.

pub use misa_proto::preparation::Arg;
use serde::{Deserialize, Serialize};

/// A local slash-command declaration; invocation authority stays in installed catalogs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Command {
    /// Without a leading slash: the slash is punctuation, and a client draws it.
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Arg>,
}

impl Command {
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
    ) -> Command {
        Command {
            id: id.into(),
            label: label.into(),
            description: description.into(),
            args: Vec::new(),
        }
    }

    pub fn arg(mut self, arg: Arg) -> Command {
        self.args.push(arg);
        self
    }

    /// The first argument that has to be supplied, if any.
    pub fn first_required(&self) -> Option<&Arg> {
        self.args.iter().find(|arg| arg.required)
    }
}

use misa_value::Value;

/// What a line amounts to.
#[derive(Clone, Debug, PartialEq)]
pub enum Parsed {
    /// Malformed local syntax is retained in the composer, never submitted.
    Invalid { message: String },
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

/// Literal command words, also usable while a quoted argument is being typed.
/// No variable, command, glob, or environment expansion is performed.
pub struct Words {
    pub values: Vec<String>,
    pub error: Option<&'static str>,
    pub trailing_space: bool,
}

pub fn words(input: &str) -> Words {
    let mut values = Vec::new();
    let mut value = String::new();
    let mut quote = None;
    let mut escape = false;
    let mut started = false;
    let mut trailing_space = false;
    for ch in input.chars() {
        if escape {
            value.push(ch);
            escape = false;
        } else if ch == '\\' && quote != Some('\'') {
            escape = true;
            started = true;
        } else if let Some(delimiter) = quote {
            if ch == delimiter {
                quote = None;
            } else {
                value.push(ch);
            }
        } else if ch == '\'' || ch == '"' {
            quote = Some(ch);
            started = true;
        } else if ch.is_whitespace() {
            if started {
                values.push(std::mem::take(&mut value));
                started = false;
            }
            trailing_space = true;
            continue;
        } else {
            value.push(ch);
            started = true;
        }
        trailing_space = false;
    }
    if started {
        values.push(value);
    }
    Words {
        values,
        error: if escape {
            Some("Finish the escaped character")
        } else if quote.is_some() {
            Some("Close the quoted argument")
        } else {
            None
        },
        trailing_space,
    }
}

/// Emit one literal argument that the local parser can read without expansion.
pub fn quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|ch| !ch.is_whitespace() && !matches!(ch, '\'' | '"' | '\\'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
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
    let words = words(rest);
    if let Some(message) = words.error {
        return Parsed::Invalid {
            message: message.into(),
        };
    }
    let name = words.values.first().cloned().unwrap_or_default();
    let values = &words.values[words.values.len().min(1)..];
    let Some(declared) = commands.iter().find(|command| command.id == name) else {
        return Parsed::Unknown { name };
    };
    if values.len() > declared.args.len() {
        return Parsed::Invalid {
            message: format!("/{name} has too many arguments; quote a value containing spaces"),
        };
    }

    // Positional arguments in the order they are declared, which is the order a
    // person types them, so no frontend has to invent a syntax for naming one.
    let mut given = Vec::new();
    for (index, arg) in declared.args.iter().enumerate() {
        if let Some(value) = values.get(index) {
            given.push((arg.name.clone(), value.clone()));
        }
    }
    let mut args = std::collections::BTreeMap::new();
    for (name, value) in &given {
        args.insert(name.clone(), Value::str(value));
    }

    if let Some(missing) = declared
        .args
        .iter()
        .find(|arg| arg.required && !given.iter().any(|(name, _)| name == &arg.name))
    {
        return Parsed::Needs {
            command: name,
            argument: missing.name.clone(),
            source: missing.source.clone(),
            given,
        };
    }
    Parsed::Command {
        name,
        args: Value::Map(std::sync::Arc::new(args)),
    }
}

/// The intent a parsed line becomes, when it is ready.
pub fn intent(parsed: &Parsed) -> Option<Intent> {
    match parsed {
        Parsed::Prompt(text) => Some(Intent::Prompt {
            text: text.clone(),
            attachments: Vec::new(),
        }),
        Parsed::Command { name, args } => Some(Intent::Command {
            name: name.clone(),
            args: args.clone(),
        }),
        // A line that is not ready is not sent: the client opens the picker its
        // declaration pointed at, which is the whole reason this returns nothing.
        Parsed::Needs { .. } | Parsed::Unknown { .. } | Parsed::Empty | Parsed::Invalid { .. } => {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::preparation::Arg;

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
    fn literal_arguments_round_trip_without_expansion() {
        for value in [
            "two words",
            "quote' and \"double\"",
            "",
            "back\\slash",
            "雪 路",
            "$HOME;`command`",
            "tabs\there",
        ] {
            let parsed = parse(&format!("/model {}", quote(value)), &commands());
            let Parsed::Command { args, .. } = parsed else {
                panic!("{parsed:?}");
            };
            assert_eq!(args.get("model").and_then(Value::as_str), Some(value));
        }
        for line in [
            "/model two\\ words",
            "/model 'two 'words",
            "/model \"two words\"",
        ] {
            let Parsed::Command { args, .. } = parse(line, &commands()) else {
                panic!("{line}");
            };
            assert_eq!(args.get("model").and_then(Value::as_str), Some("two words"));
        }
    }

    #[test]
    fn malformed_or_surplus_arguments_never_submit_but_partial_prefix_survives() {
        for line in ["/model 'two words", "/model two\\", "/model two words"] {
            let parsed = parse(line, &commands());
            assert!(matches!(parsed, Parsed::Invalid { .. }));
            assert!(intent(&parsed).is_none());
        }
        let partial = words("model 'two words");
        assert_eq!(partial.values, ["model", "two words"]);
        assert!(partial.error.is_some());
        assert!(!partial.trailing_space);
        assert!(words("model 'two words' ").trailing_space);
    }

    #[test]
    fn ordinary_text_is_a_prompt() {
        assert_eq!(
            parse("hello there", &commands()),
            Parsed::Prompt("hello there".into())
        );
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
            Parsed::Needs {
                command,
                argument,
                source,
                given,
            } => {
                assert_eq!(command, "model");
                assert_eq!(argument, "model");
                assert_eq!(source.as_deref(), Some("models"));
                assert!(given.is_empty());
            }
            other => panic!("expected a required argument, got {other:?}"),
        }
        assert!(
            intent(&parse("/model", &commands())).is_none(),
            "an unready line was sent"
        );
    }

    #[test]
    fn an_argument_is_taken_positionally_and_named_by_the_declaration() {
        match parse("/model scripted-1", &commands()) {
            Parsed::Command { name, args } => {
                assert_eq!(name, "model");
                assert_eq!(
                    args.get("model").and_then(Value::as_str),
                    Some("scripted-1")
                );
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

use misa_proto::preparation::SourceKind;

/// Local completion presentation metadata, independent of owner queries.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Source {
    pub id: String,
    pub label: String,
    pub kind: SourceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl Source {
    pub fn resident(id: impl Into<String>, label: impl Into<String>) -> Source {
        Source {
            id: id.into(),
            label: label.into(),
            kind: SourceKind::Resident,
            description: None,
        }
    }

    pub fn on_demand(
        id: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
    ) -> Source {
        Source {
            id: id.into(),
            label: label.into(),
            kind: SourceKind::OnDemand,
            description: Some(description.into()),
        }
    }
}

/// A local editor action. This is never a wire request or owner command.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "intent", rename_all = "snake_case")]
pub enum Intent {
    /// Interrupt the current turn and submit this prompt before queued prompts.
    Interrupt {
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<misa_proto::view::BlobRef>,
    },
    /// Submit a turn.
    Prompt {
        text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<misa_proto::view::BlobRef>,
    },
    /// Resolve an action a view node offered.
    Action {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        node: String,
        action: String,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        args: Value,
        /// Field values from the node, for an action with
        /// [`ActionOn::Submit`](misa_proto::view::ActionOn).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        fields: Vec<misa_proto::view::Field>,
    },
    /// Invoke a declared command by name.
    Command {
        name: String,
        #[serde(default, skip_serializing_if = "Value::is_null")]
        args: Value,
    },
    /// Cancel the named work, or everything in flight when unnamed.
    Cancel {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
    },
    /// Ask a session for candidates for a prefix.
    ///
    /// Only for a source declared `OnDemand`, and only when a client has decided it
    /// needs them: matching, ranking, and deciding *when* to show a picker are the
    /// client's, because they are cheap, local, and different on every platform.
    /// Answering "which models are there" or "which files are under this prefix" is
    /// not.
    ///
    /// The local adapter resolves this through an exported finite read.
    Complete {
        source: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        prefix: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        limit: Option<u32>,
    },
}

impl Intent {
    pub fn name(&self) -> &'static str {
        match self {
            Intent::Interrupt { .. } => "interrupt",
            Intent::Prompt { .. } => "prompt",
            Intent::Action { .. } => "action",
            Intent::Command { .. } => "command",
            Intent::Cancel { .. } => "cancel",
            Intent::Complete { .. } => "complete",
        }
    }
}
