//! One bounded local presentation instance over shared connection-independent replicas.
use crate::{bridge::Events, delivery::Documents, files::Files};
use misa_client::{
    composition::{Choice, Preferences},
    daemons::Daemon,
    interaction::Interaction,
    interface::{self, Interface},
    request::Model,
};
use misa_kit::intent::Intent;
use misa_proto::{directory::Entry, observation::Selection};
use misa_protocol::observation::{MemberState, Status};
use misa_value::Value;
use serde_json::{Value as Json, json};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
    time::Duration,
};

fn capabilities() -> [String; 1] {
    ["semantic.meter@1".into()]
}

pub fn freshness(status: &Status) -> &'static str {
    match status {
        Status::Current => "current",
        Status::Awaiting => "loading",
        Status::Closed(_) => "closed",
        Status::Recovering(_) => "recovering",
        Status::Stale(_) => "stale",
    }
}
fn prefs_name(daemon: &str) -> String {
    format!(
        "presentations-{}.json",
        blake3::hash(daemon.as_bytes()).to_hex()
    )
}
async fn preferences(files: Arc<Files>, daemon: String) -> Result<Preferences, String> {
    tokio::task::spawn_blocking(move || {
        misa_client::preference_store::Store::new(files.root.join(prefs_name(&daemon))).load()
    })
    .await
    .map_err(|error| error.to_string())?
}
async fn compose(
    daemon: &Daemon,
    interaction: &Interaction,
    files: Arc<Files>,
    events: &Events,
    preferences: &Preferences,
    generation: u64,
) -> Result<(Arc<Documents>, String), String> {
    let selected = preferences.reconcile(
        &interaction.interface.presentations,
        &capabilities(),
        &["conversation", "status"],
    );
    for (id, fault) in selected.unavailable {
        events
            .notice(format!("Presentation {id}: {}", fault.message))
            .await;
    }
    let mut members = selected.members;
    if !members.contains_key("conversation") {
        members.insert(
            "conversation".into(),
            interaction
                .interface
                .presentation("conversation", &capabilities())
                .map_err(|fault| fault.message)?,
        );
    }
    let slots: Vec<_> = members.keys().map(|id| (id.clone(), id.clone())).collect();
    for (id, member) in &mut members {
        if id != "conversation" {
            member.optional = true;
        }
    }
    for name in ["operations.summary", "requests.summary"] {
        if interaction.interface.queries.contains_key(name) {
            let mut member = interaction
                .interface
                .query(name, vec![])
                .map_err(|fault| fault.message)?;
            member.optional = true;
            members.insert(name.into(), member);
        }
    }
    let selection = Selection {
        scope: interaction.interface.scope.clone(),
        members,
    };
    let name = crate::snapshot::name(daemon.identity(), &selection);
    let checkpoint = match crate::snapshot::load(files, name.clone()).await {
        Ok(checkpoint) => checkpoint,
        Err(error) => {
            events
                .notice(format!("Saved checkpoint unavailable: {error}"))
                .await;
            None
        }
    };
    let observation = daemon
        .client
        .observe(selection, checkpoint)
        .await
        .map_err(|fault| fault.message)?;
    events.send(json!({"kind":"composition","composition":generation,"catalog":interaction.interface.presentations,"capabilities":capabilities(),"preferences":preferences,"slots":slots.iter().map(|(id,_)|id).collect::<Vec<_>>()})).await;
    Ok((
        Arc::new(Documents::new(
            observation,
            events.instance.clone(),
            generation,
            slots,
        )),
        name,
    ))
}
fn summaries(feed: &Documents) -> Option<BTreeMap<String, i64>> {
    feed.observation
        .inspect(|replica, _| {
            let members = replica.current()?;
            let mut current = BTreeMap::new();
            for name in ["operations.summary", "requests.summary"] {
                if let Some(MemberState::Value(value)) = members.get(name) {
                    for row in value.as_list().unwrap_or(&[]) {
                        if name == "operations.summary"
                            && row.get("kind").and_then(Value::as_str)
                                != Some("credentials.authorize")
                        {
                            continue;
                        }
                        if row.get("state").and_then(Value::as_str) != Some("awaiting_input") {
                            continue;
                        }
                        if let (Some(id), Some(generation)) = (
                            row.get("id").and_then(Value::as_str),
                            row.get("generation").and_then(Value::as_i64),
                        ) {
                            if current.len() < 32 {
                                current.insert(id.into(), generation);
                            }
                        }
                    }
                }
            }
            Some(current)
        })
        .flatten()
}
fn model_json(model: &Model) -> Json {
    json!({"id":model.id,"generation":model.generation,"title":model.title,"body":model.body,"form":model.form,"input":model.input.as_ref().map(|input|json!({"id":input.id,"label":input.label,"secret":input.secret})),"actions":model.actions.iter().map(|action|json!({"id":action.id,"label":action.label})).collect::<Vec<_>>()})
}
pub async fn run(
    daemon: Arc<Daemon>,
    entry: Entry,
    files: Arc<Files>,
    events: Events,
    mut inbox: tokio::sync::mpsc::Receiver<Json>,
    limits: Arc<tokio::sync::Semaphore>,
) -> Result<(), String> {
    let interface = Interface::load(&daemon.client, entry.scope())
        .await
        .map_err(|fault| fault.message)?;
    let interaction = Arc::new(
        Interaction::load(&daemon.client, interface)
            .await
            .map_err(|fault| fault.message)?,
    );
    let mut prefs = match preferences(files.clone(), daemon.identity().into()).await {
        Ok(prefs) => prefs,
        Err(error) => {
            events.notice(error).await;
            Preferences::default()
        }
    };
    let mut generation = 1u64;
    let (mut feed, mut snapshot) = compose(
        &daemon,
        &interaction,
        files.clone(),
        &events,
        &prefs,
        generation,
    )
    .await?;
    let mut changes = feed.observation.watch();
    let mut commands:Vec<_>=interaction.shortcuts.iter().map(|shortcut|json!({"id":shortcut.id,"label":shortcut.label,"description":shortcut.description,"args":shortcut.args})).collect();
    for id in interaction.interface.commands.keys().filter(|id| {
        !interaction
            .shortcuts
            .iter()
            .any(|shortcut| &shortcut.id == *id)
    }) {
        if misa_client::form::Form::command(&interaction.interface, id).is_ok() {
            commands.push(json!({"id":id,"label":"Command","description":"Open declared input form","args":[]}));
        }
    }
    let sources: Vec<_> = interaction
        .sources
        .iter()
        .map(|source| json!({"id":source.id,"label":source.label,"kind":source.kind}))
        .collect();
    events.send(json!({"kind":"session","session":{"id":entry.id,"title":entry.title,"commands":commands,"sources":sources}})).await;
    let mut jobs = tokio::task::JoinSet::new();
    let mut requests = tokio::task::JoinSet::new();
    let mut writes = tokio::task::JoinSet::new();
    let mut models = BTreeMap::<String, Model>::new();
    let mut pending = BTreeSet::new();
    let mut retry_at = BTreeMap::new();
    let mut revision = None;
    let mut status = String::new();
    let mut retry = tokio::time::interval(Duration::from_secs(1));
    loop {
        feed.notify(&events.mailbox).await;
        if let Some(current) = feed
            .observation
            .inspect(|replica, _| freshness(replica.status()).to_owned())
        {
            if status != current {
                status = current;
                events.send(json!({"kind":"state","state":if status=="current"{"connected"}else{"closed"},"message":status})).await;
            }
        }
        if writes.is_empty() {
            let checkpoint = feed
                .observation
                .inspect(|replica, _| {
                    let Some(MemberState::Document(document)) =
                        replica.current()?.get("conversation")
                    else {
                        return None;
                    };
                    if revision.as_ref() == Some(document.version()) {
                        return None;
                    }
                    revision = Some(document.version().clone());
                    replica.checkpoint(false)
                })
                .flatten();
            if let Some(checkpoint) = checkpoint {
                let files = files.clone();
                let snapshot = snapshot.clone();
                let index = crate::snapshot::index_name(daemon.identity(), &entry.scope());
                writes.spawn(async move {
                    crate::snapshot::save(files.clone(), snapshot.clone(), checkpoint).await?;
                    tokio::task::spawn_blocking(move || files.write(&index, snapshot.as_bytes()))
                        .await
                        .map_err(|error| error.to_string())?
                });
            }
        }
        if let Some(current) = summaries(&feed) {
            let removed: Vec<_> = models
                .iter()
                .filter(|(id, model)| current.get(*id) != Some(&model.generation))
                .map(|(id, _)| id.clone())
                .collect();
            for id in removed {
                models.remove(&id);
                events
                    .send(json!({"kind":"request","id":id,"model":null}))
                    .await;
            }
            retry_at.retain(|id, _| current.contains_key(id));
            for (id, version) in current {
                if models
                    .get(&id)
                    .is_some_and(|model| model.generation == version)
                    || pending.contains(&id)
                    || requests.len() >= 4
                    || retry_at
                        .get(&id)
                        .is_some_and(|at| *at > tokio::time::Instant::now())
                {
                    continue;
                }
                pending.insert(id.clone());
                let daemon = daemon.clone();
                let interaction = interaction.clone();
                requests.spawn(async move {
                    let result = async {
                        let mut member = interaction
                            .interface
                            .query("operation.request", vec![Value::str(&id)])
                            .map_err(|fault| fault.message)?;
                        member.optional = true;
                        let reply = daemon
                            .client
                            .read(
                                Selection {
                                    scope: interaction.interface.scope.clone(),
                                    members: BTreeMap::from([("request".into(), member)]),
                                },
                                Duration::from_secs(10),
                            )
                            .await
                            .map_err(|fault| fault.message)?;
                        Model::parse(
                            interface::data(&reply, "request").map_err(|fault| fault.message)?,
                            &interaction.interface,
                        )
                        .map_err(|fault| fault.message)
                    }
                    .await;
                    (id, version, result)
                });
            }
        }
        tokio::select! {
            changed=changes.changed()=>{if changed.is_err(){return Err("Observation closed".into());}},
            _=retry.tick()=>{},
            result=writes.join_next(),if !writes.is_empty()=>{if let Some(Ok(Err(error)))=result{events.notice(error).await;}},
            result=requests.join_next(),if !requests.is_empty()=>{
                if let Some(Ok((id,version,result)))=result{pending.remove(&id);if summaries(&feed).and_then(|rows|rows.get(&id).copied())!=Some(version){continue;}
                    match result{Ok(Some(model))if model.generation==version=>{events.send(json!({"kind":"request","id":id,"model":model_json(&model)})).await;models.insert(id,model);},_=>{retry_at.insert(id,tokio::time::Instant::now()+Duration::from_secs(1));}}
                }
            },
            result=jobs.join_next(),if !jobs.is_empty()=>{if let Some(Ok(event))=result{events.send(event).await;}},
            command=inbox.recv()=>{
                let Some(command)=command else{return Ok(())};
                if command["local"]=="refresh" {
                    if command["composition"].as_u64()==Some(generation) {
                        feed.invalidate()?;
                        feed.notify(&events.mailbox).await;
                    }
                    continue;
                }
                if command["local"]=="presentation"{
                    let id=command["id"].as_str().unwrap_or_default();
                    let choice=serde_json::from_value::<Choice>(command["choice"].clone()).map_err(|error|error.to_string());
                    let mut next=prefs.clone();
                    let result=choice.and_then(|choice|next.set(&interaction.interface.presentations,&capabilities(),id,choice).map_err(|fault|fault.message));
                    if let Err(error)=result{events.notice(error).await;continue;}
                    let next_generation=generation.checked_add(1).ok_or("Composition generations exhausted")?;
                    match compose(&daemon,&interaction,files.clone(),&events,&next,next_generation).await{
                        Ok((new_feed,new_snapshot))=>{feed=new_feed;snapshot=new_snapshot;changes=feed.observation.watch();generation=next_generation;prefs=next;revision=None;
                            let path=files.root.join(prefs_name(daemon.identity()));let choice=prefs.0.get(id).cloned().ok_or("Missing selected preference")?;let id=id.to_owned();
                            if let Err(error)=tokio::task::spawn_blocking(move||misa_client::preference_store::Store::new(path).update(id,choice)).await.map_err(|error|error.to_string())?{events.notice(format!("Choice applied but not saved: {error}")).await;}
                        },Err(error)=>events.notice(error).await,
                    }
                    continue;
                }
                if command["intent"]=="command" && !interaction.shortcuts.iter().any(|shortcut|Some(shortcut.id.as_str())==command["name"].as_str()) {
                    match misa_client::form::Form::command(&interaction.interface,command["name"].as_str().unwrap_or_default()) {
                        Ok(form)=>events.send(json!({"kind":"form","action":form.title,"direct":true,"fields":form.fields})).await,
                        Err(fault)=>events.notice(fault.message).await,
                    }
                    continue;
                }
                if command["intent"]=="action" && command["fields"].as_array().is_none_or(|fields|fields.is_empty()) {
                    if let Ok(form)=misa_client::form::Form::action(&interaction.interface,command["action"].as_str().unwrap_or_default()) {
                        if !form.fields.is_empty(){events.send(json!({"kind":"form","action":form.title,"fields":form.fields})).await;continue;}
                    }
                }
                let permit=match limits.clone().try_acquire_owned(){Ok(permit)=>permit,Err(_)=>{events.send(json!({"kind":"rejected","text":command["text"],"message":"Pending work limit reached; try again shortly"})).await;continue;}};
                let request_model=command["request"].as_str().and_then(|id|models.get(id)).filter(|model|Some(model.generation)==command["generation"].as_i64()).cloned();
                let daemon=daemon.clone();let interaction=interaction.clone();let files=files.clone();
                jobs.spawn(async move{
                    let _permit=permit;
                    let result=async{
                        match command["local"].as_str(){
                            Some("form")=>{let id=command["action"].as_str().unwrap_or_default();let form=if command["direct"].as_bool()==Some(true){misa_client::form::Form::command(&interaction.interface,id)}else{misa_client::form::Form::action(&interaction.interface,id)}.map_err(|fault|fault.message)?;let drafts=serde_json::from_value::<BTreeMap<String,String>>(command["drafts"].clone()).map_err(|error|error.to_string())?;let(command,input)=form.prepare(&drafts).map_err(|fault|fault.message)?;crate::commands::execute(&daemon,&interaction,misa_client::interaction::Prepared::Invoke{command:interaction.interface.commands[&command].clone(),input}).await},
                            Some("fetch")=>crate::commands::fetch(&daemon,files,command["hash"].as_str().unwrap_or_default().into(),None).await,
                            Some("upload")=>{let path=std::path::PathBuf::from(command["path"].as_str().unwrap_or_default());let upload=crate::files::UploadFile(path);let bytes=tokio::task::spawn_blocking(move||{let bytes=crate::files::upload_bytes(&upload.0);drop(upload);bytes}).await.map_err(|error|error.to_string())??;let reference=daemon.blobs.share(bytes,command["media"].as_str()).await?;Ok(json!({"kind":"uploaded","name":command["name"],"blob":reference}))},
                            Some("request")=>{let model=request_model.ok_or("Request generation is no longer available")?;let fields=serde_json::from_value::<BTreeMap<String,String>>(command["fields"].clone()).map_err(|error|error.to_string())?;let prepared=model.prepare_drafts(command["action"].as_str().unwrap_or_default(),&fields,&interaction.interface).map_err(|fault|fault.message)?;crate::commands::execute(&daemon,&interaction,prepared).await},
                            Some("complete")=>{let member=interaction.complete(command["source"].as_str().unwrap_or_default(),command["prefix"].as_str().unwrap_or_default(),misa_proto::preparation::DEFAULT_CANDIDATES).map_err(|fault|fault.message)?;let result=daemon.client.read(Selection{scope:interaction.interface.scope.clone(),members:BTreeMap::from([("result".into(),member)])},Duration::from_secs(10)).await.map_err(|fault|fault.message)?;Ok(json!({"kind":"completion","request":command["request"],"items":interface::data(&result,"result").map_err(|fault|fault.message)?}))},
                            Some(_)=>Err("Unknown local action".into()),
                            None=>{let intent=serde_json::from_value::<Intent>(command.clone()).map_err(|error|error.to_string())?;if let Intent::Action{action,node,..}=&intent{if action=="attachment.save"{return crate::commands::save(&daemon,&interaction,files,node.clone()).await;}}let prepared=crate::commands::prepare(&interaction,intent)?;crate::commands::execute(&daemon,&interaction,prepared).await},
                        }
                    }.await;
                    result.unwrap_or_else(|message|match command["local"].as_str(){Some("fetch")=>json!({"kind":"blob_failed","hash":command["hash"],"text":message}),Some("upload")=>json!({"kind":"upload_failed","text":message}),_=>json!({"kind":"rejected","text":command["text"],"message":message})})
                });
            }
        }
    }
}
