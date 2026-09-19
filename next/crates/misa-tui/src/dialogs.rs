//! Surface-owned request drafts and report visibility.
use crate::prefs::DialogSettings;
use crate::{Key, KeyOut};
use misa_client::request::Model;
use misa_proto::Node;
use misa_value::Value;
use std::collections::BTreeMap;
#[derive(Default)]
pub struct Dialogs {
    requests: BTreeMap<String, (Model, String)>,
    visible: Option<String>,
    attention: Option<(String, i64)>,
    form: Option<(
        misa_client::form::Form,
        BTreeMap<String, String>,
        usize,
        Option<String>,
    )>,
    report: Option<Node>,
    form_owner: Option<(String, misa_proto::observation::Scope)>,
    generations: BTreeMap<String, i64>,
    request_forms: BTreeMap<String, (BTreeMap<String, String>, usize, Option<String>)>,
}
impl Dialogs {
    pub fn focus_request(&mut self, id: String, generation: i64) {
        self.form = None;
        self.report = None;
        self.attention = Some((id, generation));
        self.focus_attention();
    }
    fn focus_attention(&mut self) {
        if let Some((id, generation)) = &self.attention {
            if let Some((model, _)) = self.requests.get(id) {
                if model.generation == *generation {
                    self.visible = Some(id.clone());
                }
                self.attention = None;
            }
        }
    }
    pub fn daemon_form(
        &mut self,
        daemon: String,
        scope: misa_proto::observation::Scope,
        form: misa_client::form::Form,
        drafts: BTreeMap<String, String>,
    ) {
        self.form(form);
        if let Some((_, values, _, _)) = &mut self.form {
            *values = drafts;
        }
        self.form_owner = Some((daemon, scope));
    }
    pub fn form(&mut self, form: misa_client::form::Form) {
        self.form_owner = None;
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
        self.focus_attention();
    }
    pub fn report(&mut self, report: Node) {
        // Reports are terminal surfaces, not another item in the request queue. A pending
        // form must not remain in front of the report and consume its Escape indefinitely.
        self.form = None;
        self.form_owner = None;
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
    /// Reports and focused requests are exclusive layers. A request that is only
    /// advertised remains part of normal chrome and cannot block the composer.
    pub fn modal(&self) -> bool {
        self.report.is_some() || self.focused()
    }
    pub fn key(&mut self, key: &Key, settings: &DialogSettings) -> Option<KeyOut> {
        if let Some((form, drafts, index, error)) = &mut self.form {
            if settings.matches("panel.close", key) {
                self.form = None;
                self.form_owner = None;
                return Some(KeyOut::Local);
            }
            match key {
                Key::Quit => return None,
                Key::Tab => {
                    *index = (*index + 1) % form.fields.len().max(1);
                }
                _ if settings.matches("panel.submit", key) => match form.prepare(drafts) {
                    Ok((command, input)) => {
                        self.form = None;
                        return Some(match self.form_owner.take() {
                            Some((daemon, scope)) => KeyOut::DaemonInvoke {
                                daemon,
                                scope,
                                command,
                                input,
                            },
                            None => KeyOut::Invoke { command, input },
                        });
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
            return if settings.matches("panel.close", key) {
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
            if settings.matches("panel.close", key) {
                if model.actions.iter().any(|action| action.id == "cancel") {
                    // A cancellable owner request must be cancelled, not merely hidden. Hiding
                    // an authorization is how an operation becomes a silent pending blocker.
                    let action = model
                        .actions
                        .iter()
                        .find(|action| action.id == "cancel")
                        .map(|action| &action.binding);
                    if let Some(binding) = action {
                        return match binding.prepare(&BTreeMap::new()) {
                            Ok(input) => Some(KeyOut::Invoke {
                                command: binding.command.clone(),
                                input,
                            }),
                            Err(_) => Some(KeyOut::Local),
                        };
                    }
                }
                self.visible = None;
                return Some(KeyOut::Local);
            }
            if settings.matches("cancel", key)
                && let Some(action) = model.actions.iter().find(|action| action.id == "cancel")
            {
                return match action.binding.prepare(&BTreeMap::new()) {
                    Ok(input) => Some(KeyOut::Invoke {
                        command: action.binding.command.clone(),
                        input,
                    }),
                    Err(_) => Some(KeyOut::Local),
                };
            }
            let action = match key {
                Key::Quit => return None,
                Key::Tab => {
                    *index = (*index + 1) % fields.len().max(1);
                    None
                }
                Key::Backspace | Key::Delete => {
                    if let Some((id, _)) = fields.get(*index) {
                        drafts.entry(id.clone()).or_default().pop();
                    }
                    None
                }
                Key::Submit | Key::Interrupt => model
                    .actions
                    .iter()
                    .find(|action| settings.matches(&action.id, key))
                    .map(|action| action.id.as_str()),
                Key::Char(ch) => {
                    if let Some(action) = model
                        .actions
                        .iter()
                        .find(|action| settings.matches(&action.id, key))
                    {
                        Some(action.id.as_str())
                    } else {
                        if let Some((id, _)) = fields.get(*index) {
                            drafts.entry(id.clone()).or_default().push(*ch);
                        }
                        None
                    }
                }
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
        if settings.matches("panel.close", key) {
            if let Some(action) = model.actions.iter().find(|action| action.id == "cancel") {
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
                return Some(KeyOut::Invoke {
                    command: action.binding.command.clone(),
                    input,
                });
            }
            self.visible = None;
            return Some(KeyOut::Local);
        }
        let action = match key {
            Key::Interrupt | Key::Submit => model
                .actions
                .iter()
                .find(|action| settings.matches(&action.id, key))
                .map(|action| action.id.as_str()),
            Key::Char(character) => {
                if let Some(action) = model
                    .actions
                    .iter()
                    .find(|action| settings.matches(&action.id, key))
                {
                    Some(action.id.as_str())
                } else if model.input.is_some() {
                    draft.push(*character);
                    None
                } else {
                    None
                }
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
    pub fn lines(
        &self,
        theme: &misa_render::Theme,
        width: usize,
        settings: &DialogSettings,
    ) -> Vec<misa_render::Line> {
        let mut lines = vec![];
        let mut title = None;
        let mut footer_actions = vec![("panel.close", "Close")];
        if let Some((form, drafts, index, error)) = &self.form {
            title = Some(form.title.clone());
            let line = |text| misa_render::Line {
                surface: None,
                indent: 0,
                node: None,
                spans: vec![(theme.role("notice"), text)],
            };
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
            footer_actions = vec![("panel.submit", "Submit"), ("panel.close", "Cancel")];
        }

        if !self.requests.is_empty() && !self.modal() {
            lines.push(misa_render::Line {
                surface: None,
                indent: 0,
                node: None,
                spans: vec![(
                    theme.role("notice"),
                    format!("{} pending input requests", self.requests.len()),
                )],
            });
        }
        if let Some(report) = &self.report {
            title = Some(report.label.clone().unwrap_or_else(|| "Report".into()));
            lines.extend(misa_render::render(report, theme, width));
        }
        if let Some((model, draft)) = self.visible.as_ref().and_then(|id| self.requests.get(id)) {
            title = Some(model.title.clone());
            let cancellable = model.actions.iter().any(|action| action.id == "cancel");
            footer_actions = model
                .actions
                .iter()
                .filter(|action| !cancellable || action.id != "cancel")
                .map(|action| (action.id.as_str(), action.label.as_str()))
                .collect();
            footer_actions.push(("panel.close", if cancellable { "Cancel" } else { "Close" }));
            let line = |text| misa_render::Line {
                surface: None,
                indent: 0,
                node: None,
                spans: vec![(theme.role("notice"), text)],
            };
            lines.extend(misa_render::render(&model.body, theme, width));
            if let Some(form) = &model.form
                && let misa_proto::schema::Schema::Record { fields, .. } = &form.input
            {
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
                    "{}: {}",
                    input.label,
                    if input.secret {
                        "•".repeat(draft.chars().count())
                    } else {
                        draft.clone()
                    }
                )));
            }
        }
        if !self.modal() {
            return lines;
        }

        // Dialogs are an overlay in the reference frame: the transcript remains
        // the document underneath, while the dialog owns the middle region. The
        // border is presentation policy, not dialog state, so another client can
        // render the same request as a native surface without inheriting terminal
        // bookkeeping.
        let title = title.unwrap_or_else(|| "Interaction".into());
        let mut framed = vec![misa_render::Line {
            surface: theme.surface("dialog"),
            indent: 0,
            node: None,
            spans: vec![
                (theme.role("dialog.label"), "┌─ ".into()),
                (theme.role("dialog.title"), title),
            ],
        }];
        for mut line in lines {
            line.surface = line.surface.or_else(|| theme.surface("dialog"));
            line.spans
                .insert(0, (theme.role("dialog.label"), "│ ".into()));
            framed.push(line);
        }
        framed.push(misa_render::Line {
            surface: theme.surface("dialog"),
            indent: 0,
            node: None,
            spans: crate::buttons::footer(theme, settings, footer_actions).spans,
        });
        framed
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
        assert!(matches!(
            dialogs.key(&Key::Submit, &DialogSettings::default()),
            Some(KeyOut::Local)
        ));
        assert_eq!(dialogs.request_forms["custom"].0["count"], "bad");
        dialogs.key(&Key::Escape, &DialogSettings::default());
        dialogs.open();
        for _ in 0..3 {
            dialogs.key(&Key::Backspace, &DialogSettings::default());
        }
        dialogs.paste("7");
        assert!(
            matches!(dialogs.key(&Key::Submit, &DialogSettings::default()),Some(KeyOut::Invoke{input,..}) if input.get("value").and_then(|value|value.get("count"))==Some(&Value::Int(7)))
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
        dialogs.key(&Key::Char('s'), &DialogSettings::default());
        dialogs.key(&Key::Escape, &DialogSettings::default());
        assert!(dialogs.visible.is_none());
        assert_eq!(dialogs.requests["request"].1, "s");
        dialogs.open();
        let text = dialogs
            .lines(&misa_render::Theme::plain(), 80, &DialogSettings::default())
            .iter()
            .map(misa_render::Line::text)
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains('•'));
        assert!(!text.contains("Key: s"));
        assert!(dialogs.paste("ecret pasted"));
        assert_eq!(dialogs.requests["request"].1, "secret pasted");
        let masked = misa_render::to_plain(&dialogs.lines(
            &misa_render::Theme::plain(),
            80,
            &DialogSettings::default(),
        ));
        assert!(!masked.contains("secret pasted"));
        dialogs.update("request".into(), 2, None);
        dialogs.update("request".into(), 1, Some(model));
        assert!(
            dialogs.requests.is_empty(),
            "late private read resurrected a settled request"
        );
    }

    #[test]
    fn configured_dialog_action_keys_replace_terminal_defaults() {
        let model = Model {
            form: None,
            id: "approval".into(),
            generation: 1,
            title: "Tool permission".into(),
            body: Node::section("request"),
            input: None,
            actions: vec![
                misa_client::request::Action {
                    id: "approve".into(),
                    label: "Allow tool".into(),
                    binding: misa_proto::invocation::ActionBinding {
                        command: "input.resolve".into(),
                        bound: BTreeMap::from([
                            ("request".into(), Value::str("approval")),
                            ("generation".into(), Value::Int(1)),
                            ("approved".into(), Value::Bool(true)),
                        ]),
                        inputs: BTreeMap::new(),
                    },
                },
                misa_client::request::Action {
                    id: "deny".into(),
                    label: "Deny tool".into(),
                    binding: misa_proto::invocation::ActionBinding {
                        command: "input.resolve".into(),
                        bound: BTreeMap::from([
                            ("request".into(), Value::str("approval")),
                            ("generation".into(), Value::Int(1)),
                            ("approved".into(), Value::Bool(false)),
                        ]),
                        inputs: BTreeMap::new(),
                    },
                },
            ],
        };
        let mut dialogs = Dialogs::default();
        dialogs.update("approval".into(), 1, Some(model));
        dialogs.open();
        let mut settings = DialogSettings::default();
        settings.action_keys.insert("approve".into(), "a".into());
        settings.action_keys.insert("deny".into(), "d".into());
        let text =
            misa_render::to_plain(&dialogs.lines(&misa_render::Theme::plain(), 80, &settings));
        assert!(text.contains("a Allow tool   d Deny tool"), "{text}");
        assert!(!text.contains("y Allow tool"), "{text}");
        assert!(matches!(
            dialogs.key(&Key::Char('a'), &settings),
            Some(KeyOut::Invoke { .. })
        ));
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
                    preparation: Default::default(),
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
        dialogs.key(&Key::Char('x'), &DialogSettings::default());
        assert!(matches!(
            dialogs.key(&Key::Submit, &DialogSettings::default()),
            Some(KeyOut::Local)
        ));
        assert!(dialogs.focused());
        dialogs.key(&Key::Backspace, &DialogSettings::default());
        dialogs.paste("3");
        dialogs.key(&Key::Tab, &DialogSettings::default());
        dialogs.paste("Misa");
        let Some(KeyOut::Invoke { command, input }) =
            dialogs.key(&Key::Submit, &DialogSettings::default())
        else {
            panic!("valid typed form submits")
        };
        assert_eq!(command, "change");
        assert_eq!(input.get("count"), Some(&Value::Int(3)));
        assert_eq!(input.get("name"), Some(&Value::str("Misa")));
        assert!(!dialogs.focused());
        let scope = interface.scope.clone();
        dialogs.daemon_form(
            "peer-a".into(),
            scope.clone(),
            misa_client::form::Form::command(&interface, "change").unwrap(),
            BTreeMap::from([
                ("count".into(), "7".into()),
                ("name".into(), "other".into()),
            ]),
        );
        let Some(KeyOut::DaemonInvoke {
            daemon,
            scope: submitted,
            input,
            ..
        }) = dialogs.key(&Key::Submit, &DialogSettings::default())
        else {
            panic!("qualified daemon form");
        };
        assert_eq!(daemon, "peer-a");
        assert_eq!(submitted, scope);
        assert_eq!(input.get("count"), Some(&Value::Int(7)));
    }
}

#[cfg(test)]
mod report_tests {
    use super::*;

    #[test]
    fn escape_closes_a_report() {
        let mut dialogs = Dialogs::default();
        dialogs.report(
            Node::section("report")
                .id("report")
                .label("Status for `provider`"),
        );
        assert!(
            dialogs
                .key(&Key::Escape, &DialogSettings::default())
                .is_some()
        );
        assert!(!dialogs.modal());

        let mut dialogs = Dialogs::default();
        dialogs.report(Node::section("report").id("report"));
        let mut settings = DialogSettings::default();
        settings
            .action_keys
            .insert("panel.close".into(), "q".into());
        assert!(dialogs.key(&Key::Char('q'), &settings).is_some());
        assert!(!dialogs.modal());
    }
}
