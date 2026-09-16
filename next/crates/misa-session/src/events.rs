//! Local owner instrumentation; never transported to clients.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Info,
    Warn,
    Error,
}

impl Level {
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

/// Local instrumentation for owner handlers and integration fixtures.
/// Public state is exposed through query publications and invocation outcomes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SessionEvent {
    Stream {
        update: misa_proto::sync::StreamUpdate,
    },
    Notice {
        level: Level,
        text: String,
    },
    Recover {
        text: String,
    },
}
