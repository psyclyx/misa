//! Relationships outlive selection. Bounded local instances retain their tasks
//! and outcomes; generations distinguish a closed instance from its replacement.
use super::*;
type Choice = (Arc<Daemon>, misa_proto::directory::Entry);
#[derive(Default, Clone)]
struct Facts {
    overview: Option<Result<misa_client::overview::Snapshot, String>>,
    forms: BTreeMap<String, misa_client::form::Form>,
    archive: Vec<misa_proto::view::Choice>,
    truncated: bool,
}
enum Job {
    Form(String,misa_client::form::Form,BTreeMap<String,String>),
    Completed(String),
    Opened(u64, Choice),
    Closed(String, misa_proto::observation::Scope),
    Archive(String, u64, misa_proto::preparation::Candidates),
}
struct Instance {
    generation: u64,
    commands: tokio::sync::mpsc::Sender<Command>,
    task: tokio::task::AbortHandle,
    entry: misa_proto::directory::Entry,
}
async fn connect(
    registry: Arc<Daemons>,
    target: String,
    select: bool,
) -> Result<Option<Choice>, String> {
    let (daemon, hint) = registry
        .connect_target(&target)
        .await
        .map_err(|fault| fault.message)?;
    if !select {
        return Ok(None);
    }
    let mut changed = daemon.watch();
    let entry = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let snapshot = daemon.sessions().map_err(|fault| fault.message)?;
            if matches!(snapshot.status, Status::Current) {
                return snapshot
                    .sessions
                    .into_iter()
                    .find(|entry| hint.as_ref().is_none_or(|hint| &entry.id == hint))
                    .ok_or("The daemon has no matching session".to_string());
            }
            changed
                .changed()
                .await
                .map_err(|_| "Daemon directory closed".to_string())?;
        }
    })
    .await
    .map_err(|_| "Daemon directory timed out".to_string())??;
    Ok(Some((daemon, entry)))
}
async fn discover(registry: Arc<Daemons>) -> Result<Option<Choice>, String> {
    #[cfg(unix)]
    {
        for result in registry
            .discover_local()
            .await
            .map_err(|fault| fault.message)?
        {
            result.map_err(|fault| fault.message)?;
        }
    }
    Ok(None)
}
pub(super) async fn run(
    ticket: Option<String>,
    proxy: &Sink,
    mut outgoing: tokio::sync::mpsc::Receiver<Command>,
) -> Result<(), String> {
    let key = misa_transport::identity::load(&misa_transport::identity::client_path("misa-skia")?)?;
    let endpoint = misa_transport::iroh::bind(Some(key), true).await?;
    let registry = Arc::new(Daemons::new(
        endpoint,
        misa_proto::ClientInfo::new("misa-skia", env!("CARGO_PKG_VERSION")),
    ));
    let mut connections = tokio::task::JoinSet::new();
    let mut jobs = tokio::task::JoinSet::<Result<Job, String>>::new();
    let facts = Arc::new(Mutex::new(BTreeMap::<String, Facts>::new()));
    let mut archive_versions = BTreeMap::<String, u64>::new();
    let mut navigation = 0u64;
    if let Some(target) = ticket {
        connections.spawn(connect(registry.clone(), target, true));
    } else {
        connections.spawn(discover(registry.clone()));
    }
    let dirty = Arc::new(tokio::sync::Notify::new());
    let mut watchers = tokio::task::JoinSet::new();
    let mut watching = BTreeMap::new();
    let directory = Arc::new(DirectoryDelivery {
        latest: Mutex::new(vec![]),
        queued: AtomicBool::new(false),
    });
    let mut selected = tokio::task::JoinSet::new();
    let mut commands: Option<tokio::sync::mpsc::Sender<Command>> = None;
    let mut instances: BTreeMap<(String, misa_proto::observation::Scope), Instance> =
        BTreeMap::new();
    let mut generation = 0u64;
    let mut select: Option<Choice> = None;
    dirty.notify_one();
    loop {
        if let Some((daemon, entry)) = select.take() {
            let key = (daemon.identity().to_string(), entry.scope());
            if let Some(instance) = instances
                .get(&key)
                .filter(|instance| !instance.commands.is_closed())
            {
                commands = Some(instance.commands.clone());
                proxy.send_event(Update::Selected {
                    generation: instance.generation,
                    daemon: key.0,
                    scope: key.1,
                })?;
                continue;
            }
            if instances.len() >= 8 && !instances.contains_key(&key) {
                proxy.send_event(Update::Notice(
                    "Eight session instances are already open; drafts are retained".into(),
                ))?;
                continue;
            }
            generation = generation
                .checked_add(1)
                .ok_or("Selection generations exhausted")?;
            proxy.send_event(Update::Selected {
                generation,
                daemon: daemon.identity().into(),
                scope: entry.scope(),
            })?;
            let sink = Sink {
                proxy: proxy.proxy.clone(),
                generation: Some(generation),
            };
            let (send, receive) = tokio::sync::mpsc::channel(32);
            commands = Some(send.clone());
            let retained_entry = entry.clone();
            let task = selected.spawn(async move {
                let result = session(daemon, entry, &sink, receive).await;
                if let Err(error) = result {
                    let _ = sink.send_event(Update::Notice(error));
                }
            });
            instances.insert(
                key,
                Instance {
                    generation,
                    commands: send,
                    task,
                    entry: retained_entry,
                },
            );
            dirty.notify_one();
        }
        tokio::select! {
            result=jobs.join_next(),if !jobs.is_empty()=>{
                match result.unwrap().unwrap_or_else(|error|Err(error.to_string())) {
                    Ok(Job::Form(daemon,form,drafts))=>proxy.send_event(Update::DaemonForm{daemon,form,drafts})?,
                    Ok(Job::Completed(message))=>proxy.send_event(Update::Notice(message))?,
                    Ok(Job::Opened(version,choice))=>{proxy.send_event(Update::Notice(format!("Session {} opened",choice.1.id)))?;if version==navigation{select=Some(choice);}},
                    Ok(Job::Closed(identity,scope))=>{if let Some(instance)=instances.get(&(identity,scope)){proxy.send_event(Update::Session{generation:instance.generation,update:Box::new(Update::Notice("Server session stopped; local draft retained".into()))})?;}},
                    Ok(Job::Archive(identity,version,result))=>if archive_versions.get(&identity)==Some(&version){let mut facts=facts.lock().unwrap();let row=facts.entry(identity).or_default();row.archive=result.items;row.truncated=result.truncated;},
                    Err(error)=>proxy.send_event(Update::Notice(error))?,
                }
                dirty.notify_one();
            },
            _=selected.join_next(),if !selected.is_empty()=>{},
            _=watchers.join_next(),if !watchers.is_empty()=>{},
            result=connections.join_next(),if !connections.is_empty()=>{
                match result.expect("pending connection"){Ok(Ok(choice))=>if choice.is_some(){select=choice;},Ok(Err(error))=>proxy.send_event(Update::Notice(error))?,Err(error)=>proxy.send_event(Update::Notice(error.to_string()))?}
                dirty.notify_one();
            },
            _=dirty.notified()=>{
                let connected=registry.connected().await;
                let identities:BTreeSet<_>=connected.iter().map(|daemon|daemon.identity().to_string()).collect();
                watching.retain(|identity,task:&mut tokio::task::AbortHandle|{if identities.contains(identity){true}else{task.abort();false}});
                let mut rows=vec![];
                for daemon in connected{
                    if !watching.contains_key(daemon.identity()){
                        let task=watchers.spawn(observe_daemon(daemon.clone(),facts.clone(),dirty.clone()));
                        watching.insert(daemon.identity().to_string(),task);
                    }
                    let snapshot=daemon.sessions().map_err(|fault|fault.message)?;
                    let freshness=match snapshot.status{Status::Current=>"current",Status::Awaiting=>"loading",Status::Closed(_)=>"closed",_=>"stale"};
                    let facts=facts.lock().unwrap().get(daemon.identity()).cloned().unwrap_or_default();
                    rows.push(crate::workspace::DaemonChoice{identity:daemon.identity().into(),freshness:freshness.into(),sessions:snapshot.sessions,instances:Default::default(),overview:facts.overview,forms:facts.forms,archive:facts.archive,archive_truncated:facts.truncated});
                }
                for ((identity, scope), instance) in &instances {
                    let index = rows.iter().position(|row| &row.identity == identity).unwrap_or_else(|| {
                        rows.push(crate::workspace::DaemonChoice { identity: identity.clone(), freshness: "disconnected · drafts retained".into(), sessions: vec![], instances:Default::default(),overview:None,forms:Default::default(),archive:vec![],archive_truncated:false });
                        rows.len()-1
                    });
                    rows[index].instances.insert(scope.clone());
                    if !rows[index].sessions.iter().any(|entry| &entry.scope()==scope) {
                        let mut entry = instance.entry.clone();
                        entry.title.push_str(" · retained locally");
                        entry.availability = misa_proto::directory::Availability::Unavailable;
                        rows[index].sessions.push(entry);
                    }
                }
                *directory.latest.lock().unwrap()=rows;
                if !directory.queued.swap(true,Ordering::AcqRel){proxy.send_event(Update::Directory(directory.clone()))?;}
            },
            command=outgoing.recv()=>{
                let Some(command)=command else{for daemon in registry.connected().await{registry.disconnect(daemon.identity()).await;}return Ok(());};
                match command{
                    Command::PrepareWork{daemon:identity,scope,id,command}=>{
                        if jobs.len()>=8{proxy.send_event(Update::Notice("Daemon requests are busy".into()))?;continue;}
                        let Some(daemon)=registry.connected().await.into_iter().find(|daemon|daemon.identity()==identity) else{continue;};
                        jobs.spawn(async move{
                            let interface=Interface::load(&daemon.client,scope).await.map_err(|f|f.message)?;
                            let detail=misa_client::operation::detail(&daemon.client,&interface,&id).await.map_err(|f|f.message)?.ok_or("Work result expired")?;
                            let form=misa_client::form::Form::command(&interface,&command).map_err(|f|f.message)?;
                            Ok(Job::Form(identity,form,BTreeMap::from([("id".into(),id),("generation".into(),detail.generation.to_string())])))
                        });
                    },
                    Command::Archive{daemon:identity,prefix}=>{
                        if jobs.len()>=8{proxy.send_event(Update::Notice("Daemon requests are busy".into()))?;continue;}
                        let Some(daemon)=registry.connected().await.into_iter().find(|daemon|daemon.identity()==identity) else{proxy.send_event(Update::Notice("Daemon disconnected".into()))?;continue;};
                        let version=archive_versions.entry(identity.clone()).or_default();*version=version.wrapping_add(1);let version=*version;
                        jobs.spawn(async move{Ok(Job::Archive(identity,version,misa_client::lifecycle::conversations(&daemon,&prefix,100).await.map_err(|fault|fault.message)?))});
                    },
                    Command::DaemonInvoke{daemon:identity,scope,command,input}=>{
                        if jobs.len()>=8{proxy.send_event(Update::Notice("Daemon requests are busy".into()))?;continue;}
                        let Some(daemon)=registry.connected().await.into_iter().find(|daemon|daemon.identity()==identity) else{proxy.send_event(Update::Notice("Daemon disconnected".into()))?;continue;};
                        if matches!(command.as_str(),"daemon.session.create"|"daemon.session.resume") && instances.len()>=8 {proxy.send_event(Update::Notice("Close a local instance before opening another; its server work is separate".into()))?;continue;}
                        navigation=navigation.wrapping_add(1);let version=navigation;
                        jobs.spawn(async move{invoke_daemon(daemon,scope.ok_or("Daemon owner is not current")?,command,input,version).await});
                    },
                    Command::Connect(target)=>{if connections.len()<4{connections.spawn(connect(registry.clone(),target,false));}else{proxy.send_event(Update::Notice("Connection attempts are busy".into()))?;}},
                    Command::Discover=>{if connections.len()<4{connections.spawn(discover(registry.clone()));}},
                    Command::CloseInstance { daemon, scope } => {
                        if let Some(instance) = instances.remove(&(daemon.clone(), scope.clone())) { instance.task.abort(); }
                        proxy.send_event(Update::ClosedInstance { daemon, scope })?;
                        dirty.notify_one();
                    },
                    Command::Disconnect(identity)=>{
                        registry.disconnect(&identity).await;
                        for ((daemon, _), instance) in &instances {
                            if daemon == &identity {
                                instance.task.abort();
                                proxy.send_event(Update::Session { generation: instance.generation, update: Box::new(Update::Notice("Disconnected; pending command outcomes may be unknown. Remote operations are not cancelled.".into())) })?;
                            }
                        }
                        dirty.notify_one();
                    },
                    Command::Select{daemon:identity,scope}=>{
                        navigation=navigation.wrapping_add(1);
                        for daemon in registry.connected().await{
                            if daemon.identity()!=identity{continue;}
                            if let Some(entry)=daemon.sessions().map_err(|fault|fault.message)?.sessions.into_iter().find(|entry|entry.scope()==scope){select=Some((daemon,entry));break;}
                        }
                        if select.is_none(){
                            if let Some(instance)=instances.get(&(identity.clone(),scope.clone())) {
                                commands=Some(instance.commands.clone());
                                proxy.send_event(Update::Selected {generation:instance.generation,daemon:identity,scope})?;
                            } else {proxy.send_event(Update::Notice("That session incarnation is no longer available".into()))?;}
                        }
                    },
                    command=>{
                        let result=match &commands{Some(sender)=>sender.try_send(command).map_err(|error|error.into_inner()),None=>Err(command)};
                        if let Err(command)=result{proxy.send_event(rejected(command,"Select an available session before sending".into()))?;}
                    }
                }
            }
        }
    }
}
async fn invoke_daemon(
    daemon: Arc<Daemon>,
    scope: misa_proto::observation::Scope,
    command: String,
    input: Value,
    version: u64,
) -> Result<Job, String> {
    let interface=Interface::load(&daemon.client,scope.clone()).await.map_err(|fault|fault.message)?;
    let definition=interface.commands.get(&command).ok_or("Command no longer installed")?.clone();
    let outcome=daemon.client.invoke(scope,definition,input.clone(),Duration::from_secs(30)).await.map_err(|fault|fault.message)?.outcome;
    match outcome {
        Outcome::Completed { value } => {
            if command == "daemon.session.close" {
                let scope = misa_proto::observation::Scope {
                    id: misa_proto::observation::ScopeId::Session {
                        id: input
                            .get("id")
                            .and_then(Value::as_str)
                            .ok_or("Missing session ID")?
                            .into(),
                    },
                    incarnation: input
                        .get("incarnation")
                        .and_then(Value::as_str)
                        .ok_or("Missing incarnation")?
                        .into(),
                };
                Ok(Job::Closed(daemon.identity().into(), scope))
            } else if matches!(command.as_str(), "daemon.session.create" | "daemon.session.resume") {
                let entry = misa_client::lifecycle::opened(&daemon, &value)
                    .await
                    .map_err(|fault| fault.message)?;
                Ok(Job::Opened(version, (daemon, entry)))
            } else { Ok(Job::Completed(format!("{command} completed"))) }
        }
        Outcome::Rejected { fault } => Err(fault.message),
        Outcome::Indeterminate { fault } => Err(format!(
            "Outcome uncertain; reconcile before retrying: {}",
            fault.message
        )),
        Outcome::Accepted { operation } => {
            let watch=misa_client::operation::Watch::open(&daemon.client,&interface,operation.clone(),false).await.unwrap_or_else(|fault|misa_client::operation::Watch::failed(operation,fault));
            let message=match watch.wait().await.outcome {
                misa_client::operation::Terminal::Finished{state,..}=>format!("Operation {state}"),
                misa_client::operation::Terminal::Expired=>"Accepted operation result expired; completion unknown".into(),
                misa_client::operation::Terminal::Fault(fault)=>format!("Accepted operation monitoring failed: {}",fault.message),
            };
            Ok(Job::Completed(message))
        },
    }
}
async fn observe_daemon(
    daemon: Arc<Daemon>,
    facts: Arc<Mutex<BTreeMap<String, Facts>>>,
    dirty: Arc<tokio::sync::Notify>,
) {
    let mut directory = daemon.watch();
    loop {
        let mut overview = match misa_client::overview::Overview::open(&daemon.client).await {
            Ok(overview) => overview,
            Err(fault) => {
                facts
                    .lock()
                    .unwrap()
                    .entry(daemon.identity().into())
                    .or_default()
                    .overview = Some(Err(fault.message));
                dirty.notify_one();
                if directory.changed().await.is_err() {
                    return;
                }
                continue;
            }
        };
        let mut catalog_scope = None;
        loop {
            let snapshot = overview.snapshot().map_err(|fault| fault.message);
            if let Ok(snapshot) = &snapshot
                && matches!(snapshot.status, Status::Current)
                && catalog_scope.as_ref() != Some(&snapshot.scope)
            {
                if let Ok(interface) = Interface::load(&daemon.client, snapshot.scope.clone()).await
                {
                    let forms = interface.commands.keys().map(String::as_str)
                        .filter_map(|id| {
                            misa_client::form::Form::command(&interface, id)
                                .ok()
                                .map(|form| (id.into(), form))
                        })
                        .collect();
                    facts
                        .lock()
                        .unwrap()
                        .entry(daemon.identity().into())
                        .or_default()
                        .forms = forms;
                    catalog_scope = Some(snapshot.scope.clone());
                }
            }
            facts
                .lock()
                .unwrap()
                .entry(daemon.identity().into())
                .or_default()
                .overview = Some(snapshot);
            dirty.notify_one();
            tokio::select! {result=overview.changed()=>if result.is_err(){return;},result=directory.changed()=>if result.is_err(){return;}}
        }
    }
}
