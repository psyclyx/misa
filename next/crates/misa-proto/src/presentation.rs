//! Installed presentation choices refer to ordinary exported query contracts.
use crate::{
    Fault,
    observation::Member,
    query::{Definition, ResultContract},
    schema::{Field, Literal, Schema},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

pub const CATALOG: &str = "presentation.catalog";
pub const CONTRACT: &str = "presentation.catalog@1";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Presentation {
    pub id: String,
    pub title: String,
    pub variants: Vec<Variant>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub id: String,
    pub requirements: Vec<String>,
    pub member: Member,
}

impl Presentation {
    /// Declaration order expresses preference; capabilities never grant authority.
    pub fn select(&self, capabilities: &[String]) -> Option<&Variant> {
        self.variants.iter().find(|variant| {
            variant
                .requirements
                .iter()
                .all(|required| capabilities.contains(required))
        })
    }
    pub fn validate(&self, exports: &BTreeMap<String, Definition>) -> Result<(), Fault> {
        if self.id.is_empty() || self.title.is_empty() || self.variants.is_empty() {
            return Err(Fault::query(
                "Presentation needs identity, title and variants",
            ));
        }
        let mut ids = BTreeSet::new();
        let mut fallback = false;
        for variant in &self.variants {
            if variant.id.is_empty() || !ids.insert(&variant.id) {
                return Err(Fault::query("Duplicate or empty presentation variant"));
            }
            let mut requirements = BTreeSet::new();
            for requirement in &variant.requirements {
                let valid = requirement.rsplit_once('@').is_some_and(|(name, version)| {
                    !name.is_empty()
                        && name.bytes().all(|byte| {
                            byte.is_ascii_lowercase()
                                || byte.is_ascii_digit()
                                || b".-".contains(&byte)
                        })
                        && version.parse::<u32>().is_ok_and(|version| version > 0)
                });
                if !valid || !requirements.insert(requirement) {
                    return Err(Fault::query(
                        "Invalid or duplicate versioned presentation requirement",
                    ));
                }
            }
            fallback |= variant.requirements.is_empty();
            let definition = exports
                .get(&variant.member.query.id)
                .ok_or_else(|| Fault::query("Presentation names an unexported query"))?;
            definition.validate(&variant.member)?;
            if !matches!(definition.result, ResultContract::Document {}) {
                return Err(Fault::query("Presentation query must return a document"));
            }
        }
        if !fallback {
            return Err(Fault::query("Presentation needs an unconditional fallback"));
        }
        Ok(())
    }
}

pub fn validate_catalog(
    catalog: &[Presentation],
    exports: &BTreeMap<String, Definition>,
) -> Result<(), Fault> {
    let mut ids = BTreeSet::new();
    for presentation in catalog {
        if !ids.insert(&presentation.id) {
            return Err(Fault::query("Duplicate presentation identity"));
        }
        presentation.validate(exports)?;
    }
    Ok(())
}

pub fn definition() -> Definition {
    let record = |fields: Vec<(&str, Schema)>| Schema::Record {
        fields: fields
            .into_iter()
            .map(|(id, schema)| {
                (
                    id.into(),
                    Field {
                        schema,
                        optional: false,
                    },
                )
            })
            .collect(),
        allow_unknown: false,
    };
    let list = |items| Schema::List {
        items: Box::new(items),
    };
    let query = Schema::Record {
        fields: BTreeMap::from([
            (
                "id".into(),
                Field {
                    schema: Schema::String,
                    optional: false,
                },
            ),
            (
                "args".into(),
                Field {
                    schema: list(Schema::Value),
                    optional: true,
                },
            ),
        ]),
        allow_unknown: false,
    };
    let member = record(vec![
        ("query", query),
        ("contract", Schema::String),
        (
            "encoding",
            Schema::Choice {
                values: vec![Literal::String("document".into())],
            },
        ),
        ("optional", Schema::Bool),
    ]);
    Definition {
        id: CATALOG.into(),
        arguments: vec![],
        contract: CONTRACT.into(),
        result: ResultContract::Data {
            schema: list(record(vec![
                ("id", Schema::String),
                ("title", Schema::String),
                (
                    "variants",
                    list(record(vec![
                        ("id", Schema::String),
                        ("requirements", list(Schema::String)),
                        ("member", member),
                    ])),
                ),
            ])),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Query, observation::Encoding};
    #[test]
    fn catalog_checks_contracts_duplicates_and_fallbacks() {
        let export = Definition {
            id: "document".into(),
            arguments: vec![],
            contract: "document@1".into(),
            result: ResultContract::Document {},
        };
        let exports = BTreeMap::from([(export.id.clone(), export)]);
        let mut presentation = Presentation {
            id: "test".into(),
            title: "Test".into(),
            variants: vec![Variant {
                id: "basic".into(),
                requirements: vec![],
                member: Member {
                    query: Query::new("document"),
                    contract: "document@1".into(),
                    encoding: Encoding::Document,
                    optional: false,
                },
            }],
        };
        presentation.validate(&exports).unwrap();
        assert!(validate_catalog(&[presentation.clone(), presentation.clone()], &exports).is_err());
        presentation.variants[0]
            .requirements
            .push("images@1".into());
        assert!(presentation.validate(&exports).is_err());
        presentation.variants[0].requirements.clear();
        presentation.variants[0].member.contract = "document@2".into();
        assert!(presentation.validate(&exports).is_err());
    }
}
