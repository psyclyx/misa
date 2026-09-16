//! Application relationships and bounded local instances, independent of selection.
use crate::{delivery::Mailbox, files::Files};
use misa_client::daemons::{Daemon, Daemons};
use misa_proto::{directory::Entry, observation::Scope};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

#[derive(Clone)]
pub struct Events {
    pub mailbox: Arc<Mailbox>,
    pub instance: String,
}
impl Events {
    pub async fn send(&self, mut event: Value) {
        event["instance"] = json!(self.instance);
        self.mailbox.event(event).await;
    }
    pub async fn notice(&self, text: impl Into<String>) {
        self.send(json!({"kind":"notice","level":"error","text":text.into()}))
            .await;
    }
}
struct Instance {
    daemon: String,
    entry: Entry,
    commands: tokio::sync::mpsc::Sender<Value>,
    task: tokio::task::AbortHandle,
}
pub async fn run(
    ticket: String,
    storage: PathBuf,
    mut inbox: tokio::sync::mpsc::Receiver<Value>,
    mailbox: Arc<Mailbox>,
) {
    if let Err(message) = workspace(ticket, storage, &mut inbox, mailbox.clone()).await {
        mailbox
            .event(json!({"kind":"fault","message":message}))
            .await;
    }
}
async fn connect(
    registry: Arc<Daemons>,
    target: String,
    select: bool,
) -> Result<(Arc<Daemon>, Option<String>, String), String> {
    let safe = misa_proto::Pairing::given(&target)
        .map(|(ticket, _)| ticket.to_string())
        .unwrap_or_else(|_| target.clone());
    let (daemon, hint) = registry
        .connect_target(&target)
        .await
        .map_err(|fault| fault.message)?;
    Ok((daemon, if select { hint } else { None }, safe))
}
async fn workspace(
    ticket: String,
    storage: PathBuf,
    inbox: &mut tokio::sync::mpsc::Receiver<Value>,
    mailbox: Arc<Mailbox>,
) -> Result<(), String> {
    let files = Arc::new(
        tokio::task::spawn_blocking(move || Files::new(storage))
            .await
            .map_err(|error| error.to_string())??,
    );
    if let Some(cached) = crate::snapshot::cached(files.clone()).await {
        mailbox.event(cached).await;
    }
    let identity = {
        let files = files.clone();
        tokio::task::spawn_blocking(move || files.identity())
            .await
            .map_err(|error| error.to_string())??
    };
    let endpoint = misa_transport::iroh::bind(Some(identity), !ticket.contains('@')).await?;
    let registry = Arc::new(Daemons::new(
        endpoint,
        misa_proto::ClientInfo::new("misa-android", env!("CARGO_PKG_VERSION")),
    ));
    let jobs = Arc::new(tokio::sync::Semaphore::new(16));
    let mut connections = tokio::task::JoinSet::new();
    let mut lifecycle = tokio::task::JoinSet::new();
    let mut archives = tokio::task::JoinSet::new();
    let mut overview_tasks = tokio::task::JoinSet::<(
        String,
        Scope,
        Result<misa_client::overview::Overview, misa_proto::Fault>,
    )>::new();
    let mut overview_attempts = BTreeMap::new();
    let mut overviews = BTreeMap::<String, Arc<misa_client::overview::Overview>>::new();
    let mut overview_watching = BTreeMap::<String, tokio::task::AbortHandle>::new();
    let mut watchers = tokio::task::JoinSet::new();
    let dirty = Arc::new(tokio::sync::Notify::new());
    let mut watching = BTreeMap::new();
    let mut tasks = tokio::task::JoinSet::new();
    let mut instances = BTreeMap::<String, Instance>::new();
    let mut selected: Option<String> = None;
    let mut next = 1u64;
    let mut initial: BTreeMap<String, String> = BTreeMap::new();
    let mut choose: Option<(Arc<Daemon>, Entry)> = None;
    if !ticket.is_empty() {
        connections.spawn(connect(registry.clone(), ticket, true));
    }
    mailbox.event(json!({"kind":"ready"})).await;
    dirty.notify_one();
    loop {
        if let Some((daemon, entry)) = choose.take() {
            let existing = instances
                .iter()
                .find(|(_, instance)| {
                    instance.daemon == daemon.identity()
                        && instance.entry.scope() == entry.scope()
                        && !instance.commands.is_closed()
                })
                .map(|(id, _)| id.clone());
            let id = if let Some(id) = existing {
                id
            } else {
                if instances.len() >= 8 {
                    mailbox.event(json!({"kind":"notice","level":"error","text":"Eight session instances are open; close an instance before opening another"})).await;
                    continue;
                }
                let id = next.to_string();
                next = next.checked_add(1).ok_or("Instance identities exhausted")?;
                mailbox.event(json!({"kind":"instance","instance":id,"daemon":daemon.identity(),"scope":entry.scope(),"title":entry.title})).await;
                let (send, receive) = tokio::sync::mpsc::channel(32);
                let events = Events {
                    mailbox: mailbox.clone(),
                    instance: id.clone(),
                };
                let owner = daemon.clone();
                let source = entry.clone();
                let storage = files.clone();
                let limits = jobs.clone();
                let task=tasks.spawn(async move { if let Err(error)=crate::session::run(owner,source,storage,events.clone(),receive,limits).await { events.notice(error).await;events.send(json!({"kind":"state","state":"closed","message":"Session unavailable; retained content is stale"})).await; } });
                instances.insert(
                    id.clone(),
                    Instance {
                        daemon: daemon.identity().into(),
                        entry: entry.clone(),
                        commands: send,
                        task,
                    },
                );
                id
            };
            selected = Some(id.clone());
            let storage = files.clone();
            let saved = serde_json::to_vec(
                &json!({"daemon":daemon.identity(),"scope":entry.scope(),"title":entry.title}),
            )
            .map_err(|error| error.to_string())?;
            if let Err(error) =
                tokio::task::spawn_blocking(move || storage.write("selected.json", &saved))
                    .await
                    .map_err(|error| error.to_string())?
            {
                mailbox
                    .event(json!({"kind":"notice","level":"error","text":error}))
                    .await;
            }
            mailbox.event(json!({"kind":"selected","instance":id,"daemon":daemon.identity(),"scope":entry.scope(),"title":entry.title})).await;
            dirty.notify_one();
        }
        tokio::select! {
            result=overview_tasks.join_next(),if !overview_tasks.is_empty()=>{
                if let Some(Ok((identity,scope,result)))=result {
                    if registry.connected().await.iter().any(|daemon|daemon.identity()==identity&&daemon.client.welcome().scope==scope) {
                        match result {
                            Ok(overview)=>{
                                if let Some(old)=overview_watching.remove(&identity){old.abort();}
                                let mut changes=overview.watch();let dirty=dirty.clone();
                                overview_watching.insert(identity.clone(),watchers.spawn(async move{while changes.changed().await.is_ok(){dirty.notify_one();}}));
                                overviews.insert(identity,Arc::new(overview));
                            },Err(fault)=>mailbox.event(json!({"kind":"notice","text":format!("Daemon overview unavailable: {}",fault.message)})).await,
                        }
                    }
                    dirty.notify_one();
                }
            },
            result=archives.join_next(),if !archives.is_empty()=>{
                if let Some(Ok((daemon,prefix,result)))=result {match result {Ok(items)=>mailbox.event(json!({"kind":"archives","daemon":daemon,"prefix":prefix,"items":items})).await,Err(text)=>mailbox.event(json!({"kind":"notice","text":text})).await}}
            },
            result=lifecycle.join_next(),if !lifecycle.is_empty()=>{
                match result.expect("pending lifecycle") {Ok(Ok(opened))=>{choose=opened;dirty.notify_one();},Ok(Err(text))=>mailbox.event(json!({"kind":"notice","level":"error","text":text})).await,Err(error)=>mailbox.event(json!({"kind":"notice","level":"error","text":error.to_string()})).await}
            },
            _=tasks.join_next(),if !tasks.is_empty()=>{},
            _=watchers.join_next(),if !watchers.is_empty()=>{},
            result=connections.join_next(),if !connections.is_empty()=>{
                match result.expect("pending connection") {
                    Ok(Ok((daemon,hint,target)))=>{
                        if let Some(hint)=hint { initial.insert(daemon.identity().into(),hint); }
                        mailbox.event(json!({"kind":"relationship","daemon":daemon.identity(),"target":target})).await;
                    }
                    Ok(Err(message))=>mailbox.event(json!({"kind":"notice","level":"error","text":message})).await,
                    Err(error)=>mailbox.event(json!({"kind":"notice","level":"error","text":error.to_string()})).await,
                }
                dirty.notify_one();
            },
            _=dirty.notified()=>{
                let connected=registry.connected().await;
                let identities:BTreeSet<_>=connected.iter().map(|daemon|daemon.identity().to_string()).collect();
                overviews.retain(|identity,_|identities.contains(identity));
                overview_attempts.retain(|identity,_|identities.contains(identity));
                overview_watching.retain(|identity,task|if identities.contains(identity){true}else{task.abort();false});
                watching.retain(|identity,task:&mut tokio::task::AbortHandle|if identities.contains(identity){true}else{task.abort();false});
                let mut rows=vec![];
                for daemon in connected {
                    let scope=daemon.client.welcome().scope;
                    if overview_attempts.get(daemon.identity())!=Some(&scope)&&overview_tasks.len()<4 {
                        let owner=daemon.clone();overview_attempts.insert(daemon.identity().to_string(),scope.clone());
                        overview_tasks.spawn(async move {(owner.identity().to_string(),scope,misa_client::overview::Overview::open(&owner.client).await)});
                    }
                    if !watching.contains_key(daemon.identity()) { let mut changed=daemon.watch();let dirty=dirty.clone();let task=watchers.spawn(async move{while changed.changed().await.is_ok(){dirty.notify_one();}});watching.insert(daemon.identity().to_string(),task); }
                    let snapshot=daemon.sessions().map_err(|fault|fault.message)?;
                    if let Some(hint)=initial.get(daemon.identity()) {
                        if let Some(entry)=snapshot.sessions.iter().find(|entry|&entry.id==hint) { choose=Some((daemon.clone(),entry.clone()));initial.remove(daemon.identity()); }
                    }
                    let overview=overviews.get(daemon.identity()).and_then(|overview|overview.snapshot().ok());
                    let sessions:Vec<_>=snapshot.sessions.iter().map(|entry|{
                        let mut value=json!(entry);
                        if let Some(overview)=&overview {if let Some(row)=overview.rows.iter().find(|row|row.scope==entry.scope()) {
                            value["summary"]=json!(format!("{} · {} · {} request(s) · {} tokens · ${:.4}",crate::session::freshness(&overview.status),if row.working{"working"}else{"idle"},row.attention,row.inclusive_usage.input_tokens.saturating_add(row.inclusive_usage.output_tokens),row.inclusive_usage.cost_micros as f64/1_000_000.0));
                        }}
                        value
                    }).collect();
                    rows.push(json!({"daemon":daemon.identity(),"freshness":crate::session::freshness(&snapshot.status),"sessions":sessions}));
                }
                let retained:Vec<_>=instances.iter().map(|(id,instance)|json!({"instance":id,"daemon":instance.daemon,"scope":instance.entry.scope(),"title":instance.entry.title,"available":!instance.commands.is_closed()})).collect();
                mailbox.event(json!({"kind":"directory","daemons":rows,"instances":retained})).await;
            },
            command=inbox.recv()=>{
                let Some(command)=command else { for daemon in registry.connected().await{registry.disconnect(daemon.identity()).await;}return Ok(()); };
                match command.get("local").and_then(Value::as_str) {
                    Some("archives")=>if archives.len()<4 {
                        if let Some(daemon)=registry.connected().await.into_iter().find(|daemon|Some(daemon.identity())==command["daemon"].as_str()) {
                            let prefix=command["prefix"].as_str().unwrap_or_default().to_string();
                            archives.spawn(async move {let result=misa_client::lifecycle::conversations(&daemon,&prefix,32).await.map_err(|fault|fault.message);(daemon.identity().to_string(),prefix,result)});
                        }
                    },
                    Some("lifecycle")=>{
                        if lifecycle.len()>=4 {mailbox.event(json!({"kind":"notice","text":"Daemon commands are busy"})).await;continue;}
                        if let Some(daemon)=registry.connected().await.into_iter().find(|daemon|Some(daemon.identity())==command["daemon"].as_str()) {
                            lifecycle.spawn(async move {
                                let id=command["command"].as_str().ok_or("Missing daemon command")?;
                                if !matches!(id,"daemon.session.create"|"daemon.session.resume"|"daemon.session.close"){return Err("Unsupported lifecycle action".into());}
                                let input=serde_json::from_value(command["input"].clone()).map_err(|error|error.to_string())?;
                                match misa_client::lifecycle::invoke(&daemon,id,input).await.map_err(|fault|fault.message)? {
                                    misa_proto::invocation::Outcome::Completed{value}=>if id=="daemon.session.close"{Ok(None)}else{let entry=misa_client::lifecycle::opened(&daemon,&value).await.map_err(|fault|fault.message)?;Ok(Some((daemon,entry)))},
                                    misa_proto::invocation::Outcome::Rejected{fault}|misa_proto::invocation::Outcome::Indeterminate{fault}=>Err(fault.message),
                                    misa_proto::invocation::Outcome::Accepted{operation}=>Err(format!("Operation {} accepted; reconcile before retrying",operation.id)),
                                }
                            });
                        }
                    },
                    Some("connect")=>if connections.len()<4 { if let Some(target)=command["target"].as_str(){connections.spawn(connect(registry.clone(),target.into(),command["select"].as_bool().unwrap_or(true)));} },
                    Some("disconnect")=>if let Some(identity)=command["daemon"].as_str(){registry.disconnect(identity).await;for(id,instance)in&instances{if instance.daemon==identity{instance.task.abort();Events{mailbox:mailbox.clone(),instance:id.clone()}.send(json!({"kind":"state","state":"closed","message":"Disconnected; pending outcomes may be unknown. Remote operations remain owned by the daemon."})).await;}}dirty.notify_one();},
                    Some("select")=>{
                        if let Some(id)=command["instance"].as_str().filter(|id|instances.contains_key(*id)) { let instance=&instances[id];selected=Some(id.into());mailbox.event(json!({"kind":"selected","instance":id,"daemon":instance.daemon,"scope":instance.entry.scope(),"title":instance.entry.title})).await; }
                        else if let (Some(identity),Ok(scope))=(command["daemon"].as_str(),serde_json::from_value::<Scope>(command["scope"].clone())) { for daemon in registry.connected().await {if daemon.identity()==identity {if let Some(entry)=daemon.sessions().map_err(|fault|fault.message)?.sessions.into_iter().find(|entry|entry.scope()==scope){choose=Some((daemon,entry));break;}}} }
                    },
                    Some("close")=>if let Some(id)=command["instance"].as_str(){if let Some(instance)=instances.remove(id){instance.task.abort();}if selected.as_deref()==Some(id){selected=None;}mailbox.event(json!({"kind":"instance_closed","instance":id})).await;dirty.notify_one();},
                    _=>{
                        let target=command.get("instance").and_then(Value::as_str).map(str::to_owned).or_else(||selected.clone());
                        let sent=target.as_ref().and_then(|id|instances.get(id)).is_some_and(|instance|instance.commands.try_send(command.clone()).is_ok());
                        if !sent { let mut event=json!({"kind":"rejected","text":command["text"],"message":"Session unavailable or local command queue full"});if let Some(id)=target{event["instance"]=json!(id);}mailbox.event(event).await; }
                    }
                }
            }
        }
    }
}
