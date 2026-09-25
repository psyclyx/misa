//! Host-owned command selection and form drafts. Hidden local drafts survive workflow changes.
use super::{Action, action, chooser::Chooser};
use misa_pixel_document::ui::DocumentUi;
use misa_pixel_ui::TextMetrics;
use misa_proto::view::{ActionOn, Field, FieldKind, Kind, Node, Span};
use misa_value::Value;
use std::{collections::BTreeMap, sync::Arc};

type Form = misa_client::form::Form;

#[derive(PartialEq, Eq)]
enum FormKey {
    Installed(String),
    Action(String),
}

struct LocalDraft {
    key: FormKey,
    title: String,
    app: DocumentUi,
}

pub(super) struct Forms {
    metrics: Arc<dyn TextMetrics>,
    installed_commands: BTreeMap<String, Result<Form, String>>,
    command_chooser: Option<DocumentUi>,
    local: Option<LocalDraft>,
    local_visible: bool,
    daemon: Option<(String, Form, DocumentUi)>,
}

impl Forms {
    pub(super) fn new(metrics: Arc<dyn TextMetrics>) -> Self {
        Self {
            metrics,
            installed_commands: BTreeMap::new(),
            command_chooser: None,
            local: None,
            local_visible: false,
            daemon: None,
        }
    }

    pub(super) fn hide(&mut self) {
        self.command_chooser = None;
        self.local_visible = false;
        self.daemon = None;
    }

    pub(super) fn app(&mut self) -> Option<&mut DocumentUi> {
        if let Some((_, _, app)) = &mut self.daemon {
            return Some(app);
        }
        if self.local_visible {
            return self.local.as_mut().map(|draft| &mut draft.app);
        }
        self.command_chooser.as_mut()
    }

    pub(super) fn is_active(&self) -> bool {
        self.daemon.is_some() || self.local_visible || self.command_chooser.is_some()
    }

    pub(super) fn commands(&mut self, commands: BTreeMap<String, Result<Form, String>>) {
        self.installed_commands = commands;
        if self.command_chooser.is_some() {
            self.open_commands();
        }
    }

    pub(super) fn open_commands(&mut self) {
        self.hide();
        let mut root = Node::section("commands")
            .id("commands")
            .label("Installed commands · Escape closes · Ctrl+P keeps shortcut completion");
        for (index, (id, form)) in self.installed_commands.iter().enumerate() {
            let mut row = Node::section("command")
                .id(format!("command.{index}"))
                .label(id);
            match form {
                Ok(_) => {
                    row = row.action(action(
                        "installed-command",
                        "Prepare command",
                        Value::str(id),
                        ActionOn::Click,
                    ))
                }
                Err(reason) => {
                    row = row.child(Node::text(
                        "notice",
                        [Span::plain(format!(
                            "Unavailable as a local form: {reason}"
                        ))],
                    ))
                }
            }
            root = root.child(row);
        }
        if self.installed_commands.is_empty() {
            root = root.child(Node::text(
                "notice",
                [Span::plain(
                    "Connect to a session to discover its installed commands",
                )],
            ));
        }
        let mut app = DocumentUi::new(root, self.metrics.clone());
        app.scroll(-f32::MAX);
        self.command_chooser = Some(app);
    }

    pub(super) fn open_form(&mut self, form: Form) {
        let id = form.title.clone();
        self.show_form(form, FormKey::Action(id));
    }

    fn show_form(&mut self, form: Form, key: FormKey) {
        if self
            .local
            .as_ref()
            .is_some_and(|draft| draft.key == key && draft.title == form.title)
        {
            self.hide();
            self.local_visible = true;
            return;
        }
        self.hide();
        let fields = form
            .fields
            .iter()
            .map(|(id, field)| Field {
                id: id.clone(),
                label: format!("{id}{}", if field.optional { " (optional)" } else { "" }),
                value: String::new(),
                hint: Some(
                    if matches!(field.schema, misa_proto::schema::Schema::String) {
                        "Text"
                    } else {
                        "JSON value"
                    }
                    .into(),
                ),
                kind: FieldKind::default(),
                read_only: false,
                secret: false,
            })
            .collect();
        let node = Node::new("local.form", Kind::Fields { fields })
            .id("local.form")
            .label(form.title.clone())
            .action(action("submit", "Submit", Value::Null, ActionOn::Submit));
        self.local = Some(LocalDraft {
            key,
            title: form.title,
            app: DocumentUi::new(node, self.metrics.clone()),
        });
        self.local_visible = true;
    }

    pub(super) fn prepared_daemon_form(
        &mut self,
        chooser: &Chooser,
        daemon: String,
        form: Form,
        drafts: BTreeMap<String, String>,
    ) {
        if chooser.managing() != Some(&daemon) {
            return;
        }
        let fields = form
            .fields
            .iter()
            .map(|(id, field)| Field {
                id: id.clone(),
                label: format!("{id}{}", if field.optional { " (optional)" } else { "" }),
                value: drafts.get(id).cloned().unwrap_or_default(),
                hint: None,
                kind: FieldKind::default(),
                read_only: false,
                secret: false,
            })
            .collect();
        let node = Node::new("daemon.form", Kind::Fields { fields })
            .id("daemon.form")
            .label(format!("{} · Escape dismisses", form.title))
            .action(action(
                "daemon-submit",
                "Submit",
                Value::Null,
                ActionOn::Submit,
            ));
        self.hide();
        self.daemon = Some((daemon, form, DocumentUi::new(node, self.metrics.clone())));
    }

    pub(super) fn open_daemon_form(&mut self, chooser: &mut Chooser, args: &Value) {
        let Some(daemon) = chooser.managing().map(str::to_owned) else {
            return;
        };
        let command = args.get("command").and_then(Value::as_str).unwrap_or("");
        let Some(form) = chooser.form(&daemon, command) else {
            if let Some(app) = chooser.app() {
                app.notice("Daemon command is not available");
            }
            return;
        };
        let drafts = form
            .fields
            .iter()
            .filter_map(|(id, _)| {
                args.get(id)
                    .and_then(Value::as_str)
                    .map(|value| (id.clone(), value.into()))
            })
            .collect();
        self.prepared_daemon_form(chooser, daemon, form, drafts);
    }

    pub(super) fn action(
        &mut self,
        action: &str,
        fields: Vec<Field>,
        args: Value,
    ) -> Option<Action> {
        if let Some((daemon, form, app)) = &mut self.daemon {
            if action != "daemon-submit" {
                return None;
            }
            let drafts = fields
                .into_iter()
                .map(|field| (field.id, field.value))
                .collect();
            return match form.prepare(&drafts) {
                Ok((command, input)) => Some(Action::DaemonInvoke {
                    scope: Some(form.scope.clone()),
                    daemon: daemon.clone(),
                    command,
                    input,
                }),
                Err(fault) => {
                    app.notice(&fault.message);
                    None
                }
            };
        }
        if self.local_visible {
            if action != "submit" {
                return None;
            }
            let draft = self.local.as_mut()?;
            let drafts = fields
                .into_iter()
                .map(|field| (field.id, field.value))
                .collect();
            return match &draft.key {
                FormKey::Installed(id) => match self
                    .installed_commands
                    .get(id)?
                    .as_ref()
                    .ok()?
                    .prepare(&drafts)
                {
                    Ok((command, input)) => Some(Action::InvokeInstalled { command, input }),
                    Err(fault) => {
                        draft.app.notice(&fault.message);
                        None
                    }
                },
                FormKey::Action(id) => Some(Action::Form {
                    action: id.clone(),
                    drafts,
                }),
            };
        }
        if action != "installed-command" {
            return None;
        }
        let id = args.as_str()?;
        let form = self.installed_commands.get(id)?.as_ref().ok()?.clone();
        self.show_form(form, FormKey::Installed(id.to_owned()));
        None
    }

    pub(super) fn management_action(
        chooser: &Chooser,
        action: &str,
        args: &Value,
        fields: &[Field],
    ) -> Option<Action> {
        let daemon = chooser.managing()?.to_owned();
        match action {
            "archive-search" => Some(Action::Archive {
                daemon,
                prefix: fields
                    .iter()
                    .find(|field| field.id == "prefix")?
                    .value
                    .clone(),
            }),
            "work-form" => Some(Action::PrepareWork {
                daemon,
                scope: misa_client::interface::decode(args.get("scope")?).ok()?,
                id: args.get("id")?.as_str()?.into(),
                command: args.get("command")?.as_str()?.into(),
            }),
            "stop-session" => {
                let scope = misa_client::interface::decode(args).ok()?;
                Some(Action::DaemonInvoke {
                    scope: chooser.overview_scope(&daemon),
                    daemon,
                    command: "daemon.session.close".into(),
                    input: misa_client::lifecycle::close_input(&scope).ok()?,
                })
            }
            _ => None,
        }
    }
}
