//! Local request presentation data. Visibility and unfinished field values belong
//! to a surface; only an explicitly prepared action crosses the owner boundary.
use crate::{interaction::Prepared, interface::Interface};
use misa_proto::{
    Fault, Node,
    invocation::ActionBinding,
    view::{Kind, Span},
};
use misa_value::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct Input {
    pub id: String,
    pub label: String,
    pub secret: bool,
}
#[derive(Clone, Debug)]
pub struct Action {
    pub id: String,
    pub label: String,
    pub binding: ActionBinding,
}
#[derive(Clone, Debug)]
pub struct Model {
    pub id: String,
    pub generation: i64,
    pub title: String,
    pub body: Node,
    pub input: Option<Input>,
    pub form: Option<misa_proto::input::Form>,
    pub actions: Vec<Action>,
}
impl Model {
    pub fn parse(value: &Value, interface: &Interface) -> Result<Option<Self>, Fault> {
        if *value == Value::Null {
            return Ok(None);
        }
        let text = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
                .ok_or_else(|| Fault::query(format!("Request needs {key}")))
        };
        let id = text("id")?.to_string();
        let generation = value
            .get("generation")
            .and_then(Value::as_i64)
            .filter(|generation| *generation >= 0)
            .ok_or_else(|| Fault::query("Request needs a generation"))?;
        let bound = BTreeMap::from([
            ("request".into(), Value::str(&id)),
            ("generation".into(), Value::Int(generation)),
        ]);
        let mut model = Self {
            id: id.clone(),
            generation,
            title: String::new(),
            body: Node::section("request.body").id(format!("request.{id}.body")),
            input: None,
            form: None,
            actions: vec![],
        };
        match text("kind")? {
            "form" => {
                let form: misa_proto::input::Form = crate::interface::decode(value)?;
                form.validate()?;
                model.title = form.title.clone();
                model.form = Some(form);
                for (key, label) in [("resolve", "Submit"), ("cancel", "Cancel request")] {
                    let Some(value) = value.get(key) else {
                        continue;
                    };
                    let binding: ActionBinding = crate::interface::decode(value)?;
                    if binding.bound.get("request") != Some(&Value::str(&id))
                        || binding.bound.get("generation") != Some(&Value::Int(generation))
                    {
                        return Err(Fault::protocol(
                            "Request binding does not match its identity",
                        ));
                    }
                    model.actions.push(Action {
                        id: key.into(),
                        label: label.into(),
                        binding,
                    });
                }
                if !model.actions.iter().any(|action| action.id == "resolve") {
                    return Err(Fault::protocol("Input form needs a resolve action"));
                }
            }
            "credential_value" => {
                model.title = format!("Credential for {}", text("provider")?);
                model.input = Some(Input {
                    id: "value".into(),
                    label: "Credential".into(),
                    secret: true,
                });
                model.actions.push(Action {
                    id: "submit".into(),
                    label: "Store credential".into(),
                    binding: ActionBinding {
                        command: "credentials.resolve".into(),
                        bound: bound.clone(),
                        inputs: BTreeMap::from([("value".into(), "value".into())]),
                    },
                });
            }
            "device_authorization" => {
                model.title = format!("Authorize {}", text("provider")?);
                let challenge = value
                    .get("challenge")
                    .ok_or_else(|| Fault::query("Missing authorization challenge"))?;
                model.body = report("Device authorization", challenge);
            }
            "tool_approval" => {
                model.title = "Tool permission".into();
                model.body = report(
                    "Requested tool",
                    value
                        .get("tool")
                        .ok_or_else(|| Fault::query("Missing requested tool"))?,
                );
                for (id, label, approved) in [
                    ("approve", "Allow tool", true),
                    ("deny", "Deny tool", false),
                ] {
                    let mut bound = bound.clone();
                    bound.insert("approved".into(), Value::Bool(approved));
                    model.actions.push(Action {
                        id: id.into(),
                        label: label.into(),
                        binding: ActionBinding {
                            command: "input.resolve".into(),
                            bound,
                            inputs: BTreeMap::new(),
                        },
                    });
                }
            }
            _ => return Err(Fault::unsupported("Unsupported input request kind")),
        }
        if matches!(text("kind")?, "credential_value" | "device_authorization") {
            model.actions.push(Action {
                id: "cancel".into(),
                label: "Cancel operation".into(),
                binding: ActionBinding {
                    command: "operation.cancel".into(),
                    bound: BTreeMap::from([
                        ("operation".into(), Value::str(&id)),
                        ("generation".into(), Value::Int(generation)),
                    ]),
                    inputs: BTreeMap::new(),
                },
            });
        }
        for action in &model.actions {
            let command = interface
                .commands
                .get(&action.binding.command)
                .ok_or_else(|| Fault::query("Request action is not installed"))?;
            action.binding.validate_for(command)?;
        }
        Ok(Some(model))
    }
    pub fn prepare(
        &self,
        action: &str,
        fields: &BTreeMap<String, Value>,
        interface: &Interface,
    ) -> Result<Prepared, Fault> {
        if action == "resolve"
            && let Some(form) = &self.form
        {
            form.input
                .validate(
                    fields
                        .get("value")
                        .ok_or_else(|| Fault::protocol("Input form needs a value"))?,
                )
                .map_err(|error| Fault::protocol(error.to_string()))?;
        }
        let action = self
            .actions
            .iter()
            .find(|candidate| candidate.id == action)
            .ok_or_else(|| Fault::unsupported("Request action is unavailable"))?;
        let command = interface
            .commands
            .get(&action.binding.command)
            .ok_or_else(|| Fault::unsupported("Request command is unavailable"))?;
        let input = action.binding.prepare(fields)?;
        command
            .input
            .validate(&input)
            .map_err(|error| Fault::protocol(error.to_string()))?;
        Ok(Prepared::Invoke {
            command: command.clone(),
            input,
        })
    }

    pub fn prepare_drafts(
        &self,
        action: &str,
        drafts: &BTreeMap<String, String>,
        interface: &Interface,
    ) -> Result<Prepared, Fault> {
        let binding = &self
            .actions
            .iter()
            .find(|candidate| candidate.id == action)
            .ok_or_else(|| Fault::unsupported("Request action is unavailable"))?
            .binding;
        let values = if action == "resolve"
            && let Some(form) = &self.form
        {
            let misa_proto::schema::Schema::Record { fields, .. } = &form.input else {
                return Err(Fault::protocol("Input form needs record schema"));
            };
            let fields = fields
                .iter()
                .map(|(id, field)| (id.clone(), field.clone()))
                .collect::<Vec<_>>();
            let values = crate::form::parse_fields(&fields, drafts)?;
            BTreeMap::from([("value".into(), Value::Map(values.into()))])
        } else {
            drafts
                .iter()
                .filter(|(id, _)| binding.inputs.contains_key(*id))
                .map(|(id, value)| (id.clone(), Value::str(value)))
                .collect()
        };
        self.prepare(action, &values, interface)
    }
}

/// Generic finite report data, retained locally and rendered by existing semantic
/// components. It is never inserted into a server's canonical conversation.
pub fn report(title: &str, value: &Value) -> Node {
    fn rows(node: &mut Node, path: &str, value: &Value) {
        match value {
            Value::Map(values) => {
                for (key, value) in values.iter() {
                    rows(node, &format!("{path}/{key}"), value);
                }
            }
            Value::List(values) => {
                for (index, value) in values.iter().enumerate() {
                    rows(node, &format!("{path}/{}", index + 1), value);
                }
            }
            _ => {
                let identity = format!("report.row.{}", node.children.len());
                node.children.push(
                    Node::section("report.row")
                        .id(&identity)
                        .child(
                            Node::text(
                                "report.label",
                                [Span::plain(
                                    path.trim_start_matches('/').replace(['/', '_'], " "),
                                )],
                            )
                            .id(format!("{identity}.label")),
                        )
                        .child(
                            Node::new(
                                "value.text",
                                Kind::Fact {
                                    value: if *value == Value::Null {
                                        Value::str("Unavailable")
                                    } else {
                                        value.clone()
                                    },
                                },
                            )
                            .id(format!("{identity}.value")),
                        ),
                );
            }
        }
    }
    let mut node = Node::section("report").id("local.report").label(title);
    rows(&mut node, "", value);
    node
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generic_form_parses_typed_drafts_and_fences_bindings() {
        use misa_proto::schema::{Field, Schema};
        let record = |fields| Schema::Record {
            fields,
            allow_unknown: false,
        };
        let command = misa_proto::invocation::Command {
            id: "input.resolve".into(),
            input: record(BTreeMap::from([
                (
                    "request".into(),
                    Field {
                        schema: Schema::String,
                        optional: false,
                    },
                ),
                (
                    "generation".into(),
                    Field {
                        schema: Schema::Int,
                        optional: false,
                    },
                ),
                (
                    "value".into(),
                    Field {
                        schema: Schema::Value,
                        optional: false,
                    },
                ),
            ])),
            result: Schema::Value,
        };
        let interface = Interface {
            scope: misa_proto::observation::Scope {
                id: misa_proto::observation::ScopeId::Daemon,
                incarnation: "run".into(),
            },
            queries: BTreeMap::new(),
            commands: BTreeMap::from([(command.id.clone(), command)]),
            actions: BTreeMap::new(),
            presentations: vec![],
        };
        let form = misa_proto::input::Form {
            title: "Choose retries".into(),
            input: record(BTreeMap::from([(
                "retries".into(),
                Field {
                    schema: Schema::Int,
                    optional: false,
                },
            )])),
            fields: BTreeMap::new(),
        };
        let binding = ActionBinding {
            command: "input.resolve".into(),
            bound: BTreeMap::from([
                ("request".into(), Value::str("request")),
                ("generation".into(), Value::Int(4)),
            ]),
            inputs: BTreeMap::from([("value".into(), "value".into())]),
        };
        let mut json = serde_json::to_value(form).unwrap();
        for (key, value) in [
            ("id", serde_json::json!("request")),
            ("generation", serde_json::json!(4)),
            ("kind", serde_json::json!("form")),
            ("resolve", serde_json::to_value(binding).unwrap()),
        ] {
            json[key] = value;
        }
        let value = serde_json::from_value(json.clone()).unwrap();
        let model = Model::parse(&value, &interface).unwrap().unwrap();
        assert!(
            model
                .prepare_drafts(
                    "resolve",
                    &BTreeMap::from([("retries".into(), "bad".into())]),
                    &interface
                )
                .is_err()
        );
        let Prepared::Invoke { input, .. } = model
            .prepare_drafts(
                "resolve",
                &BTreeMap::from([("retries".into(), "3".into())]),
                &interface,
            )
            .unwrap()
        else {
            panic!("expected invocation")
        };
        assert_eq!(
            input.get("value").and_then(|v| v.get("retries")),
            Some(&Value::Int(3))
        );
        assert_eq!(input.get("generation"), Some(&Value::Int(4)));
        let mut cancel_model = model.clone();
        cancel_model.actions.push(Action {
            id: "cancel".into(),
            label: "Cancel".into(),
            binding: ActionBinding {
                command: "input.resolve".into(),
                bound: BTreeMap::from([
                    ("request".into(), Value::str("request")),
                    ("generation".into(), Value::Int(4)),
                    ("value".into(), Value::Null),
                ]),
                inputs: BTreeMap::new(),
            },
        });
        assert!(
            cancel_model
                .prepare_drafts(
                    "cancel",
                    &BTreeMap::from([("retries".into(), "bad".into())]),
                    &interface
                )
                .is_ok()
        );
        json["resolve"]["bound"]["generation"] = serde_json::json!(5);
        assert!(Model::parse(&serde_json::from_value(json).unwrap(), &interface).is_err());
    }
    #[test]
    fn report_identities_do_not_alias_nested_paths_and_null_is_not_zero() {
        let value = Value::map([
            ("a/b", Value::Int(0)),
            ("a", Value::map([("b", Value::Null)])),
        ]);
        let document = report("Limits", &value);
        misa_proto::view::validate(&document).unwrap();
        let zero = &document.children[1].children[1].kind;
        assert!(matches!(
            zero,
            Kind::Fact {
                value: Value::Int(0)
            }
        ));
        assert!(
            matches!(&document.children[0].children[1].kind, Kind::Fact { value } if value.as_str()==Some("Unavailable"))
        );
    }
    #[test]
    fn unknown_request_kinds_are_rejected_and_closed_requests_clear() {
        let interface = Interface {
            scope: misa_proto::observation::Scope {
                id: misa_proto::observation::ScopeId::Daemon,
                incarnation: "run".into(),
            },
            queries: BTreeMap::new(),
            commands: BTreeMap::new(),
            actions: BTreeMap::new(),
            presentations: vec![],
        };
        assert!(Model::parse(&Value::Null, &interface).unwrap().is_none());
        assert!(
            Model::parse(
                &Value::map([
                    ("id", Value::str("request")),
                    ("generation", Value::Int(1)),
                    ("kind", Value::str("arbitrary-code"))
                ]),
                &interface
            )
            .is_err()
        );
        assert!(
            Model::parse(
                &Value::map([
                    ("id", Value::str("request")),
                    ("generation", Value::Int(1)),
                    ("kind", Value::str("credential_value")),
                    ("provider", Value::str("provider"))
                ]),
                &interface
            )
            .is_err(),
            "uninstalled request actions must fail validation"
        );
    }
}
