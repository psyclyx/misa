//! Small data contracts for exported query and command inputs/results.
//!
//! Visibility and credential handling belong to export policy, not schemas. These
//! bounds protect validation work after decoding; framing/decoding has its own bounds.
use misa_value::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Default small-contract budgets. Export owners select larger value budgets when needed.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub depth: usize,
    pub nodes: usize,
    pub bytes: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            depth: 32,
            nodes: 16_384,
            bytes: 1_048_576,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Schema {
    /// Arbitrary immutable data, still subject to recursive limits and finite numbers.
    /// Use for metadata carrying other queries' arguments, not instead of known schemas.
    Value,
    Bool,
    Int,
    Number,
    String,
    Bytes,
    Nullable {
        inner: Box<Schema>,
    },
    List {
        items: Box<Schema>,
    },
    Record {
        fields: BTreeMap<String, Field>,
        #[serde(default)]
        allow_unknown: bool,
    },
    Choice {
        values: Vec<Literal>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub schema: Schema,
    #[serde(default)]
    pub optional: bool,
}

/// Exact scalar alternatives. Floating-point values are deliberately not literals.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum Literal {
    Null,
    Bool(bool),
    Int(i64),
    String(String),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{path}: {message}")]
pub struct ValidationError {
    /// JSON Pointer; the empty string denotes the root.
    pub path: String,
    pub message: String,
}
fn error(path: &str, message: impl Into<String>) -> ValidationError {
    ValidationError {
        path: path.into(),
        message: message.into(),
    }
}
fn child(path: &str, key: &str) -> String {
    format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"))
}
struct Budget {
    nodes: usize,
    bytes: usize,
    limits: Limits,
}
impl Default for Budget {
    fn default() -> Self {
        Self::new(Limits::default())
    }
}
impl Budget {
    fn new(limits: Limits) -> Self {
        Self {
            nodes: 0,
            bytes: 0,
            limits,
        }
    }
    fn visit(&mut self, path: &str, depth: usize, bytes: usize) -> Result<(), ValidationError> {
        self.nodes += 1;
        self.bytes = self.bytes.saturating_add(bytes);
        if depth > self.limits.depth {
            return Err(error(path, "maximum depth exceeded"));
        }
        if self.nodes > self.limits.nodes {
            return Err(error(path, "maximum node count exceeded"));
        }
        if self.bytes > self.limits.bytes {
            return Err(error(path, "maximum byte count exceeded"));
        }
        Ok(())
    }
}
impl Schema {
    /// Check a declaration before installing it in a registry.
    pub fn check(&self) -> Result<(), ValidationError> {
        self.check_at("", 0, &mut Budget::default())
    }
    fn check_at(
        &self,
        path: &str,
        depth: usize,
        budget: &mut Budget,
    ) -> Result<(), ValidationError> {
        budget.visit(path, depth, 0)?;
        match self {
            Self::Nullable { inner } => inner.check_at(&child(path, "inner"), depth + 1, budget)?,
            Self::List { items } => items.check_at(&child(path, "items"), depth + 1, budget)?,
            Self::Record { fields, .. } => {
                for (name, field) in fields {
                    let path = child(&child(path, "fields"), name);
                    budget.visit(&path, depth + 1, name.len())?;
                    field.schema.check_at(&path, depth + 1, budget)?;
                }
            }
            Self::Choice { values } => {
                if values.is_empty() {
                    return Err(error(path, "choice must contain a literal"));
                }
                let mut seen = std::collections::BTreeSet::new();
                for (index, value) in values.iter().enumerate() {
                    let path = child(&child(path, "values"), &index.to_string());
                    budget.visit(
                        &path,
                        depth + 1,
                        match value {
                            Literal::String(s) => s.len(),
                            _ => 0,
                        },
                    )?;
                    if !seen.insert(value) {
                        return Err(error(&path, "duplicate choice literal"));
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    pub fn validate(&self, value: &Value) -> Result<(), ValidationError> {
        self.validate_with(value, Limits::default())
    }
    /// Value budgets belong to the invocation/export, independently of schema declaration limits.
    pub fn validate_with(&self, value: &Value, limits: Limits) -> Result<(), ValidationError> {
        self.check()?;
        // Bound the entire value, including permitted unknown fields, before matching.
        bound(value, "", 0, &mut Budget::new(limits))?;
        self.matches(value, "")
    }
    fn matches(&self, value: &Value, path: &str) -> Result<(), ValidationError> {
        let valid = match (self, value) {
            (Self::Value, _) => true,
            (Self::Bool, Value::Bool(_))
            | (Self::Int, Value::Int(_))
            | (Self::Number, Value::Int(_))
            | (Self::String, Value::Str(_))
            | (Self::Bytes, Value::Bytes(_)) => true,
            (Self::Number, Value::Float(n)) => n.is_finite(),
            (Self::Nullable { .. }, Value::Null) => true,
            (Self::Nullable { inner }, value) => return inner.matches(value, path),
            (Self::List { items }, Value::List(values)) => {
                for (index, value) in values.iter().enumerate() {
                    items.matches(value, &child(path, &index.to_string()))?;
                }
                true
            }
            (
                Self::Record {
                    fields,
                    allow_unknown,
                },
                Value::Map(values),
            ) => {
                for (name, field) in fields {
                    let path = child(path, name);
                    match values.get(name) {
                        Some(value) => field.schema.matches(value, &path)?,
                        None if !field.optional => {
                            return Err(error(&path, "required field is missing"));
                        }
                        None => {}
                    }
                }
                if !allow_unknown {
                    for name in values.keys() {
                        if !fields.contains_key(name) {
                            return Err(error(&child(path, name), "unknown field"));
                        }
                    }
                }
                true
            }
            (Self::Choice { values }, value) => {
                values.iter().any(|literal| match (literal, value) {
                    (Literal::Null, Value::Null) => true,
                    (Literal::Bool(a), Value::Bool(b)) => a == b,
                    (Literal::Int(a), Value::Int(b)) => a == b,
                    (Literal::String(a), Value::Str(b)) => a.as_str() == b.as_ref(),
                    _ => false,
                })
            }
            _ => false,
        };
        if valid {
            Ok(())
        } else {
            Err(error(
                path,
                format!("{} does not satisfy the declared type", value.kind()),
            ))
        }
    }
}
fn bound(
    value: &Value,
    path: &str,
    depth: usize,
    budget: &mut Budget,
) -> Result<(), ValidationError> {
    budget.visit(
        path,
        depth,
        match value {
            Value::Str(s) => s.len(),
            Value::Bytes(b) => b.len(),
            _ => 0,
        },
    )?;
    match value {
        Value::Float(n) if !n.is_finite() => return Err(error(path, "number must be finite")),
        Value::List(values) => {
            for (index, value) in values.iter().enumerate() {
                bound(value, &child(path, &index.to_string()), depth + 1, budget)?;
            }
        }
        Value::Map(values) => {
            for (name, value) in values.iter() {
                let path = child(path, name);
                budget.visit(&path, depth + 1, name.len())?;
                bound(value, &path, depth + 1, budget)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arbitrary_metadata_values_keep_all_recursive_bounds() {
        assert!(
            Schema::Value
                .validate(&Value::map([(
                    "args",
                    Value::list([Value::Bool(true), Value::Null])
                )]))
                .is_ok()
        );
        assert!(
            Schema::Value
                .validate(&Value::list([Value::Float(f64::INFINITY)]))
                .is_err()
        );
        assert!(
            Schema::Value
                .validate_with(
                    &Value::str("too long"),
                    Limits {
                        bytes: 2,
                        ..Limits::default()
                    }
                )
                .is_err()
        );
        assert!(
            Schema::Value
                .validate_with(
                    &Value::list([Value::Null, Value::Null]),
                    Limits {
                        nodes: 2,
                        ..Limits::default()
                    }
                )
                .is_err()
        );
        assert!(
            Schema::Value
                .validate_with(
                    &Value::list([Value::list([Value::Null])]),
                    Limits {
                        depth: 1,
                        ..Limits::default()
                    }
                )
                .is_err()
        );
    }
    fn record() -> Schema {
        Schema::Record {
            fields: BTreeMap::from([
                (
                    "name".into(),
                    Field {
                        schema: Schema::String,
                        optional: false,
                    },
                ),
                (
                    "count".into(),
                    Field {
                        schema: Schema::Nullable {
                            inner: Box::new(Schema::Int),
                        },
                        optional: true,
                    },
                ),
            ]),
            allow_unknown: false,
        }
    }
    #[test]
    fn absent_optional_is_distinct_from_required_nullable() {
        let schema = record();
        assert!(
            schema
                .validate(&Value::map([("name", Value::str("a"))]))
                .is_ok()
        );
        assert!(
            schema
                .validate(&Value::map([
                    ("name", Value::str("a")),
                    ("count", Value::Null)
                ]))
                .is_ok()
        );
        assert_eq!(schema.validate(&Value::map([])).unwrap_err().path, "/name");
        assert_eq!(
            schema
                .validate(&Value::map([("name", Value::Null)]))
                .unwrap_err()
                .path,
            "/name"
        );
        assert_eq!(
            schema
                .validate(&Value::map([
                    ("name", Value::str("a")),
                    ("extra", Value::Bool(true))
                ]))
                .unwrap_err()
                .path,
            "/extra"
        );
    }
    #[test]
    fn paths_escape_field_names_and_report_list_indices_without_values() {
        let schema = Schema::Record {
            fields: BTreeMap::from([(
                "a/b~c".into(),
                Field {
                    schema: Schema::List {
                        items: Box::new(Schema::Int),
                    },
                    optional: false,
                },
            )]),
            allow_unknown: false,
        };
        let err = schema
            .validate(&Value::map([(
                "a/b~c",
                Value::list([Value::str("secret")]),
            )]))
            .unwrap_err();
        assert_eq!(err.path, "/a~1b~0c/0");
        assert!(!err.to_string().contains("secret"));
    }
    #[test]
    fn numbers_and_choices_are_precise() {
        assert!(Schema::Number.validate(&Value::Int(1)).is_ok());
        assert!(Schema::Number.validate(&Value::Float(1.5)).is_ok());
        assert!(Schema::Number.validate(&Value::Float(f64::NAN)).is_err());
        assert!(Schema::Int.validate(&Value::Float(1.0)).is_err());
        let choices = Schema::Choice {
            values: vec![Literal::Int(1), Literal::String("one".into())],
        };
        assert!(choices.validate(&Value::Int(1)).is_ok());
        assert!(choices.validate(&Value::Float(1.0)).is_err());
        assert!(Schema::Choice { values: vec![] }.check().is_err());
        assert!(
            Schema::Choice {
                values: vec![Literal::Null, Literal::Null]
            }
            .check()
            .is_err()
        );
    }
    #[test]
    fn unknown_fields_still_obey_resource_and_number_limits() {
        let schema = Schema::Record {
            fields: BTreeMap::new(),
            allow_unknown: true,
        };
        assert!(
            schema
                .validate(&Value::map([(
                    "x",
                    Value::str("a".repeat(Limits::default().bytes + 1))
                )]))
                .is_err()
        );
        assert!(
            schema
                .validate(&Value::map([("x", Value::Float(f64::INFINITY))]))
                .is_err()
        );
        assert!(
            schema
                .validate(&Value::map([(
                    "x",
                    Value::list((0..Limits::default().nodes).map(|_| Value::Null))
                )]))
                .is_err()
        );
        let mut deep = Value::Null;
        for _ in 0..=Limits::default().depth {
            deep = Value::list([deep]);
        }
        assert!(schema.validate(&Value::map([("x", deep)])).is_err());
        let mut deep = Schema::Int;
        for _ in 0..=Limits::default().depth {
            deep = Schema::Nullable {
                inner: Box::new(deep),
            };
        }
        assert!(deep.check().is_err());
    }
    #[test]
    fn exports_can_allow_large_values_without_weakening_declaration_limits() {
        let value = Value::str("x".repeat(Limits::default().bytes + 1));
        assert!(Schema::String.validate(&value).is_err());
        assert!(
            Schema::String
                .validate_with(
                    &value,
                    Limits {
                        bytes: 2_000_000,
                        ..Limits::default()
                    }
                )
                .is_ok()
        );
    }
    #[test]
    fn schema_roundtrips_and_record_defaults_closed() {
        let mut bytes = Vec::new();
        ciborium::into_writer(&record(), &mut bytes).unwrap();
        let restored: Schema = ciborium::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(restored, record());
        let value = Value::map([("type", Value::str("record")), ("fields", Value::map([]))]);
        bytes.clear();
        ciborium::into_writer(&value, &mut bytes).unwrap();
        let restored: Schema = ciborium::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(
            restored,
            Schema::Record {
                fields: BTreeMap::new(),
                allow_unknown: false
            }
        );
    }
}
