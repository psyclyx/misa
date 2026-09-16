//! Native-local relationship selection and request instances. Neither a hidden
//! dialog nor a session switch cancels the operation that owns a request.
use crate::{
    Scene,
    app::{App, Command, Key},
};
use misa_client::request::Model;
use misa_proto::{
    directory::Entry,
    view::{Action, ActionOn, Field, FieldKind, Kind, Node, Span},
};
use misa_value::Value;
use std::collections::BTreeMap;

#[derive(Clone)]
pub struct DaemonChoice {
    pub identity: String,
    pub freshness: String,
    pub sessions: Vec<Entry>,
    pub instances: std::collections::BTreeSet<misa_proto::observation::Scope>,
    pub overview: Option<Result<misa_client::overview::Snapshot, String>>,
    pub forms: BTreeMap<String, misa_client::form::Form>,
    pub archive: Vec<misa_proto::view::Choice>,
    pub archive_truncated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Control;

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
    fn requests_never_take_focus_and_hiding_clears_secrets_without_cancelling() {
        let mut local = Local::default();
        local.request("credential".into(), 2, Some(request(2)));
        assert!(local.app().is_none());
        local.open_requests();
        let app = local.app().unwrap();
        app.focus = Some(Control::Field {
            node: "request.form".into(),
            field: "value".into(),
        });
        app.key(Key::Text("not-for-the-wire-yet".into()));
        assert_eq!(
            app.field_text("request.form", "value"),
            Some("not-for-the-wire-yet")
        );
        assert!(local.key(Key::Escape).unwrap().is_empty());
        assert_eq!(local.pending_requests(), 1);
        local.open_requests();
        assert_eq!(
            local.app().unwrap().field_text("request.form", "value"),
            Some("")
        );
        local.request("credential".into(), 1, Some(request(1)));
        assert_eq!(local.requests["credential"].0.generation, 2);
        local.request("credential".into(), 3, None);
        assert!(local.app().is_none());
        assert_eq!(local.pending_requests(), 0);
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
        local.request("credential".into(), 2, Some(model));
        local.open_requests();
        for (id, value) in [("count", "3"), ("name", "example")] {
            local.app().unwrap().focus = Some(Control::Field {
                node: "request.form".into(),
                field: id.into(),
            });
            local.key(Key::Text(value.into()));
        }
        local.key(Key::Escape);
        local.open_requests();
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
    fn same_label_session_choices_keep_daemon_and_incarnation() {
        let mut local = Local::default();
        local.open_chooser();
        for daemon in ["daemon-a", "daemon-b"] {
            let commands = local.convert(vec![Command::Intent(misa_proto::Intent::Action {
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
                matches!(&commands[..], [Command::Select { daemon: actual, scope }] if actual == daemon && scope.incarnation == "incarnation")
            );
            local.open_chooser();
        }
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
                    preparation: Default::default(), id: "feed".into(),
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
        local.app().unwrap().focus = Some(Control::Field {
            node: "local.form".into(),
            field: "quantity".into(),
        });
        assert!(local.key(Key::Text("3".into())).unwrap().is_empty());
        let commands = local.convert(vec![Command::Intent(misa_proto::Intent::Action {
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
            matches!(&commands[..],[Command::Form{action,drafts}] if action=="pet.feed" && drafts["quantity"]=="3")
        );
        assert!(local.key(Key::Escape).unwrap().is_empty());
        assert!(local.app().is_none());
        local.form(misa_client::form::Form::action(&interface, "pet.feed").unwrap());
        assert_eq!(local.app().unwrap().field_text("local.form", "quantity"), Some("3"));
    }
}
#[derive(Default)]
pub struct Local {
    report: Option<App>,
    appearance: crate::appearance::Choice,
    installed_commands: BTreeMap<String, Result<misa_client::form::Form, String>>,
    choosing_commands: bool,
    command_form: bool,
    form_visible: bool,
    managing: Option<String>,
    daemon_form: Option<(String, misa_client::form::Form, App)>,
    form: Option<(String, App)>,
    directories: Vec<DaemonChoice>,
    chooser: Option<App>,
    requests: BTreeMap<String, (Model, App)>,
    active: Option<String>,
    catalog: Vec<misa_proto::presentation::Presentation>,
    preferences: misa_client::composition::Preferences,
    pub observation: Option<misa_client::ObservationId>,
    choosing_presentations: bool,
}
fn action(id: &str, label: &str, args: Value, on: ActionOn) -> Action {
    Action {
        id: id.into(),
        label: Some(label.into()),
        args,
        on,
    }
}
impl Local {
    pub fn report(&mut self, document: Node) {
        self.deactivate();
        self.report = Some(App::new(document));
    }
    pub fn set_light(&mut self, light: bool) {
        if let Some(app) = self.app() {
            app.set_light(light);
        }
    }
    pub fn appearance(&mut self, choice: crate::appearance::Choice) {
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
        self.installed_commands = commands;
        if self.choosing_commands {
            self.open_commands();
        }
    }
    pub fn open_commands(&mut self) {
        self.deactivate();
        self.choosing_commands = true;
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
        let mut app = App::new(root);
        app.scroll(-f32::MAX);
        self.chooser = Some(app);
    }
    pub fn notice(&mut self, notice: &str) {
        if let Some(app) = self.app() {
            app.notice = notice.into();
        }
    }
    pub fn form(&mut self, form: misa_client::form::Form) {
        self.open_form(form, false);
    }
    fn open_form(&mut self, form: misa_client::form::Form, command: bool) {
        if self.command_form == command
            && self.form.as_ref().is_some_and(|(id, _)| id == &form.title)
        {
            self.chooser = None;
            self.report = None;
            self.hide_request();
            self.form_visible = true;
            return;
        }
        self.deactivate();
        self.command_form = command;
        self.form_visible = true;
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
        self.form = Some((form.title, App::new(node)));
    }
    pub fn deactivate(&mut self) {
        self.report = None;
        self.choosing_commands = false;
        self.form_visible = false;
        self.daemon_form = None;
        self.managing = None;
        self.chooser = None;
        self.choosing_presentations = false;
        self.hide_request();
    }
    pub fn directory(&mut self, entries: Vec<DaemonChoice>) {
        self.directories = entries;
        if self.chooser.is_some() && !self.choosing_presentations && !self.choosing_commands {
            self.rebuild_chooser();
        }
    }
    pub fn open_chooser(&mut self) {
        self.report = None;
        self.form_visible = false;
        self.choosing_commands = false;
        self.managing = None;
        self.daemon_form = None;
        self.choosing_presentations = false;
        self.hide_request();
        self.chooser = Some(App::new(Node::section("chooser")));
        self.rebuild_chooser();
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
        self.daemon_form = None;
        self.form_visible = false;
        self.choosing_commands = false;
        self.hide_request();
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
            crate::appearance::Choice::System,
            crate::appearance::Choice::Dark,
            crate::appearance::Choice::Light,
        ] {
            root = root.action(action(
                &format!("appearance.{}", choice.name()),
                &format!("Theme: {}", choice.name()),
                Value::str(choice.name()),
                ActionOn::Click,
            ));
        }
        self.chooser = Some(App::new(root));
    }
    fn rebuild_chooser(&mut self) {
        if self.managing.is_some() {
            self.rebuild_management();
            return;
        }
        let mut root = Node::section("workspace").id("workspace").label("Daemons and sessions · Escape closes")
            .child(Node::text("hint", [Span::plain("Connect a daemon, then choose a session. Connections stay open when selection changes.")]))
            .child(Node::new("connect", Kind::Fields { fields: vec![Field { id: "target".into(), label: "Daemon address or pairing ticket".into(), value: String::new(), hint: None, kind: FieldKind::default(), read_only: false, secret: false }] }).id("connect").action(action("connect", "Connect daemon", Value::Null, ActionOn::Submit)))
            .child(Node::section("discover").id("discover").action(action("discover", "Find local daemons", Value::Null, ActionOn::Click)));
        for (index, daemon) in self.directories.iter().enumerate() {
            let mut group = Node::section("daemon")
                .id(format!("daemon.{index}"))
                .label(format!(
                    "{} · {}",
                    daemon.identity.chars().take(12).collect::<String>(),
                    daemon.freshness
                ))
                .action(action(
                    "disconnect",
                    "Disconnect daemon",
                    Value::str(&daemon.identity),
                    ActionOn::Click,
                ));
            for (ordinal, entry) in daemon.sessions.iter().take(128).enumerate() {
                let mut session = Node::section("session")
                    .id(format!("daemon.{index}.session.{ordinal}"))
                    .label(entry.title.clone())
                    .action(action(
                        "choose",
                        "Open session",
                        Value::map([
                            ("daemon", Value::str(&daemon.identity)),
                            ("id", Value::str(&entry.id)),
                            ("incarnation", Value::str(&entry.incarnation)),
                        ]),
                        ActionOn::Click,
                    ));
                if daemon.instances.contains(&entry.scope()) {
                    session = session.action(action(
                        "close-instance",
                        "Close · discard draft",
                        Value::map([
                            ("daemon", Value::str(&daemon.identity)),
                            ("id", Value::str(&entry.id)),
                            ("incarnation", Value::str(&entry.incarnation)),
                        ]),
                        ActionOn::Click,
                    ));
                }
                group = group.child(session);
            }
            root = root.child(group);
        }
        for (index, daemon) in self.directories.iter().enumerate() {
            root = root.child(
                Node::section("manage")
                    .id(format!("manage.{index}"))
                    .action(action(
                        "manage",
                        &format!(
                            "Manage {} · overview and history",
                            daemon.identity.chars().take(12).collect::<String>()
                        ),
                        Value::str(&daemon.identity),
                        ActionOn::Click,
                    )),
            );
        }
        if let Some(chooser) = &mut self.chooser {
            chooser.set_view(root);
        }
    }
    pub fn request(&mut self, id: String, generation: i64, model: Option<Model>) {
        if self.requests.len() >= 32 && !self.requests.contains_key(&id) {
            return;
        }
        if self
            .requests
            .get(&id)
            .is_some_and(|(old, _)| old.generation > generation)
        {
            return;
        }
        let Some(model) = model else {
            self.requests.remove(&id);
            if self.active.as_ref() == Some(&id) {
                self.active = None;
            }
            return;
        };
        if model.generation != generation {
            return;
        }
        if self
            .requests
            .get(&id)
            .is_some_and(|(old, _)| old.generation == generation)
        {
            return;
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
        self.requests.insert(id.clone(), (model, App::new(view)));
    }
    fn hide_request(&mut self) {
        if let Some(id) = self.active.take() {
            if let Some((_, app)) = self.requests.get_mut(&id) {
                app.clear_secret_drafts();
            }
        }
    }
    pub fn open_requests(&mut self) {
        self.report = None;
        self.daemon_form = None;
        self.form_visible = false;
        self.chooser = None;
        let ids: Vec<_> = self.requests.keys().cloned().collect();
        let next = self
            .active
            .as_ref()
            .and_then(|id| ids.iter().position(|value| value == id))
            .map_or(0, |index| (index + 1) % ids.len().max(1));
        self.hide_request();
        self.active = ids.get(next).cloned();
    }
    pub fn pending_requests(&self) -> usize {
        self.requests.len()
    }
    fn app(&mut self) -> Option<&mut App> {
        if self.report.is_some() {
            return self.report.as_mut();
        }
        if self.daemon_form.is_some() {
            return self.daemon_form.as_mut().map(|(_, _, app)| app);
        }
        if self.form_visible && self.form.is_some() {
            return self.form.as_mut().map(|(_, app)| app);
        }
        if self.chooser.is_some() {
            self.chooser.as_mut()
        } else {
            self.active
                .as_ref()
                .and_then(|id| self.requests.get_mut(id).map(|(_, app)| app))
        }
    }
    pub fn frame(&mut self, width: u32, height: u32) -> Option<Scene> {
        self.app().map(|app| app.frame(width, height))
    }
    pub fn scroll(&mut self, delta: f32) -> bool {
        if let Some(app) = self.app() {
            app.scroll(delta);
            true
        } else {
            false
        }
    }
    pub fn key(&mut self, key: Key) -> Option<Vec<Command>> {
        if self.app().is_none() {
            return None;
        }
        if matches!(key, Key::Escape) {
            self.deactivate();
            return Some(vec![]);
        }
        let commands = self.app().unwrap().key(key);
        Some(self.convert(commands))
    }
    pub fn pointer(&mut self, x: f32, y: f32, dragging: bool) -> Option<Vec<Command>> {
        let commands = self.app()?.pointer(x, y, dragging);
        Some(self.convert(commands))
    }
    fn convert(&mut self, commands: Vec<Command>) -> Vec<Command> {
        commands
            .into_iter()
            .filter_map(|command| match command {
                Command::Intent(misa_proto::Intent::Action { fields, .. })
                    if self.daemon_form.is_some() =>
                {
                    let (daemon, form, app) = self.daemon_form.as_mut()?;
                    let drafts = fields
                        .into_iter()
                        .map(|field| (field.id, field.value))
                        .collect();
                    match form.prepare(&drafts) {
                        Ok((command, input)) => Some(Command::DaemonInvoke {
                            daemon: daemon.clone(),
                            command,
                            input,
                        }),
                        Err(fault) => {
                            app.notice = fault.message;
                            None
                        }
                    }
                }
                Command::Intent(misa_proto::Intent::Action { fields, .. })
                    if self.form_visible && self.form.is_some() =>
                {
                    let drafts = fields
                        .into_iter()
                        .map(|field| (field.id, field.value))
                        .collect();
                    let (id, app) = self.form.as_mut()?;
                    if self.command_form {
                        match self
                            .installed_commands
                            .get(id)?
                            .as_ref()
                            .ok()?
                            .prepare(&drafts)
                        {
                            Ok((command, input)) => {
                                Some(Command::InvokeInstalled { command, input })
                            }
                            Err(fault) => {
                                app.notice = fault.message;
                                None
                            }
                        }
                    } else {
                        Some(Command::Form {
                            action: id.clone(),
                            drafts,
                        })
                    }
                }
                Command::Intent(misa_proto::Intent::Action {
                    action,
                    args,
                    fields,
                    ..
                }) if self.chooser.is_some() => match action.as_str() {
                    action if action.starts_with("appearance.") => Some(Command::Appearance(
                        crate::appearance::Choice::parse(args.as_str()?)?,
                    )),
                    "installed-command" => {
                        let form = self
                            .installed_commands
                            .get(args.as_str()?)?
                            .as_ref()
                            .ok()?
                            .clone();
                        self.open_form(form, true);
                        None
                    }
                    "manage" => {
                        self.managing = Some(args.as_str()?.into());
                        self.rebuild_chooser();
                        None
                    }
                    "manager-back" => {
                        self.managing = None;
                        self.rebuild_chooser();
                        None
                    }
                    "daemon-form" => {
                        self.open_daemon_form(&args);
                        None
                    }
                    "archive-search" => Some(Command::Archive {
                        daemon: self.managing.clone()?,
                        prefix: fields
                            .iter()
                            .find(|field| field.id == "prefix")?
                            .value
                            .clone(),
                    }),
                    "stop-session" => {
                        let daemon = self.managing.clone()?;
                        let scope: misa_proto::observation::Scope =
                            misa_client::interface::decode(&args).ok()?;
                        Some(Command::DaemonInvoke {
                            daemon,
                            command: "daemon.session.close".into(),
                            input: misa_client::lifecycle::close_input(&scope).ok()?,
                        })
                    }
                    value
                        if value.starts_with("presentation.") || value.starts_with("variant.") =>
                    {
                        let id = args.get("id")?.as_str()?.to_owned();
                        let choice = match args.get("choice")?.as_str()? {
                            "hide" => misa_client::composition::Choice::Hidden,
                            "auto" => misa_client::composition::Choice::Auto,
                            value => misa_client::composition::Choice::Variant(value.into()),
                        };
                        Some(Command::Presentation { id, choice })
                    }
                    "connect" => Some(Command::Connect(
                        fields
                            .iter()
                            .find(|field| field.id == "target")?
                            .value
                            .clone(),
                    )),
                    "discover" => Some(Command::Discover),
                    "disconnect" => Some(Command::Disconnect(args.as_str()?.into())),
                    "choose" | "close-instance" => {
                        let get = |key| args.get(key).and_then(Value::as_str).map(str::to_owned);
                        let daemon = get("daemon")?;
                        let scope = misa_proto::observation::Scope {
                            id: misa_proto::observation::ScopeId::Session { id: get("id")? },
                            incarnation: get("incarnation")?,
                        };
                        let command = if action == "choose" {
                            Command::Select { daemon, scope }
                        } else {
                            Command::CloseInstance { daemon, scope }
                        };
                        self.chooser = None;
                        Some(command)
                    }
                    _ => None,
                },
                Command::Intent(misa_proto::Intent::Action { action, fields, .. }) => {
                    let id = self.active.as_ref()?;
                    let (model, _) = self.requests.get(id)?;
                    Some(Command::Request {
                        id: id.clone(),
                        generation: model.generation,
                        action,
                        fields: fields
                            .into_iter()
                            .map(|field| (field.id, Value::str(field.value)))
                            .collect(),
                    })
                }
                Command::Copy(text) => Some(Command::Copy(text)),
                _ => None,
            })
            .collect()
    }
}
impl Local {
    fn open_daemon_form(&mut self, args: &Value) {
        let Some(daemon) = self.managing.clone() else {
            return;
        };
        let command = args.get("command").and_then(Value::as_str).unwrap_or("");
        let Some(form) = self
            .directories
            .iter()
            .find(|row| row.identity == daemon)
            .and_then(|row| row.forms.get(command))
            .cloned()
        else {
            self.notice("Daemon command is not available");
            return;
        };
        let fields = form
            .fields
            .iter()
            .map(|(id, field)| Field {
                id: id.clone(),
                label: format!("{id}{}", if field.optional { " (optional)" } else { "" }),
                value: args.get(id).and_then(Value::as_str).unwrap_or("").into(),
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
        self.daemon_form = Some((daemon, form, App::new(node)));
    }
    fn rebuild_management(&mut self) {
        let Some(identity) = self.managing.as_ref() else {
            return;
        };
        let Some(daemon) = self
            .directories
            .iter()
            .find(|row| &row.identity == identity)
        else {
            return;
        };
        let mut root = Node::section("manager")
            .id("manager")
            .label(format!(
                "Daemon {} · {}",
                identity.chars().take(12).collect::<String>(),
                daemon.freshness
            ))
            .action(action(
                "manager-back",
                "Back to daemon selection",
                Value::Null,
                ActionOn::Click,
            ))
            .action(action(
                "daemon-form",
                "New session",
                Value::map([("command", Value::str("daemon.session.create"))]),
                ActionOn::Click,
            ));
        for (index, entry) in daemon.sessions.iter().enumerate() {
            let mut label = entry.title.clone();
            if let Some(Ok(snapshot)) = &daemon.overview
                && let Some(row) = snapshot.rows.iter().find(|row| row.scope == entry.scope())
            {
                label = format!(
                    "{} · {} · {} pending · {} blocking · {} own / {} total tokens{}",
                    label,
                    if row.working { "working" } else { "idle" },
                    row.attention,
                    row.blocking.len(),
                    row.direct_usage
                        .input_tokens
                        .saturating_add(row.direct_usage.output_tokens),
                    row.inclusive_usage
                        .input_tokens
                        .saturating_add(row.inclusive_usage.output_tokens),
                    if row.availability != misa_proto::directory::Availability::Current {
                        " · stale"
                    } else {
                        ""
                    }
                );
            }
            let scope =
                serde_json::from_value(serde_json::to_value(entry.scope()).unwrap()).unwrap();
            let mut row = Node::section("session")
                .id(format!("managed.{index}"))
                .label(label)
                .action(action(
                    "choose",
                    "Open session",
                    Value::map([
                        ("daemon", Value::str(identity)),
                        ("id", Value::str(&entry.id)),
                        ("incarnation", Value::str(&entry.incarnation)),
                    ]),
                    ActionOn::Click,
                ));
            if entry.availability == misa_proto::directory::Availability::Current {
                row = row.action(action(
                    "stop-session",
                    "Stop server session · keep conversation",
                    scope,
                    ActionOn::Click,
                ));
            }
            root = root.child(row);
        }
        if let Some(overview) = &daemon.overview {
            match overview {
                Err(error) => root = root.child(Node::text("notice", [Span::plain(error)])),
                Ok(snapshot) => {
                    if !matches!(snapshot.status, misa_protocol::observation::Status::Current) {
                        root = root.child(Node::text(
                            "notice",
                            [Span::plain("Overview is stale; reconnecting to its owner")],
                        ))
                    }
                    for (index, work) in snapshot.work.iter().enumerate() {
                        root = root.child(
                            Node::text(
                                "work",
                                [Span::plain(format!(
                                    "{} · {} · {}{}",
                                    work.id,
                                    work.state,
                                    work.lifetime,
                                    if work.blocking {
                                        " · blocks parent"
                                    } else {
                                        ""
                                    }
                                ))],
                            )
                            .id(format!("work.{index}")),
                        );
                    }
                    for (index, request) in snapshot
                        .rows
                        .iter()
                        .flat_map(|row| &row.requests)
                        .enumerate()
                    {
                        if let misa_proto::observation::ScopeId::Session { id } = &request.scope.id
                        {
                            root = root.child(
                                Node::section("attention")
                                    .id(format!("attention.{index}"))
                                    .label(format!("Input requested in {id}"))
                                    .action(action(
                                        "choose",
                                        "Open request session",
                                        Value::map([
                                            ("daemon", Value::str(identity)),
                                            ("id", Value::str(id)),
                                            ("incarnation", Value::str(&request.scope.incarnation)),
                                        ]),
                                        ActionOn::Click,
                                    )),
                            );
                        }
                    }
                }
            }
        }
        root = root.child(
            Node::new(
                "archive",
                Kind::Fields {
                    fields: vec![Field {
                        id: "prefix".into(),
                        label: "Search stored conversations".into(),
                        value: String::new(),
                        hint: Some("Empty search lists recent conversations".into()),
                        kind: FieldKind::default(),
                        read_only: false,
                        secret: false,
                    }],
                },
            )
            .id("archive")
            .action(action(
                "archive-search",
                "Search archive",
                Value::Null,
                ActionOn::Submit,
            )),
        );
        for (index, item) in daemon.archive.iter().enumerate() {
            root = root.child(
                Node::section("archive.item")
                    .id(format!("archive.{index}"))
                    .label(format!(
                        "{} · {}",
                        item.label,
                        item.detail.as_deref().unwrap_or("")
                    ))
                    .action(action(
                        "daemon-form",
                        "Resume conversation",
                        Value::map([
                            ("command", Value::str("daemon.session.resume")),
                            ("conversation", Value::str(&item.value)),
                        ]),
                        ActionOn::Click,
                    )),
            );
        }
        if daemon.archive_truncated {
            root = root.child(Node::text(
                "notice",
                [Span::plain("More conversations match; narrow the search")],
            ))
        }
        if let Some(chooser) = &mut self.chooser {
            chooser.set_view(root);
        }
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
        Command::Intent(misa_proto::Intent::Action {
            node: "manager".into(),
            action: action_id.into(),
            fields: vec![],
            args,
        })
    }
    #[test]
    fn installed_command_chooser_uses_schema_without_shortcut_or_action() {
        let command = Definition {
            preparation: Default::default(), id: "plugin.custom".into(),
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
        assert!(local.command_form);
        let result = local.convert(vec![intent("submit", Value::Null)]);
        assert!(
            matches!(&result[..],[Command::InvokeInstalled{command,input}] if command=="plugin.custom" && input==&Value::map([]))
        );
    }
    #[test]
    fn stop_targets_exact_owner_and_archive_search_stays_local() {
        let mut local = Local::default();
        local.open_chooser();
        local.managing = Some("daemon-b".into());
        let scope = misa_proto::observation::Scope {
            id: misa_proto::observation::ScopeId::Session {
                id: "same-name".into(),
            },
            incarnation: "old-incarnation".into(),
        };
        let value = serde_json::from_value(serde_json::to_value(&scope).unwrap()).unwrap();
        let commands = local.convert(vec![intent("stop-session", value)]);
        assert!(
            matches!(&commands[..], [Command::DaemonInvoke{daemon,command,input}] if daemon=="daemon-b" && command=="daemon.session.close" && input.get("incarnation").and_then(Value::as_str)==Some("old-incarnation"))
        );
        let mut search = intent("archive-search", Value::Null);
        if let Command::Intent(misa_proto::Intent::Action { fields, .. }) = &mut search {
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
            matches!(&local.convert(vec![search])[..],[Command::Archive{daemon,prefix}] if daemon=="daemon-b" && prefix=="history")
        );
    }
    #[test]
    fn resume_form_prefills_conversation_and_survives_directory_refresh() {
        let definition = Definition {
            preparation: Default::default(), id: "daemon.session.resume".into(),
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
        local.managing = Some("daemon-b".into());
        local.open_daemon_form(&Value::map([
            ("command", Value::str("daemon.session.resume")),
            ("conversation", Value::str("archived")),
        ]));
        assert_eq!(
            local
                .app()
                .unwrap()
                .field_text("daemon.form", "conversation"),
            Some("archived")
        );
        local.app().unwrap().focus = Some(crate::app::Control::Field {
            node: "daemon.form".into(),
            field: "id".into(),
        });
        local.key(Key::Text("resumed".into()));
        local.directory(vec![]);
        assert_eq!(
            local.app().unwrap().field_text("daemon.form", "id"),
            Some("resumed")
        );
    }
}
