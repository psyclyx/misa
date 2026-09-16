//! Usage data derived from recorded attempts. These queries have no presentation contract.
use misa_reframe::{Registry, derived_query};
use misa_proto::Query;
use misa_value::Value;

pub fn subscriptions(registry: Registry) -> Registry {
    registry
        .subscription("session.status-report", derived_query([Query::new("session.status")], |inputs| {
            let session = &inputs[0];
            Value::map(["id", "status", "provider", "model", "effort"].map(|key| (key, session.get(key).cloned().unwrap_or(Value::Null))))
        }))
        .subscription("usage.report", derived_query([Query::new("usage.session"), Query::new("usage.last-request"), Query::new("usage.selected-quota")], |inputs| {
            Value::map([("session", inputs[0].clone()), ("last_request", inputs[1].clone()), ("quota", inputs[2].clone())])
        }))
        .subscription("usage.session", derived_query([Query::new("session.attempts")], |inputs| {
            let rows = inputs[0].as_list().unwrap_or(&[]);
            let sum = |key: &str| Value::Int(rows.iter().filter_map(|row| row.get(key).and_then(Value::as_i64)).sum());
            Value::map([("input_tokens", sum("input_tokens")), ("output_tokens", sum("output_tokens")), ("cost_micros", sum("cost_micros"))])
        }))
        .subscription("usage.last-request", derived_query([Query::new("session.attempts")], |inputs| {
            inputs[0].as_list().and_then(|rows| rows.last()).cloned().unwrap_or(Value::Null)
        }))
        .subscription("usage.selected-quota", derived_query([Query::new("session.status")], |inputs| {
            let session = &inputs[0];
            if session.get("usage_provider") != session.get("provider") { return Value::Null; }
            session.get("usage").cloned().unwrap_or(Value::Null)
        }))
}

pub fn exports() -> Vec<misa_proto::query::Definition> {
    use misa_proto::{query::{Definition, ResultContract}, schema::{Schema, Field}};
    [("session.status-report", vec!["id", "status", "provider", "model", "effort"]), ("usage.report", vec!["session", "last_request", "quota"])]
        .into_iter().map(|(id, fields)| Definition {
            id: id.into(), arguments: vec![], contract: format!("{id}@1"),
            result: ResultContract::Data { schema: Schema::Record {
                fields: fields.into_iter().map(|name| (name.into(), Field {
                    schema: if id == "session.status-report" { Schema::Nullable { inner: Box::new(Schema::String) } } else { Schema::Value },
                    optional: false,
                })).collect(),
                allow_unknown: false,
            } },
        }).collect()
}

pub fn total_tokens(row: &Value) -> i64 {
    ["input_tokens", "output_tokens"].iter().map(|key| row.get(key).and_then(Value::as_i64).unwrap_or(0)).sum()
}
