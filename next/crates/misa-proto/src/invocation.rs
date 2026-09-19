//! Commands are domain operations; bindings are optional presentation affordances.
//! Local actions (focus, dismiss, copy) are frontend state and are not invocations.
use crate::{
    Fault,
    observation::Scope,
    schema::{Limits, Schema},
};
use misa_value::Value;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub id: String,
    pub input: Schema,
    pub result: Schema,
    /// Preparation affordance, not caller authorization. Request responses need
    /// their private schema/binding and specialized secret-value presentation.
    #[serde(default)]
    pub preparation: Preparation,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Preparation {
    #[default]
    Direct,
    Request,
}
impl Command {
    pub fn validate(&self) -> Result<(), Fault> {
        if self.id.is_empty() {
            return Err(Fault::protocol("Command needs an identity"));
        }
        self.input
            .check()
            .map_err(|e| Fault::protocol(e.to_string()))?;
        self.result
            .check()
            .map_err(|e| Fault::protocol(e.to_string()))
    }
}

/// Request identity correlates results; it is not an idempotency key.
/// Caller authority comes from the connection, never from serialized input.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Invocation {
    pub id: u64,
    pub scope: Scope,
    pub command: String,
    pub input: Value,
}
impl Invocation {
    /// Structural validation only. The owner must also check current incarnation,
    /// authorization and operation-specific resource preconditions.
    pub fn validate(&self, command: &Command, limits: Limits) -> Result<(), Fault> {
        self.scope.validate()?;
        command.validate()?;
        if self.command != command.id {
            return Err(Fault::protocol("Invocation names a different command"));
        }
        command
            .input
            .validate_with(&self.input, limits)
            .map_err(|e| Fault::protocol(e.to_string()))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationRef {
    pub scope: Scope,
    pub id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Reply {
    pub id: u64,
    pub outcome: Outcome,
}

/// Accepted is not completed or necessarily durable. The installed command
/// declares its acceptance boundary; external work must not be retried blindly.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum Outcome {
    Completed {
        value: Value,
    },
    Rejected {
        fault: Fault,
    },
    /// Effects may have occurred; reconcile state and never retry automatically.
    Indeterminate {
        fault: Fault,
    },
    Accepted {
        operation: OperationRef,
    },
}

/// Maps stable presentation field identities to top-level command parameters.
/// No expressions, state paths, implicit merging, or caller-supplied authority.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ActionBinding {
    pub command: String,
    #[serde(default)]
    pub bound: BTreeMap<String, Value>,
    #[serde(default)]
    pub inputs: BTreeMap<String, String>,
}
/// Installed affordance metadata. Its identity is referenced by semantic actions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub id: String,
    pub binding: ActionBinding,
}
pub const CATALOG: &str = "commands.catalog";
pub const BINDING_CATALOG: &str = "actions.catalog";
pub fn catalog_definition() -> crate::query::Definition {
    crate::query::Definition {
        id: CATALOG.into(),
        arguments: vec![],
        contract: "commands.catalog@2".into(),
        result: crate::query::ResultContract::Data {
            schema: Schema::List {
                items: Box::new(Schema::Record {
                    fields: [
                        ("id", Schema::String),
                        ("input", Schema::Value),
                        ("result", Schema::Value),
                        (
                            "preparation",
                            Schema::Choice {
                                values: vec![
                                    crate::schema::Literal::String("direct".into()),
                                    crate::schema::Literal::String("request".into()),
                                ],
                            },
                        ),
                    ]
                    .into_iter()
                    .map(|(name, schema)| {
                        (
                            name.into(),
                            crate::schema::Field {
                                schema,
                                optional: false,
                            },
                        )
                    })
                    .collect(),
                    allow_unknown: false,
                }),
            },
        },
    }
}
impl ActionBinding {
    /// Validate installation against the command rather than waiting for a click.
    pub fn validate_for(&self, command: &Command) -> Result<(), Fault> {
        self.validate()?;
        command.validate()?;
        if self.command != command.id {
            return Err(Fault::protocol("Binding names a different command"));
        }
        Schema::Value
            .validate(&Value::Map(self.bound.clone().into()))
            .map_err(|error| Fault::protocol(error.to_string()))?;
        let Schema::Record {
            fields,
            allow_unknown,
        } = &command.input
        else {
            return Err(Fault::protocol(
                "Action binding requires record command input",
            ));
        };
        for (parameter, value) in &self.bound {
            match fields.get(parameter) {
                Some(field) => field
                    .schema
                    .validate(value)
                    .map_err(|error| Fault::protocol(error.to_string()))?,
                None if !allow_unknown => {
                    return Err(Fault::protocol("Binding names unknown command parameter"));
                }
                None => {}
            }
        }
        for parameter in self.inputs.values() {
            if !allow_unknown && !fields.contains_key(parameter) {
                return Err(Fault::protocol("Binding maps to unknown command parameter"));
            }
        }
        for (parameter, field) in fields {
            if !field.optional
                && !self.bound.contains_key(parameter)
                && !self.inputs.values().any(|value| value == parameter)
            {
                return Err(Fault::protocol(
                    "Binding cannot supply a required command parameter",
                ));
            }
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), Fault> {
        if self.command.is_empty() {
            return Err(Fault::protocol("Action binding needs a command"));
        }
        let mut targets = BTreeSet::new();
        for (field, parameter) in &self.inputs {
            if field.is_empty() || parameter.is_empty() {
                return Err(Fault::protocol(
                    "Action input mapping needs field and parameter identities",
                ));
            }
            if self.bound.contains_key(parameter) || !targets.insert(parameter) {
                return Err(Fault::protocol(
                    "Action input mapping collides with another parameter",
                ));
            }
        }
        if self.bound.keys().any(String::is_empty) {
            return Err(Fault::protocol("Bound parameters need identities"));
        }
        Ok(())
    }
    /// Client-side preparation is convenience, never authorization. The server
    /// independently validates resulting parameters and resolves owned resources.
    /// Missing fields remain missing; command schemas decide whether they are required.
    pub fn prepare(&self, fields: &BTreeMap<String, Value>) -> Result<Value, Fault> {
        self.validate()?;
        let mut input = self.bound.clone();
        for (field, value) in fields {
            let parameter = self
                .inputs
                .get(field)
                .ok_or_else(|| Fault::protocol("Submitted field is not declared by the action"))?;
            input.insert(parameter.clone(), value.clone());
        }
        Ok(Value::Map(input.into()))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn preparation_roundtrips_and_omitted_metadata_defaults_to_direct() {
        let command = super::Command {
            id: "respond".into(),
            input: crate::schema::Schema::Bool,
            result: crate::schema::Schema::Bool,
            preparation: super::Preparation::Request,
        };
        let mut bytes = Vec::new();
        ciborium::ser::into_writer(&command, &mut bytes).unwrap();
        let decoded: super::Command = ciborium::de::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(decoded, command);
        let mut value: misa_value::Value = ciborium::de::from_reader(bytes.as_slice()).unwrap();
        let misa_value::Value::Map(fields) = &mut value else {
            panic!()
        };
        std::sync::Arc::make_mut(fields).remove("preparation");
        bytes.clear();
        ciborium::ser::into_writer(&value, &mut bytes).unwrap();
        let decoded: super::Command = ciborium::de::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(decoded.preparation, super::Preparation::Direct);
    }
    use super::*;
    use crate::{observation::ScopeId, schema::Field};
    fn scope() -> Scope {
        Scope {
            id: ScopeId::Session { id: "demo".into() },
            incarnation: "run-1".into(),
        }
    }
    fn binding() -> ActionBinding {
        ActionBinding {
            command: "answer".into(),
            bound: BTreeMap::from([("request".into(), Value::str("owned-request"))]),
            inputs: BTreeMap::from([("answer-field".into(), "answer".into())]),
        }
    }
    #[test]
    fn binding_maps_fields_without_overriding_context() {
        let action = binding();
        let value = action
            .prepare(&BTreeMap::from([(
                "answer-field".into(),
                Value::Bool(true),
            )]))
            .unwrap();
        assert_eq!(
            value,
            Value::map([
                ("request", Value::str("owned-request")),
                ("answer", Value::Bool(true))
            ])
        );
        assert!(
            action
                .prepare(&BTreeMap::from([("request".into(), Value::str("other"))]))
                .is_err()
        );
        let mut invalid = action.clone();
        invalid.inputs.insert("evil".into(), "request".into());
        assert!(invalid.prepare(&BTreeMap::new()).is_err());
        let mut invalid = action;
        invalid.inputs.insert("other".into(), "answer".into());
        assert!(invalid.validate().is_err());
    }
    #[test]
    fn input_validation_does_not_echo_secret_values() {
        let command = Command {
            preparation: Default::default(),
            id: "answer".into(),
            input: Schema::Record {
                fields: BTreeMap::from([(
                    "answer".into(),
                    Field {
                        schema: Schema::Bool,
                        optional: false,
                    },
                )]),
                allow_unknown: false,
            },
            result: Schema::Bool,
        };
        let call = Invocation {
            id: 1,
            scope: scope(),
            command: "answer".into(),
            input: Value::map([("answer", Value::str("secret-credential"))]),
        };
        let fault = call.validate(&command, Limits::default()).unwrap_err();
        assert!(!fault.message.contains("secret-credential"));
        let fault = binding()
            .prepare(&BTreeMap::from([(
                "secret-field-name".into(),
                Value::str("secret-credential"),
            )]))
            .unwrap_err();
        assert!(!fault.message.contains("secret"));
    }
    #[test]
    fn operation_acceptance_carries_an_incarnation_bound_reference() {
        let reply = Reply {
            id: 42,
            outcome: Outcome::Accepted {
                operation: OperationRef {
                    scope: scope(),
                    id: "work-7".into(),
                },
            },
        };
        let mut bytes = Vec::new();
        ciborium::into_writer(&reply, &mut bytes).unwrap();
        let decoded: Reply = ciborium::from_reader(bytes.as_slice()).unwrap();
        assert_eq!(decoded, reply);
        let call = Invocation {
            id: 42,
            scope: Scope {
                incarnation: String::new(),
                ..scope()
            },
            command: "x".into(),
            input: Value::Bool(true),
        };
        assert!(
            call.validate(
                &Command {
                    preparation: Default::default(),
                    id: "x".into(),
                    input: Schema::Bool,
                    result: Schema::Bool
                },
                Limits::default()
            )
            .is_err()
        );
    }
}
