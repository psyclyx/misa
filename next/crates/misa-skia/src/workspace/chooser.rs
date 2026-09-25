//! Host-owned directory and session chooser. DocumentUi retains drafts; it never drives daemon effects.
use super::{Action, action};
use misa_pixel_document::ui::DocumentUi;
use misa_pixel_ui::TextMetrics;
use misa_proto::{
    directory::Entry,
    view::{ActionOn, Field, FieldKind, Kind, Node, Span},
};
use misa_value::Value;
use std::{collections::BTreeMap, sync::Arc};

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

pub(super) struct Chooser {
    directories: Vec<DaemonChoice>,
    app: Option<DocumentUi>,
    managing: Option<String>,
    selected: Option<(String, misa_proto::observation::Scope)>,
}

impl Chooser {
    pub(super) fn new() -> Self {
        Self {
            directories: Vec::new(),
            app: None,
            managing: None,
            selected: None,
        }
    }

    pub(super) fn app(&mut self) -> Option<&mut DocumentUi> {
        self.app.as_mut()
    }
    pub(super) fn is_open(&self) -> bool {
        self.app.is_some()
    }
    pub(super) fn close(&mut self) {
        self.app = None;
        self.managing = None;
    }
    pub(super) fn managing(&self) -> Option<&str> {
        self.managing.as_deref()
    }
    pub(super) fn manage(&mut self, daemon: String) {
        self.managing = Some(daemon);
        self.rebuild_chooser();
    }
    pub(super) fn form(&self, daemon: &str, command: &str) -> Option<misa_client::form::Form> {
        self.directories
            .iter()
            .find(|row| row.identity == daemon)?
            .forms
            .get(command)
            .cloned()
    }
    pub(super) fn overview_scope(&self, daemon: &str) -> Option<misa_proto::observation::Scope> {
        self.directories
            .iter()
            .find(|row| row.identity == daemon)?
            .overview
            .as_ref()?
            .as_ref()
            .ok()
            .map(|snapshot| snapshot.scope.clone())
    }
    pub(super) fn directory(&mut self, entries: Vec<DaemonChoice>) {
        self.directories = entries;
        if self.is_open() {
            self.rebuild_chooser();
        }
    }
    pub(super) fn open(&mut self, metrics: Arc<dyn TextMetrics>) {
        self.managing = None;
        self.app = Some(DocumentUi::new(Node::section("chooser"), metrics));
        self.rebuild_chooser();
    }

    pub(super) fn action(
        &mut self,
        action: &str,
        args: &Value,
        fields: &[Field],
    ) -> Option<Action> {
        match action {
            "manage" => {
                self.manage(args.as_str()?.into());
                None
            }
            "manager-back" => {
                self.managing = None;
                self.rebuild_chooser();
                None
            }
            "connect" => Some(Action::Connect(
                fields
                    .iter()
                    .find(|field| field.id == "target")?
                    .value
                    .clone(),
            )),
            "discover" => Some(Action::Discover),
            "disconnect" => Some(Action::Disconnect(args.as_str()?.into())),
            "choose" | "attention" | "close-instance" => {
                let get = |key| args.get(key).and_then(Value::as_str).map(str::to_owned);
                let daemon = get("daemon")?;
                let scope = misa_proto::observation::Scope {
                    id: misa_proto::observation::ScopeId::Session { id: get("id")? },
                    incarnation: get("incarnation")?,
                };
                let command = if action == "attention" {
                    Action::SelectRequest {
                        daemon: daemon.clone(),
                        scope: scope.clone(),
                        request: get("request")?,
                        generation: args.get("generation")?.as_i64()?,
                    }
                } else if action == "choose" {
                    Action::Select {
                        daemon: daemon.clone(),
                        scope: scope.clone(),
                    }
                } else {
                    Action::CloseInstance {
                        daemon: daemon.clone(),
                        scope: scope.clone(),
                    }
                };
                if action != "close-instance" {
                    self.selected = Some((daemon, scope));
                }
                self.close();
                Some(command)
            }
            _ => None,
        }
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
                let selected = self.selected.as_ref().is_some_and(|(identity, scope)| {
                    identity == &daemon.identity && scope == &entry.scope()
                });
                let mut session = Node::section("session")
                    .id(format!("daemon.{index}.session.{ordinal}"))
                    .label(if selected {
                        format!("{} · selected", entry.title)
                    } else {
                        entry.title.clone()
                    })
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
        if let Some(chooser) = &mut self.app {
            chooser.set_view(root);
        }
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
        for id in daemon.forms.keys().filter(|id| {
            !matches!(
                id.as_str(),
                "daemon.session.create" | "daemon.session.resume"
            )
        }) {
            root.children.push(
                Node::section("command")
                    .id(format!("daemon-command-{id}"))
                    .action(action(
                        "daemon-form",
                        id,
                        Value::map([("command", Value::str(id))]),
                        ActionOn::Click,
                    )),
            );
        }
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
                    for work in &snapshot.work {
                        for (command, label) in [
                            ("operation.cancel", "Cancel work"),
                            ("daemon.work.forget", "Forget terminal work"),
                        ] {
                            if daemon.forms.contains_key(command) {
                                let scope = serde_json::to_value(&snapshot.scope)
                                    .ok()
                                    .and_then(|value| serde_json::from_value::<Value>(value).ok());
                                if let Some(scope) = scope {
                                    root.children.push(
                                        Node::section("work-action")
                                            .id(format!("work-action-{}-{command}", work.id))
                                            .action(action(
                                                "work-form",
                                                label,
                                                Value::map([
                                                    ("id", Value::str(&work.id)),
                                                    ("command", Value::str(command)),
                                                    ("scope", scope),
                                                ]),
                                                ActionOn::Click,
                                            )),
                                    );
                                }
                            }
                        }
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
        if let Some(chooser) = &mut self.app {
            chooser.set_view(root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::directory::Availability;

    fn daemon(identity: &str) -> DaemonChoice {
        DaemonChoice {
            identity: identity.into(),
            freshness: "current".into(),
            sessions: vec![Entry {
                id: "same-label".into(),
                incarnation: "run-1".into(),
                title: "Same label".into(),
                availability: Availability::Current,
                source_position: 0,
                summary: Value::Null,
            }],
            instances: Default::default(),
            overview: None,
            forms: Default::default(),
            archive: vec![],
            archive_truncated: false,
        }
    }

    #[test]
    fn refresh_does_not_replace_active_daemon_or_scope_when_labels_and_positions_collide() {
        let mut chooser = Chooser::new();
        let metrics = misa_skia_paint::text_metrics().unwrap();
        chooser.directory(vec![daemon("daemon-a"), daemon("daemon-b")]);
        chooser.open(metrics);
        let args = Value::map([
            ("daemon", Value::str("daemon-b")),
            ("id", Value::str("same-label")),
            ("incarnation", Value::str("run-1")),
        ]);
        let selected = chooser.action("choose", &args, &[]).unwrap();
        assert!(matches!(&selected, Action::Select { daemon, .. } if daemon == "daemon-b"));
        chooser.directory(vec![daemon("daemon-b"), daemon("daemon-a")]);
        assert_eq!(
            chooser.selected,
            Some(("daemon-b".into(), daemon("daemon-b").sessions[0].scope()))
        );
        // Refresh while open must retain the active identity even if an unrelated
        // daemon now occupies its previous row index.
        chooser.open(misa_skia_paint::text_metrics().unwrap());
        chooser.directory(vec![daemon("daemon-a"), daemon("daemon-b")]);
        assert_eq!(
            chooser.selected,
            Some(("daemon-b".into(), daemon("daemon-b").sessions[0].scope()))
        );
        assert!(chooser.is_open());
    }
}
