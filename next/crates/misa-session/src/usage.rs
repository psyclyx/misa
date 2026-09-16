//! Usage facts and an independently requested typed document presentation.
use misa_proto::Query;
use misa_reframe::{Registry, derived_query};
use misa_value::Value;

pub fn subscriptions(registry: Registry) -> Registry {
    registry
        .subscription(
            "session.status-report",
            derived_query([Query::new("session.status")], |inputs| {
                let session = &inputs[0];
                Value::map(
                    ["id", "status", "provider", "model", "effort"]
                        .map(|key| (key, session.get(key).cloned().unwrap_or(Value::Null))),
                )
            }),
        )
        .subscription(
            "usage.report",
            derived_query(
                [
                    Query::new("usage.session"),
                    Query::new("usage.last-request"),
                    Query::new("usage.selected-quota"),
                ],
                |inputs| {
                    Value::map([
                        ("session", inputs[0].clone()),
                        ("last_request", inputs[1].clone()),
                        ("quota", inputs[2].clone()),
                    ])
                },
            ),
        )
        .subscription(
            "usage.presentation",
            derived_query([Query::new("usage.report")], |inputs| {
                crate::wire::render(&presentation(&inputs[0]))
            }),
        )
        .subscription(
            "usage.session",
            derived_query([Query::new("session.attempts")], |inputs| {
                ledger(inputs[0].as_list().unwrap_or(&[]))
            }),
        )
        .subscription(
            "usage.last-request",
            derived_query([Query::new("session.attempts")], |inputs| {
                inputs[0]
                    .as_list()
                    .and_then(|rows| rows.last())
                    .cloned()
                    .unwrap_or(Value::Null)
            }),
        )
        .subscription(
            "usage.selected-quota",
            derived_query([Query::new("session.status")], |inputs| {
                let session = &inputs[0];
                if session.get("usage_provider") != session.get("provider") {
                    return Value::Null;
                }
                session.get("usage").cloned().unwrap_or(Value::Null)
            }),
        )
}

pub fn exports() -> Vec<misa_proto::query::Definition> {
    use misa_proto::{
        query::{Definition, ResultContract},
        schema::{Field, Schema},
    };
    let mut definitions: Vec<_> = [
        (
            "session.status-report",
            vec!["id", "status", "provider", "model", "effort"],
        ),
        ("usage.report", vec!["session", "last_request", "quota"]),
    ]
    .into_iter()
    .map(|(id, fields)| Definition {
        id: id.into(),
        arguments: vec![],
        contract: format!("{id}@1"),
        result: ResultContract::Data {
            schema: Schema::Record {
                fields: fields
                    .into_iter()
                    .map(|name| {
                        (
                            name.into(),
                            Field {
                                schema: if id == "session.status-report" {
                                    Schema::Nullable {
                                        inner: Box::new(Schema::String),
                                    }
                                } else {
                                    Schema::Value
                                },
                                optional: false,
                            },
                        )
                    })
                    .collect(),
                allow_unknown: false,
            },
        },
    })
    .collect();
    definitions.push(Definition {
        id: "usage.presentation".into(),
        arguments: vec![],
        contract: "usage.presentation@1".into(),
        result: ResultContract::Document {},
    });
    definitions
}

pub fn total_tokens(row: &Value) -> i64 {
    ["input_tokens", "output_tokens"]
        .iter()
        .map(|key| row.get(key).and_then(Value::as_i64).unwrap_or(0))
        .sum()
}

/// Domain presentation for an explicitly requested report. This never opens a
/// shared panel or changes the session, and clients need no field-name heuristics.
pub fn presentation(report: &Value) -> misa_proto::Node {
    use misa_proto::view::{Kind, Node, Span};
    let mut document = Node::section("usage.report")
        .id("usage.report")
        .label("Usage");
    for (index, (label, role, value)) in report_rows(report).into_iter().enumerate() {
        let id = format!("usage.row.{index}");
        document.children.push(
            Node::section("report.row")
                .id(&id)
                .child(Node::text("report.label", [Span::plain(label)]).id(format!("{id}.label")))
                .child(Node::new(role, Kind::Fact { value }).id(format!("{id}.value"))),
        );
    }
    document
}

pub(crate) fn ledger(attempts: &[Value]) -> Value {
    let sum = |key: &str| {
        Value::Int(
            attempts
                .iter()
                .filter_map(|row| row.get(key).and_then(Value::as_i64))
                .sum(),
        )
    };
    Value::map([
        ("calls", Value::Int(attempts.len() as i64)),
        ("input_tokens", sum("input_tokens")),
        ("output_tokens", sum("output_tokens")),
        ("cost_micros", sum("cost_micros")),
    ])
}

pub(crate) fn report_rows(report: &Value) -> Vec<(String, &'static str, Value)> {
    let mut rows = vec![];
    let mut row = |label: String, role, value: Value| {
        if !matches!(value, Value::List(_) | Value::Map(_) | Value::Bytes(_)) {
            rows.push((label, role, value));
        }
    };
    for (section, prefix) in [("session", "Session"), ("last_request", "Last request")] {
        let Some(values) = report.get(section).filter(|value| **value != Value::Null) else {
            continue;
        };
        for (key, label, role) in [
            ("calls", "calls", "value.number"),
            ("input_tokens", "input tokens", "value.tokens"),
            ("output_tokens", "output tokens", "value.tokens"),
            ("cost_micros", "spend", "value.money"),
        ] {
            if let Some(value) = values.get(key) {
                row(format!("{prefix} {label}"), role, value.clone());
            }
        }
    }
    let facts = report.get("quota").unwrap_or(&Value::Null);
    if let Some(plan) = facts.get("plan").filter(|value| **value != Value::Null) {
        row("Plan".into(), "value.text", plan.clone());
    }
    if facts
        .get("unavailable")
        .and_then(Value::as_bool)
        .unwrap_or(true)
    {
        row(
            "Provider quota".into(),
            "value.text",
            Value::str("Unavailable"),
        );
    }
    for window in facts.get("windows").and_then(Value::as_list).unwrap_or(&[]) {
        let label = window
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("Quota");
        for (key, suffix, role) in [
            ("used", "used", "value.number"),
            ("limit", "limit", "value.number"),
            ("remaining", "remaining", "value.number"),
            ("reset", "reset", "value.datetime"),
            ("reset_after_seconds", "reset after seconds", "value.number"),
        ] {
            if let Some(value) = window.get(key).filter(|value| **value != Value::Null) {
                let role = if ["used", "limit", "remaining"].contains(&key)
                    && window.get("unit").and_then(Value::as_str) == Some("percent")
                {
                    "value.percent"
                } else {
                    role
                };
                row(format!("{label} · {suffix}"), role, value.clone());
            }
        }
    }
    if let Some(count) = facts
        .get("reset_count")
        .filter(|value| **value != Value::Null)
    {
        row(
            "Quota resets available".into(),
            "value.number",
            count.clone(),
        );
    }
    for credit in facts
        .get("reset_credits")
        .and_then(Value::as_list)
        .unwrap_or(&[])
    {
        let label = credit
            .get("label")
            .and_then(Value::as_str)
            .unwrap_or("Quota reset");
        for key in ["status", "granted", "expires"] {
            if let Some(value) = credit.get(key).filter(|value| **value != Value::Null) {
                row(
                    format!("{label} · {key}"),
                    if key == "status" {
                        "value.text"
                    } else {
                        "value.datetime"
                    },
                    value.clone(),
                );
            }
        }
    }
    if let Some(credits) = facts.get("credits") {
        for key in [
            "enabled",
            "currency",
            "limit",
            "used",
            "remaining",
            "unlimited",
            "has_credits",
        ] {
            if let Some(value) = credits.get(key).filter(|value| **value != Value::Null) {
                row(
                    format!("Credits · {key}"),
                    if matches!(value, Value::Bool(_) | Value::Str(_)) {
                        "value.text"
                    } else {
                        "value.number"
                    },
                    value.clone(),
                );
            }
        }
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_preserves_money_tokens_percentages_and_reset_time_as_typed_facts() {
        let document = presentation(&Value::map([
            (
                "session",
                Value::map([
                    ("calls", Value::Int(2)),
                    ("cost_micros", Value::Int(1_230_000)),
                    ("input_tokens", Value::Int(2048)),
                ]),
            ),
            (
                "quota",
                Value::map([
                    ("unavailable", Value::Bool(false)),
                    (
                        "windows",
                        Value::list([Value::map([
                            ("label", Value::str("Daily")),
                            ("unit", Value::str("percent")),
                            ("used", Value::Int(42)),
                            ("reset", Value::str("2026-09-17T00:00:00Z")),
                        ])]),
                    ),
                ]),
            ),
        ]));
        misa_proto::view::validate(&document).unwrap();
        let facts = document
            .children
            .iter()
            .flat_map(|row| &row.children)
            .filter_map(|node| {
                if let misa_proto::view::Kind::Fact { value } = &node.kind {
                    Some((node.role.as_str(), value.clone()))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>();
        assert!(facts.contains(&("value.money", Value::Int(1_230_000))));
        assert!(facts.contains(&("value.tokens", Value::Int(2048))));
        assert!(facts.contains(&("value.percent", Value::Int(42))));
        assert!(facts.contains(&("value.datetime", Value::str("2026-09-17T00:00:00Z"))));
        assert!(document.actions.is_empty());
    }

    #[tokio::test]
    async fn usage_document_is_a_finite_read_without_shared_panel_mutation() {
        use misa_proto::observation::{Content, Encoding, Member, Selection};
        let runtime = crate::Runtime::start(
            "usage-report",
            "Usage",
            None,
            std::sync::Arc::new(misa_kernel::LocalKernel::new(
                misa_kernel::ScriptedProvider::always("done"),
            )),
            "scripted",
            "test",
            Value::Null,
        );
        let before = runtime.rev();
        let snapshot = runtime
            .read_selection(&Selection {
                scope: runtime.scope(),
                members: std::collections::BTreeMap::from([
                    (
                        "document".into(),
                        Member {
                            query: Query::new("usage.presentation"),
                            contract: "usage.presentation@1".into(),
                            encoding: Encoding::Document,
                            optional: false,
                        },
                    ),
                    (
                        "data".into(),
                        Member {
                            query: Query::new("usage.report"),
                            contract: "usage.report@1".into(),
                            encoding: Encoding::Value,
                            optional: false,
                        },
                    ),
                ]),
            })
            .unwrap();
        assert!(
            matches!(&snapshot.members["document"],Content::Document(document) if document.tree.role=="usage.report")
        );
        assert!(
            matches!(&snapshot.members["data"],Content::Value(value) if value.get("session").is_some())
        );
        assert_eq!(runtime.rev(), before);
        assert_eq!(
            runtime.state.lock().unwrap().state.db().get("panel"),
            Some(&Value::Null)
        );
        runtime.shutdown_complete().await;
    }
}
