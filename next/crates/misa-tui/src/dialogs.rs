//! Surface-owned request drafts and report visibility.
use crate::{Key, KeyOut};
use misa_client::request::Model;
use misa_proto::Node;
use misa_value::Value;
use std::collections::BTreeMap;
#[derive(Default)]
pub struct Dialogs {
    requests: BTreeMap<String, (Model, String)>,
    visible: Option<String>,
    form: Option<(
        misa_client::form::Form,
        BTreeMap<String, String>,
        usize,
        Option<String>,
    )>,
    report: Option<Node>,
    generations: BTreeMap<String, i64>,
    request_forms: BTreeMap<String, (BTreeMap<String, String>, usize, Option<String>)>,
}
impl Dialogs {
    pub fn form(&mut self, form: misa_client::form::Form) {
        self.visible = None;
        self.report = None;
        self.form = Some((form, BTreeMap::new(), 0, None));
    }

    pub fn update(&mut self, id: String, generation: i64, model: Option<Model>) {
        if self
            .generations
            .get(&id)
            .is_some_and(|old| *old > generation)
        {
            return;
        }
        self.generations.insert(id.clone(), generation);
        // Responses are generation-filtered by the adapter before delivery; a
        // small tombstone window also protects already queued local updates.
        while self.generations.len() > 256 {
            let Some(old) = self
                .generations
                .keys()
                .find(|key| *key != &id && !self.requests.contains_key(*key))
                .cloned()
            else {
                break;
            };
            self.generations.remove(&old);
        }
        match model {
            Some(model) => {
                if self
                    .requests
                    .get(&id)
                    .is_some_and(|(old, _)| old.generation == model.generation)
                {
                    return;
                }
                self.requests.insert(id.clone(), (model, String::new()));
                self.request_forms.remove(&id);
            }
            None => {
                self.requests.remove(&id);
                self.request_forms.remove(&id);
                if self.visible.as_ref() == Some(&id) {
                    self.visible = None;
                }
            }
        }
    }
    pub fn report(&mut self, report: Node) {
        self.report = Some(report);
        self.visible = None;
    }
    pub fn open(&mut self) {
        self.report = None;
        self.visible = self.requests.keys().next().cloned();
    }
    pub fn paste(&mut self, text: &str) -> bool {
        if let Some((form, drafts, index, _)) = &mut self.form {
            if let Some((id, _)) = form.fields.get(*index) {
                drafts.entry(id.clone()).or_default().push_str(text);
            }
            return true;
        }
        let Some((model, draft)) = self
            .visible
            .as_ref()
            .and_then(|id| self.requests.get_mut(id))
        else {
            return false;
        };
        if let Some(form) = &model.form
            && let misa_proto::schema::Schema::Record { fields, .. } = &form.input
        {
            let (drafts, index, _) = self.request_forms.entry(model.id.clone()).or_default();
            if let Some(id) = fields.keys().nth(*index) {
                drafts.entry(id.clone()).or_default().push_str(text);
            }
            return true;
        }
        if model.input.is_some() {
            draft.push_str(text);
        }
        true
    }
    pub fn focused(&self) -> bool {
        self.visible.is_some() || self.form.is_some()
    }
    pub fn key(&mut self, key: &Key) -> Option<KeyOut> {
        if let Some((form, drafts, index, error)) = &mut self.form {
            match key {
                Key::Escape | Key::Interrupt => {
                    self.form = None;
                    return Some(KeyOut::Local);
                }
                Key::Quit => return None,
                Key::Tab => {
                    *index = (*index + 1) % form.fields.len().max(1);
                }
                Key::Submit => match form.prepare(drafts) {
                    Ok((command, input)) => {
                        self.form = None;
                        return Some(KeyOut::Invoke { command, input });
                    }
                    Err(fault) => *error = Some(fault.message),
                },
                Key::Char(c) => {
                    if let Some((id, _)) = form.fields.get(*index) {
                        drafts.entry(id.clone()).or_default().push(*c);
                    }
                }
                Key::Backspace | Key::Delete => {
                    if let Some((id, _)) = form.fields.get(*index) {
                        drafts.entry(id.clone()).or_default().pop();
                    }
                }
                _ => {}
            }
            return Some(KeyOut::Local);
        }

        if self.report.is_some() {
            return if matches!(key, Key::Escape) {
                self.report = None;
                Some(KeyOut::Local)
            } else {
                None
            };
        }
        let id = self.visible.clone()?;
        if let Some((model, _)) = self.requests.get(&id)
            && let Some(form) = &model.form
            && let misa_proto::schema::Schema::Record { fields, .. } = &form.input
        {
            let fields = fields
                .iter()
                .map(|(id, field)| (id.clone(), field.clone()))
                .collect::<Vec<_>>();
            let (drafts, index, error) = self.request_forms.entry(id.clone()).or_default();
            let action = match key {
                Key::Escape => {
                    self.visible = None;
                    return Some(KeyOut::Local);
                }
                Key::Quit => return None,
                Key::Tab => {
                    *index = (*index + 1) % fields.len().max(1);
                    None
                }
                Key::Char(ch) => {
                    if let Some((id, _)) = fields.get(*index) {
                        drafts.entry(id.clone()).or_default().push(*ch);
                    }
                    None
                }
                Key::Backspace | Key::Delete => {
                    if let Some((id, _)) = fields.get(*index) {
                        drafts.entry(id.clone()).or_default().pop();
                    }
                    None
                }
                Key::Submit => Some("resolve"),
                Key::Interrupt => Some("cancel"),
                _ => None,
            };
            let Some(action) = action else {
                return Some(KeyOut::Local);
            };
            let Some(binding) = model
                .actions
                .iter()
                .find(|item| item.id == action)
                .map(|item| &item.binding)
            else {
                return Some(KeyOut::Local);
            };
            let values = if action == "resolve" {
                match misa_client::form::parse_fields(&fields, drafts) {
                    Ok(values) => BTreeMap::from([("value".into(), Value::Map(values.into()))]),
                    Err(fault) => {
                        *error = Some(fault.message);
                        return Some(KeyOut::Local);
                    }
                }
            } else {
                BTreeMap::new()
            };
            return match binding.prepare(&values) {
                Ok(input) => Some(KeyOut::Invoke {
                    command: binding.command.clone(),
                    input,
                }),
                Err(fault) => {
                    *error = Some(fault.message);
                    Some(KeyOut::Local)
                }
            };
        }
        if matches!(key, Key::Tab) {
            self.visible = self
                .requests
                .keys()
                .find(|candidate| *candidate > &id)
                .or_else(|| self.requests.keys().next())
                .cloned();
            return Some(KeyOut::Local);
        }
        let (model, draft) = self.requests.get_mut(&id)?;
        if matches!(key, Key::Quit)
            || matches!(key, Key::Interrupt)
                && !model.actions.iter().any(|action| action.id == "cancel")
        {
            return None;
        }
        let action = match key {
            Key::Interrupt => Some("cancel"),
            Key::Escape => {
                self.visible = None;
                return Some(KeyOut::Local);
            }
            Key::Submit if model.input.is_some() => Some("submit"),
            Key::Char('y')
                if model.input.is_none()
                    && model.actions.iter().any(|action| action.id == "approve") =>
            {
                Some("approve")
            }
            Key::Char('n')
                if model.input.is_none()
                    && model.actions.iter().any(|action| action.id == "deny") =>
            {
                Some("deny")
            }
            Key::Char('x')
                if model.input.is_none()
                    && model.actions.iter().any(|action| action.id == "cancel") =>
            {
                Some("cancel")
            }
            Key::Char(character) if model.input.is_some() => {
                draft.push(*character);
                None
            }
            Key::Backspace | Key::Delete if model.input.is_some() => {
                draft.pop();
                None
            }
            _ => None,
        };
        let Some(action) = action else {
            return Some(KeyOut::Local);
        };
        let action = model
            .actions
            .iter()
            .find(|candidate| candidate.id == action)?;
        let fields = model
            .input
            .as_ref()
            .map(|input| BTreeMap::from([(input.id.clone(), Value::str(draft.clone()))]))
            .unwrap_or_default();
        let input = match action.binding.prepare(&fields) {
            Ok(input) => input,
            Err(_) => return Some(KeyOut::Local),
        };
        draft.clear();
        Some(KeyOut::Invoke {
            command: action.binding.command.clone(),
            input,
        })
    }
    pub fn lines(&self, theme: &misa_render::Theme, width: usize) -> Vec<misa_render::Line> {
        let mut lines = vec![];
        if let Some((form, drafts, index, error)) = &self.form {
            let line = |text| misa_render::Line {
                indent: 0,
                node: None,
                spans: vec![(theme.role("notice"), text)],
            };
            lines.push(line(format!(
                "{} · Tab next field · Enter submit · Esc dismiss",
                form.title
            )));
            for (i, (id, field)) in form.fields.iter().enumerate() {
                let hint = if matches!(field.schema, misa_proto::schema::Schema::String) {
                    "text"
                } else {
                    "JSON"
                };
                lines.push(line(format!(
                    "{} {id} ({hint}{}): {}",
                    if *index == i { ">" } else { " " },
                    if field.optional { ", optional" } else { "" },
                    drafts.get(id).map(String::as_str).unwrap_or("")
                )));
            }
            if let Some(error) = error {
                lines.push(line(error.clone()));
            }
        }

        if !self.requests.is_empty() {
            lines.push(misa_render::Line {
                indent: 0,
                node: None,
                spans: vec![(
                    theme.role("notice"),
                    format!(
                    "{} pending requests · /operations opens · Tab next · Esc hides without cancelling",
                        self.requests.len()
                    ),
                )],
            });
        }
        if let Some(report) = &self.report {
            lines.extend(misa_render::render(report, theme, width));
        }
        if let Some((model, draft)) = self.visible.as_ref().and_then(|id| self.requests.get(id)) {
            let line = |text| misa_render::Line {
                indent: 0,
                node: None,
                spans: vec![(theme.role("notice"), text)],
            };
            lines.push(line(model.title.clone()));
            lines.extend(misa_render::render(&model.body, theme, width));
            if let Some(form) = &model.form
                && let misa_proto::schema::Schema::Record { fields, .. } = &form.input
            {
                lines.push(line(
                    "Tab next field · Enter submit · Ctrl-C cancel · Esc hide".into(),
                ));
                let state = self.request_forms.get(&model.id);
                for (index, (id, field)) in fields.iter().enumerate() {
                    let label = form
                        .fields
                        .get(id)
                        .map(|field| field.label.as_str())
                        .unwrap_or(id);
                    let draft = state
                        .and_then(|(drafts, _, _)| drafts.get(id))
                        .map(String::as_str)
                        .unwrap_or("");
                    let selected = state.map(|(_, selected, _)| *selected).unwrap_or(0) == index;
                    lines.push(line(format!(
                        "{} {label} ({}): {draft}",
                        if selected { ">" } else { " " },
                        if matches!(field.schema, misa_proto::schema::Schema::String) {
                            "text"
                        } else {
                            "JSON"
                        }
                    )));
                }
                if let Some((_, _, Some(error))) = state {
                    lines.push(line(error.clone()));
                }
            }
            if let Some(input) = &model.input {
                lines.push(line(format!(
                    "{}: {} · Enter submits · Ctrl-C cancels",
                    input.label,
                    if input.secret {
                        "•".repeat(draft.chars().count())
                    } else {
                        draft.clone()
                    }
                )));
            } else {
                lines.push(line(
                    model
                        .actions
                        .iter()
                        .map(|action| {
                            format!(
                                "{} {}",
                                match action.id.as_str() {
                                    "approve" => "y",
                                    "deny" => "n",
                                    "cancel" => "x",
                                    _ => "",
                                },
                                action.label
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(" · "),
                ));
            }
        }
        lines
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn typed_request_drafts_survive_hiding_and_invalid_submission() {
        use super::*;
        use misa_proto::{
            invocation::ActionBinding,
            schema::{Field, Schema},
        };
        let mut dialogs = Dialogs::default();
        let model = Model {
            id: "custom".into(),
            generation: 1,
            title: "Retries".into(),
            body: Node::section("body"),
            input: None,
            form: Some(misa_proto::input::Form {
                title: "Retries".into(),
                input: Schema::Record {
                    fields: BTreeMap::from([(
                        "count".into(),
                        Field {
                            schema: Schema::Int,
                            optional: false,
                        },
                    )]),
                    allow_unknown: false,
                },
                fields: BTreeMap::new(),
            }),
            actions: vec![misa_client::request::Action {
                id: "resolve".into(),
                label: "Submit".into(),
                binding: ActionBinding {
                    command: "input.resolve".into(),
                    bound: BTreeMap::from([
                        ("request".into(), Value::str("custom")),
                        ("generation".into(), Value::Int(1)),
                    ]),
                    inputs: BTreeMap::from([("value".into(), "value".into())]),
                },
            }],
        };
        dialogs.update("custom".into(), 1, Some(model));
        dialogs.open();
        dialogs.paste("bad");
        assert!(matches!(dialogs.key(&Key::Submit), Some(KeyOut::Local)));
        assert_eq!(dialogs.request_forms["custom"].0["count"], "bad");
        dialogs.key(&Key::Escape);
        dialogs.open();
        for _ in 0..3 {
            dialogs.key(&Key::Backspace);
        }
        dialogs.paste("7");
        assert!(
            matches!(dialogs.key(&Key::Submit),Some(KeyOut::Invoke{input,..}) if input.get("value").and_then(|value|value.get("count"))==Some(&Value::Int(7)))
        );
        dialogs.update("custom".into(), 2, None);
        assert!(!dialogs.request_forms.contains_key("custom"));
    }
    use super::*;
    #[test]
    fn hiding_preserves_private_draft_and_generation_replacement_clears_it() {
        let model = Model {
            form: None,
            id: "request".into(),
            generation: 1,
            title: "Credential".into(),
            body: Node::section("request"),
            input: Some(misa_client::request::Input {
                id: "value".into(),
                label: "Key".into(),
                secret: true,
            }),
            actions: vec![],
        };
        let mut dialogs = Dialogs::default();
        dialogs.update("request".into(), 1, Some(model.clone()));
        assert!(
            dialogs.visible.is_none(),
            "a server request cannot steal local keyboard focus"
        );
        dialogs.open();
        dialogs.key(&Key::Char('s'));
        dialogs.key(&Key::Escape);
        assert!(dialogs.visible.is_none());
        assert_eq!(dialogs.requests["request"].1, "s");
        dialogs.open();
        let text = dialogs
            .lines(&misa_render::Theme::plain(), 80)
            .iter()
            .map(misa_render::Line::text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains('•'));
        assert!(!text.contains("Key: s"));
        assert!(dialogs.paste("ecret pasted"));
        assert_eq!(dialogs.requests["request"].1, "secret pasted");
        let masked = misa_render::to_plain(&dialogs.lines(&misa_render::Theme::plain(), 80));
        assert!(!masked.contains("secret pasted"));
        dialogs.update("request".into(), 2, None);
        dialogs.update("request".into(), 1, Some(model));
        assert!(
            dialogs.requests.is_empty(),
            "late private read resurrected a settled request"
        );
    }
}
#[cfg(test)]
mod action_form_tests {
    use super::*;
    use misa_proto::{
        invocation::{ActionBinding, Binding, Command},
        observation::{Scope, ScopeId},
        schema::{Field, Schema},
    };
    #[test]
    fn local_action_form_edits_multiple_typed_fields_and_keeps_invalid_drafts() {
        let interface = misa_client::interface::Interface {
            scope: Scope {
                id: ScopeId::Session { id: "a".into() },
                incarnation: "one".into(),
            },
            queries: BTreeMap::new(),
            presentations: vec![],
            commands: BTreeMap::from([(
                "change".into(),
                Command {
                    id: "change".into(),
                    input: Schema::Record {
                        fields: BTreeMap::from([
                            (
                                "count".into(),
                                Field {
                                    schema: Schema::Int,
                                    optional: false,
                                },
                            ),
                            (
                                "name".into(),
                                Field {
                                    schema: Schema::String,
                                    optional: false,
                                },
                            ),
                        ]),
                        allow_unknown: false,
                    },
                    result: Schema::Value,
                },
            )]),
            actions: BTreeMap::from([(
                "edit".into(),
                Binding {
                    id: "edit".into(),
                    binding: ActionBinding {
                        command: "change".into(),
                        bound: BTreeMap::new(),
                        inputs: BTreeMap::from([
                            ("count".into(), "count".into()),
                            ("name".into(), "name".into()),
                        ]),
                    },
                },
            )]),
        };
        let mut dialogs = Dialogs::default();
        dialogs.form(misa_client::form::Form::action(&interface, "edit").unwrap());
        dialogs.key(&Key::Char('x'));
        assert!(matches!(dialogs.key(&Key::Submit), Some(KeyOut::Local)));
        assert!(dialogs.focused());
        dialogs.key(&Key::Backspace);
        dialogs.paste("3");
        dialogs.key(&Key::Tab);
        dialogs.paste("Misa");
        let Some(KeyOut::Invoke { command, input }) = dialogs.key(&Key::Submit) else {
            panic!("valid typed form submits")
        };
        assert_eq!(command, "change");
        assert_eq!(input.get("count"), Some(&Value::Int(3)));
        assert_eq!(input.get("name"), Some(&Value::str("Misa")));
        assert!(!dialogs.focused());
    }
}
