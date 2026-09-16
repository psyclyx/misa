//! Local presentation choice, resolved before opening subscriptions.
use misa_proto::{Fault, observation::Member, presentation::Presentation};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "variant", rename_all = "snake_case")]
pub enum Choice {
    Hidden,
    Auto,
    Variant(String),
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Preferences(pub BTreeMap<String, Choice>);
#[derive(Debug, Default)]
pub struct Reconciled {
    pub members: BTreeMap<String, Member>,
    pub unavailable: BTreeMap<String, Fault>,
}
impl Preferences {
    /// Persisted preferences may outlive a plugin version or a surface capability.
    /// Keep those choices intact, while isolating each optional document's failure.
    pub fn reconcile(
        &self,
        catalog: &[Presentation],
        capabilities: &[String],
        defaults: &[&str],
    ) -> Reconciled {
        let mut result = Reconciled::default();
        for presentation in catalog {
            match self.resolve(std::slice::from_ref(presentation), capabilities, defaults) {
                Ok(members) => result.members.extend(members),
                Err(fault) => {
                    result.unavailable.insert(presentation.id.clone(), fault);
                }
            }
        }
        result
    }
    pub fn resolve(
        &self,
        catalog: &[Presentation],
        capabilities: &[String],
        defaults: &[&str],
    ) -> Result<BTreeMap<String, Member>, Fault> {
        let mut selected = BTreeMap::new();
        for presentation in catalog {
            let default = if defaults.contains(&presentation.id.as_str()) {
                Choice::Auto
            } else {
                Choice::Hidden
            };
            let choice = self.0.get(&presentation.id).unwrap_or(&default);
            let variant = match choice {
                Choice::Hidden => continue,
                Choice::Auto => presentation
                    .select(capabilities)
                    .ok_or_else(|| Fault::query("No supported presentation variant"))?,
                Choice::Variant(id) => presentation
                    .variants
                    .iter()
                    .find(|variant| {
                        &variant.id == id
                            && variant
                                .requirements
                                .iter()
                                .all(|requirement| capabilities.contains(requirement))
                    })
                    .ok_or_else(|| {
                        Fault::query(format!(
                            "Unsupported presentation variant: {} / {id}",
                            presentation.id
                        ))
                    })?,
            };
            selected.insert(presentation.id.clone(), variant.member.clone());
        }
        Ok(selected)
    }
    pub fn set(
        &mut self,
        catalog: &[Presentation],
        capabilities: &[String],
        id: &str,
        choice: Choice,
    ) -> Result<(), Fault> {
        let presentation = catalog
            .iter()
            .find(|presentation| presentation.id == id)
            .ok_or_else(|| Fault::query("Unknown presentation"))?;
        let mut next = self.clone();
        next.0.insert(id.into(), choice);
        next.resolve(std::slice::from_ref(presentation), capabilities, &[])?;
        *self = next;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{
        presentation::Variant,
        query::{Definition, ResultContract},
    };
    #[test]
    fn hidden_documents_are_never_selected_and_capabilities_fence_explicit_variants() {
        let definition = Definition {
            id: "pet".into(),
            arguments: vec![],
            contract: "pet@1".into(),
            result: ResultContract::Document {},
        };
        let catalog = vec![Presentation {
            id: "pet".into(),
            title: "Pet".into(),
            variants: vec![
                Variant {
                    id: "rich".into(),
                    requirements: vec!["image@1".into()],
                    member: definition.member(vec![]).unwrap(),
                },
                Variant {
                    id: "text".into(),
                    requirements: vec![],
                    member: definition.member(vec![]).unwrap(),
                },
            ],
        }];
        let mut preferences = Preferences::default();
        let mut primary = catalog[0].clone();
        primary.id = "conversation".into();
        let all = vec![primary, catalog[0].clone()];
        preferences.0.insert(
            "pet".into(),
            Choice::Variant("removed-after-upgrade".into()),
        );
        let reconciled = preferences.reconcile(&all, &[], &["conversation"]);
        assert!(reconciled.members.contains_key("conversation"));
        assert!(reconciled.unavailable.contains_key("pet"));
        assert!(
            preferences
                .set(&all, &[], "conversation", Choice::Auto)
                .is_ok(),
            "a stale optional choice must not block changing another document"
        );
        assert!(
            matches!(preferences.0.get("pet"),Some(Choice::Variant(id)) if id=="removed-after-upgrade"),
            "retain unavailable user preferences for later capability restoration"
        );
        preferences.0.clear();
        assert!(preferences.resolve(&catalog, &[], &[]).unwrap().is_empty());
        assert!(
            preferences
                .set(&catalog, &[], "pet", Choice::Variant("rich".into()))
                .is_err()
        );
        preferences.set(&catalog, &[], "pet", Choice::Auto).unwrap();
        assert_eq!(preferences.resolve(&catalog, &[], &[]).unwrap().len(), 1);
        preferences
            .set(&catalog, &[], "pet", Choice::Hidden)
            .unwrap();
        assert!(preferences.resolve(&catalog, &[], &[]).unwrap().is_empty());
    }
}
