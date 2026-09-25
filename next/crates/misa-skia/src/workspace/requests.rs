//! Pending native requests, exact attention and local request drafts.
//! Hiding a request clears secret fields, but never cancels the pending request.
use super::{Action, action};
use misa_client::request::Model;
use misa_pixel_document::ui::DocumentUi;
use misa_pixel_ui::TextMetrics;
use misa_proto::view::{ActionOn, Field, FieldKind, Kind, Node};
use misa_value::Value;
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Visibility {
    Hidden,
    Visible,
}

pub(crate) struct Requests {
    pending: BTreeMap<String, (Model, DocumentUi)>,
    attention: Option<(String, i64)>,
    active: Option<String>,
    metrics: Arc<dyn TextMetrics>,
}

impl Requests {
    pub(super) fn new(metrics: Arc<dyn TextMetrics>) -> Self {
        Self {
            pending: BTreeMap::new(),
            attention: None,
            active: None,
            metrics,
        }
    }

    pub(crate) fn len(&self) -> usize {
        self.pending.len()
    }

    pub(super) fn visible_app(&mut self) -> Option<&mut DocumentUi> {
        self.active
            .as_ref()
            .and_then(|id| self.pending.get_mut(id).map(|(_, app)| app))
    }

    pub(super) fn attend(&mut self, id: String, generation: i64) -> Visibility {
        self.attention = Some((id, generation));
        self.focus_attention()
    }

    fn focus_attention(&mut self) -> Visibility {
        if let Some((id, generation)) = &self.attention {
            if let Some((model, _)) = self.pending.get(id) {
                if model.generation == *generation {
                    self.active = Some(id.clone());
                }
                // An already present but different generation cannot fulfill this attention.
                self.attention = None;
            }
        }
        self.visibility()
    }

    fn visibility(&self) -> Visibility {
        if self.active.is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        }
    }

    pub(crate) fn update(
        &mut self,
        id: String,
        generation: i64,
        model: Option<Model>,
    ) -> Visibility {
        if self.pending.len() >= 32 && !self.pending.contains_key(&id) {
            return self.visibility();
        }
        if self
            .pending
            .get(&id)
            .is_some_and(|(old, _)| old.generation > generation)
        {
            return self.visibility();
        }
        let Some(model) = model else {
            self.pending.remove(&id);
            if self.active.as_ref() == Some(&id) {
                self.active = None;
            }
            return self.visibility();
        };
        if model.generation != generation {
            return self.visibility();
        }
        if self
            .pending
            .get(&id)
            .is_some_and(|(old, _)| old.generation == generation)
        {
            return self.visibility();
        }
        let mut inputs = model.input.iter().cloned().collect::<Vec<_>>();
        if let Some(form) = &model.form
            && let misa_proto::schema::Schema::Record { fields, .. } = &form.input
        {
            inputs.extend(
                fields
                    .iter()
                    .map(|(id, field)| misa_client::request::Input {
                        id: id.clone(),
                        label: format!(
                            "{}{}{}",
                            form.fields
                                .get(id)
                                .map(|field| field.label.as_str())
                                .unwrap_or(id),
                            if field.optional { " (optional)" } else { "" },
                            if matches!(field.schema, misa_proto::schema::Schema::String) {
                                ""
                            } else {
                                " · JSON value"
                            }
                        ),
                        secret: false,
                    }),
            );
        }
        let mut form = Node::new(
            "request.form",
            Kind::Fields {
                fields: inputs
                    .iter()
                    .map(|input| Field {
                        id: input.id.clone(),
                        label: input.label.clone(),
                        value: String::new(),
                        hint: None,
                        kind: FieldKind::default(),
                        read_only: false,
                        secret: input.secret,
                    })
                    .collect(),
            },
        )
        .id("request.form");
        for (index, choice) in model.actions.iter().enumerate() {
            form = form.action(action(
                &choice.id,
                &choice.label,
                Value::Null,
                if index == 0 && (model.input.is_some() || model.form.is_some()) {
                    ActionOn::Submit
                } else {
                    ActionOn::Click
                },
            ));
        }
        let view = Node::section("request")
            .id("request")
            .label(format!("{} · Escape hides · Ctrl+R reopens", model.title))
            .child(model.body.clone())
            .child(form);
        self.pending
            .insert(id, (model, DocumentUi::new(view, self.metrics.clone())));
        self.focus_attention()
    }

    pub(super) fn hide(&mut self) -> Visibility {
        if let Some(id) = self.active.take() {
            if let Some((_, app)) = self.pending.get_mut(&id) {
                app.clear_secret_drafts();
            }
        }
        Visibility::Hidden
    }

    pub(super) fn cycle(&mut self) -> Visibility {
        let ids: Vec<_> = self.pending.keys().cloned().collect();
        let next = self
            .active
            .as_ref()
            .and_then(|id| ids.iter().position(|value| value == id))
            .map_or(0, |index| (index + 1) % ids.len().max(1));
        self.hide();
        self.active = ids.get(next).cloned();
        self.visibility()
    }

    pub(super) fn action(&self, action: String, fields: Vec<Field>) -> Option<Action> {
        let id = self.active.as_ref()?;
        let (model, _) = self.pending.get(id)?;
        Some(Action::Request {
            id: id.clone(),
            generation: model.generation,
            action,
            fields: fields
                .into_iter()
                .map(|field| (field.id, Value::str(field.value)))
                .collect(),
        })
    }
}
