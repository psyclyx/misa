//! Installed query contracts, independent of presentation and transport policy.
use serde::{Deserialize, Serialize};
use misa_value::Value;

pub const CATALOG: &str = "queries.catalog";
pub fn catalog_definition() -> Definition {
    Definition {
        id: CATALOG.into(),
        arguments: vec![],
        contract: "queries.catalog@1".into(),
        result: ResultContract::Data {
            schema: Schema::List {
                items: Box::new(Schema::Record {
                    fields: [
                        ("id", Schema::String),
                        (
                            "arguments",
                            Schema::List {
                                items: Box::new(Schema::Value),
                            },
                        ),
                        ("contract", Schema::String),
                        ("result", Schema::Value),
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

use crate::{
    Fault,
    observation::{Encoding, Member},
    schema::Schema,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ResultContract {
    Data { schema: Schema },
    Document {},
}

impl ResultContract {
    pub fn encoding(&self) -> Encoding {
        match self {
            Self::Data { .. } => Encoding::Value,
            Self::Document {} => Encoding::Document,
        }
    }
}

/// Export visibility is an owner decision. Merely installing a query does not
/// expose it, and listing a definition is not an authorization grant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Definition {
    pub id: String,
    pub arguments: Vec<Schema>,
    pub contract: String,
    pub result: ResultContract,
}

impl Definition {
    pub fn member(&self, arguments: Vec<misa_value::Value>) -> Result<Member, Fault> {
        let member = Member { query: crate::Query { id: self.id.clone(), args: arguments },
            contract: self.contract.clone(), encoding: self.result.encoding(), optional: false };
        self.validate(&member)?;
        Ok(member)
    }

    pub fn check(&self) -> Result<(), Fault> {
        if self.id.is_empty() || self.contract.is_empty() {
            return Err(Fault::query(
                "Export needs an identity and versioned result contract",
            ));
        }
        for schema in &self.arguments {
            schema
                .check()
                .map_err(|error| Fault::query(error.to_string()))?;
        }
        if let ResultContract::Data { schema } = &self.result {
            schema
                .check()
                .map_err(|error| Fault::query(error.to_string()))?;
        }
        Ok(())
    }

    pub fn validate(&self, member: &Member) -> Result<(), Fault> {
        if self.id != member.query.id
            || self.contract != member.contract
            || self.result.encoding() != member.encoding
        {
            return Err(Fault::query(
                "Selected export contract does not match the installed definition",
            ));
        }
        if member.query.args.len() != self.arguments.len() {
            return Err(Fault::query(
                "Query argument count does not match its definition",
            ));
        }
        for (schema, value) in self.arguments.iter().zip(&member.query.args) {
            schema
                .validate(value)
                .map_err(|error| Fault::query(error.to_string()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Query;
    use misa_value::Value;

    #[test]
    fn exported_identity_encoding_and_arguments_are_checked_together() {
        let definition = Definition {
            id: "counter".into(),
            arguments: vec![Schema::Int],
            contract: "counter@1".into(),
            result: ResultContract::Data {
                schema: Schema::Int,
            },
        };
        definition.check().unwrap();
        let mut member = Member {
            query: Query::new("counter").arg(Value::Int(3)),
            contract: "counter@1".into(),
            encoding: Encoding::Value,
            optional: false,
        };
        definition.validate(&member).unwrap();
        member.query.args[0] = Value::str("3");
        assert!(definition.validate(&member).is_err());
        member.query.args[0] = Value::Int(3);
        member.encoding = Encoding::Document;
        assert!(definition.validate(&member).is_err());
        member.encoding = Encoding::Value;
        member.contract = "counter@2".into();
        assert!(definition.validate(&member).is_err());
    }
}

/// A declared query: a name and its arguments.
///
/// The name is a dotted, lowercase identifier. The arguments are data, so a query
/// is a value a client can log, compare, and retain, and a session can key a
/// scope entry by.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Query {
    pub id: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Value>,
}

impl Query {
    pub fn new(id: impl Into<String>) -> Self {
        Query { id: id.into(), args: Vec::new() }
    }

    pub fn arg(mut self, value: Value) -> Self {
        self.args.push(value);
        self
    }

    /// The query's canonical form, used as a scope key.
    ///
    /// A separator that cannot appear unescaped in a canonical value key keeps a
    /// query with one argument distinct from a query with two.
    pub fn key(&self) -> String {
        let mut out = self.id.clone();
        for arg in &self.args {
            out.push('\u{1}');
            out.push_str(&arg.canonical_key());
        }
        out
    }
}
