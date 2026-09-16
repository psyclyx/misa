//! A typed, non-secret input form. Appearance and unfinished values stay local.
use crate::{Fault, schema::Schema};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Field {
    pub label: String,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Form {
    pub title: String,
    pub input: Schema,
    #[serde(default)]
    pub fields: BTreeMap<String, Field>,
}
impl Form {
    pub fn validate(&self) -> Result<(), Fault> {
        self.input
            .check()
            .map_err(|error| Fault::protocol(error.to_string()))?;
        let Schema::Record {
            fields,
            allow_unknown: false,
        } = &self.input
        else {
            return Err(Fault::protocol(
                "Input forms require a closed record schema",
            ));
        };
        if self.title.is_empty()
            || self.title.len() > 1024
            || fields.is_empty()
            || fields.len() > 32
            || self
                .fields
                .iter()
                .any(|(name, field)| !fields.contains_key(name) || field.label.len() > 256)
        {
            return Err(Fault::protocol("Invalid input form declaration"));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn forms_require_a_bounded_closed_record_and_known_field_labels() {
        let mut form=Form{title:"Choose a project".into(),input:Schema::Record{fields:BTreeMap::from([("project".into(),crate::schema::Field{schema:Schema::String,optional:false})]),allow_unknown:false},fields:BTreeMap::new()};
        assert!(form.validate().is_ok());
        form.fields.insert("unknown".into(),Field{label:"Unknown".into()});assert!(form.validate().is_err());
        form.fields.clear();form.input=Schema::String;assert!(form.validate().is_err());
    }
}
