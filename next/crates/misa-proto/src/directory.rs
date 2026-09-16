//! The standard daemon directory export. These are domain records, not UI rows.
use std::collections::BTreeMap;

use crate::{
    Fault, Query,
    observation::{Encoding, Member, Scope, ScopeId, Selection},
    query::{Definition, ResultContract},
    schema::{Field, Literal, Schema},
};
use misa_value::Value;
use serde::{Deserialize, Serialize};

pub const SESSIONS: &str = "daemon.sessions";
pub const CONTRACT: &str = "daemon.sessions@1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Availability {
    Current,
    Stale,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub incarnation: String,
    pub title: String,
    pub availability: Availability,
    pub source_position: u64,
    pub summary: Value,
}

impl Entry {
    pub fn scope(&self) -> Scope {
        Scope {
            id: ScopeId::Session {
                id: self.id.clone(),
            },
            incarnation: self.incarnation.clone(),
        }
    }
}

pub fn definition() -> Definition {
    let field = |schema| Field {
        schema,
        optional: false,
    };
    Definition {
        id: SESSIONS.into(),
        arguments: vec![],
        contract: CONTRACT.into(),
        result: ResultContract::Data {
            schema: Schema::List {
                items: Box::new(Schema::Record {
                    fields: BTreeMap::from([
                        ("id".into(), field(Schema::String)),
                        ("incarnation".into(), field(Schema::String)),
                        ("title".into(), field(Schema::String)),
                        ("source_position".into(), field(Schema::Int)),
                        (
                            "availability".into(),
                            field(Schema::Choice {
                                values: ["current", "stale", "unavailable"]
                                    .into_iter()
                                    .map(|value| Literal::String(value.into()))
                                    .collect(),
                            }),
                        ),
                        // Summary exports validate their own domain contract. The directory
                        // preserves that data without inventing a second copy of its schema.
                        ("summary".into(), field(Schema::Value)),
                    ]),
                    allow_unknown: false,
                }),
            },
        },
    }
}

pub fn selection(scope: Scope) -> Selection {
    Selection {
        scope,
        members: BTreeMap::from([(
            "sessions".into(),
            Member {
                query: Query::new(SESSIONS),
                contract: CONTRACT.into(),
                encoding: Encoding::Value,
                optional: false,
            },
        )]),
    }
}

/// Decode domain data once when a directory changes, rather than opening every
/// session's transcript. Further interpretation belongs to summary consumers.
pub fn entries(value: &Value) -> Result<Vec<Entry>, Fault> {
    let ResultContract::Data { schema } = definition().result else {
        unreachable!()
    };
    schema
        .validate(value)
        .map_err(|error| Fault::query(error.to_string()))?;
    value
        .as_list()
        .expect("schema validated list")
        .iter()
        .map(|row| {
            let text = |name| {
                row.get(name)
                    .and_then(Value::as_str)
                    .expect("schema validated string")
                    .to_owned()
            };
            let entry = Entry {
                id: text("id"),
                incarnation: text("incarnation"),
                title: text("title"),
                availability: match text("availability").as_str() {
                    "current" => Availability::Current,
                    "stale" => Availability::Stale,
                    _ => Availability::Unavailable,
                },
                source_position: u64::try_from(
                    row.get("source_position")
                        .and_then(Value::as_i64)
                        .expect("schema validated integer"),
                )
                .map_err(|_| Fault::query("Source publication must not be negative"))?,
                summary: row
                    .get("summary")
                    .expect("schema validated summary")
                    .clone(),
            };
            entry.scope().validate()?;
            Ok(entry)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn availability_is_not_an_activity_guess() {
        let value = Value::list([Value::map([
            ("id", Value::str("one")),
            ("incarnation", Value::str("run")),
            ("title", Value::str("One")),
            ("availability", Value::str("stale")),
            ("source_position", Value::Int(3)),
            ("summary", Value::map([("activity", Value::str("working"))])),
        ])]);
        let entries = entries(&value).unwrap();
        assert_eq!(entries[0].availability, Availability::Stale);
        assert_eq!(
            entries[0].summary.get("activity").and_then(Value::as_str),
            Some("working")
        );
        assert_eq!(entries[0].scope().incarnation, "run");
    }
}
