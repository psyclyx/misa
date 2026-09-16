//! Terminal adaptation of shared scoped observations and installed interfaces.
use crate::{Presentation, Session, SessionReply, SessionRequest};
use misa_client::{
    driver::{Client, Observation},
    interaction::{Interaction, Prepared},
    interface::{self, Interface},
};
use misa_proto::{
    Intent, Node, invocation::Outcome, observation::Selection, view::Choice,
    preparation::SourceKind,
};
use misa_protocol::observation::MemberState;
use misa_value::Value;
use std::{
    collections::{BTreeMap, VecDeque},
    sync::Arc,
    time::Duration,
};

pub struct ScopedRemote {
    daemon: Arc<misa_client::daemons::Daemon>,
    interaction: Arc<Interaction>,
    observation: Observation,
    selection: Selection,
    reader: misa_client::document::Reader,
    presentations: BTreeMap<String, misa_client::document::Reader>,
    preferences: misa_client::composition::Preferences,
    preference_path: std::path::PathBuf,
    sources: BTreeMap<String, Value>,
    request_versions: BTreeMap<String, i64>,
    replacements: tokio::task::JoinSet<Result<Replacement, String>>,
    pending: tokio::task::JoinSet<(SessionReply, Option<misa_client::operation::Watch>)>,
    operations: misa_client::operation::Tracker,
    turn_settled: bool,
    turn_failure: Option<String>,
    updates: VecDeque<Presentation>,
    catalog: crate::Catalog,
    entry: misa_proto::directory::Entry,
}
struct Replacement {
    unavailable: BTreeMap<String, misa_proto::Fault>,
    preferences: misa_client::composition::Preferences,
    selection: Selection,
    readers: BTreeMap<String, misa_client::document::Reader>,
    observation: Observation,
}
impl ScopedRemote {
    fn track_operation(&mut self, operation: misa_client::operation::Watch) {
        if let Err(fault)=self.operations.insert(operation) {self.turn_failure=Some(fault.message.clone());self.updates.push_back(Presentation::Reply(SessionReply::Notice(fault.message)));}
    }
    fn load_preferences() -> Result<misa_client::composition::Preferences, String> {
        misa_client::preference_store::Store::new(crate::storage::File::default_path().with_extension("presentations.json")).load()
    }
    pub fn identity(&self) -> String {
        format!("{}:{:?}", self.daemon.identity(), self.selection.scope)
    }
    pub fn scope(&self)->&misa_proto::observation::Scope {&self.selection.scope}
    pub fn daemon_identity(&self) -> &str {
        self.daemon.identity()
    }
    pub fn parked_busy(&self) -> bool {
        !self.pending.is_empty() || !self.operations.is_empty() || !self.replacements.is_empty()
    }
    pub fn reactivate(&mut self) {
        self.updates.retain(|update| {
            matches!(update, Presentation::Reply(_) | Presentation::TurnOutput(_))
        });
        self.reader = misa_client::document::Reader::new("document");
        for (id, reader) in &mut self.presentations {
            *reader = misa_client::document::Reader::new(format!("presentation:{id}"));
        }
        self.sources.clear();
    }
    pub fn presentation_choices(&self) -> Vec<Choice> {
        self.interaction
            .interface
            .presentations
            .iter()
            .filter(|item| item.id != "conversation")
            .map(|item| Choice {
                value: item.id.clone(),
                label: item.title.clone(),
                detail: Some(format!(
                    "auto · hide · {}",
                    item.variants
                        .iter()
                        .map(|variant| variant.id.as_str())
                        .collect::<Vec<_>>()
                        .join(" · ")
                )),
            })
            .collect()
    }
    fn prepare_presentation(
        &self,
        args: &Value,
    ) -> Result<
        impl std::future::Future<Output = Result<Replacement, String>> + Send + 'static,
        String,
    > {
        use misa_client::composition::Choice;
        let id = args
            .get("presentation")
            .and_then(Value::as_str)
            .ok_or("Presentation required")?;
        if id == "conversation" {
            return Err("The conversation is the primary document".into());
        }
        let choice = match args
            .get("variant")
            .and_then(Value::as_str)
            .unwrap_or("auto")
        {
            "hide" => Choice::Hidden,
            "auto" => Choice::Auto,
            other => Choice::Variant(other.into()),
        };
        let mut preferences = self.preferences.clone();
        preferences
            .set(
                &self.interaction.interface.presentations,
                &["semantic.meter@1".into()],
                id,
                choice.clone(),
            )
            .map_err(|fault| fault.message)?;
        let mut selection = self.selection.clone();
        selection
            .members
            .retain(|key, _| !key.starts_with("presentation:"));
        let mut readers = BTreeMap::new();
        let composition = preferences.reconcile(
            &self.interaction.interface.presentations,
            &["semantic.meter@1".into()],
            &["conversation", "status"],
        );
        for (id, mut member) in composition.members {
            if id == "conversation" {
                continue;
            }
            member.optional = true;
            let name = format!("presentation:{id}");
            selection.members.insert(name.clone(), member);
            readers.insert(id, misa_client::document::Reader::new(name));
        }
        let client = self.daemon.client.clone();
        let path = self.preference_path.clone();
        let preference_id=id.to_string();
        Ok(async move {
            let observation = client
                .observe(selection.clone(), None)
                .await
                .map_err(|fault| fault.message)?;
            tokio::task::spawn_blocking(move || misa_client::preference_store::Store::new(path).update(preference_id,choice))
                .await
                .map_err(|error| error.to_string())??;
            Ok(Replacement {
                unavailable: composition.unavailable,
                preferences,
                selection,
                readers,
                observation,
            })
        })
    }
    fn replace_presentation(&mut self, replacement: Replacement) {
        let Replacement {
            unavailable,
            preferences,
            selection,
            readers,
            observation,
        } = replacement;
        for (id, fault) in unavailable {
            self.updates
                .push_back(Presentation::Reply(SessionReply::Notice(format!(
                    "{id}: {}",
                    fault.message
                ))));
        }
        for id in self
            .presentations
            .keys()
            .filter(|id| !readers.contains_key(*id))
        {
            self.updates.push_back(Presentation::Contribution {
                id: id.clone(),
                update: misa_client::document::Update::Unavailable(misa_proto::Fault::new(
                    "hidden",
                    "Hidden locally",
                )),
            });
        }
        self.preferences = preferences;
        self.selection = selection;
        self.observation = observation;
        self.presentations = readers;
        self.reader = misa_client::document::Reader::new("document");
    }
    fn collect_operation(&mut self) -> Result<(), String> {
        for completion in self.operations.drain() {
            use misa_client::operation::Terminal;
            let error=match completion.outcome {
                Terminal::Finished{state,document,..} if state=="succeeded"=>{if let Some(tree)=document {self.updates.push_back(Presentation::TurnOutput(tree));}None},
                Terminal::Finished{state,..}=>Some(format!("Operation {state}")),
                Terminal::Expired=>Some("Operation result expired before reconciliation".into()),
                Terminal::Fault(fault)=>Some(fault.message),
            };
            if let Some(error)=error {self.turn_failure=Some(error.clone());self.updates.push_back(Presentation::Reply(SessionReply::Notice(error)));}
        }
        // Completion belongs to admitted prompt operations. Unrelated finite
        // reads, uploads, and credential forms cannot delay their terminal output.
        self.turn_settled = self.operations.is_empty();
        Ok(())
    }
    pub async fn on(
        daemon: Arc<misa_client::daemons::Daemon>,
        entry: misa_proto::directory::Entry,
    ) -> Result<Self, String> {
        let interface = Interface::load(&daemon.client, entry.scope())
            .await
            .map_err(|fault| fault.message)?;
        let interaction = Arc::new(
            Interaction::load(&daemon.client, interface)
                .await
                .map_err(|fault| fault.message)?,
        );
        let preferences = Self::load_preferences()?;
        let mut members = BTreeMap::from([(
            "document".into(),
            interaction
                .interface
                .presentation("conversation", &[])
                .map_err(|fault| fault.message)?,
        )]);
        let mut presentations = BTreeMap::new();
        let composition = preferences.reconcile(
            &interaction.interface.presentations,
            &["semantic.meter@1".into()],
            &["conversation", "status"],
        );
        for (id, mut member) in composition.members {
            if id == "conversation" {
                continue;
            }
            let name = format!("presentation:{id}");
            member.optional = true;
            members.insert(name.clone(), member);
            presentations.insert(id, misa_client::document::Reader::new(name));
        }
        for source in &interaction.sources {
            if source.kind == SourceKind::Resident {
                members.insert(format!("source:{}", source.id), source.member.clone());
            }
        }
        for id in ["operations.summary", "requests.summary"] {
            if interaction.interface.queries.contains_key(id) {
                members.insert(
                    id.into(),
                    interaction
                        .interface
                        .query(id, vec![])
                        .map_err(|fault| fault.message)?,
                );
            }
        }
        let selection = Selection {
            scope: entry.scope(),
            members,
        };
        let observation = daemon
            .client
            .observe(selection.clone(), None)
            .await
            .map_err(|fault| fault.message)?;
        let catalog = crate::Catalog {
            commands: interaction
                .shortcuts
                .iter()
                .map(|shortcut| misa_kit::intent::Command {
                    id: shortcut.id.clone(),
                    label: shortcut.label.clone(),
                    description: shortcut.description.clone(),
                    args: shortcut.args.clone(),
                })
                .collect(),
            sources: interaction
                .sources
                .iter()
                .map(|source| misa_kit::intent::Source {
                    id: source.id.clone(),
                    label: source.label.clone(),
                    kind: source.kind.clone(),
                    description: None,
                })
                .collect(),
        };
        Ok(Self {
            daemon,
            interaction,
            observation,
            selection,
            reader: misa_client::document::Reader::new("document"),
            presentations,
            sources: BTreeMap::new(),
            request_versions: BTreeMap::new(),
            preferences,
            preference_path: crate::storage::File::default_path()
                .with_extension("presentations.json"),
            replacements: Default::default(),
            pending: Default::default(),
            operations: misa_client::operation::Tracker::new(32),
            turn_settled: true,
            turn_failure: None,
            updates: std::iter::once(Presentation::Declaration{catalog:catalog.clone(),location:entry.title.clone()})
                .chain(composition.unavailable.into_iter().map(|(id, fault)| {
                    Presentation::Reply(SessionReply::Notice(format!("{id}: {}", fault.message)))
                }))
                .collect(),
            catalog,
            entry,
        })
    }
    fn collect(&mut self) {
        let readers = std::iter::once(("", &mut self.reader)).chain(
            self.presentations
                .iter_mut()
                .map(|(id, reader)| (id.as_str(), reader)),
        );
        let documents = misa_client::document::capture_many(&self.observation, readers);
        if !documents.is_empty() {
            self.updates.push_back(Presentation::Documents(documents));
        }
        let sources = &mut self.sources;
        let updates = &mut self.updates;
        self.observation.inspect(|replica, _| {
            if let Some(members) = replica.current() {
                for (name, state) in members {
                    let Some(source) = name.strip_prefix("source:") else {
                        continue;
                    };
                    if let MemberState::Value(value) = state {
                        if sources.get(source).is_some_and(|old| old.same(value)) {
                            continue;
                        }
                        sources.insert(source.into(), value.clone());
                        updates.push_back(Presentation::Candidates {
                            source: source.into(),
                            items: misa_proto::preparation::candidates(value),
                            truncated: false,
                        });
                    }
                }
            }
        });
        if !self
            .observation
            .inspect(|replica, _| replica.current().is_some())
            .unwrap_or(false)
        {
            return;
        }
        let requests = self
            .observation
            .inspect(|replica, _| {
                let mut requests = vec![];
                if let Some(members) = replica.current() {
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
                                        id.to_string(),
                                        generation,
                                        row.get("state").and_then(Value::as_str)
                                            == Some("awaiting_input"),
                                    ));
                                }
                            }
                        }
                    }
                }
                requests
            })
            .unwrap_or_default();
        let present: std::collections::BTreeSet<_> =
            requests.iter().map(|(id, _, _)| id.clone()).collect();
        let removed: Vec<_> = self
            .request_versions
            .iter()
            .filter(|(id, _)| !present.contains(*id))
            .map(|(id, generation)| (id.clone(), *generation))
            .collect();
        for (id, generation) in removed {
            self.request_versions.remove(&id);
            self.updates
                .push_back(Presentation::Reply(SessionReply::Request {
                    id,
                    generation: generation.saturating_add(1),
                    model: None,
                }));
        }
        for (id, generation, awaiting) in requests {
            if self.request_versions.get(&id) == Some(&generation) || self.pending.len() >= 32 {
                continue;
            }
            self.request_versions.insert(id.clone(), generation);
            if !awaiting {
                self.updates
                    .push_back(Presentation::Reply(SessionReply::Request {
                        id,
                        generation,
                        model: None,
                    }));
                continue;
            }
            let interaction = self.interaction.clone();
            let client = self.daemon.client.clone();
            self.pending.spawn(async move {
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
                    let value = interface::data(&reply, "request")?;
                    misa_client::request::Model::parse(value, &interaction.interface)
                }
                .await;
                let reply = match result {
                    Ok(model) => SessionReply::Request {
                        id,
                        generation,
                        model,
                    },
                    Err(fault) if fault.code == "request_unavailable" => SessionReply::Request {
                        id,
                        generation,
                        model: None,
                    },
                    Err(fault) => {
                        SessionReply::Notice(format!("Input request {id}: {}", fault.message))
                    }
                };
                (reply, None)
            });
        }
    }
    fn prepare(&self, intent: Intent) -> Result<Prepared, String> {
        let result = match intent {
            Intent::Prompt { text, attachments } => {
                self.interaction.prompt(text, attachments, false)
            }
            Intent::Interrupt { text, attachments } => {
                self.interaction.prompt(text, attachments, true)
            }
            Intent::Cancel { target } => self.interaction.cancel(target),
            Intent::Command { name, args } => self.interaction.shortcut(&name, args),
            Intent::Action { action, fields, .. } => self.interaction.action(
                &action,
                &fields
                    .into_iter()
                    .map(|field| (field.id, Value::str(field.value)))
                    .collect(),
            ),
            _ => return Err("This operation has no installed scoped command".into()),
        };
        result.map_err(|fault| fault.message)
    }
}
async fn execute(
    client: &Client,
    interaction: &Interaction,
    prepared: Prepared,
) -> Result<(Option<SessionReply>, Option<misa_client::operation::Watch>), String> {
    match prepared {
        Prepared::Invoke { command, input } => {
            let tracks_turn = matches!(command.id.as_str(), "session.prompt" | "session.interrupt");
            match client
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
                Outcome::Completed { value } => Ok((
                    (value != Value::Null).then(|| {
                        SessionReply::Report(misa_client::request::report("Result", &value))
                    }),
                    None,
                )),
                Outcome::Accepted { operation } => {
                    let notice = Some(SessionReply::Notice(format!(
                        "Operation accepted: {}",
                        operation.id
                    )));
                    let observation = Some(misa_client::operation::Watch::open(client,&interaction.interface,operation.clone(),tracks_turn).await.unwrap_or_else(|fault|misa_client::operation::Watch::failed(operation,fault)));
                    Ok((notice, observation))
                }
                Outcome::Rejected { fault } | Outcome::Indeterminate { fault } => {
                    Err(fault.message)
                }
            }
        }
        Prepared::Read { member } => {
            let result = client
                .read(
                    Selection {
                        scope: interaction.interface.scope.clone(),
                        members: BTreeMap::from([("result".into(), member)]),
                    },
                    Duration::from_secs(10),
                )
                .await
                .map_err(|fault| fault.message)?;
            Ok((
                Some(SessionReply::Report(interface::report(&result, "result", "Report").map_err(|fault| fault.message)?)),
                None,
            ))
        }
    }
}
async fn complete(
    client: &Client,
    interaction: &Interaction,
    source: &str,
    prefix: &str,
) -> Result<(Vec<Choice>, bool), String> {
    let member = interaction
        .complete(source, prefix, misa_proto::preparation::DEFAULT_CANDIDATES)
        .map_err(|fault| fault.message)?;
    let result = client
        .read(
            Selection {
                scope: interaction.interface.scope.clone(),
                members: BTreeMap::from([("result".into(), member)]),
            },
            Duration::from_secs(10),
        )
        .await
        .map_err(|fault| fault.message)?;
    let result: misa_proto::preparation::Candidates =
        interface::decode(interface::data(&result, "result").map_err(|fault| fault.message)?)
            .map_err(|fault| fault.message)?;
    Ok((result.items, result.truncated))
}
async fn save(
    daemon: &misa_client::daemons::Daemon,
    interaction: &Interaction,
    node: &str,
    destination: &str,
) -> Result<(), String> {
    let Prepared::Invoke { command, input } = interaction
        .invoke(
            "session.attachment.resolve",
            Value::map([("node", Value::str(node))]),
        )
        .map_err(|fault| fault.message)?
    else {
        unreachable!()
    };
    let reply = daemon
        .client
        .invoke(
            interaction.interface.scope.clone(),
            command,
            input,
            Duration::from_secs(10),
        )
        .await
        .map_err(|fault| fault.message)?;
    let reference: misa_proto::view::BlobRef = match reply.outcome {
        Outcome::Completed { value } => interface::decode(&value).map_err(|fault| fault.message)?,
        Outcome::Rejected { fault } | Outcome::Indeterminate { fault } => return Err(fault.message),
        Outcome::Accepted { .. } => {
            return Err("Attachment resolution returned an unexpected operation".into());
        }
    };
    let blob = daemon.blobs.download(&reference).await?;
    let destination = destination.to_string();
    tokio::task::spawn_blocking(move || crate::save::write_new(&destination, &blob.bytes))
        .await
        .map_err(|error| error.to_string())?
}
#[async_trait::async_trait]
impl Session for ScopedRemote {
    fn turn_settled(&self) -> Option<bool> {
        Some(self.turn_settled)
    }
    fn catalog(&self) -> crate::Catalog { self.catalog.clone() }
    fn location(&self) -> String { self.entry.title.clone() }
    fn selected(&self) -> Option<misa_proto::directory::Entry> { Some(self.entry.clone()) }
    async fn next_presentation(&mut self) -> Result<Option<Presentation>, String> {
        loop {
            self.collect();
            self.collect_operation()?;
            if let Some(update) = self.updates.pop_front() {
                return Ok(Some(update));
            }
            tokio::select! {
                replacement=self.replacements.join_next(), if !self.replacements.is_empty()=>{
                    match replacement.unwrap().unwrap_or_else(|error|Err(error.to_string())) {Ok(replacement)=>self.replace_presentation(replacement),Err(error)=>self.updates.push_back(Presentation::Reply(SessionReply::Notice(error)))}
                },
                changed = self.observation.changed() => changed.map_err(|fault| fault.message)?,
                reply = self.pending.join_next(), if !self.pending.is_empty() => {
                    let (reply, operation) = reply.unwrap().unwrap_or_else(|error| (SessionReply::Notice(error.to_string()), None));
                    if let SessionReply::Request { id, generation, model } = &reply {
                        if self.request_versions.get(id) != Some(generation) || model.as_ref().is_some_and(|model| model.generation != *generation) { continue; }
                    }
                    if let Some(operation) = operation { self.track_operation(operation); self.turn_settled = false; }
                    return Ok(Some(Presentation::Reply(reply)));
                },
                changed = self.operations.changed() => changed.map_err(|error|error.message)?,
            }
        }
    }
    async fn next(&mut self) -> Result<Option<Node>, String> {
        loop {
            if let Some(error) = self.turn_failure.take() {
                return Err(error);
            }
            match self.next_presentation().await? {
                Some(Presentation::Snapshot(tree) | Presentation::TurnOutput(tree)) => {
                    return Ok(Some(tree));
                }
                Some(Presentation::Reply(SessionReply::Report(tree))) => return Ok(Some(tree)),
                Some(Presentation::Document(misa_client::document::Update::Unavailable(fault))) => {
                    return Err(fault.message);
                }
                Some(Presentation::Document(misa_client::document::Update::Status(
                    misa_protocol::observation::Status::Closed(fault),
                ))) => return Err(format!("Session closed: {fault:?}")),
                Some(Presentation::Documents(documents)) => {
                    for (id, update) in documents {
                        if id.is_empty() {
                            self.updates.push_front(Presentation::Document(update));
                        }
                    }
                }
                Some(Presentation::Document(_)) => {
                    // Pipeline output belongs to the accepted operation, not to
                    // unrelated clients' conversation updates.
                    if let Some(tree)=self.operations.latest_document() {
                        return Ok(Some(tree));
                    } else if self.turn_settled
                        && !self
                            .updates
                            .iter()
                            .any(|update| matches!(update, Presentation::TurnOutput(_)))
                    {
                        return Ok(Some(Node::section("session").id("session")));
                    }
                }
                Some(_) => {}
                None => return Ok(None),
            }
        }
    }
    async fn send(&mut self, intent: Intent) -> Result<(), String> {
        if let Intent::Command { name, args } = &intent
            && name == "presentation"
        {
            let replacement = self.prepare_presentation(args)?.await?;
            self.replace_presentation(replacement);
            return Ok(());
        }
        let prepared = self.prepare(intent)?;
        let (notice, operation) = execute(&self.daemon.client, &self.interaction, prepared).await?;
        if let Some(operation) = operation {
            self.track_operation(operation);
            self.turn_settled = false;
            self.turn_failure = None;
        }
        if let Some(notice) = notice {
            self.updates.push_back(Presentation::Reply(notice));
        }
        Ok(())
    }
    async fn request(&mut self, request: SessionRequest) -> Option<SessionReply> {
        if let SessionRequest::Intent(Intent::Action { action, fields, .. }) = &request
            && fields.is_empty()
            && self.interaction.interface.actions.get(action).is_some_and(|entry|!entry.binding.inputs.is_empty())
        {
            return Some(match misa_client::form::Form::action(&self.interaction.interface,action) {
                Ok(form)=>SessionReply::Form(form),Err(fault)=>SessionReply::Notice(fault.message),
            });
        }

        if let SessionRequest::Intent(Intent::Command { name, args }) = &request
            && name == "presentation"
        {
            if !self.replacements.is_empty() {
                return Some(SessionReply::Notice(
                    "Presentation change still pending".into(),
                ));
            }
            return match self.prepare_presentation(args) {
                Ok(work) => {
                    self.replacements.spawn(work);
                    None
                }
                Err(error) => Some(SessionReply::Notice(error)),
            };
        }
        if matches!(request, SessionRequest::RefreshRequests) {
            self.request_versions.clear();
            return None;
        }
        if self.pending.len() + self.operations.len() >= 32 {
            let error = "Too many pending requests".to_string();
            return Some(match request {
                SessionRequest::Intent(intent) => {
                    let draft = match intent {
                        Intent::Prompt { text, attachments }
                        | Intent::Interrupt { text, attachments } => Some((text, attachments)),
                        _ => None,
                    };
                    SessionReply::Sent {
                        draft,
                        result: Err(error),
                    }
                }
                SessionRequest::Upload { generation, .. } => SessionReply::Uploaded {
                    generation,
                    result: Err(error),
                },
                SessionRequest::Complete { source, prefix } => SessionReply::Complete {
                    source,
                    prefix,
                    result: Err(error),
                },
                SessionRequest::Save { .. } | SessionRequest::Invoke { .. } => {
                    SessionReply::Notice(error)
                }
                SessionRequest::RefreshRequests => unreachable!(),
            });
        }
        let client = self.daemon.client.clone();
        let interaction = self.interaction.clone();
        match request {
            SessionRequest::RefreshRequests => unreachable!(),
            SessionRequest::Invoke { command, input } => {
                let prepared = match interaction.invoke(&command, input) {
                    Ok(prepared) => prepared,
                    Err(fault) => return Some(SessionReply::Notice(fault.message)),
                };
                self.pending.spawn(async move {
                    match execute(&client, &interaction, prepared).await {
                        Ok((reply, observation)) => (
                            reply.unwrap_or_else(|| SessionReply::Notice("Submitted".into())),
                            observation,
                        ),
                        Err(error) => (SessionReply::Notice(error), None),
                    }
                });
            }
            SessionRequest::Intent(intent) => {
                let draft = match &intent {
                    Intent::Prompt { text, attachments }
                    | Intent::Interrupt { text, attachments } => {
                        Some((text.clone(), attachments.clone()))
                    }
                    _ => None,
                };
                let prepared = match self.prepare(intent) {
                    Ok(prepared) => prepared,
                    Err(error) => {
                        return Some(SessionReply::Sent {
                            draft,
                            result: Err(error),
                        });
                    }
                };
                self.pending.spawn(async move {
                    match execute(&client, &interaction, prepared).await {
                        Ok((notice, operation)) => (
                            match notice {
                                Some(notice) => notice,
                                None => SessionReply::Sent {
                                    draft,
                                    result: Ok(()),
                                },
                            },
                            operation,
                        ),
                        Err(error) => (
                            SessionReply::Sent {
                                draft,
                                result: Err(error),
                            },
                            None,
                        ),
                    }
                });
            }
            SessionRequest::Complete { source, prefix } => {
                self.pending.spawn(async move {
                    let result = complete(&client, &interaction, &source, &prefix).await;
                    (
                        SessionReply::Complete {
                            source,
                            prefix,
                            result,
                        },
                        None,
                    )
                });
            }
            SessionRequest::Upload {
                generation,
                bytes,
                media,
            } => {
                let blobs = self.daemon.blobs.clone();
                self.pending.spawn(async move {
                    (
                        SessionReply::Uploaded {
                            generation,
                            result: blobs.share(bytes, Some(&media)).await,
                        },
                        None,
                    )
                });
            }
            SessionRequest::Save { node, destination } => {
                let daemon = self.daemon.clone();
                self.pending.spawn(async move {
                    let result = save(&daemon, &interaction, &node, &destination).await;
                    (
                        SessionReply::Notice(match result {
                            Ok(()) => format!("Saved {destination}"),
                            Err(error) => error,
                        }),
                        None,
                    )
                });
            }
        }
        None
    }
    async fn complete(
        &mut self,
        source: &str,
        prefix: &str,
    ) -> Result<(Vec<Choice>, bool), String> {
        complete(&self.daemon.client, &self.interaction, source, prefix).await
    }
    async fn upload(
        &mut self,
        bytes: Vec<u8>,
        media: &str,
    ) -> Result<misa_proto::view::BlobRef, String> {
        self.daemon.blobs.share(bytes, Some(media)).await
    }
    async fn save_attachment(&mut self, node: &str, destination: &str) -> Result<(), String> {
        save(&self.daemon, &self.interaction, node, destination).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_protocol::invocation::CommandOwner;

    #[tokio::test]
    async fn real_scoped_daemon_drives_documents_catalogs_and_finite_completion() {
        let runtime = misa_session::Runtime::start(
            "scoped",
            "Scoped",
            None,
            Arc::new(misa_kernel::LocalKernel::new(
                misa_kernel::ScriptedProvider::new([
                    misa_kernel::Turn::say("reply"),
                    misa_kernel::Turn::say("reply"),
                    misa_kernel::Turn::say("reply"),
                ]),
            )),
            "scripted",
            "scripted-1",
            Value::Null,
        );
        let directory = misa_daemon::directory::Directory::new("test-daemon").unwrap();
        let archive=misa_daemon::archive::Store::new(Arc::new(misa_kernel::MemoryStore::default()));
        misa_kernel::Store::append(archive.as_ref(),"stored-conversation","message",&Value::str("Archived prompt"),1).unwrap();
        directory.install_archive(archive).await.unwrap();
        directory.insert(runtime).unwrap();
        directory.install_factory(Arc::new(|_,spec|Box::pin(async move {
            Ok(misa_session::Runtime::prepare_with(spec.id,spec.title,spec.conversation,Arc::new(misa_kernel::LocalKernel::new(misa_kernel::ScriptedProvider::new([]))),spec.provider.unwrap_or_else(||"scripted".into()),spec.model.unwrap_or_else(||"scripted-1".into()),Value::Null,misa_session::Contribution::default()))
        }))).unwrap();
        let server = misa_transport::iroh::bind(None, false).await.unwrap();
        let endpoint = misa_transport::iroh::bind(None, false).await.unwrap();
        let router = iroh::protocol::Router::builder(server.clone())
            .accept(
                misa_proto::scoped::ALPN,
                misa_transport::scoped_server::Handler {
                    daemon: server.id().to_string(),
                    scope: directory.scope(),
                    resolver: Arc::new(misa_daemon::directory::Routes(directory)),
                    admission: Arc::new(misa_transport::admission::Admission::open()),
                },
            )
            .spawn();
        let daemons = Arc::new(misa_client::daemons::Daemons::new(
            endpoint.clone(),
            misa_proto::ClientInfo::new("tui-test", "1"),
        ));
        let daemon = daemons
            .connect(
                misa_transport::iroh::address_of(&misa_transport::iroh::node_of(&server)).unwrap(),
            )
            .await
            .unwrap();
        let mut updates = daemon.watch();
        let entry = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(entry) = daemon.sessions().unwrap().sessions.into_iter().next() {
                    break entry;
                }
                updates.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        let mut remote = ScopedRemote::on(daemon, entry).await.unwrap();
        assert!(remote.presentations.contains_key("status"));
        assert!(
            remote
                .catalog()
                .commands
                .iter()
                .any(|command| command.id == "model")
        );
        let document = tokio::time::timeout(Duration::from_secs(5), remote.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(!document.id.is_empty());
        let (models, _) = remote.complete("models", "scripted").await.unwrap();
        assert!(models.iter().any(|choice| choice.value == "scripted-1"));
        let preference_path = std::env::temp_dir().join(format!(
            "misa-presentation-test-{}.json",
            crate::test_unique_id()
        ));
        remote.preference_path = preference_path.clone();
        misa_client::preference_store::Store::new(preference_path.clone()).update("another-window".into(),misa_client::composition::Choice::Hidden).unwrap();
        for (variant, visible) in [("hide", false), ("auto", true)] {
            let old = remote.observation.id();
            assert!(
                remote
                    .request(SessionRequest::Intent(Intent::Command {
                        name: "presentation".into(),
                        args: Value::map([
                            ("presentation", Value::str("status")),
                            ("variant", Value::str(variant))
                        ])
                    }))
                    .await
                    .is_none()
            );
            assert_eq!(
                remote.observation.id(),
                old,
                "pending selection must not mutate the active subscription"
            );
            tokio::time::timeout(Duration::from_secs(5), async {
                while !remote.replacements.is_empty() {
                    remote.next_presentation().await.unwrap();
                }
            })
            .await
            .unwrap();
            assert_eq!(remote.presentations.contains_key("status"), visible);
            assert_ne!(remote.observation.id(), old);
        }
        let persisted: misa_client::composition::Preferences =
            serde_json::from_str(&std::fs::read_to_string(&preference_path).unwrap()).unwrap();
        assert_eq!(
            persisted.0.get("status"),
            Some(&misa_client::composition::Choice::Auto)
        );
        assert_eq!(persisted.0.get("another-window"),Some(&misa_client::composition::Choice::Hidden),"a stale remote must not overwrite another process choice");
        std::fs::remove_file(preference_path).unwrap();
        let old = remote.observation.id();
        let blocked = std::env::temp_dir().join(format!(
            "misa-presentation-blocked-{}",
            crate::test_unique_id()
        ));
        std::fs::write(&blocked, b"not a directory").unwrap();
        remote.preference_path = blocked.join("preferences.json");
        let change = || {
            SessionRequest::Intent(Intent::Command {
                name: "presentation".into(),
                args: Value::map([
                    ("presentation", Value::str("status")),
                    ("variant", Value::str("hide")),
                ]),
            })
        };
        assert!(remote.request(change()).await.is_none());
        assert!(
            matches!(
                remote.request(change()).await,
                Some(SessionReply::Notice(_))
            ),
            "changes must have bounded admission"
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while !remote.replacements.is_empty() {
                remote.next_presentation().await.unwrap();
            }
        })
        .await
        .unwrap();
        assert_eq!(
            remote.observation.id(),
            old,
            "failed persistence must preserve the active selection"
        );
        assert!(remote.presentations.contains_key("status"));
        std::fs::remove_file(blocked).unwrap();
        assert!(
            remote
                .request(SessionRequest::Intent(Intent::Interrupt {
                    text: " ".into(),
                    attachments: vec![]
                }))
                .await
                .is_none()
        );
        let rejected = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(Presentation::Reply(reply @ SessionReply::Sent { .. })) =
                    remote.next_presentation().await.unwrap()
                {
                    break reply;
                }
            }
        })
        .await
        .unwrap();
        assert!(
            matches!(rejected, SessionReply::Sent { draft: Some((text, attachments)), result: Err(_) } if text == " " && attachments.is_empty())
        );
        assert!(
            remote
                .send(Intent::Command {
                    name: "no-installed-command".into(),
                    args: Value::map([])
                })
                .await
                .is_err()
        );
        remote
            .send(Intent::Command {
                name: "status".into(),
                args: Value::map([]),
            })
            .await
            .unwrap();
        assert!(
            remote
                .updates
                .iter()
                .any(|update| matches!(update, Presentation::Reply(SessionReply::Report(_))))
        );
        let report = remote.next().await.unwrap().unwrap();
        assert!(
            !misa_render::to_plain(&misa_render::render(
                &report,
                &misa_render::Theme::plain(),
                100,
            ))
            .is_empty()
        );
        remote
            .send(Intent::Prompt {
                text: "hello".into(),
                attachments: vec![],
            })
            .await
            .unwrap();
        assert_eq!(remote.turn_settled(), Some(false));
        let answer = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let tree = remote.next().await.unwrap().unwrap();
                if remote.turn_settled() == Some(true) {
                    break misa_render::to_plain(&misa_render::render(
                        &tree,
                        &misa_render::Theme::plain(),
                        100,
                    ));
                }
            }
        })
        .await
        .unwrap();
        assert!(answer.contains("reply"), "{answer}");
        let entry = remote.daemon.sessions().unwrap().sessions.remove(0);
        let mut remote = ScopedRemote::on(remote.daemon.clone(), entry)
            .await
            .unwrap();
        // EOF drains each accepted operation, even when consecutive outputs are
        // identical; it does not wait for or print another client's session work.
        let (input, receiver) = tokio::sync::mpsc::channel(2);
        input.send(Ok("one".into())).await.unwrap();
        input.send(Ok("two".into())).await.unwrap();
        drop(input);
        let mut output = Vec::new();
        let mut errors = Vec::new();
        tokio::time::timeout(
            Duration::from_secs(5),
            crate::print::run(&mut remote, receiver, &mut output, &mut errors),
        )
        .await
        .unwrap()
        .unwrap();
        let output = String::from_utf8(output).unwrap();
        assert_eq!(
            output.matches("reply").count(),
            2,
            "output={output:?} errors={errors:?}"
        );
        for text in ["overlap one", "overlap two"] {
            remote
                .send(Intent::Prompt {
                    text: text.into(),
                    attachments: vec![],
                })
                .await
                .unwrap();
        }
        assert_eq!(
            remote.operations.len(),
            2,
            "a second admission must retain the first correlation"
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut finished = 0;
            while finished < 2 {
                if matches!(
                    remote.next_presentation().await.unwrap(),
                    Some(Presentation::TurnOutput(_))
                ) {
                    finished += 1;
                }
            }
        })
        .await
        .expect("both accepted operations produce terminal results");
        assert!(remote.operations.is_empty());
        remote
            .send(Intent::Command {
                name: "login".into(),
                args: Value::map([("provider", Value::str("anthropic"))]),
            })
            .await
            .unwrap();
        let model = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(Presentation::Reply(SessionReply::Request {
                    model: Some(model), ..
                })) = remote.next_presentation().await.unwrap()
                {
                    break model;
                }
            }
        })
        .await
        .unwrap();
        assert!(model.input.as_ref().unwrap().secret);
        let Prepared::Invoke { command, input } = model
            .prepare(
                "submit",
                &BTreeMap::from([("value".into(), Value::str("test-only-private-key"))]),
                &remote.interaction.interface,
            )
            .unwrap()
        else {
            panic!()
        };
        assert!(
            remote
                .request(SessionRequest::Invoke {
                    command: command.id,
                    input
                })
                .await
                .is_none()
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Some(Presentation::Reply(SessionReply::Request {
                    id,
                    model: None,
                    generation,
                })) = remote.next_presentation().await.unwrap()
                {
                    if id == model.id && generation > model.generation {
                        break;
                    }
                }
            }
        })
        .await
        .unwrap();
        let overview=misa_client::overview::Overview::open(&remote.daemon.client).await.unwrap();
        let mut overview_changes=overview.watch();
        tokio::time::timeout(Duration::from_secs(5),async {
            loop {let snapshot=overview.snapshot().unwrap();if matches!(snapshot.status,misa_protocol::observation::Status::Current) {
                assert!(snapshot.unavailable.is_empty(),"optional overview contracts must decode: {:?}",snapshot.unavailable);
                assert!(snapshot.rows.iter().any(|row|row.scope==remote.selection.scope));
                assert!(snapshot.sessions.iter().any(|entry|entry.scope()==remote.selection.scope));break;
            }overview_changes.changed().await.unwrap();}
        }).await.unwrap();
        drop(overview);
        drop(remote);
        let mut workspace = crate::workspace::Workspace::using(daemons.clone()).await;
        assert!(workspace.request(SessionRequest::Complete{source:"client.conversations".into(),prefix:"Archived".into()}).await.is_none());
        tokio::time::timeout(Duration::from_secs(5),async {loop {if let Some(Presentation::Reply(SessionReply::Complete{source,result,..}))=workspace.next_presentation().await.unwrap() && source=="client.conversations" {let (items,truncated)=result.unwrap();assert!(!truncated);assert_eq!(items[0].value,"stored-conversation");break;}}}).await.unwrap();
        let choose = || Intent::Command {
            name: "session".into(),
            args: Value::map([("session", Value::str("scoped"))]),
        };
        workspace.send(choose()).await.unwrap();
        assert!(
            workspace
                .request(SessionRequest::Intent(Intent::Prompt {
                    text: "parked result".into(),
                    attachments: vec![]
                }))
                .await
                .is_none()
        );
        workspace
            .send(Intent::Command {
                name: "daemons".into(),
                args: Value::map([]),
            })
            .await
            .unwrap();
        workspace.send(choose()).await.unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if matches!(
                    workspace.next_presentation().await.unwrap(),
                    Some(Presentation::TurnOutput(_))
                ) {
                    break;
                }
            }
        })
        .await
        .expect("parked accepted result was retained");
        workspace.send(Intent::Command{name:"new".into(),args:Value::map([("id",Value::str("created-from-tui")),("title",Value::str("Local create"))])}).await.unwrap();
        assert_eq!(workspace.selected().unwrap().id,"created-from-tui");
        workspace.send(Intent::Command{name:"close".into(),args:Value::map([])}).await.unwrap();
        assert!(workspace.selected().is_none());
        workspace.send(Intent::Command{name:"resume".into(),args:Value::map([("id",Value::str("resumed-from-tui")),("conversation",Value::str("stored-conversation"))])}).await.unwrap();
        assert_eq!(workspace.selected().unwrap().id,"resumed-from-tui");
        workspace.send(Intent::Command{name:"close".into(),args:Value::map([])}).await.unwrap();
        drop(workspace);
        daemons.disconnect(&server.id().to_string()).await;
        router.shutdown().await.unwrap();
        endpoint.close().await;
    }
}
