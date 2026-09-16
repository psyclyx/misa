//! Shared Misa input preparation. This chooses a domain command or a finite read;
//! it does not open a picker, own field drafts, or choose presentation placement.
use std::{collections::BTreeMap, time::Duration};

use misa_proto::{
    Fault,
    invocation::Command,
    observation::{Member, Selection},
    preparation::{Shortcut, Source, Target},
    view::BlobRef,
};
use misa_value::Value;

use crate::{
    driver::Client,
    interface::{self, Interface},
};

pub enum Prepared {
    Invoke { command: Command, input: Value },
    Read { member: Member },
}

pub struct Interaction {
    pub interface: Interface,
    pub sources: Vec<Source>,
    pub shortcuts: Vec<Shortcut>,
}

impl Interaction {
    pub async fn load(client: &Client, interface: Interface) -> Result<Self, Fault> {
        let mut members = BTreeMap::new();
        for (name, id) in [
            ("sources", misa_proto::preparation::SOURCES),
            ("shortcuts", misa_proto::preparation::SHORTCUTS),
        ] {
            if interface.queries.contains_key(id) {
                members.insert(name.into(), interface.query(id, vec![])?);
            }
        }
        let mut prepared = Self {
            interface,
            sources: vec![],
            shortcuts: vec![],
        };
        if !members.is_empty() {
            let data = client
                .read(
                    Selection {
                        scope: prepared.interface.scope.clone(),
                        members,
                    },
                    Duration::from_secs(10),
                )
                .await?;
            if data.members().contains_key("sources") {
                prepared.sources = interface::decode(interface::data(&data, "sources")?)?;
            }
            if data.members().contains_key("shortcuts") {
                prepared.shortcuts = interface::decode(interface::data(&data, "shortcuts")?)?;
            }
        }
        prepared.validate()?;
        Ok(prepared)
    }

    /// Validate references before exposing remotely supplied preparation metadata
    /// to a composer. Catalog visibility is not command or query authority.
    pub fn validate(&self) -> Result<(), Fault> {
        let mut sources = std::collections::BTreeSet::new();
        for source in &self.sources {
            if source.id.is_empty() || !sources.insert(source.id.as_str()) {
                return Err(Fault::query(
                    "Completion source identities must be unique and nonempty",
                ));
            }
            self.validate_member(&source.member)?;
            if source.member.encoding != misa_proto::observation::Encoding::Value {
                return Err(Fault::query("Completion source must return data"));
            }
        }
        let mut shortcuts = std::collections::BTreeSet::new();
        for shortcut in &self.shortcuts {
            if shortcut.id.is_empty() || !shortcuts.insert(shortcut.id.as_str()) {
                return Err(Fault::query(
                    "Shortcut identities must be unique and nonempty",
                ));
            }
            match &shortcut.target {
                Target::Command { command } if !self.interface.commands.contains_key(command) => {
                    return Err(Fault::query("Shortcut names an unavailable command"));
                }
                Target::Read { member } => self.validate_member(member)?,
                _ => {}
            }
            let mut arguments = std::collections::BTreeSet::new();
            for argument in &shortcut.args {
                if argument.name.is_empty() || !arguments.insert(argument.name.as_str()) {
                    return Err(Fault::query(
                        "Shortcut argument identities must be unique and nonempty",
                    ));
                }
                if argument
                    .source
                    .as_ref()
                    .is_some_and(|source| !sources.contains(source.as_str()))
                {
                    return Err(Fault::query(
                        "Shortcut names an unavailable completion source",
                    ));
                }
            }
        }
        Ok(())
    }

    fn validate_member(&self, member: &Member) -> Result<(), Fault> {
        self.interface
            .queries
            .get(&member.query.id)
            .ok_or_else(|| Fault::query("Preparation names an unavailable query"))?
            .validate(member)
    }

    pub fn invoke(&self, id: &str, input: Value) -> Result<Prepared, Fault> {
        let command = self
            .interface
            .commands
            .get(id)
            .ok_or_else(|| Fault::unsupported("Command is not available in this session"))?;
        command
            .input
            .validate(&input)
            .map_err(|error| Fault::protocol(error.to_string()))?;
        Ok(Prepared::Invoke {
            command: command.clone(),
            input,
        })
    }

    pub fn prompt(
        &self,
        text: String,
        attachments: Vec<BlobRef>,
        interrupt: bool,
    ) -> Result<Prepared, Fault> {
        let attachments = attachments
            .into_iter()
            .map(|attachment| {
                Ok(Value::map([
                    ("hash", Value::str(attachment.hash)),
                    (
                        "len",
                        Value::Int(i64::try_from(attachment.len).map_err(|_| {
                            Fault::protocol("Attachment length exceeds the data contract")
                        })?),
                    ),
                    (
                        "media",
                        attachment.media.map(Value::str).unwrap_or(Value::Null),
                    ),
                ]))
            })
            .collect::<Result<Vec<_>, Fault>>()?;
        let input = Value::map([
            ("text", Value::str(text)),
            ("attachments", Value::list(attachments)),
        ]);
        self.invoke(
            if interrupt {
                "session.interrupt"
            } else {
                "session.prompt"
            },
            input,
        )
    }

    pub fn cancel(&self, target: Option<String>) -> Result<Prepared, Fault> {
        self.invoke(
            "session.cancel",
            Value::map([("target", target.map(Value::str).unwrap_or(Value::Null))]),
        )
    }

    pub fn shortcut(&self, id: &str, input: Value) -> Result<Prepared, Fault> {
        let shortcut = self
            .shortcuts
            .iter()
            .find(|shortcut| shortcut.id == id)
            .ok_or_else(|| Fault::unsupported("Shortcut is not available"))?;
        match &shortcut.target {
            Target::Command { command } => self.invoke(command, input),
            Target::Read { member } => Ok(Prepared::Read {
                member: member.clone(),
            }),
        }
    }

    pub fn action(&self, id: &str, fields: &BTreeMap<String, Value>) -> Result<Prepared, Fault> {
        let (command, input) = self.interface.action(id, fields)?;
        Ok(Prepared::Invoke {
            command: command.clone(),
            input,
        })
    }

    pub fn complete(&self, source: &str, prefix: &str, limit: u32) -> Result<Member, Fault> {
        if !self.sources.iter().any(|entry| entry.id == source) {
            return Err(Fault::unsupported("Completion source is not available"));
        }
        self.interface.query(
            misa_proto::preparation::SEARCH,
            vec![
                Value::str(source),
                Value::str(prefix),
                Value::Int(limit.into()),
            ],
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{
        observation::{Scope, ScopeId},
        query::{Definition, ResultContract},
        schema::Schema,
        preparation::{Arg, SourceKind},
    };

    fn interaction() -> Interaction {
        let query = Definition {
            id: "models".into(),
            arguments: vec![],
            contract: "models@1".into(),
            result: ResultContract::Data {
                schema: Schema::Value,
            },
        };
        let member = query.member(vec![]).unwrap();
        Interaction {
            interface: Interface {
                scope: Scope {
                    id: ScopeId::Session {
                        id: "session".into(),
                    },
                    incarnation: "first".into(),
                },
                queries: BTreeMap::from([(query.id.clone(), query)]),
                commands: BTreeMap::from([(
                    "model.select".into(),
                    Command {
                        preparation: Default::default(), id: "model.select".into(),
                        input: Schema::String,
                        result: Schema::Value,
                    },
                )]),
                actions: BTreeMap::new(),
                presentations: vec![],
            },
            sources: vec![Source {
                id: "models".into(),
                label: "Models".into(),
                kind: SourceKind::Resident,
                member,
            }],
            shortcuts: vec![Shortcut {
                id: "model".into(),
                label: "Model".into(),
                description: String::new(),
                args: vec![Arg::new("model", "Model").from("models")],
                target: Target::Command {
                    command: "model.select".into(),
                },
            }],
        }
    }

    #[test]
    fn rejects_broken_remote_preparation_before_exposing_it() {
        let mut prepared = interaction();
        prepared.validate().unwrap();
        prepared.sources[0].member.contract = "models@2".into();
        assert!(prepared.validate().is_err());
        prepared.sources[0].member.contract = "models@1".into();
        prepared.shortcuts[0].args[0].source = Some("missing".into());
        assert!(prepared.validate().is_err());
        prepared.shortcuts[0].args[0].source = Some("models".into());
        prepared.shortcuts[0].target = Target::Command {
            command: "missing".into(),
        };
        assert!(prepared.validate().is_err());
        prepared.shortcuts[0].target = Target::Read {
            member: prepared.sources[0].member.clone(),
        };
        prepared.validate().unwrap();
        prepared.sources.push(prepared.sources[0].clone());
        assert!(prepared.validate().is_err());
    }

    #[test]
    fn local_read_preparation_never_becomes_a_mutation() {
        let mut prepared = interaction();
        prepared.shortcuts[0].target = Target::Read {
            member: prepared.sources[0].member.clone(),
        };
        prepared.validate().unwrap();
        assert!(matches!(
            prepared.shortcut("model", Value::Null).unwrap(),
            Prepared::Read { .. }
        ));
        assert!(prepared.invoke("model.select", Value::Int(1)).is_err());
        assert!(matches!(
            prepared
                .invoke("model.select", Value::str("available"))
                .unwrap(),
            Prepared::Invoke { .. }
        ));
    }
}
