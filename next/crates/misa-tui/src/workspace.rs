//! A client's daemon directory and current session, independently owned.
use crate::{Presentation, Session, SessionReply, SessionRequest};
use misa_proto::view::Choice;
use misa_kit::intent::Command;
use misa_proto::preparation::Arg;
use misa_kit::intent::Source;
use misa_proto::{ClientInfo, Node};
use misa_kit::intent::Intent;
use std::collections::VecDeque;

pub struct Workspace {
    pub daemons: std::sync::Arc<misa_client::daemons::Daemons>,
    connected: std::collections::BTreeMap<String, std::sync::Arc<misa_client::daemons::Daemon>>,
    selected: Option<String>,
    overviews: std::sync::Arc<
        std::sync::Mutex<
            std::collections::BTreeMap<String, Result<misa_client::overview::Snapshot, String>>,
        >,
    >,
    active: Option<crate::scoped_remote::ScopedRemote>,
    parked: std::collections::BTreeMap<String, crate::scoped_remote::ScopedRemote>,
    updates: VecDeque<Presentation>,
    local_pending: tokio::task::JoinSet<Result<LocalChange, String>>,
    directory_changes: tokio::sync::watch::Receiver<u64>,
    directory_signal: tokio::sync::watch::Sender<u64>,
    monitors: tokio::task::JoinSet<()>,
    selection_generation: u64,
}
enum LocalChange {
    Reply(SessionReply),
    Connected(std::sync::Arc<misa_client::daemons::Daemon>),
    Attached(u64, crate::scoped_remote::ScopedRemote),
    Opened(u64, crate::scoped_remote::ScopedRemote),
    Closed(String),
    Completion(u64, String, String, Result<(Vec<Choice>, bool), String>),
}

impl Workspace {
    fn park(&mut self) -> Result<(), String> {
        if self.active.is_some() && self.parked.len() >= 8 {
            return Err("Eight sessions are already retained".into());
        }
        if let Some(remote) = self.active.take() {
            self.parked.insert(remote.identity(), remote);
        }
        Ok(())
    }
    fn activate(&mut self, mut remote: crate::scoped_remote::ScopedRemote) -> Result<(), String> {
        self.park()?;
        remote.reactivate();
        self.active = Some(remote);
        Ok(())
    }
    pub async fn using(daemons: std::sync::Arc<misa_client::daemons::Daemons>) -> Self {
        let (directory_signal, directory_changes) = tokio::sync::watch::channel(0);
        let mut workspace = Self {
            daemons: daemons.clone(),
            connected: Default::default(),
            selected: None,
            overviews: Default::default(),
            active: None,
            parked: Default::default(),
            updates: Default::default(),
            local_pending: Default::default(),
            directory_changes,
            directory_signal,
            monitors: Default::default(),
            selection_generation: 0,
        };
        for daemon in daemons.connected().await {
            workspace.remember(daemon);
        }
        workspace.selected = workspace.connected.keys().next().cloned();
        workspace
    }
    pub async fn start(targets: &[String]) -> Result<Self, String> {
        let identity =
            misa_transport::identity::load(&misa_transport::identity::client_path("misa-tui")?)?;
        let endpoint = misa_transport::iroh::bind(Some(identity), true).await?;
        let mut workspace = Self::using(std::sync::Arc::new(misa_client::daemons::Daemons::new(
            endpoint,
            ClientInfo::new("misa-tui", env!("CARGO_PKG_VERSION")),
        )))
        .await;
        let mut all = targets.to_vec();
        #[cfg(unix)]
        if all.is_empty() {
            all = misa_transport::local::discover().await?;
        }
        for target in &all {
            workspace.connect(target).await?;
        }
        // A unique daemon/session is unambiguous; otherwise the directory is the landing page.
        if targets.len() == 1 {
            if let Ok((ticket, _)) = misa_proto::Pairing::given(&targets[0])
                && !ticket.session.is_empty()
            {
                workspace.attach(&ticket.session).await?;
            }
        } else if workspace.connected.len() == 1 {
            let daemon = workspace.connected.values().next().unwrap();
            let sessions = daemon.sessions().map_err(|fault| fault.message)?.sessions;
            if sessions.len() == 1 {
                let session = sessions[0].id.clone();
                workspace.attach(&session).await?;
            }
        }
        workspace.refresh();
        Ok(workspace)
    }

    async fn connect(&mut self, target: &str) -> Result<(), String> {
        let (daemon, _) = self
            .daemons
            .connect_target(target)
            .await
            .map_err(|fault| fault.message)?;
        let mut changes = daemon.watch();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                if daemon.sessions().is_ok_and(|snapshot| {
                    matches!(snapshot.status, misa_protocol::observation::Status::Current)
                }) {
                    break Ok::<(), String>(());
                }
                changes
                    .changed()
                    .await
                    .map_err(|_| "Daemon directory closed".to_string())?;
            }
        })
        .await
        .map_err(|_| "Daemon directory timed out".to_string())??;
        let id = daemon.identity().to_string();
        self.remember(daemon);
        self.selected = Some(id);
        Ok(())
    }

    fn remember(&mut self, daemon: std::sync::Arc<misa_client::daemons::Daemon>) {
        let id = daemon.identity().to_string();
        if !self.connected.contains_key(&id) {
            self.monitors.spawn(monitor_overview(
                daemon.clone(),
                self.overviews.clone(),
                self.directory_signal.clone(),
            ));
            let mut changes = daemon.watch();
            let signal = self.directory_signal.clone();
            self.monitors.spawn(async move {
                while changes.changed().await.is_ok() {
                    signal.send_modify(|sequence| *sequence = sequence.wrapping_add(1));
                }
            });
        }
        self.connected.insert(id, daemon);
    }

    async fn attach(&mut self, session: &str) -> Result<(), String> {
        let daemon = self
            .selected
            .as_ref()
            .and_then(|id| self.connected.get(id))
            .ok_or("Connect to a daemon first with /connect")?;
        let entry = daemon
            .sessions()
            .map_err(|fault| fault.message)?
            .sessions
            .into_iter()
            .find(|entry| entry.id == session)
            .ok_or("Session is not in the daemon directory")?;
        self.attach_entry(daemon.clone(),entry).await
    }
    async fn attach_entry(&mut self,daemon:std::sync::Arc<misa_client::daemons::Daemon>,entry:misa_proto::directory::Entry)->Result<(),String> {
        let key = format!("{}:{:?}", daemon.identity(), entry.scope());
        let remote = if let Some(remote) = self.parked.remove(&key) {
            remote
        } else {
            crate::scoped_remote::ScopedRemote::on(daemon.clone(), entry).await?
        };
        self.activate(remote)?;
        Ok(())
    }

    fn declaration(&self) -> crate::Catalog {
        let mut info = self.active.as_ref().map(Session::catalog).unwrap_or_default();
        info.commands.extend([
            Command::new("new","New session","Create a daemon-owned session").arg(Arg::new("id","Session ID").required()).arg(Arg::new("title","Title")).arg(Arg::new("provider","Provider")).arg(Arg::new("model","Model")),
            Command::new("resume","Resume conversation","Mount a stored conversation as a live session").arg(Arg::new("id","Session ID").required()).arg(Arg::new("conversation","Conversation").required().from("client.conversations")).arg(Arg::new("title","Title")).arg(Arg::new("provider","Provider")).arg(Arg::new("model","Model")),
            Command::new("close","Close daemon session","Stop the selected server session; its conversation remains stored"),
            Command::new("visit","Open daemon session","Open a session on a specific connected daemon").arg(Arg::new("daemon","Daemon").required().from("client.daemons")).arg(Arg::new("session","Session").required()),
            Command::new("work-command","Prepare work action","Read exact daemon work generation before preparing an action").arg(Arg::new("daemon","Daemon").required()).arg(Arg::new("incarnation","Owner incarnation").required()).arg(Arg::new("command","Command").required()).arg(Arg::new("operation","Work ID").required()),
            Command::new("daemon-commands","Daemon commands","List installed daemon commands"),
            Command::new("daemon-command","Prepare daemon command","Open a schema-validated local form for an installed daemon command").arg(Arg::new("command","Command").required()),
            Command::new("attention","Open exact input request","Open the request incarnation shown by the overview").arg(Arg::new("daemon","Daemon").required()).arg(Arg::new("session","Session").required()).arg(Arg::new("incarnation","Incarnation").required()).arg(Arg::new("request","Request").required()).arg(Arg::new("generation","Generation").required()),
            Command::new("overview", "Work overview", "Show summaries and delegated work across connected daemons without closing sessions"),
            Command::new("detach", "Detach local session", "Release this local session after directed requests settle; server work remains on its daemon"),
            Command::new("actions", "Document actions", "Choose an action offered by a visible document"),
            Command::new("action", "Run document action", "Invoke the installed binding offered by a visible document").arg(Arg::new("action","Action").required()),
            Command::new(
                "presentation",
                "Presentation",
                "Choose auto, hide, or a named variant before subscribing",
            )
            .arg(
                Arg::new("presentation", "Presentation")
                    .required()
                    .from("client.presentations"),
            )
            .arg(Arg::new("variant", "auto, hide, or variant").required()),
            Command::new(
                "operations",
                "Pending requests",
                "Open local input requests; hiding them leaves work running",
            ),
            Command::new(
                "connect",
                "Connect daemon",
                "Connect another daemon by address or pairing ticket",
            )
            .arg(Arg::new("target", "Daemon address").required()),
            Command::new("daemon", "Choose daemon", "Choose a connected daemon").arg(
                Arg::new("daemon", "Daemon")
                    .required()
                    .from("client.daemons"),
            ),
            Command::new(
                "session",
                "Choose session",
                "Attach a session on the selected daemon",
            )
            .arg(
                Arg::new("session", "Session")
                    .required()
                    .from("client.sessions"),
            ),
            Command::new(
                "daemons",
                "Daemon directory",
                "Show connected daemons without attaching a session",
            ),
        ]);
        info.sources.extend([
            Source::resident("client.presentations", "Presentations"),
            Source::resident("client.daemons", "Daemons"),
            Source::resident("client.sessions", "Sessions"),
            Source::on_demand(
                "client.conversations",
                "Stored conversations",
                "Search the daemon archive",
            ),
        ]);
        info
    }

    fn choices(&self, source: &str) -> Vec<Choice> {
        match source {
            "client.presentations" => self
                .active
                .as_ref()
                .map(|remote| remote.presentation_choices())
                .unwrap_or_default(),
            "client.daemons" => self
                .connected
                .iter()
                .map(|(id, daemon)| Choice {
                    value: id.clone(),
                    label: id.chars().take(12).collect(),
                    detail: Some(format!(
                        "{} sessions",
                        daemon
                            .sessions()
                            .map(|snapshot| snapshot.sessions.len())
                            .unwrap_or(0)
                    )),
                })
                .collect(),
            "client.sessions" => self
                .selected
                .as_ref()
                .and_then(|id| self.connected.get(id))
                .map(|daemon| {
                    daemon
                        .sessions()
                        .map(|snapshot| snapshot.sessions)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|entry| Choice {
                            value: entry.id,
                            label: entry.title,
                            detail: None,
                        })
                        .collect()
                })
                .unwrap_or_default(),
            _ => vec![],
        }
    }

    fn refresh(&mut self) {
        self.updates.clear();
        self.updates.push_back(Presentation::Activate(
            self.active
                .as_ref()
                .map(|remote| remote.identity())
                .unwrap_or_else(|| "directory".into()),
        ));
        self.updates
            .push_back(Presentation::Declaration {catalog:self.declaration(),location:self.location()});
        if self.active.is_none() {
            let mut view = Node::section("session").id("session");
            view.children.push(
                Node::text(
                    "session.title",
                    [misa_proto::view::Span::plain("Misa · Daemons")],
                )
                .id("directory-title"),
            );
            for (id, daemon) in &self.connected {
                let selected = self.selected.as_ref() == Some(id);
                view.children.push(
                    Node::text(
                        "session.header",
                        [misa_proto::view::Span::plain(format!(
                            "{} {}",
                            if selected { ">" } else { " " },
                            &id[..id.len().min(12)]
                        ))],
                    )
                    .id(format!("daemon-{id}")),
                );
                let snapshots = self.overviews.lock().expect("overview state poisoned");
                match snapshots.get(id) {
                    Some(Ok(snapshot)) => view.children.extend(overview_rows(id, snapshot)),
                    Some(Err(error)) => view.children.push(
                        Node::text("notice", [misa_proto::view::Span::plain(error)])
                            .id(format!("overview-fault-{id}")),
                    ),
                    None => {
                        if let Ok(snapshot) = daemon.sessions() {
                            for entry in snapshot.sessions {
                                view.children.push(
                                    Node::text(
                                        "status",
                                        [misa_proto::view::Span::plain(format!(
                                            "{} · {}",
                                            entry.id, entry.title
                                        ))],
                                    )
                                    .id(format!("session-{id}-{}", entry.id)),
                                );
                            }
                        }
                    }
                }
            }
            view.children.push(Node::text("notice", [misa_proto::view::Span::plain("/connect adds a daemon · /daemon chooses one · /session chooses its session · /overview returns here")]).id("directory-help"));
            self.updates.push_back(Presentation::Snapshot(view));
        }
        for source in ["client.daemons", "client.sessions", "client.presentations"] {
            self.updates.push_back(Presentation::Candidates {
                source: source.into(),
                items: self.choices(source),
                truncated: false,
            });
        }
    }

    async fn local(&mut self, intent: &Intent) -> Option<Result<(), String>> {
        let Intent::Command { name, args } = intent else {
            return None;
        };
        let value = |key: &str| {
            args.get(key)
                .and_then(misa_value::Value::as_str)
                .unwrap_or("")
        };
        let mut forgotten = None;
        if matches!(
            name.as_str(),
            "overview" | "daemon" | "daemons" | "visit" | "session" | "detach"
        ) {
            self.selection_generation = self.selection_generation.wrapping_add(1);
        }
        let result = match name.as_str() {
            "new" | "resume" => {
                let daemon = self
                    .selected
                    .as_ref()
                    .and_then(|id| self.connected.get(id))
                    .cloned();
                match daemon {
                    Some(daemon) => {
                        match open_session(daemon, args.clone(), name == "resume").await {
                            Ok(remote) => self.activate(remote),
                            Err(error) => Err(error),
                        }
                    }
                    None => Err("Choose a daemon first".into()),
                }
            }
            "close" => {
                if let Some(remote) = &self.active {
                    let identity = remote.identity();
                    let daemon = self.connected[remote.daemon_identity()].clone();
                    let scope = remote.scope().clone();
                    match close_session(&daemon, &scope).await {
                        Ok(()) => {
                            self.active = None;
                            forgotten = Some(identity);
                            Ok(())
                        }
                        Err(error) => Err(error),
                    }
                } else {
                    Err("Choose a session first".into())
                }
            }
            "overview" => self.park(),
            "detach" => {
                if self
                    .active
                    .as_ref()
                    .is_some_and(|remote| remote.parked_busy())
                {
                    Err("Wait for pending directed work before closing this local session".into())
                } else {
                    forgotten = self.active.take().map(|remote| remote.identity());
                    Ok(())
                }
            }
            "connect" => self.connect(value("target")).await,
            "daemon" => {
                let id = value("daemon");
                if self.connected.contains_key(id) {
                    self.park().map(|()| self.selected = Some(id.into()))
                } else {
                    Err("No connected daemon with that identity".into())
                }
            }
            "attention" => {
                let id=value("daemon");
                let current=self.connected.get(id).and_then(|daemon|daemon.sessions().ok()).filter(|snapshot|matches!(snapshot.status,misa_protocol::observation::Status::Current)).and_then(|snapshot|snapshot.sessions.into_iter().find(|entry|entry.id==value("session") && entry.incarnation==value("incarnation")));
                if current.is_none() {Err("That request owner incarnation is no longer current".into())}
                else if let Ok(generation)=value("generation").parse::<i64>() {
                    self.selected=Some(id.into());
                    match self.attach_entry(self.connected[id].clone(),current.unwrap()).await {
                        Ok(())=>{self.active.as_mut().unwrap().focus_request(value("request").into(),generation);Ok(())},
                        Err(error)=>Err(error),
                    }
                } else {Err("Invalid request generation".into())}
            }
            "visit" => {
                let id = value("daemon");
                if self.connected.contains_key(id) {
                    self.selected = Some(id.into());
                    self.attach(value("session")).await
                } else {
                    Err("No connected daemon with that identity".into())
                }
            }
            "session" => self.attach(value("session")).await,
            "daemons" => self.park(),
            _ => return None,
        };
        if result.is_ok() {
            self.refresh();
            if let Some(id) = forgotten {
                self.updates.push_front(Presentation::Forget(id));
            }
        }
        Some(result)
    }
}

#[async_trait::async_trait]
impl Session for Workspace {
    fn turn_settled(&self) -> Option<bool> {
        self.active.as_ref().and_then(Session::turn_settled)
    }
    fn catalog(&self) -> crate::Catalog { self.declaration() }
    fn location(&self) -> String { self.active.as_ref().map(Session::location).unwrap_or_else(|| "Daemons".into()) }
    fn selected(&self) -> Option<misa_proto::directory::Entry> { self.active.as_ref().and_then(Session::selected) }
    async fn next_presentation(&mut self) -> Result<Option<Presentation>, String> {
        loop {
            if let Some(update) = self.updates.pop_front() {
                return Ok(Some(update));
            }
            tokio::select! {
                update = async { match self.active.as_mut() { Some(remote) => remote.next_presentation().await, None => std::future::pending().await } } => return match update? {
                    Some(Presentation::Declaration{..}) => Ok(Some(Presentation::Declaration {catalog:self.declaration(),location:self.location()})), update => Ok(update),
                },
                changed = self.directory_changes.changed() => {
                    changed.map_err(|_| "Directory updates closed")?;
                    for source in ["client.daemons", "client.sessions", "client.presentations"] { self.updates.push_back(Presentation::Candidates { source: source.into(), items: self.choices(source), truncated: false }); }
                    if self.active.is_none() { self.refresh(); self.updates.pop_front(); }
                },
                result = self.local_pending.join_next(), if !self.local_pending.is_empty() => {
                    match result.unwrap().map_err(|error| error.to_string()).and_then(|result| result) {
                        Ok(LocalChange::Connected(daemon)) => {
                            if self.selected.is_none() { self.selected = Some(daemon.identity().into()); }
                            self.remember(daemon);
                            self.directory_signal.send_modify(|sequence| *sequence = sequence.wrapping_add(1));
                        },
                        Ok(LocalChange::Completion(generation,source,prefix,result))=>{if generation==self.selection_generation {self.updates.push_back(Presentation::Reply(SessionReply::Complete{source,prefix,result}));}},
                        Ok(LocalChange::Closed(identity))=>{if self.active.as_ref().is_some_and(|remote|remote.identity()==identity){self.active=None;}self.parked.remove(&identity);self.refresh();self.updates.push_front(Presentation::Forget(identity));self.updates.push_back(Presentation::Reply(SessionReply::Notice("Daemon session closed; conversation remains stored".into())));},
                        Ok(LocalChange::Reply(reply))=>self.updates.push_back(Presentation::Reply(reply)),
                        Ok(LocalChange::Opened(generation,remote))=>{let id=remote.selected().map(|entry|entry.id).unwrap_or_default();if generation==self.selection_generation {if let Err(error)=self.activate(remote){self.updates.push_back(Presentation::Reply(SessionReply::Notice(error)));}else{self.refresh();}}self.updates.push_back(Presentation::Reply(SessionReply::Notice(format!("Session opened: {id}"))));},
                        Ok(LocalChange::Attached(generation, remote)) if generation == self.selection_generation => { if let Err(error)=self.activate(remote) {self.updates.push_back(Presentation::Reply(SessionReply::Notice(error)));} else {self.refresh();} },
                        Ok(_) => {},
                        Err(error) => self.updates.push_back(Presentation::Reply(SessionReply::Notice(error))),
                    }
                },
            }
        }
    }
    async fn next(&mut self) -> Result<Option<Node>, String> {
        match self.active.as_mut() {
            Some(remote) => remote.next().await,
            None => Err("Choose a daemon session first".into()),
        }
    }
    async fn request(&mut self, request: SessionRequest) -> Option<SessionReply> {
        if let SessionRequest::DaemonInvoke{daemon,scope,command,input}=&request {
            let Some(owner)=self.connected.get(daemon).cloned() else{return Some(SessionReply::Notice("Daemon disconnected".into()));};
            if self.local_pending.len()>=8{return Some(SessionReply::Notice("Daemon requests are busy".into()));}
            let (scope,command,input)=(scope.clone(),command.clone(),input.clone());
            self.local_pending.spawn(async move {
                let interface=misa_client::interface::Interface::load(&owner.client,scope.clone()).await.map_err(|f|f.message)?;
                let definition=interface.commands.get(&command).ok_or("Command is no longer installed")?.clone();
                let reply=owner.client.invoke(scope,definition,input,std::time::Duration::from_secs(30)).await.map_err(|f|f.message)?;
                let message=match reply.outcome {
                    misa_proto::invocation::Outcome::Completed{value}=>{if !value.is_null(){return Ok(LocalChange::Reply(SessionReply::Report(misa_client::request::report(&format!("{} · {command}",owner.identity()),&value))));}format!("{command} completed")},
                    misa_proto::invocation::Outcome::Rejected{fault}=>fault.message,
                    misa_proto::invocation::Outcome::Indeterminate{fault}=>format!("Outcome uncertain: {}",fault.message),
                    misa_proto::invocation::Outcome::Accepted{operation}=>{
                        let watch=misa_client::operation::Watch::open(&owner.client,&interface,operation.clone(),false).await.unwrap_or_else(|fault|misa_client::operation::Watch::failed(operation,fault));
                        match watch.wait().await.outcome {misa_client::operation::Terminal::Finished{state,..}=>format!("Operation {state}"),misa_client::operation::Terminal::Expired=>"Operation expired; result unknown".into(),misa_client::operation::Terminal::Fault(fault)=>fault.message}
                    }
                };
                Ok(LocalChange::Reply(SessionReply::Notice(message)))
            });return None;
        }
        if let SessionRequest::Intent(Intent::Command{name,args})=&request && matches!(name.as_str(),"daemon-commands"|"daemon-command"|"work-command") {
            if self.local_pending.len()>=8{return Some(SessionReply::Notice("Daemon requests are busy".into()));}
            let Some(owner)=args.get("daemon").and_then(misa_value::Value::as_str).or(self.selected.as_deref()).and_then(|id|self.connected.get(id)).cloned() else{return Some(SessionReply::Notice("Choose a daemon first".into()));};
            let command=args.get("command").and_then(misa_value::Value::as_str).map(str::to_owned);
            let work=args.get("operation").and_then(misa_value::Value::as_str).map(str::to_owned);
            let incarnation=args.get("incarnation").and_then(misa_value::Value::as_str).map(str::to_owned);
            self.local_pending.spawn(async move{
                let mut scope=owner.client.welcome().scope;
                if let Some(incarnation)=incarnation{scope.incarnation=incarnation;}
                let interface=misa_client::interface::Interface::load(&owner.client,scope).await.map_err(|f|f.message)?;
                let drafts=if let Some(id)=work {
                    let detail=misa_client::operation::detail(&owner.client,&interface,&id).await.map_err(|f|f.message)?.ok_or("Work result expired")?;
                    std::collections::BTreeMap::from([("operation".into(),id),("generation".into(),detail.generation.to_string())])
                }else{Default::default()};
                let reply=if let Some(command)=command {SessionReply::DaemonForm{daemon:owner.identity().into(),scope:interface.scope.clone(),form:misa_client::form::Form::command(&interface,&command).map_err(|f|f.message)?,drafts}}
                else {SessionReply::Report(Node::section("daemon-commands").children(interface.commands.keys().map(|id|Node::text("command",[misa_proto::view::Span::plain(format!("/daemon-command {}",misa_kit::intent::quote(id)))]).id(id))))};
                Ok(LocalChange::Reply(reply))
            });return None;
        }
        if let SessionRequest::Intent(Intent::Command { name, args }) = &request
            && matches!(name.as_str(), "new" | "resume" | "close")
        {
            if self.local_pending.len() >= 8 {
                return Some(SessionReply::Notice(
                    "Too many pending workspace requests".into(),
                ));
            }
            if name == "close" {
                let Some(remote) = &self.active else {
                    return Some(SessionReply::Notice("Choose a session first".into()));
                };
                let daemon = self.connected[remote.daemon_identity()].clone();
                let scope = remote.scope().clone();
                let identity = remote.identity();
                self.local_pending.spawn(async move {
                    close_session(&daemon, &scope).await?;
                    Ok(LocalChange::Closed(identity))
                });
            } else {
                let Some(daemon) = self
                    .selected
                    .as_ref()
                    .and_then(|id| self.connected.get(id))
                    .cloned()
                else {
                    return Some(SessionReply::Notice("Choose a daemon first".into()));
                };
                if self.active.is_some() && self.parked.len() >= 8 {
                    return Some(SessionReply::Notice(
                        "Detach a local session before opening another".into(),
                    ));
                }
                let args = args.clone();
                let resume = name == "resume";
                self.selection_generation = self.selection_generation.wrapping_add(1);
                let generation = self.selection_generation;
                self.local_pending.spawn(async move {
                    Ok(LocalChange::Opened(
                        generation,
                        open_session(daemon, args, resume).await?,
                    ))
                });
            }
            return None;
        }

        if let SessionRequest::Intent(ref intent) = request {
            if let Intent::Command { name, args } = intent {
                if name == "connect" || name == "session" || name == "visit" {
                    if self.local_pending.len() >= 8 {
                        return Some(SessionReply::Notice(
                            "Too many pending workspace requests".into(),
                        ));
                    }
                    if name == "connect" {
                        let target = args
                            .get("target")
                            .and_then(misa_value::Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let daemons = self.daemons.clone();
                        self.local_pending.spawn(async move {
                            daemons
                                .connect_target(&target)
                                .await
                                .map(|(daemon, _)| LocalChange::Connected(daemon))
                                .map_err(|fault| fault.message)
                        });
                    } else {
                        if name == "visit" {
                            let id = args
                                .get("daemon")
                                .and_then(misa_value::Value::as_str)
                                .unwrap_or("");
                            if !self.connected.contains_key(id) {
                                return Some(SessionReply::Notice(
                                    "No connected daemon with that identity".into(),
                                ));
                            }
                            self.selected = Some(id.into());
                        }
                        let Some(daemon) = self
                            .selected
                            .as_ref()
                            .and_then(|id| self.connected.get(id))
                            .cloned()
                        else {
                            return Some(SessionReply::Notice("Choose a daemon first".into()));
                        };
                        let id = args
                            .get("session")
                            .and_then(misa_value::Value::as_str)
                            .unwrap_or("");
                        let entry = daemon.sessions().ok().and_then(|snapshot| {
                            snapshot.sessions.into_iter().find(|entry| entry.id == id)
                        });
                        let Some(entry) = entry else {
                            return Some(SessionReply::Notice(
                                "Session is not in the daemon directory".into(),
                            ));
                        };
                        let key = format!("{}:{:?}", daemon.identity(), entry.scope());
                        if let Some(remote) = self.parked.remove(&key) {
                            let result = self.activate(remote);
                            if result.is_ok() {
                                self.refresh();
                            }
                            return Some(SessionReply::Sent {
                                draft: None,
                                result,
                            });
                        }
                        if self.parked.len() >= 8 {
                            return Some(SessionReply::Notice("Eight sessions are already retained; close a workspace before adding another".into()));
                        }
                        self.selection_generation = self.selection_generation.wrapping_add(1);
                        let generation = self.selection_generation;
                        self.local_pending.spawn(async move {
                            crate::scoped_remote::ScopedRemote::on(daemon, entry)
                                .await
                                .map(|remote| LocalChange::Attached(generation, remote))
                        });
                    }
                    return None;
                }
                self.selection_generation = self.selection_generation.wrapping_add(1);
            }
            if let Some(result) = self.local(intent).await {
                return Some(SessionReply::Sent {
                    draft: None,
                    result,
                });
            }
        }
        if let SessionRequest::Complete { source, prefix } = &request {
            if source == "client.conversations" {
                if self.local_pending.len() >= 8 {
                    return Some(SessionReply::Complete {
                        source: source.clone(),
                        prefix: prefix.clone(),
                        result: Err("Too many pending workspace requests".into()),
                    });
                }
                let Some(daemon) = self
                    .selected
                    .as_ref()
                    .and_then(|id| self.connected.get(id))
                    .cloned()
                else {
                    return Some(SessionReply::Complete {
                        source: source.clone(),
                        prefix: prefix.clone(),
                        result: Err("Choose a daemon first".into()),
                    });
                };
                let generation = self.selection_generation;
                let source = source.clone();
                let prefix = prefix.clone();
                self.local_pending.spawn(async move {
                    let result = misa_client::lifecycle::conversations(&daemon, &prefix, 100)
                        .await
                        .map(|value| (value.items, value.truncated))
                        .map_err(|fault| fault.message);
                    Ok(LocalChange::Completion(generation, source, prefix, result))
                });
                return None;
            }

            if source.starts_with("client.") {
                return Some(SessionReply::Complete {
                    source: source.clone(),
                    prefix: prefix.clone(),
                    result: Ok((self.choices(source), false)),
                });
            }
        }
        match self.active.as_mut() {
            Some(remote) => remote.request(request).await,
            None => Some(match request {
                SessionRequest::Intent(
                    Intent::Prompt { text, attachments } | Intent::Interrupt { text, attachments },
                ) => SessionReply::Sent {
                    draft: Some((text, attachments)),
                    result: Err("Choose a session with /session first".into()),
                },
                _ => SessionReply::Notice("Choose a session with /session first".into()),
            }),
        }
    }
    async fn send(&mut self, intent: Intent) -> Result<(), String> {
        if let Some(result) = self.local(&intent).await {
            return result;
        }
        self.active
            .as_mut()
            .ok_or("Choose a session first")?
            .send(intent)
            .await
    }
    async fn complete(
        &mut self,
        source: &str,
        prefix: &str,
    ) -> Result<(Vec<Choice>, bool), String> {
        if source.starts_with("client.") {
            return Ok((self.choices(source), false));
        }
        self.active
            .as_mut()
            .ok_or("Choose a session first")?
            .complete(source, prefix)
            .await
    }
    async fn upload(
        &mut self,
        bytes: Vec<u8>,
        media: &str,
    ) -> Result<misa_proto::view::BlobRef, String> {
        self.active
            .as_mut()
            .ok_or("Choose a session first")?
            .upload(bytes, media)
            .await
    }
    async fn save_attachment(&mut self, node: &str, destination: &str) -> Result<(), String> {
        self.active
            .as_mut()
            .ok_or("Choose a session first")?
            .save_attachment(node, destination)
            .await
    }
}
async fn monitor_overview(
    daemon: std::sync::Arc<misa_client::daemons::Daemon>,
    snapshots: std::sync::Arc<
        std::sync::Mutex<
            std::collections::BTreeMap<String, Result<misa_client::overview::Snapshot, String>>,
        >,
    >,
    signal: tokio::sync::watch::Sender<u64>,
) {
    let mut directory = daemon.watch();
    loop {
        let mut overview = match misa_client::overview::Overview::open(&daemon.client).await {
            Ok(overview) => overview,
            Err(fault) => {
                snapshots
                    .lock()
                    .unwrap()
                    .insert(daemon.identity().into(), Err(fault.message));
                signal.send_modify(|v| *v = v.wrapping_add(1));
                if directory.changed().await.is_err() {
                    return;
                }
                continue;
            }
        };
        loop {
            let snapshot = overview.snapshot().map_err(|fault| fault.message);
            snapshots
                .lock()
                .unwrap()
                .insert(daemon.identity().into(), snapshot);
            signal.send_modify(|v| *v = v.wrapping_add(1));
            if overview.changed().await.is_err() {
                return;
            }
        }
    }
}
fn overview_rows(daemon: &str, snapshot: &misa_client::overview::Snapshot) -> Vec<Node> {
    use misa_proto::{observation::ScopeId, view::Span};
    let mut nodes = vec![];
    let text =
        |id: String, role: &str, value: String| Node::text(role, [Span::plain(value)]).id(id);
    if !matches!(snapshot.status, misa_protocol::observation::Status::Current) {
        nodes.push(text(
            format!("freshness-{daemon}"),
            "notice",
            format!("Directory {:?}", snapshot.status),
        ));
    }
    for entry in &snapshot.sessions {
        let row = snapshot.rows.iter().find(|row| row.scope == entry.scope());
        let detail = row
            .map(|row| {
                format!(
                    "{} · {} pending · {} blocking · {} own / {} total tokens · ${:.4} own / ${:.4} total{}{}",
                    if row.working { "working" } else { "idle" },
                    row.attention,
                    row.blocking.len(),
                    row.direct_usage.input_tokens.saturating_add(row.direct_usage.output_tokens),
                    row.inclusive_usage.input_tokens.saturating_add(row.inclusive_usage.output_tokens),
                    row.direct_usage.cost_micros as f64 / 1_000_000.0,
                    row.inclusive_usage.cost_micros as f64 / 1_000_000.0,
                    if row.availability != misa_proto::directory::Availability::Current {
                        " · stale"
                    } else {
                        ""
                    },
                    if row.cyclic {
                        " · cyclic relationship"
                    } else {
                        ""
                    }
                )
            })
            .unwrap_or_else(|| format!("{:?}", entry.availability));
        nodes.push(text(
            format!("session-{daemon}-{}", entry.id),
            "status",
            format!("{} · {} · {detail}", entry.id, entry.title),
        ));
        if let Some(row) = row {
            for request in &row.requests {
                let ScopeId::Session { id } = &request.scope.id else {
                    continue;
                };
                let kind = request
                    .request
                    .get("kind")
                    .and_then(misa_value::Value::as_str)
                    .unwrap_or("input");
                nodes.push(text(
                    format!("request-{daemon}-{}-{}", entry.id, nodes.len()),
                    "notice",
                    format!("  {kind} in {id} · /attention {} {} {} {} {}",misa_kit::intent::quote(daemon),misa_kit::intent::quote(id),misa_kit::intent::quote(&request.scope.incarnation),misa_kit::intent::quote(request.request.get("id").and_then(misa_value::Value::as_str).unwrap_or("unknown")),request.request.get("generation").and_then(misa_value::Value::as_i64).unwrap_or(0)),
                ));
            }
        }
    }
    for work in &snapshot.work {
        let parent = match &work.parent.id {
            ScopeId::Session { id } => id.as_str(),
            ScopeId::Daemon => "daemon",
        };
        let child = work
            .child
            .as_ref()
            .and_then(|scope| match &scope.id {
                ScopeId::Session { id } => Some(id.as_str()),
                _ => None,
            })
            .unwrap_or("unavailable");
        nodes.push(text(
            format!("work-{daemon}-{}", work.id),
            "status",
            format!(
                "  {parent} → {child} · {} · {}{}",
                work.state,
                work.lifetime,
                if work.blocking {
                    " · blocks parent"
                } else {
                    ""
                }
            ),
        ));
    }
    for work in &snapshot.work {
        nodes.push(text(format!("work-actions-{daemon}-{}",work.id),"notice",format!("  /work-command {} {} operation.cancel {} · /work-command {} {} daemon.work.forget {}",misa_kit::intent::quote(daemon),misa_kit::intent::quote(&snapshot.scope.incarnation),misa_kit::intent::quote(&work.id),misa_kit::intent::quote(daemon),misa_kit::intent::quote(&snapshot.scope.incarnation),misa_kit::intent::quote(&work.id))));
    }
    for (id, fault) in &snapshot.unavailable {
        nodes.push(text(
            format!("unavailable-{daemon}-{id}"),
            "notice",
            format!("{id}: {}", fault.message),
        ));
    }
    nodes
}
#[cfg(test)]
mod overview_tests {
    use super::*;
    use misa_client::overview::{Attention, Row, Snapshot, Usage};
    use misa_proto::{
        directory::{Availability, Entry},
        observation::{Scope, ScopeId},
    };
    use misa_value::Value;
    #[test]
    fn summary_rows_keep_staleness_usage_and_cross_session_attention_visible() {
        let scope = Scope {
            id: ScopeId::Session {
                id: "parent".into(),
            },
            incarnation: "p".into(),
        };
        let child = Scope {
            id: ScopeId::Session { id: "child".into() },
            incarnation: "c".into(),
        };
        let snapshot = Snapshot {
            scope: Scope {
                id: ScopeId::Daemon,
                incarnation: "d".into(),
            },
            status: misa_protocol::observation::Status::Current,
            position: Some(3),
            sessions: vec![Entry {
                id: "parent".into(),
                incarnation: "p".into(),
                title: "Parent".into(),
                availability: Availability::Current,
                source_position: 2,
                summary: Value::Null,
            }],
            rows: vec![Row {
                scope,
                source_position: 2,
                availability: Availability::Stale,
                direct_working: false,
                working: true,
                direct_attention: 0,
                attention: 1,
                requests: vec![Attention {
                    scope: child,
                    source_position: 1,
                    availability: Availability::Current,
                    request: Value::map([("kind", Value::str("tool_approval")),("id",Value::str("request-1")),("generation",Value::Int(7))]),
                }],
                blocking: vec!["delegation".into()],
                unavailable: vec![],
                cyclic: true,
                direct_usage: Usage {
                    input_tokens: 2,
                    output_tokens: 1,
                    cost_micros: 10,
                },
                inclusive_usage: Usage {
                    input_tokens: 20,
                    output_tokens: 10,
                    cost_micros: 1000,
                },
            }],
            work: vec![],
            unavailable: Default::default(),
        };
        let text = overview_rows("daemon-a", &snapshot)
            .iter()
            .flat_map(|node| misa_render::render(node, &misa_render::Theme::plain(), 200))
            .map(|line| line.text())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("working · 1 pending · 1 blocking · 3 own / 30 total tokens"));
        assert!(text.contains("stale"));
        assert!(text.contains("cyclic"));
        assert!(text.contains("/attention daemon-a child c request-1 7"));
    }
}
async fn lifecycle(
    daemon: &misa_client::daemons::Daemon,
    command: &str,
    input: misa_value::Value,
) -> Result<misa_value::Value, String> {
    use misa_proto::invocation::Outcome;
    match misa_client::lifecycle::invoke(daemon, command, input)
        .await
        .map_err(|fault| fault.message)?
    {
        Outcome::Completed { value } => Ok(value),
        Outcome::Rejected { fault } => Err(fault.message),
        Outcome::Indeterminate { fault } => Err(format!(
            "Outcome uncertain; reconcile before retrying: {}",
            fault.message
        )),
        Outcome::Accepted { operation } => Err(format!(
            "Operation {} accepted; reconcile before retrying",
            operation.id
        )),
    }
}
async fn open_session(
    daemon: std::sync::Arc<misa_client::daemons::Daemon>,
    input: misa_value::Value,
    resume: bool,
) -> Result<crate::scoped_remote::ScopedRemote, String> {
    let value = lifecycle(
        &daemon,
        if resume {
            "daemon.session.resume"
        } else {
            "daemon.session.create"
        },
        input,
    )
    .await?;
    let entry = misa_client::lifecycle::opened(&daemon, &value)
        .await
        .map_err(|fault| fault.message)?;
    crate::scoped_remote::ScopedRemote::on(daemon, entry).await
}
async fn close_session(
    daemon: &misa_client::daemons::Daemon,
    scope: &misa_proto::observation::Scope,
) -> Result<(), String> {
    lifecycle(
        daemon,
        "daemon.session.close",
        misa_client::lifecycle::close_input(scope).map_err(|fault| fault.message)?,
    )
    .await?;
    Ok(())
}
