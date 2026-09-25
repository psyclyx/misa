//! Native-local relationship selection and request instances. Neither a hidden
//! dialog nor a session switch cancels the operation that owns a request.
use misa_pixel_document::ui::{Command, DocumentUi, Key};
use misa_pixel_ui::{Scene, TextMetrics};
use misa_proto::view::{Action as ViewAction, ActionOn, Node};
#[cfg(test)]
use misa_proto::view::{Field, FieldKind, Kind};
use misa_value::Value;
use std::{collections::BTreeMap, sync::Arc};

mod chooser;
mod forms;
mod requests;
use chooser::Chooser;
pub use chooser::DaemonChoice;
use forms::Forms;
use requests::Requests;

/// Actions produced by host-owned workspace controls. UI DocumentUi effects are kept
/// distinct so relationship and request workflow never leaks into the UI crate.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Ui(Command),
    Appearance(misa_pixel_document::appearance::Choice),
    InvokeInstalled {
        command: String,
        input: Value,
    },
    Form {
        action: String,
        drafts: BTreeMap<String, String>,
    },
    Request {
        id: String,
        generation: i64,
        action: String,
        fields: BTreeMap<String, Value>,
    },
    Presentation {
        id: String,
        choice: PresentationChoice,
    },
    Connect(String),
    Discover,
    Disconnect(String),
    Select {
        daemon: String,
        scope: misa_proto::observation::Scope,
    },
    SelectRequest {
        daemon: String,
        scope: misa_proto::observation::Scope,
        request: String,
        generation: i64,
    },
    CloseInstance {
        daemon: String,
        scope: misa_proto::observation::Scope,
    },
    PrepareWork {
        daemon: String,
        scope: misa_proto::observation::Scope,
        id: String,
        command: String,
    },
    DaemonInvoke {
        daemon: String,
        scope: Option<misa_proto::observation::Scope>,
        command: String,
        input: Value,
    },
    Archive {
        daemon: String,
        prefix: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PresentationChoice {
    Hidden,
    Auto,
    Variant(String),
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_client::request::Model;
    use misa_pixel_document::ui::Control;

    fn request(generation: i64) -> Model {
        Model {
            form: None,
            id: "credential".into(),
            generation,
            title: "Credential".into(),
            body: Node::section("body"),
            input: Some(misa_client::request::Input {
                id: "value".into(),
                label: "Secret".into(),
                secret: true,
            }),
            actions: vec![misa_client::request::Action {
                id: "submit".into(),
                label: "Submit".into(),
                binding: misa_proto::invocation::ActionBinding {
                    command: "credentials.resolve".into(),
                    bound: BTreeMap::new(),
                    inputs: BTreeMap::new(),
                },
            }],
        }
    }

    #[test]
    fn exact_attention_waits_for_matching_request_and_rejects_replaced_generation() {
        let mut local = Local::default();
        local.show_request("credential".into(), 2);
        assert!(local.app().is_none());
        local
            .requests
            .update("credential".into(), 2, Some(request(2)));
        assert!(local.app().is_some());
        local.show_request("credential".into(), 1);
        assert!(local.app().is_none());
        local
            .requests
            .update("credential".into(), 3, Some(request(3)));
        assert!(local.app().is_none());
    }
    #[test]
    fn requests_never_take_focus_and_hiding_clears_secrets_without_cancelling() {
        let mut local = Local::default();
        local
            .requests
            .update("credential".into(), 2, Some(request(2)));
        assert!(local.app().is_none());
        local.show_requests();
        let app = local.app().unwrap();
        app.focus_control(Some(Control::Field {
            node: "request.form".into(),
            field: "value".into(),
        }));
        app.drive(
            misa_window_core::Event::Text("not-for-the-wire-yet".into()),
            std::time::Duration::ZERO,
        );
        assert_eq!(
            app.field_text("request.form", "value"),
            Some("not-for-the-wire-yet")
        );
        assert!(
            local
                .input(misa_window_core::Event::Key(Key::Escape))
                .unwrap()
                .is_empty()
        );
        assert_eq!(local.requests.len(), 1);
        local.show_requests();
        assert_eq!(
            local.app().unwrap().field_text("request.form", "value"),
            Some("")
        );
        local
            .requests
            .update("credential".into(), 1, Some(request(1)));
        assert!(matches!(
            local.requests.action("submit".into(), vec![]),
            Some(Action::Request { generation: 2, .. })
        ));
        local.requests.update("credential".into(), 3, None);
        assert!(local.app().is_none());
        assert_eq!(local.requests.len(), 0);
    }

    #[test]
    fn replaced_generation_never_reuses_secret_drafts_or_old_attention() {
        let mut local = Local::default();
        local.show_request("credential".into(), 2);
        local
            .requests
            .update("credential".into(), 2, Some(request(2)));
        let app = local.app().unwrap();
        app.focus_control(Some(Control::Field {
            node: "request.form".into(),
            field: "value".into(),
        }));
        local.input(misa_window_core::Event::Text("old-secret".into()));
        local
            .requests
            .update("credential".into(), 3, Some(request(3)));
        assert_eq!(
            local.app().unwrap().field_text("request.form", "value"),
            Some("")
        );
        assert!(matches!(
            local.requests.action("submit".into(), vec![]),
            Some(Action::Request { generation: 3, .. })
        ));
        local.input(misa_window_core::Event::Key(Key::Escape));
        local.show_request("credential".into(), 2);
        assert!(local.app().is_none());
        local
            .requests
            .update("credential".into(), 2, Some(request(2)));
        assert!(local.app().is_none());
        local.show_requests();
        assert_eq!(
            local.app().unwrap().field_text("request.form", "value"),
            Some("")
        );
    }

    #[test]
    fn generic_request_keeps_each_local_field_when_hidden() {
        use misa_proto::schema::{Field, Schema};
        let mut model = request(2);
        model.input = None;
        model.form = Some(misa_proto::input::Form {
            title: "Parameters".into(),
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
            fields: BTreeMap::new(),
        });
        model.actions[0].id = "resolve".into();
        let mut local = Local::default();
        local.requests.update("credential".into(), 2, Some(model));
        local.show_requests();
        for (id, value) in [("count", "3"), ("name", "example")] {
            local.app().unwrap().focus_control(Some(Control::Field {
                node: "request.form".into(),
                field: id.into(),
            }));
            local.input(misa_window_core::Event::Text(value.into()));
        }
        local.input(misa_window_core::Event::Key(Key::Escape));
        local.show_requests();
        assert_eq!(
            local.app().unwrap().field_text("request.form", "count"),
            Some("3")
        );
        assert_eq!(
            local.app().unwrap().field_text("request.form", "name"),
            Some("example")
        );
    }

    #[test]
    fn workspace_converts_its_own_actions_without_retyping_app_effects() {
        let mut local = Local::default();
        local.open_chooser();
        assert_eq!(
            local.convert(vec![Command::Copy("text".into())]),
            vec![Action::Ui(Command::Copy("text".into()))]
        );
        let discover = Command::Intent(misa_kit::intent::Intent::Action {
            node: "discover".into(),
            action: "discover".into(),
            fields: vec![],
            args: Value::Null,
        });
        assert_eq!(local.convert(vec![discover]), vec![Action::Discover]);
    }

    #[test]
    fn same_label_session_choices_keep_daemon_and_incarnation() {
        let mut local = Local::default();
        local.open_chooser();
        for daemon in ["daemon-a", "daemon-b"] {
            let commands = local.convert(vec![Command::Intent(misa_kit::intent::Intent::Action {
                node: "same-label".into(),
                action: "choose".into(),
                fields: vec![],
                args: Value::map([
                    ("daemon", Value::str(daemon)),
                    ("id", Value::str("same-label")),
                    ("incarnation", Value::str("incarnation")),
                ]),
            })]);
            assert!(
                matches!(&commands[..], [Action::Select { daemon: actual, scope }] if actual == daemon && scope.incarnation == "incarnation")
            );
            local.open_chooser();
        }
    }
    #[test]
    fn directory_refresh_while_open_keeps_connect_draft_and_manager() {
        let mut local = Local::default();
        local.open_chooser();
        let app = local.app().unwrap();
        app.focus_control(Some(Control::Field {
            node: "connect".into(),
            field: "target".into(),
        }));
        assert!(
            local
                .input(misa_window_core::Event::Text("pairing-ticket".into()))
                .unwrap()
                .is_empty()
        );
        local.directory(vec![DaemonChoice {
            identity: "daemon-a".into(),
            freshness: "current".into(),
            sessions: vec![],
            instances: Default::default(),
            overview: None,
            forms: Default::default(),
            archive: vec![],
            archive_truncated: false,
        }]);
        assert_eq!(
            local.app().unwrap().field_text("connect", "target"),
            Some("pairing-ticket")
        );
        let connect = Command::Intent(misa_kit::intent::Intent::Action {
            node: "connect".into(),
            action: "connect".into(),
            fields: vec![Field {
                id: "target".into(),
                value: "pairing-ticket".into(),
                label: String::new(),
                hint: None,
                kind: FieldKind::default(),
                read_only: false,
                secret: false,
            }],
            args: Value::Null,
        });
        assert_eq!(
            local.convert(vec![connect]),
            vec![Action::Connect("pairing-ticket".into())]
        );
        local.chooser.manage("daemon-a".into());
        local.directory(vec![]);
        assert_eq!(local.chooser.managing(), Some("daemon-a"));
        assert!(local.app().is_some());
    }

    #[test]
    fn optional_action_form_uses_installed_inputs_and_keeps_drafts_local() {
        use misa_proto::{
            invocation::{ActionBinding, Binding, Command as Definition},
            schema::{Field as Input, Schema},
        };
        let interface = misa_client::interface::Interface {
            scope: misa_proto::observation::Scope {
                id: misa_proto::observation::ScopeId::Session {
                    id: "session".into(),
                },
                incarnation: "owner".into(),
            },
            queries: BTreeMap::new(),
            presentations: vec![],
            commands: BTreeMap::from([(
                "feed".into(),
                Definition {
                    preparation: Default::default(),
                    id: "feed".into(),
                    input: Schema::Record {
                        fields: BTreeMap::from([(
                            "amount".into(),
                            Input {
                                schema: Schema::Int,
                                optional: false,
                            },
                        )]),
                        allow_unknown: false,
                    },
                    result: Schema::Value,
                },
            )]),
            actions: BTreeMap::from([(
                "pet.feed".into(),
                Binding {
                    id: "pet.feed".into(),
                    binding: ActionBinding {
                        command: "feed".into(),
                        bound: BTreeMap::new(),
                        inputs: BTreeMap::from([("quantity".into(), "amount".into())]),
                    },
                },
            )]),
        };
        let mut local = Local::default();
        local.form(misa_client::form::Form::action(&interface, "pet.feed").unwrap());
        local.app().unwrap().focus_control(Some(Control::Field {
            node: "local.form".into(),
            field: "quantity".into(),
        }));
        assert!(
            local
                .input(misa_window_core::Event::Text("3".into()))
                .unwrap()
                .is_empty()
        );
        let commands = local.convert(vec![Command::Intent(misa_kit::intent::Intent::Action {
            node: "local.form".into(),
            action: "submit".into(),
            fields: vec![misa_proto::view::Field {
                id: "quantity".into(),
                value: "3".into(),
                label: "Quantity".into(),
                hint: None,
                kind: FieldKind::default(),
                read_only: false,
                secret: false,
            }],
            args: Value::Null,
        })]);
        assert!(
            matches!(&commands[..],[Action::Form{action,drafts}] if action=="pet.feed" && drafts["quantity"]=="3")
        );
        assert!(
            local
                .input(misa_window_core::Event::Key(Key::Escape))
                .unwrap()
                .is_empty()
        );
        assert!(local.app().is_none());
        local.form(misa_client::form::Form::action(&interface, "pet.feed").unwrap());
        assert_eq!(
            local.app().unwrap().field_text("local.form", "quantity"),
            Some("3")
        );
    }
}
pub struct Local {
    metrics: Arc<dyn TextMetrics>,
    report: Option<DocumentUi>,
    appearance: misa_pixel_document::appearance::Choice,
    forms: Forms,
    chooser: Chooser,
    overlay: Option<DocumentUi>,
    pub(crate) requests: Requests,
    catalog: Vec<misa_proto::presentation::Presentation>,
    preferences: misa_client::composition::Preferences,
    pub observation: Option<misa_client::ObservationId>,
    choosing_presentations: bool,
}
fn action(id: &str, label: &str, args: Value, on: ActionOn) -> ViewAction {
    ViewAction {
        id: id.into(),
        label: Some(label.into()),
        args,
        on,
    }
}
#[cfg(test)]
impl Default for Local {
    fn default() -> Self {
        Self::new(misa_skia_paint::text_metrics().expect("Skia text metrics for workspace tests"))
    }
}

impl Local {
    pub fn new(metrics: Arc<dyn TextMetrics>) -> Self {
        Self {
            metrics: metrics.clone(),
            report: None,
            appearance: Default::default(),
            forms: Forms::new(metrics.clone()),
            chooser: Chooser::new(),
            overlay: None,
            requests: Requests::new(metrics.clone()),
            catalog: Vec::new(),
            preferences: Default::default(),
            observation: None,
            choosing_presentations: false,
        }
    }

    pub fn report(&mut self, document: Node) {
        self.deactivate();
        self.report = Some(DocumentUi::new(document, self.metrics.clone()));
    }
    pub fn set_light(&mut self, light: bool) {
        if let Some(app) = self.app() {
            app.set_light(light);
        }
    }
    pub fn appearance(&mut self, choice: misa_pixel_document::appearance::Choice) {
        if self.appearance != choice {
            self.appearance = choice;
            if self.choosing_presentations {
                self.open_presentations();
            }
        }
    }
    pub fn commands(
        &mut self,
        commands: BTreeMap<String, Result<misa_client::form::Form, String>>,
    ) {
        self.forms.commands(commands);
    }
    pub fn open_commands(&mut self) {
        self.deactivate();
        self.forms.open_commands();
    }
    pub fn notice(&mut self, notice: &str) {
        if let Some(app) = self.app() {
            app.notice(notice);
        }
    }
    pub fn form(&mut self, form: misa_client::form::Form) {
        self.deactivate();
        self.forms.open_form(form);
    }
    pub fn deactivate(&mut self) {
        self.report = None;
        self.forms.hide();
        self.chooser.close();
        self.overlay = None;
        self.choosing_presentations = false;
        self.requests.hide();
    }
    pub fn directory(&mut self, entries: Vec<DaemonChoice>) {
        self.chooser.directory(entries);
    }
    pub fn open_chooser(&mut self) {
        self.report = None;
        self.forms.hide();
        self.choosing_presentations = false;
        self.overlay = None;
        self.requests.hide();
        self.chooser.open(self.metrics.clone());
    }
    pub fn composition(
        &mut self,
        catalog: Vec<misa_proto::presentation::Presentation>,
        preferences: misa_client::composition::Preferences,
        observation: misa_client::ObservationId,
    ) {
        self.catalog = catalog;
        self.preferences = preferences;
        self.observation = Some(observation);
        if self.choosing_presentations {
            self.open_presentations();
        }
    }
    pub fn open_presentations(&mut self) {
        self.report = None;
        self.forms.hide();
        self.requests.hide();
        self.choosing_presentations = true;
        let mut root = Node::section("presentations")
            .id("presentations")
            .label(format!(
                "Presentations · appearance {} · Escape closes",
                self.appearance.name()
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
            misa_pixel_document::appearance::Choice::System,
            misa_pixel_document::appearance::Choice::Dark,
            misa_pixel_document::appearance::Choice::Light,
        ] {
            root = root.action(action(
                &format!("appearance.{}", choice.name()),
                &format!("Theme: {}", choice.name()),
                Value::str(choice.name()),
                ActionOn::Click,
            ));
        }
        self.chooser.close();
        self.overlay = Some(DocumentUi::new(root, self.metrics.clone()));
    }
    /// Opening attention dismisses other local overlays while awaiting its exact generation.
    pub fn show_request(&mut self, id: String, generation: i64) {
        self.deactivate();
        self.requests.attend(id, generation);
    }
    pub fn show_requests(&mut self) {
        self.report = None;
        self.forms.hide();
        self.chooser.close();
        self.overlay = None;
        self.requests.cycle();
    }
    fn app(&mut self) -> Option<&mut DocumentUi> {
        if self.report.is_some() {
            return self.report.as_mut();
        }
        if self.forms.is_active() {
            return self.forms.app();
        }
        if self.overlay.is_some() {
            return self.overlay.as_mut();
        }
        if self.chooser.is_open() {
            self.chooser.app()
        } else {
            self.requests.visible_app()
        }
    }
    pub fn frame_at(
        &mut self,
        width: u32,
        height: u32,
        elapsed: std::time::Duration,
    ) -> Option<(Scene, Option<std::time::Duration>)> {
        self.app().and_then(|app| {
            let output = app.drive(
                misa_window_core::Event::Redraw(misa_window_core::Size { width, height }),
                elapsed,
            );
            output.frame.map(|scene| (scene, output.deadline))
        })
    }
    pub fn scroll(&mut self, delta: f32) -> bool {
        if let Some(app) = self.app() {
            app.scroll(delta);
            true
        } else {
            false
        }
    }
    pub fn input(&mut self, event: misa_window_core::Event) -> Option<Vec<Action>> {
        if self.app().is_none() {
            return None;
        }
        if matches!(event, misa_window_core::Event::Key(Key::Escape)) {
            self.deactivate();
            return Some(vec![]);
        }
        let commands = self
            .app()
            .unwrap()
            .drive(event, std::time::Duration::ZERO)
            .commands;
        Some(self.convert(commands))
    }
    pub fn pointer(&mut self, x: f32, y: f32, dragging: bool) -> Option<Vec<Action>> {
        let commands = self.app()?.pointer(x, y, dragging);
        Some(self.convert(commands))
    }
    fn convert(&mut self, commands: Vec<Command>) -> Vec<Action> {
        commands
            .into_iter()
            .filter_map(|command| match command {
                Command::Intent(misa_kit::intent::Intent::Action {
                    action,
                    fields,
                    args,
                    ..
                }) if self.forms.is_active() => self.forms.action(&action, fields, args),
                Command::Intent(misa_kit::intent::Intent::Action { action, args, .. })
                    if self.overlay.is_some() =>
                {
                    match action.as_str() {
                        action if action.starts_with("appearance.") => Some(Action::Appearance(
                            misa_pixel_document::appearance::Choice::parse(args.as_str()?)?,
                        )),
                        value
                            if value.starts_with("presentation.")
                                || value.starts_with("variant.") =>
                        {
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
                Command::Intent(misa_kit::intent::Intent::Action {
                    action,
                    args,
                    fields,
                    ..
                }) if self.chooser.is_open() => match action.as_str() {
                    "daemon-form" => {
                        self.forms.open_daemon_form(&mut self.chooser, &args);
                        None
                    }
                    "archive-search" | "work-form" | "stop-session" => {
                        Forms::management_action(&self.chooser, &action, &args, &fields)
                    }
                    _ => self.chooser.action(&action, &args, &fields),
                },
                Command::Intent(misa_kit::intent::Intent::Action { action, fields, .. }) => {
                    self.requests.action(action, fields)
                }
                Command::Copy(text) => Some(Action::Ui(Command::Copy(text))),
                _ => None,
            })
            .collect()
    }
}
impl Local {
    pub fn prepared_daemon_form(
        &mut self,
        daemon: String,
        form: misa_client::form::Form,
        drafts: BTreeMap<String, String>,
    ) {
        self.forms
            .prepared_daemon_form(&self.chooser, daemon, form, drafts);
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use misa_proto::{
        invocation::Command as Definition,
        schema::{Field as Input, Schema},
    };

    fn intent(action_id: &str, args: Value) -> Command {
        Command::Intent(misa_kit::intent::Intent::Action {
            node: "manager".into(),
            action: action_id.into(),
            fields: vec![],
            args,
        })
    }
    #[test]
    fn installed_command_chooser_uses_schema_without_shortcut_or_action() {
        let command = Definition {
            preparation: Default::default(),
            id: "plugin.custom".into(),
            input: Schema::Record {
                fields: BTreeMap::new(),
                allow_unknown: false,
            },
            result: Schema::Value,
        };
        let interface = misa_client::interface::Interface {
            scope: misa_proto::observation::Scope {
                id: misa_proto::observation::ScopeId::Daemon,
                incarnation: "run".into(),
            },
            queries: BTreeMap::new(),
            commands: BTreeMap::from([(command.id.clone(), command)]),
            presentations: vec![],
            actions: BTreeMap::new(),
        };
        let mut local = Local::default();
        local.commands(BTreeMap::from([(
            "plugin.custom".into(),
            misa_client::form::Form::command(&interface, "plugin.custom")
                .map_err(|fault| fault.message),
        )]));
        local.open_commands();
        local.convert(vec![intent(
            "installed-command",
            Value::str("plugin.custom"),
        )]);
        assert!(local.forms.is_active());
        let result = local.convert(vec![intent("submit", Value::Null)]);
        assert!(
            matches!(&result[..],[Action::InvokeInstalled{command,input}] if command=="plugin.custom" && input==&Value::map([]))
        );
    }
    #[test]
    fn installed_ids_not_labels_own_drafts_across_other_workflows_and_optional_inputs() {
        use misa_pixel_document::ui::Control;
        let interface = misa_client::interface::Interface {
            scope: misa_proto::observation::Scope {
                id: misa_proto::observation::ScopeId::Daemon,
                incarnation: "owner".into(),
            },
            queries: BTreeMap::new(),
            presentations: vec![],
            actions: BTreeMap::new(),
            commands: ["first", "second"]
                .into_iter()
                .map(|id| {
                    (
                        id.into(),
                        Definition {
                            preparation: Default::default(),
                            id: id.into(),
                            input: Schema::Record {
                                fields: BTreeMap::from([(
                                    "amount".into(),
                                    Input {
                                        schema: Schema::Int,
                                        optional: true,
                                    },
                                )]),
                                allow_unknown: false,
                            },
                            result: Schema::Value,
                        },
                    )
                })
                .collect(),
        };
        let mut local = Local::default();
        let mut installed = BTreeMap::new();
        for id in ["first", "second"] {
            let mut form = misa_client::form::Form::command(&interface, id).unwrap();
            form.title = "Same label".into();
            installed.insert(id.into(), Ok(form));
        }
        local.commands(installed);
        local.open_commands();
        local.convert(vec![intent("installed-command", Value::str("first"))]);
        local.app().unwrap().focus_control(Some(Control::Field {
            node: "local.form".into(),
            field: "amount".into(),
        }));
        local.input(misa_window_core::Event::Text("3".into()));
        local.directory(vec![]);
        local.open_chooser();
        local.open_commands();
        local.convert(vec![intent("installed-command", Value::str("first"))]);
        assert_eq!(
            local.app().unwrap().field_text("local.form", "amount"),
            Some("3")
        );
        local.open_commands();
        local.convert(vec![intent("installed-command", Value::str("second"))]);
        assert_eq!(
            local.app().unwrap().field_text("local.form", "amount"),
            Some("")
        );
        let fields = vec![Field {
            id: "amount".into(),
            value: String::new(),
            label: String::new(),
            hint: None,
            kind: FieldKind::default(),
            read_only: false,
            secret: false,
        }];
        let submit = |fields| {
            Command::Intent(misa_kit::intent::Intent::Action {
                node: "local.form".into(),
                action: "submit".into(),
                fields,
                args: Value::Null,
            })
        };
        let mut invalid = fields.clone();
        invalid[0].value = "not-a-number".into();
        assert!(local.convert(vec![submit(invalid)]).is_empty());
        assert!(!local.app().unwrap().notice_text().is_empty());
        assert!(matches!(&local.convert(vec![submit(fields)])[..],
            [Action::InvokeInstalled { command, input }] if command == "second" && input == &Value::map([])));
    }

    #[test]
    fn stop_targets_exact_owner_and_archive_search_stays_local() {
        let mut local = Local::default();
        local.open_chooser();
        local.chooser.manage("daemon-b".into());
        let scope = misa_proto::observation::Scope {
            id: misa_proto::observation::ScopeId::Session {
                id: "same-name".into(),
            },
            incarnation: "old-incarnation".into(),
        };
        let value = serde_json::from_value(serde_json::to_value(&scope).unwrap()).unwrap();
        let commands = local.convert(vec![intent("stop-session", value)]);
        assert!(
            matches!(&commands[..], [Action::DaemonInvoke{daemon,command,input,..}] if daemon=="daemon-b" && command=="daemon.session.close" && input.get("incarnation").and_then(Value::as_str)==Some("old-incarnation"))
        );
        let mut search = intent("archive-search", Value::Null);
        if let Command::Intent(misa_kit::intent::Intent::Action { fields, .. }) = &mut search {
            fields.push(Field {
                id: "prefix".into(),
                label: String::new(),
                value: "history".into(),
                hint: None,
                kind: FieldKind::default(),
                read_only: false,
                secret: false,
            });
        }
        assert!(
            matches!(&local.convert(vec![search])[..],[Action::Archive{daemon,prefix}] if daemon=="daemon-b" && prefix=="history")
        );
    }
    #[test]
    fn resume_form_prefills_conversation_and_survives_directory_refresh() {
        let definition = Definition {
            preparation: Default::default(),
            id: "daemon.session.resume".into(),
            input: Schema::Record {
                fields: BTreeMap::from([
                    (
                        "conversation".into(),
                        Input {
                            schema: Schema::String,
                            optional: false,
                        },
                    ),
                    (
                        "id".into(),
                        Input {
                            schema: Schema::String,
                            optional: false,
                        },
                    ),
                ]),
                allow_unknown: false,
            },
            result: Schema::Value,
        };
        let interface = misa_client::interface::Interface {
            scope: misa_proto::observation::Scope {
                id: misa_proto::observation::ScopeId::Daemon,
                incarnation: "owner".into(),
            },
            queries: BTreeMap::new(),
            presentations: vec![],
            commands: BTreeMap::from([(definition.id.clone(), definition)]),
            actions: BTreeMap::new(),
        };
        let mut local = Local::default();
        local.directory(vec![DaemonChoice {
            identity: "daemon-b".into(),
            instances: Default::default(),
            freshness: "current".into(),
            sessions: vec![],
            overview: None,
            forms: BTreeMap::from([(
                "daemon.session.resume".into(),
                misa_client::form::Form::command(&interface, "daemon.session.resume").unwrap(),
            )]),
            archive: vec![],
            archive_truncated: false,
        }]);
        local.open_chooser();
        local.chooser.manage("daemon-b".into());
        local.forms.open_daemon_form(
            &mut local.chooser,
            &Value::map([
                ("command", Value::str("daemon.session.resume")),
                ("conversation", Value::str("archived")),
            ]),
        );
        assert_eq!(
            local
                .app()
                .unwrap()
                .field_text("daemon.form", "conversation"),
            Some("archived")
        );
        local
            .app()
            .unwrap()
            .focus_control(Some(misa_pixel_document::ui::Control::Field {
                node: "daemon.form".into(),
                field: "id".into(),
            }));
        local.input(misa_window_core::Event::Text("resumed".into()));
        local.directory(vec![]);
        assert_eq!(
            local.app().unwrap().field_text("daemon.form", "id"),
            Some("resumed")
        );
    }
}

#[cfg(test)]
mod overlay_clock_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn visible_overlay_uses_the_host_clock_and_owns_its_next_redraw() {
        let mut local = Local::default();
        local.report(
            Node::section("report").id("report").child(
                Node::section("status.indicators").id("status").child(
                    Node::new(
                        "indicator.activity",
                        Kind::Status {
                            text: "working".into(),
                        },
                    )
                    .id("activity"),
                ),
            ),
        );
        let (first, deadline) = local.frame_at(320, 200, Duration::ZERO).unwrap();
        assert_eq!(deadline, Some(Duration::from_millis(160)));
        let (same, deadline) = local
            .frame_at(320, 200, Duration::from_millis(159))
            .unwrap();
        assert_eq!(first, same);
        assert_eq!(deadline, Some(Duration::from_millis(160)));
        let (changed, deadline) = local
            .frame_at(320, 200, Duration::from_millis(320))
            .unwrap();
        assert_ne!(first, changed);
        assert_eq!(deadline, Some(Duration::from_millis(480)));
    }
}
