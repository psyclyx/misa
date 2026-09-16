//! Misa interface discovery and preparation, shared by interactive and headless
//! consumers. This describes available operations; placement and drafts are local.
use std::{collections::BTreeMap, time::Duration};

use misa_proto::{
    Fault,
    invocation::{Binding, Command},
    observation::{Member, Scope, Selection},
    presentation::Presentation,
    query::Definition,
};
use misa_protocol::observation::MemberState;
use misa_value::Value;
use serde::de::DeserializeOwned;

use crate::driver::Client;

pub struct Interface {
    pub scope: Scope,
    pub queries: BTreeMap<String, Definition>,
    pub commands: BTreeMap<String, Command>,
    pub actions: BTreeMap<String, Binding>,
    pub presentations: Vec<Presentation>,
}

impl Interface {
    pub async fn load(client: &Client, scope: Scope) -> Result<Self, Fault> {
        let bootstrap = misa_proto::query::catalog_definition().member(vec![])?;
        let definitions = client
            .read(
                Selection {
                    scope: scope.clone(),
                    members: BTreeMap::from([("queries".into(), bootstrap)]),
                },
                Duration::from_secs(10),
            )
            .await?;
        let queries: Vec<Definition> = decode(data(&definitions, "queries")?)?;
        let mut installed = BTreeMap::new();
        for definition in queries {
            definition.check()?;
            if installed
                .insert(definition.id.clone(), definition)
                .is_some()
            {
                return Err(Fault::query("Duplicate exported query identity"));
            }
        }
        let mut interface = Self {
            scope,
            queries: installed,
            commands: BTreeMap::new(),
            actions: BTreeMap::new(),
            presentations: vec![],
        };
        let mut members = BTreeMap::new();
        for (member, query) in [
            ("commands", misa_proto::invocation::CATALOG),
            ("actions", misa_proto::invocation::BINDING_CATALOG),
            ("presentations", misa_proto::presentation::CATALOG),
        ] {
            if let Some(definition) = interface.queries.get(query) {
                members.insert(member.into(), definition.member(vec![])?);
            }
        }
        if members.is_empty() {
            return Ok(interface);
        }
        // Composition is immutable within an owner incarnation. Metadata is read
        // coherently together after discovering its installed result contracts.
        let catalogs = client
            .read(
                Selection {
                    scope: interface.scope.clone(),
                    members,
                },
                Duration::from_secs(10),
            )
            .await?;
        if catalogs.members().contains_key("commands") {
            for definition in decode::<Vec<Command>>(data(&catalogs, "commands")?)? {
                definition.validate()?;
                if interface
                    .commands
                    .insert(definition.id.clone(), definition)
                    .is_some()
                {
                    return Err(Fault::query("Duplicate command identity"));
                }
            }
        }
        if catalogs.members().contains_key("actions") {
            for binding in decode::<Vec<Binding>>(data(&catalogs, "actions")?)? {
                let command = interface
                    .commands
                    .get(&binding.binding.command)
                    .ok_or_else(|| Fault::query("Action names an unavailable command"))?;
                binding.binding.validate_for(command)?;
                if interface
                    .actions
                    .insert(binding.id.clone(), binding)
                    .is_some()
                {
                    return Err(Fault::query("Duplicate action identity"));
                }
            }
        }
        if catalogs.members().contains_key("presentations") {
            interface.presentations = decode(data(&catalogs, "presentations")?)?;
            misa_proto::presentation::validate_catalog(
                &interface.presentations,
                &interface.queries,
            )?;
        }
        Ok(interface)
    }

    pub fn query(&self, id: &str, arguments: Vec<Value>) -> Result<Member, Fault> {
        self.queries
            .get(id)
            .ok_or_else(|| Fault::query("Query is not available in this scope"))?
            .member(arguments)
    }

    pub fn presentation(&self, id: &str, capabilities: &[String]) -> Result<Member, Fault> {
        self.presentations
            .iter()
            .find(|presentation| presentation.id == id)
            .and_then(|presentation| presentation.select(capabilities))
            .map(|variant| variant.member.clone())
            .ok_or_else(|| Fault::unsupported("No supported presentation variant"))
    }

    pub fn action(
        &self,
        id: &str,
        fields: &BTreeMap<String, Value>,
    ) -> Result<(&Command, Value), Fault> {
        let action = self
            .actions
            .get(id)
            .ok_or_else(|| Fault::unsupported("Action is not available"))?;
        let command = self
            .commands
            .get(&action.binding.command)
            .ok_or_else(|| Fault::unsupported("Action command is not available"))?;
        let input = action.binding.prepare(fields)?;
        command
            .input
            .validate(&input)
            .map_err(|error| Fault::protocol(error.to_string()))?;
        Ok((command, input))
    }
}

pub fn data<'a>(result: &'a crate::ReadValue, member: &str) -> Result<&'a Value, Fault> {
    match result.members().get(member) {
        Some(MemberState::Value(value)) => Ok(value),
        Some(MemberState::Unavailable(fault)) => Err(fault.clone()),
        _ => Err(Fault::query("Expected a data result member")),
    }
}

/// Typed decoding is finite catalog/result work, never token-update work.
pub fn decode<T: DeserializeOwned>(value: &Value) -> Result<T, Fault> {
    let mut bytes = Vec::new();
    ciborium::ser::into_writer(value, &mut bytes)
        .map_err(|_| Fault::query("Cannot decode exported data"))?;
    ciborium::de::from_reader(bytes.as_slice())
        .map_err(|_| Fault::query("Exported data has an invalid shape"))
}
