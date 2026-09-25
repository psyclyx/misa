//! Local presentation catalog and the composition chooser document.
use super::{Action, PresentationChoice, action};
use misa_pixel_document::{appearance, ui::DocumentUi};
use misa_pixel_ui::TextMetrics;
use misa_proto::{
    presentation::Presentation,
    view::{ActionOn, Node},
};
use misa_value::Value;
use std::sync::Arc;

pub(super) struct Presentations {
    catalog: Vec<Presentation>,
    preferences: misa_client::composition::Preferences,
    observation: Option<misa_client::ObservationId>,
    choosing: bool,
    app: Option<DocumentUi>,
}

impl Presentations {
    pub(super) fn new() -> Self {
        Self {
            catalog: Vec::new(),
            preferences: Default::default(),
            observation: None,
            choosing: false,
            app: None,
        }
    }

    pub(super) fn observation(&self) -> Option<misa_client::ObservationId> {
        self.observation
    }
    pub(super) fn is_open(&self) -> bool {
        self.choosing
    }
    pub(super) fn app(&mut self) -> Option<&mut DocumentUi> {
        self.app.as_mut()
    }
    pub(super) fn close(&mut self) {
        self.choosing = false;
        self.app = None;
    }
    pub(super) fn update(
        &mut self,
        catalog: Vec<Presentation>,
        preferences: misa_client::composition::Preferences,
        observation: misa_client::ObservationId,
        metrics: Arc<dyn TextMetrics>,
        appearance: appearance::Choice,
    ) {
        self.catalog = catalog;
        self.preferences = preferences;
        self.observation = Some(observation);
        if self.choosing {
            self.rebuild(metrics, appearance);
        }
    }
    pub(super) fn open(&mut self, metrics: Arc<dyn TextMetrics>, appearance: appearance::Choice) {
        self.choosing = true;
        self.rebuild(metrics, appearance);
    }
    pub(super) fn refresh_appearance(
        &mut self,
        metrics: Arc<dyn TextMetrics>,
        appearance: appearance::Choice,
    ) {
        if self.choosing {
            self.rebuild(metrics, appearance);
        }
    }
    pub(super) fn action(&self, action: &str, args: &Value) -> Option<Action> {
        match action {
            action if action.starts_with("appearance.") => Some(Action::Appearance(
                appearance::Choice::parse(args.as_str()?)?,
            )),
            value if value.starts_with("presentation.") || value.starts_with("variant.") => {
                let id = args.get("id")?.as_str()?.to_owned();
                let choice = match args.get("choice")?.as_str()? {
                    "hide" => PresentationChoice::Hidden,
                    "auto" => PresentationChoice::Auto,
                    value => PresentationChoice::Variant(value.into()),
                };
                Some(Action::Presentation { id, choice })
            }
            _ => None,
        }
    }
    fn rebuild(&mut self, metrics: Arc<dyn TextMetrics>, appearance: appearance::Choice) {
        let mut root = Node::section("presentations")
            .id("presentations")
            .label(format!(
                "Presentations · appearance {} · Escape closes",
                appearance.name()
            ));
        for presentation in self
            .catalog
            .iter()
            .filter(|entry| entry.id != "conversation")
            .chain(
                self.catalog
                    .iter()
                    .filter(|entry| entry.id == "conversation"),
            )
        {
            let choice = self
                .preferences
                .0
                .get(&presentation.id)
                .cloned()
                .unwrap_or_else(|| {
                    if presentation.id == "status" || presentation.id == "conversation" {
                        misa_client::composition::Choice::Auto
                    } else {
                        misa_client::composition::Choice::Hidden
                    }
                });
            let chosen = match &choice {
                misa_client::composition::Choice::Hidden => "Hidden",
                misa_client::composition::Choice::Auto => "Automatic",
                misa_client::composition::Choice::Variant(id) => id,
            };
            let mut row = Node::section("presentation")
                .id(format!("presentation.{}", presentation.id))
                .label(format!("{} · {}", presentation.title, chosen));
            for (label, value) in [("Hide", "hide"), ("Automatic", "auto")] {
                if presentation.id == "conversation" && value == "hide" {
                    continue;
                }
                row = row.action(action(
                    &format!("presentation.{value}"),
                    label,
                    Value::map([
                        ("id", Value::str(&presentation.id)),
                        ("choice", Value::str(value)),
                    ]),
                    ActionOn::Click,
                ));
            }
            for variant in presentation
                .variants
                .iter()
                .filter(|variant| variant.requirements.is_empty())
            {
                row = row.action(action(
                    &format!("variant.{}", variant.id),
                    &variant.id,
                    Value::map([
                        ("id", Value::str(&presentation.id)),
                        ("choice", Value::str(&variant.id)),
                    ]),
                    ActionOn::Click,
                ));
            }
            root = root.child(row);
        }
        for choice in [
            appearance::Choice::System,
            appearance::Choice::Dark,
            appearance::Choice::Light,
        ] {
            root = root.action(action(
                &format!("appearance.{}", choice.name()),
                &format!("Theme: {}", choice.name()),
                Value::str(choice.name()),
                ActionOn::Click,
            ));
        }
        self.app = Some(DocumentUi::new(root, metrics));
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    #[test]
    fn composition_refresh_keeps_choice_by_id_and_routes_only_the_visible_app() {
        use misa_client::composition::{Choice, Preferences};
        use misa_proto::{
            observation::{Encoding, Member, Scope, ScopeId, Selection},
            presentation::Presentation,
            query::Query,
        };
        let mut connection = misa_client::Connection::new(Default::default());
        let observation = connection
            .observe(
                Selection {
                    scope: Scope {
                        id: ScopeId::Session {
                            id: "session".into(),
                        },
                        incarnation: "run".into(),
                    },
                    members: BTreeMap::from([(
                        "conversation".into(),
                        Member {
                            query: Query::new("conversation"),
                            contract: "document@1".into(),
                            encoding: Encoding::Document,
                            optional: false,
                        },
                    )]),
                },
                None,
            )
            .unwrap()
            .0;
        let catalog = |title: &str| {
            vec![Presentation {
                id: "feed".into(),
                title: title.into(),
                variants: vec![],
            }]
        };
        let intent = |id: &str| {
            Command::Intent(misa_kit::intent::Intent::Action {
                node: "presentation.feed".into(),
                action: "presentation.auto".into(),
                fields: vec![],
                args: Value::map([("id", Value::str(id)), ("choice", Value::str("auto"))]),
            })
        };
        let mut local = Local::default();
        local.composition(
            catalog("Old title"),
            Preferences(BTreeMap::from([(
                "feed".into(),
                Choice::Variant("compact".into()),
            )])),
            observation,
        );
        local.open_presentations();
        assert_eq!(local.observation(), Some(observation));
        local.composition(
            catalog("New title"),
            Preferences(BTreeMap::from([(
                "feed".into(),
                Choice::Variant("compact".into()),
            )])),
            observation,
        );
        assert!(local.presentations.is_open());
        assert_eq!(local.presentations.catalog[0].title, "New title");
        assert_eq!(
            local.presentations.preferences.0["feed"],
            Choice::Variant("compact".into())
        );
        assert_eq!(
            local.convert(vec![intent("feed")]),
            vec![Action::Presentation {
                id: "feed".into(),
                choice: PresentationChoice::Auto,
            }]
        );
        local.open_chooser();
        assert!(local.convert(vec![intent("feed")]).is_empty());
        assert_eq!(local.observation(), Some(observation));
    }
}
