//! Client-owned contributions are registered alongside daemon presentation data.
use crate::Screen;
use misa_proto::{Node, view::Kind};
use misa_value::Value;
use std::{collections::BTreeMap, sync::Arc};

pub type Query = Arc<dyn Fn(&Screen) -> Option<Node> + Send + Sync>;
#[derive(Default)]
pub struct Local {
    entries: BTreeMap<String, Vec<(String, Query)>>,
}
impl Local {
    pub fn register(&mut self, role: &str, id: &str, query: Query) -> Result<(), String> {
        let entries = self.entries.entry(role.into()).or_default();
        if entries.iter().any(|(registered, _)| registered == id) {
            return Err(format!("Duplicate local presentation `{id}`"));
        }
        entries.push((id.into(), query));
        Ok(())
    }
    pub fn model<'a>(&self, source: &'a Node, screen: &Screen) -> std::borrow::Cow<'a, Node> {
        let Some(entries) = self.entries.get(&source.role) else {
            return std::borrow::Cow::Borrowed(source);
        };
        let mut model = source.clone();
        for (_, query) in entries {
            if let Some(node) = query(screen) {
                model.children.push(node);
            }
        }
        std::borrow::Cow::Owned(model)
    }
}

pub fn stock() -> Local {
    let mut local = Local::default();
    local
        .register(
            "status.indicators",
            "transcript-detail",
            Arc::new(|screen| {
                Some(
                    Node::section("indicator.transcript-detail")
                        .id("client.transcript-detail")
                        .label("detail")
                        .child(Node::new(
                            "value.text",
                            Kind::Fact {
                                value: Value::str(if screen.prefs.any_open() {
                                    "verbose"
                                } else {
                                    "summary"
                                }),
                            },
                        )),
                )
            }),
        )
        .unwrap();
    local
}
