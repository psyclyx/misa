//! Native relationship owner. Replica delivery, requests and blob work have
//! independent bounded lifetimes; the window never performs network IO.
use crate::app::Command;
use misa_client::{
    daemons::{Daemon, Daemons},
    document,
    driver::Observation,
    interaction::{Interaction, Prepared},
    interface::{self, Interface},
};
use misa_proto::{
    Intent,
    invocation::Outcome,
    observation::Selection,
    view::{BlobRef, Kind, Node},
};
use misa_protocol::observation::{Applied, MemberChange, MemberState, Status};
use misa_value::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use winit::event_loop::EventLoopProxy;
#[path = "native_manager.rs"]
mod manager;

pub enum Update {
    DocumentReport(Node),
    InstalledCommands(BTreeMap<String, Result<misa_client::form::Form, String>>),
    Form(misa_client::form::Form),
    Documents(Vec<Arc<Delivery>>),
    Session {
        generation: u64,
        update: Box<Update>,
    },
    Selected {
        generation: u64,
        daemon: String,
        scope: misa_proto::observation::Scope,
    },
    Directory(Arc<DirectoryDelivery>),
    ClosedInstance {
        daemon: String,
        scope: misa_proto::observation::Scope,
    },
    Composition {
        catalog: Vec<misa_proto::presentation::Presentation>,
        preferences: misa_client::composition::Preferences,
        slots: Vec<String>,
        observation: misa_client::ObservationId,
    },
    Request {
        id: String,
        generation: i64,
        model: Option<misa_client::request::Model>,
    },
    Shortcuts(Vec<misa_kit::intent::Command>),
    Image {
        hash: String,
        image: Arc<image::RgbaImage>,
        _delivery: tokio::sync::OwnedSemaphorePermit,
    },
    Report {
        title: String,
        value: Value,
    },
    RejectedDraft {
        text: String,
        reason: String,
    },
    Notice(String),
}

/// One queued window wakeup per observation. The window captures from the
/// coherent replica when it runs; skipped delivery sequences rebuild once.
pub struct Delivery {
    observation: Arc<Observation>,
    reader: Mutex<document::Reader>,
    queued: AtomicBool,
    slot: String,
    noticed: AtomicU64,
}
impl Delivery {
    pub fn observation_id(&self) -> misa_client::ObservationId {
        self.observation.id()
    }
}
pub struct DirectoryDelivery {
    latest: Mutex<Vec<crate::workspace::DaemonChoice>>,
    queued: AtomicBool,
}
impl DirectoryDelivery {
    pub fn capture(&self) -> Vec<crate::workspace::DaemonChoice> {
        self.queued.store(false, Ordering::Release);
        self.latest.lock().unwrap().clone()
    }
}
#[derive(Clone)]
struct Sink {
    proxy: EventLoopProxy<Update>,
    generation: Option<u64>,
}
impl Sink {
    fn send_event(&self, update: Update) -> Result<(), String> {
        let update = match self.generation {
            Some(generation) => Update::Session {
                generation,
                update: Box::new(update),
            },
            None => update,
        };
        self.proxy
            .send_event(update)
            .map_err(|_| "Window closed".into())
    }
}
impl Delivery {
    fn notify(self: &Arc<Self>, proxy: &Sink, deliveries: &[Arc<Delivery>]) -> Result<(), String> {
        let sequence = self
            .observation
            .inspect(|_, notice| notice.map_or(0, |notice| notice.sequence))
            .unwrap_or(0);
        if self.noticed.swap(sequence, Ordering::AcqRel) == sequence {
            return Ok(());
        }
        if !self.queued.swap(true, Ordering::AcqRel) {
            proxy
                .send_event(Update::Documents(deliveries.to_vec()))
                .map_err(|_| "Window closed".to_string())?;
        }
        Ok(())
    }
    pub fn capture_many(deliveries: &[Arc<Delivery>]) -> Vec<(String, document::Update)> {
        let Some(first) = deliveries.first() else {
            return vec![];
        };
        first.queued.store(false, Ordering::Release);
        let mut readers: Vec<_> = deliveries
            .iter()
            .map(|delivery| delivery.reader.lock().unwrap())
            .collect();
        document::capture_many(
            &first.observation,
            deliveries
                .iter()
                .zip(&mut readers)
                .map(|(delivery, reader)| (delivery.slot.as_str(), &mut **reader)),
        )
    }
}

pub fn start(
    ticket: Option<String>,
    proxy: EventLoopProxy<Update>,
) -> tokio::sync::mpsc::Sender<Command> {
    let (send, receive) = tokio::sync::mpsc::channel(32);
    tokio::spawn(async move {
        let proxy = Sink {
            proxy,
            generation: None,
        };
        if let Err(error) = manager::run(ticket, &proxy, receive).await {
            let _ = proxy.send_event(Update::Notice(error));
        }
    });
    send
}
async fn session(
    daemon: Arc<Daemon>,
    entry: misa_proto::directory::Entry,
    proxy: &Sink,
    mut outgoing: tokio::sync::mpsc::Receiver<Command>,
) -> Result<(), String> {
    let interface = Interface::load(&daemon.client, entry.scope())
        .await
        .map_err(|fault| fault.message)?;
    let interaction = Arc::new(
        Interaction::load(&daemon.client, interface)
            .await
            .map_err(|fault| fault.message)?,
    );
    let mut preferences = match crate::preferences::load(daemon.identity().into()).await {
        Ok(preferences) => preferences,
        Err(error) => {
            proxy.send_event(Update::Notice(error))?;
            misa_client::composition::Preferences::default()
        }
    };
    let (mut observation, mut deliveries) =
        compose(&daemon, &interaction, &preferences, proxy).await?;
    let commands = interaction.shortcuts.iter().map(|shortcut| misa_kit::intent::Command { id: shortcut.id.clone(), label: shortcut.label.clone(), description: shortcut.description.clone(), args: shortcut.args.clone() }).collect();
    proxy.send_event(Update::Shortcuts(commands))?;
    proxy.send_event(Update::InstalledCommands(interaction.interface.commands.keys().map(|id| (id.clone(), misa_client::form::Form::command(&interaction.interface,id).map_err(|fault|fault.message))).collect()))?;
    let mut changes = observation.watch();
    let mut image_reader = document::Reader::new("document");
    let mut requested = BTreeSet::new();
    let mut images = BTreeMap::<String, BlobRef>::new();
    let mut image_tasks = tokio::task::JoinSet::new();
    let mut commands = tokio::task::JoinSet::new();
    let mut request_tasks = tokio::task::JoinSet::new();
    let mut request_models = BTreeMap::<String, misa_client::request::Model>::new();
    let mut request_versions = BTreeMap::<String, i64>::new();
    let mut retry = tokio::time::interval(Duration::from_secs(1));
    let mut retry_at = BTreeMap::<String, tokio::time::Instant>::new();
    let mut request_pending = BTreeSet::new();
    // At most two decoded image payloads can wait for window consumption.
    let image_delivery = Arc::new(tokio::sync::Semaphore::new(2));
    loop {
        if let Some(update) = image_reader.capture(&observation) {
            for reference in document_images(&update) {
                if requested.contains(&reference.hash) || images.contains_key(&reference.hash) {
                    continue;
                }
                if requested.len() + images.len() >= 256 {
                    break;
                }
                images.insert(reference.hash.clone(), reference);
            }
        }
        if let Some(delivery) = deliveries.first() {
            delivery.notify(proxy, &deliveries)?;
        }
        if let Some(current) = request_summaries(&observation) {
            let present: BTreeSet<_> = current.iter().map(|(id, _, _)| id.clone()).collect();
            let removed: Vec<_> = request_versions
                .keys()
                .filter(|id| !present.contains(*id))
                .cloned()
                .collect();
            for id in removed {
                let generation = request_versions.remove(&id).unwrap().saturating_add(1);
                request_models.remove(&id);
                retry_at.remove(&id);
                proxy.send_event(Update::Request {
                    id,
                    generation,
                    model: None,
                })?;
            }
            for (id, generation, awaiting) in current {
                if !request_versions.contains_key(&id) && request_versions.len() >= 32 {
                    continue;
                }
                if !awaiting {
                    if request_models.remove(&id).is_some()
                        || request_versions.get(&id) != Some(&generation)
                    {
                        request_versions.insert(id.clone(), generation);
                        proxy.send_event(Update::Request {
                            id,
                            generation,
                            model: None,
                        })?;
                    }
                    continue;
                }
                if request_versions
                    .get(&id)
                    .is_some_and(|old| *old != generation)
                {
                    request_models.remove(&id);
                    proxy.send_event(Update::Request {
                        id: id.clone(),
                        generation,
                        model: None,
                    })?;
                }
                request_versions.insert(id.clone(), generation);
                if request_models
                    .get(&id)
                    .is_some_and(|model| model.generation == generation)
                    || request_tasks.len() >= 8
                    || request_pending.contains(&id)
                    || retry_at
                        .get(&id)
                        .is_some_and(|time| *time > tokio::time::Instant::now())
                {
                    continue;
                }
                request_versions.insert(id.clone(), generation);
                request_pending.insert(id.clone());
                let client = daemon.client.clone();
                let interaction = interaction.clone();
                request_tasks.spawn(async move {
                    let result = async {
                        let mut member = interaction
                            .interface
                            .query("operation.request", vec![Value::str(&id)])?;
                        member.optional = true;
                        let reply = client
                            .read(
                                Selection {
                                    scope: interaction.interface.scope.clone(),
                                    members: BTreeMap::from([("request".into(), member)]),
                                },
                                Duration::from_secs(10),
                            )
                            .await?;
                        misa_client::request::Model::parse(
                            interface::data(&reply, "request")?,
                            &interaction.interface,
                        )
                    }
                    .await;
                    (id, generation, result)
                });
            }
        }
        while image_tasks.len() < 2 {
            let Some((hash, reference)) = images.pop_first() else {
                break;
            };
            if requested.len() >= 256 && !requested.contains(&hash) {
                requested.pop_first();
            }
            requested.insert(hash.clone());
            let transfers = daemon.blobs.clone();
            let budget = image_delivery.clone();
            image_tasks.spawn(async move {
                let permit = budget
                    .acquire_owned()
                    .await
                    .map_err(|_| "Window image delivery closed".to_string())?;
                let image = transfers
                    .decode(&reference, |blob| {
                        let mut reader = image::ImageReader::new(std::io::Cursor::new(&blob.bytes))
                            .with_guessed_format()
                            .map_err(|error| error.to_string())?;
                        let mut limits = image::Limits::default();
                        limits.max_image_width = Some(2048);
                        limits.max_image_height = Some(2048);
                        limits.max_alloc = Some(32 * 1024 * 1024);
                        reader.limits(limits);
                        reader
                            .decode()
                            .map(|image| Arc::new(image.into_rgba8()))
                            .map_err(|error| error.to_string())
                    })
                    .await?;
                Ok::<_, String>(Update::Image {
                    hash,
                    image,
                    _delivery: permit,
                })
            });
        }
        tokio::select! {
            _=retry.tick()=>{},
            result=request_tasks.join_next(),if !request_tasks.is_empty()=>{
                if let Some(Ok((id,generation,result)))=result{
                    request_pending.remove(&id);
                    if request_versions.get(&id)!=Some(&generation)||!request_summaries(&observation).is_some_and(|rows|rows.iter().any(|(current,version,awaiting)|current==&id&&*version==generation&&*awaiting)){continue;}
                    match result{
                        Ok(Some(model)) if model.generation==generation=>{request_models.insert(id.clone(),model.clone());proxy.send_event(Update::Request{id,generation,model:Some(model)})?;},
                        _=>{retry_at.insert(id,tokio::time::Instant::now()+Duration::from_secs(1));},
                    }
                }
            },
            changed = changes.changed() => changed.map_err(|_| "Session observation closed".to_string())?,
            completed = image_tasks.join_next(), if !image_tasks.is_empty() => {
                let update = match completed.expect("pending image") { Ok(Ok(update)) => update, Ok(Err(error)) => Update::Notice(format!("Could not load attachment: {error}")), Err(error) => Update::Notice(error.to_string()) };
                proxy.send_event(update).map_err(|_| "Window closed".to_string())?;
            },
            completed = commands.join_next(), if !commands.is_empty() => {
                let update = completed.expect("pending command").unwrap_or_else(|error| Update::Notice(error.to_string()));
                proxy.send_event(update).map_err(|_| "Window closed".to_string())?;
            },
            command = outgoing.recv() => {
                let Some(command) = command else { return Ok(()); };
                if let Command::Presentation { id, choice } = command {
                    let mut next = preferences.clone();
                    let result = next.set(&interaction.interface.presentations, &[], &id, choice.clone()).map_err(|fault| fault.message);
                    match result {
                        Err(error) => { proxy.send_event(Update::Notice(error))?; }
                        Ok(()) => match compose(&daemon, &interaction, &next, proxy).await {
                            Ok((new_observation, new_deliveries)) => {
                                preferences = next;
                                observation = new_observation;
                                deliveries = new_deliveries;
                                changes = observation.watch();
                                image_reader = document::Reader::new("document");
                                if let Err(error) = crate::preferences::save(daemon.identity().into(), id, choice).await {
                                    proxy.send_event(Update::Notice(format!("Presentation choice applied, but not saved: {error}")))?;
                                }
                            }
                            Err(error) => { proxy.send_event(Update::Notice(error))?; }
                        }
                    }
                    continue;
                }
                if let Command::LoadImage(reference) = command {
                    // Explicit retry works after decode failure or renderer eviction.
                    if images.len() < 64 { images.insert(reference.hash.clone(), reference); }
                    continue;
                }
                if matches!(command, Command::Copy(_)) { continue; }
                if let Command::Intent(Intent::Action { action, fields, .. }) = &command {
                    if fields.is_empty() {
                        if let Ok(form) = misa_client::form::Form::action(&interaction.interface, action) {
                            if !form.fields.is_empty() { proxy.send_event(Update::Form(form))?; continue; }
                        }
                    }
                }
                if commands.len() >= 16 { proxy.send_event(rejected(command, "Too many pending requests; try again shortly".into())).map_err(|_| "Window closed".to_string())?; continue; }
                let daemon = daemon.clone(); let interaction = interaction.clone();
                let request_model=match &command{Command::Request{id,generation,..}=>request_models.get(id).filter(|model|model.generation==*generation).cloned(),_=>None};
                commands.spawn(async move {
                    let result = match &command {
                        Command::InvokeInstalled {command,input} => match interaction.invoke(command,input.clone()) {Ok(prepared)=>execute(&daemon,&interaction,prepared).await,Err(fault)=>Err(fault.message)},
                        Command::Form{action,drafts} => match misa_client::form::Form::action(&interaction.interface,action).and_then(|form|form.prepare(drafts)) { Ok((command,input))=>execute(&daemon,&interaction,Prepared::Invoke{command:interaction.interface.commands[&command].clone(),input}).await,Err(fault)=>Err(fault.message) },
                        Command::Intent(intent) => match prepare(&interaction, intent.clone()) { Ok(prepared) => execute(&daemon, &interaction, prepared).await, Err(error) => Err(error) },
                        Command::Save { node, destination } => save(&daemon, &interaction, node, destination).await.map(Update::Notice),
                        Command::Request{action,fields,..}=>match request_model {Some(model)=>match model.prepare_drafts(action,&fields.iter().map(|(id,value)|(id.clone(),value.as_str().unwrap_or_default().to_string())).collect(),&interaction.interface){Ok(prepared)=>execute(&daemon,&interaction,prepared).await,Err(fault)=>Err(fault.message)},None=>Err("Request generation is no longer current".into())},
                        _=>Err("This action belongs to the daemon chooser".into()),
                    };
                    result.unwrap_or_else(|reason| rejected(command, reason))
                });
            },
        }
    }
}

async fn compose(
    daemon: &Arc<Daemon>,
    interaction: &Arc<Interaction>,
    preferences: &misa_client::composition::Preferences,
    proxy: &Sink,
) -> Result<(Arc<Observation>, Vec<Arc<Delivery>>), String> {
    let reconciled = preferences.reconcile(
        &interaction.interface.presentations,
        &[],
        &["conversation", "status"],
    );
    for (id, fault) in &reconciled.unavailable {
        proxy.send_event(Update::Notice(format!(
            "Presentation {id} unavailable: {}",
            fault.message
        )))?;
    }
    let mut selected = reconciled.members;
    let member = match selected.remove("conversation") {
        Some(member) => member,
        None => interaction
            .interface
            .presentation("conversation", &[])
            .map_err(|fault| fault.message)?,
    };
    let mut members = BTreeMap::from([("document".into(), member)]);
    let mut slots = vec![("conversation".to_string(), "document".to_string())];
    for (id, mut member) in selected {
        member.optional = true;
        let name = format!("presentation:{id}");
        members.insert(name.clone(), member);
        slots.push((id, name));
    }
    for id in ["operations.summary", "requests.summary"] {
        if interaction.interface.queries.contains_key(id) {
            let mut member = interaction
                .interface
                .query(id, vec![])
                .map_err(|fault| fault.message)?;
            member.optional = true;
            members.insert(id.into(), member);
        }
    }
    let observation = Arc::new(
        daemon
            .client
            .observe(
                Selection {
                    scope: interaction.interface.scope.clone(),
                    members,
                },
                None,
            )
            .await
            .map_err(|fault| fault.message)?,
    );
    proxy.send_event(Update::Composition {
        catalog: interaction.interface.presentations.clone(),
        preferences: preferences.clone(),
        slots: slots.iter().map(|(slot, _)| slot.clone()).collect(),
        observation: observation.id(),
    })?;
    let deliveries: Vec<_> = slots
        .into_iter()
        .map(|(slot, member)| {
            Arc::new(Delivery {
                observation: observation.clone(),
                reader: Mutex::new(document::Reader::new(member)),
                queued: AtomicBool::new(false),
                slot,
                noticed: AtomicU64::new(0),
            })
        })
        .collect();
    Ok((observation, deliveries))
}

fn rejected(command: Command, reason: String) -> Update {
    match command {
        Command::Intent(Intent::Prompt { text, .. } | Intent::Interrupt { text, .. }) => {
            Update::RejectedDraft { text, reason }
        }
        _ => Update::Notice(reason),
    }
}
fn prepare(interaction: &Interaction, intent: Intent) -> Result<Prepared, String> {
    match intent {
        Intent::Prompt { text, attachments } => interaction.prompt(text, attachments, false),
        Intent::Interrupt { text, attachments } => interaction.prompt(text, attachments, true),
        Intent::Cancel { target } => interaction.cancel(target),
        Intent::Command { name, args } => interaction.shortcut(&name, args),
        Intent::Action { action, fields, .. } => interaction.action(
            &action,
            &fields
                .into_iter()
                .map(|field| (field.id, Value::str(field.value)))
                .collect(),
        ),
        _ => return Err("This action has no installed scoped command".into()),
    }
    .map_err(|fault| fault.message)
}
async fn execute(
    daemon: &Daemon,
    interaction: &Interaction,
    prepared: Prepared,
) -> Result<Update, String> {
    match prepared {
        Prepared::Read { member } => {
            let title = member.query.id.clone();
            let result = daemon
                .client
                .read(
                    Selection {
                        scope: interaction.interface.scope.clone(),
                        members: BTreeMap::from([("result".into(), member)]),
                    },
                    Duration::from_secs(10),
                )
                .await
                .map_err(|fault| fault.message)?;
            Ok(Update::DocumentReport(interface::report(&result,"result",&title).map_err(|fault|fault.message)?))
        }
        Prepared::Invoke { command, input } => {
            let title = command.id.clone();
            match daemon
                .client
                .invoke(
                    interaction.interface.scope.clone(),
                    command,
                    input,
                    Duration::from_secs(20),
                )
                .await
                .map_err(|fault| fault.message)?
                .outcome
            {
                Outcome::Completed { value } => Ok(if value == Value::Null {
                    Update::Notice("Done".into())
                } else {
                    Update::Report { title, value }
                }),
                Outcome::Accepted { operation } => {
                    let watch = misa_client::operation::Watch::open(
                        &daemon.client,
                        &interaction.interface,
                        operation.clone(),
                        false,
                    )
                    .await
                    .unwrap_or_else(|fault| {
                        misa_client::operation::Watch::failed(operation, fault)
                    });
                    let text = match watch.wait().await.outcome {
                        misa_client::operation::Terminal::Finished { state, .. } => {
                            format!("Operation {state}")
                        }
                        misa_client::operation::Terminal::Expired => {
                            "Operation result expired; completion is unknown".into()
                        }
                        misa_client::operation::Terminal::Fault(fault) => fault.message,
                    };
                    Ok(Update::Notice(text))
                }
                Outcome::Rejected { fault } | Outcome::Indeterminate { fault } => {
                    Err(fault.message)
                }
            }
        }
    }
}
async fn save(
    daemon: &Daemon,
    interaction: &Interaction,
    node: &str,
    destination: &str,
) -> Result<String, String> {
    let prepared = interaction
        .invoke(
            "session.attachment.resolve",
            Value::map([("node", Value::str(node))]),
        )
        .map_err(|fault| fault.message)?;
    let Prepared::Invoke { command, input } = prepared else {
        return Err("Attachment resolution is not an installed command".into());
    };
    let reply = daemon
        .client
        .invoke(
            interaction.interface.scope.clone(),
            command,
            input,
            Duration::from_secs(20),
        )
        .await
        .map_err(|fault| fault.message)?;
    let reference: BlobRef = match reply.outcome {
        Outcome::Completed { value } => interface::decode(&value).map_err(|fault| fault.message)?,
        Outcome::Rejected { fault } | Outcome::Indeterminate { fault } => return Err(fault.message),
        _ => return Err("Attachment resolution did not return a file".into()),
    };
    let blob = daemon.blobs.download(&reference).await?;
    let path = destination.to_owned();
    tokio::task::spawn_blocking(move || write_new(&path, &blob.bytes))
        .await
        .map_err(|error| error.to_string())??;
    Ok(format!("Saved {destination}"))
}
fn document_images(update: &document::Update) -> Vec<BlobRef> {
    match update {
        document::Update::Reset(document) => images(&document.tree),
        document::Update::Changed { member, applied } => match applied.as_ref() {
            Applied::Changed(members) => match members.get(member) {
                Some(MemberChange::Document { tree, .. }) => tree
                    .iter()
                    .flat_map(|op| match op {
                        misa_proto::sync::ViewOp::Insert { node, .. }
                        | misa_proto::sync::ViewOp::Replace { node, .. } => images(node),
                        _ => vec![],
                    })
                    .collect(),
                _ => vec![],
            },
            _ => vec![],
        },
        _ => vec![],
    }
}
fn images(node: &Node) -> Vec<BlobRef> {
    let mut references = vec![];
    if let Kind::Image { blob, .. } = &node.kind {
        references.push(blob.clone());
    }
    for child in &node.children {
        references.extend(images(child));
    }
    if let Kind::List { items, .. } = &node.kind {
        for child in items.iter().flatten() {
            references.extend(images(child));
        }
    }
    references
}
pub fn write_new(path: &str, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write as _;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| format!("Could not create {path}: {error}"))?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|error| format!("Could not finish {path}: {error}"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_received_file_is_findable_and_never_overwrites_an_existing_file() {
        let path = std::env::temp_dir().join(format!("misa-pixel-save-{}", std::process::id()));
        let path = path.to_str().unwrap();
        let _ = std::fs::remove_file(path);
        super::write_new(path, b"received attachment").unwrap();
        assert!(super::write_new(path, b"replacement").is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"received attachment");
        std::fs::remove_file(path).unwrap();
    }
}

fn request_summaries(observation: &Observation) -> Option<Vec<(String, i64, bool)>> {
    observation
        .inspect(|replica, _| {
            let members = replica.current()?;
            let mut requests = vec![];
            for name in ["operations.summary", "requests.summary"] {
                if let Some(MemberState::Value(value)) = members.get(name) {
                    for row in value.as_list().unwrap_or(&[]) {
                        if name == "operations.summary"
                            && row.get("kind").and_then(Value::as_str)
                                != Some("credentials.authorize")
                        {
                            continue;
                        }
                        if let (Some(id), Some(generation)) = (
                            row.get("id").and_then(Value::as_str),
                            row.get("generation").and_then(Value::as_i64),
                        ) {
                            requests.push((
                                id.to_owned(),
                                generation,
                                row.get("state").and_then(Value::as_str) == Some("awaiting_input"),
                            ));
                        }
                    }
                }
            }
            Some(requests)
        })
        .flatten()
}
