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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Control;

    fn request(generation: i64) -> Model {
        Model {
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
                label:"Quantity".into(),hint:None,kind:FieldKind::default(),read_only:false,secret:false,
            }],
            args: Value::Null,
        })]);
        assert!(
            matches!(&commands[..],[Command::Form{action,drafts}] if action=="pet.feed" && drafts["quantity"]=="3")
        );
        assert!(local.key(Key::Escape).unwrap().is_empty());
        assert!(local.app().is_none());
    }
}
#[derive(Default)]
pub struct Local {
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
    pub fn notice(&mut self, notice: &str) {
        if let Some(app) = self.app() {
            app.notice = notice.into();
        }
    }
    pub fn form(&mut self, form: misa_client::form::Form) {
        self.deactivate();
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
        self.form = None;
        self.chooser = None;
        self.choosing_presentations = false;
        self.hide_request();
    }
    pub fn directory(&mut self, entries: Vec<DaemonChoice>) {
        self.directories = entries;
        if self.chooser.is_some() && !self.choosing_presentations {
            self.rebuild_chooser();
        }
    }
    pub fn open_chooser(&mut self) {
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
        self.hide_request();
        self.choosing_presentations = true;
        let mut root = Node::section("presentations")
            .id("presentations")
            .label("Presentations · Escape closes");
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
        self.chooser = Some(App::new(root));
    }
    fn rebuild_chooser(&mut self) {
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
        if let Some(chooser) = &mut self.chooser {
            chooser.set_view(root);
            chooser.scroll(-f32::MAX);
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
        let mut form = Node::new(
            "request.form",
            Kind::Fields {
                fields: model
                    .input
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
                if index == 0 && model.input.is_some() {
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
        if self.form.is_some() {
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
                    if self.form.is_some() =>
                {
                    Some(Command::Form {
                        action: self.form.as_ref()?.0.clone(),
                        drafts: fields
                            .into_iter()
                            .map(|field| (field.id, field.value))
                            .collect(),
                    })
                }
                Command::Intent(misa_proto::Intent::Action {
                    action,
                    args,
                    fields,
                    ..
                }) if self.chooser.is_some() => match action.as_str() {
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
