//! Contained protocol and domain failures.
use misa_value::Value;
use serde::{Deserialize, Serialize};

/// A refusal or a contained failure, as a client hears about it.
///
/// Faults are data, not errors. The previous system discovered that a policy fault
/// must roll its transaction back and leave the session running; a client likewise
/// must be able to say what happened without falling over. `code` is a stable
/// identifier, `message` is for a person.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Fault {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
}

impl Fault {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Fault {
            code: code.into(),
            message: message.into(),
            data: Value::Null,
        }
    }

    pub fn with(mut self, data: Value) -> Self {
        self.data = data;
        self
    }

    /// The intent was refused because the client asked for something this session
    /// does not accept.
    pub fn unsupported(message: impl Into<String>) -> Self {
        Fault::new("unsupported", message)
    }

    pub fn protocol(message: impl Into<String>) -> Self {
        Fault::new("protocol", message)
    }

    pub fn query(message: impl Into<String>) -> Self {
        Fault::new("query", message)
    }
    /// A command was sent without an argument it cannot run without.
    ///
    /// Carries enough for a client to open the right picker without asking, so the
    /// error path narrows to the same place the declaration pointed at. A fault that
    /// only said "missing argument" would make every client re-derive what the
    /// session already knows.
    pub fn argument(command: &str, arg: &str, source: Option<&str>) -> Fault {
        let mut data = std::collections::BTreeMap::new();
        data.insert("command".to_string(), Value::str(command));
        data.insert("argument".to_string(), Value::str(arg));
        if let Some(source) = source {
            data.insert("source".to_string(), Value::str(source));
        }
        Fault::new(
            "argument.required",
            format!("`/{command}` needs an argument for `{arg}`"),
        )
        .with(Value::Map(std::sync::Arc::new(data)))
    }
}
