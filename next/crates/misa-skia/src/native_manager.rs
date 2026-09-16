//! Relationships outlive selection. Bounded local instances retain their tasks
//! and outcomes; generations distinguish a closed instance from its replacement.
use super::*;
type Choice = (Arc<Daemon>, misa_proto::directory::Entry);
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
                        let mut changed=daemon.watch();let dirty=dirty.clone();
                        let task=watchers.spawn(async move{while changed.changed().await.is_ok(){dirty.notify_one();}});
                        watching.insert(daemon.identity().to_string(),task);
                    }
                    let snapshot=daemon.sessions().map_err(|fault|fault.message)?;
                    let freshness=match snapshot.status{Status::Current=>"current",Status::Awaiting=>"loading",Status::Closed(_)=>"closed",_=>"stale"};
                    rows.push(crate::workspace::DaemonChoice{identity:daemon.identity().into(),freshness:freshness.into(),sessions:snapshot.sessions,instances:Default::default()});
                }
                for ((identity, scope), instance) in &instances {
                    let index = rows.iter().position(|row| &row.identity == identity).unwrap_or_else(|| {
                        rows.push(crate::workspace::DaemonChoice { identity: identity.clone(), freshness: "disconnected · drafts retained".into(), sessions: vec![], instances:Default::default() });
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
