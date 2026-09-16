//! Local forms derived from installed bindings, never owner-visible drafts.
use crate::interface::Interface;
use misa_proto::{
    Fault,
    invocation::{ActionBinding, Command},
    schema::{Field, Schema},
};
use misa_value::Value;
use std::collections::BTreeMap;
#[derive(Clone, Debug)]
pub struct Form {
    pub title: String,
    pub fields: Vec<(String, Field)>,
    binding: ActionBinding,
    command: Command,
}
impl Form {
    pub fn command(interface: &Interface, id: &str) -> Result<Self, Fault> {
        let command = interface
            .commands
            .get(id)
            .ok_or_else(|| Fault::query("Unknown command"))?
            .clone();
        let Schema::Record { fields, .. } = &command.input else {
            return Err(Fault::unsupported("Command form requires record input"));
        };
        let binding = ActionBinding {
            command: id.into(),
            bound: BTreeMap::new(),
            inputs: fields.keys().map(|id| (id.clone(), id.clone())).collect(),
        };
        binding.validate_for(&command)?;
        Ok(Self {
            title: id.into(),
            fields: fields
                .iter()
                .map(|(id, field)| (id.clone(), field.clone()))
                .collect(),
            binding,
            command,
        })
    }

    pub fn action(interface: &Interface, id: &str) -> Result<Self, Fault> {
        let binding = interface
            .actions
            .get(id)
            .ok_or_else(|| Fault::query("Unknown action"))?
            .binding
            .clone();
        let command = interface
            .commands
            .get(&binding.command)
            .ok_or_else(|| Fault::query("Unknown command"))?
            .clone();
        binding.validate_for(&command)?;
        let Schema::Record { fields, .. } = &command.input else {
            return Err(Fault::query("Action requires record input"));
        };
        let fields = binding
            .inputs
            .iter()
            .map(|(id, parameter)| (id.clone(), fields[parameter].clone()))
            .collect();
        Ok(Self {
            title: id.into(),
            fields,
            binding,
            command,
        })
    }
    pub fn prepare(&self, drafts: &BTreeMap<String, String>) -> Result<(String, Value), Fault> {
        let mut values = BTreeMap::new();
        for (id, field) in &self.fields {
            let text = drafts.get(id).map(String::as_str).unwrap_or("");
            if text.is_empty() && field.optional {
                continue;
            }
            if text.len() > misa_proto::schema::Limits::default().bytes {
                return Err(Fault::query("Form field exceeds input limit"));
            }
            let value = if matches!(field.schema, Schema::String) {
                Value::str(text)
            } else {
                serde_json::from_str::<Value>(text)
                    .map_err(|_| Fault::query(format!("{id}: enter a valid JSON value")))?
            };
            let value = typed(&field.schema, value)?;
            field
                .schema
                .validate(&value)
                .map_err(|error| Fault::query(format!("{id}: {error}")))?;
            values.insert(id.clone(), value);
        }
        let input = self.binding.prepare(&values)?;
        self.command
            .input
            .validate(&input)
            .map_err(|error| Fault::query(error.to_string()))?;
        Ok((self.command.id.clone(), input))
    }
}
// JSON arrays provide an explicit portable spelling for schema-declared bytes.
fn typed(schema: &Schema, value: Value) -> Result<Value, Fault> {
    match (schema, value) {
        (Schema::Bytes, Value::List(items)) => Ok(Value::Bytes(
            items
                .iter()
                .map(|item| {
                    item.as_i64()
                        .and_then(|v| u8::try_from(v).ok())
                        .ok_or_else(|| Fault::query("Bytes require integers from 0 to 255"))
                })
                .collect::<Result<Vec<_>, _>>()?
                .into(),
        )),
        (Schema::Nullable { inner }, value) if value != Value::Null => typed(inner, value),
        (Schema::List { items }, Value::List(values)) => Ok(Value::list(
            values
                .iter()
                .cloned()
                .map(|value| typed(items, value))
                .collect::<Result<Vec<_>, _>>()?,
        )),
        (Schema::Record { fields, .. }, Value::Map(values)) => Ok(Value::Map(
            values
                .iter()
                .map(|(id, value)| {
                    Ok((
                        id.clone(),
                        match fields.get(id) {
                            Some(field) => typed(&field.schema, value.clone())?,
                            None => value.clone(),
                        },
                    ))
                })
                .collect::<Result<BTreeMap<_, _>, Fault>>()?
                .into(),
        )),
        (_, value) => Ok(value),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn typed_fields_validate_without_overwriting_bound_context() {
        let command = Command {
            id: "feed".into(),
            input: Schema::Record {
                fields: BTreeMap::from([
                    (
                        "amount".into(),
                        Field {
                            schema: Schema::Int,
                            optional: false,
                        },
                    ),
                    (
                        "pet".into(),
                        Field {
                            schema: Schema::String,
                            optional: false,
                        },
                    ),
                ]),
                allow_unknown: false,
            },
            result: Schema::Nullable {
                inner: Box::new(Schema::String),
            },
        };
        let form = Form {
            title: "Feed".into(),
            fields: vec![(
                "quantity".into(),
                Field {
                    schema: Schema::Int,
                    optional: false,
                },
            )],
            binding: ActionBinding {
                command: "feed".into(),
                bound: BTreeMap::from([("pet".into(), Value::str("owned"))]),
                inputs: BTreeMap::from([("quantity".into(), "amount".into())]),
            },
            command,
        };
        assert!(
            form.prepare(&BTreeMap::from([("quantity".into(), "wrong".into())]))
                .is_err()
        );
        let (_, input) = form
            .prepare(&BTreeMap::from([("quantity".into(), "3".into())]))
            .unwrap();
        assert_eq!(input.get("amount"), Some(&Value::Int(3)));
        assert_eq!(input.get("pet"), Some(&Value::str("owned")));
    }
}
