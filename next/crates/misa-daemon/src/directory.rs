//! A coherent directory of the latest received session summaries.
//!
//! Directory publications are not simultaneous snapshots of every session. Each
//! row retains its source publication and availability, without opening transcripts.
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};

use misa_proto::{
    Fault, Query,
    invocation::{Command, Invocation},
    observation::*,
};
use misa_protocol::{
    invocation::{CallContext, CommandOwner, Execution},
    owner::{Owner, Resolver},
};
use misa_session::Runtime;
use misa_value::Value;
use tokio::sync::watch;

const MAX_SESSIONS: usize = 256;

struct Entry {
    runtime: Arc<Runtime>,
    source: u64,
    summary: Value,
    availability: &'static str,
    task: tokio::task::AbortHandle,
}
impl Drop for Entry {
    fn drop(&mut self) {
        self.task.abort();
    }
}

struct State {
    connections: BTreeMap<u64, Value>,
    archive: Result<Value,Fault>,
    archive_installed: bool,
    entries: BTreeMap<String, Entry>,
    position: u64,
    value: Value,
    changed: watch::Sender<u64>,
}

/// A work mutation and its directory publication position share one lock boundary.
pub(crate) struct WorkEdit<'a> {
    state: std::sync::MutexGuard<'a, State>,
    work: std::sync::MutexGuard<'a, crate::delegation::Work>,
    dirty: bool,
}
impl std::ops::Deref for WorkEdit<'_> {
    type Target = crate::delegation::Work;
    fn deref(&self) -> &Self::Target {
        &self.work
    }
}
impl std::ops::DerefMut for WorkEdit<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.dirty = true;
        &mut self.work
    }
}
impl Drop for WorkEdit<'_> {
    fn drop(&mut self) {
        if self.dirty {
            publish(&mut self.state)
        }
    }
}

pub struct Directory {
    closed: std::sync::atomic::AtomicBool,
    scope: Scope,
    state: Mutex<State>,
    me: Weak<Directory>,
    factory: Mutex<Option<Arc<crate::lifecycle::Factory>>>,
    lifecycle: tokio::sync::Mutex<()>,
    membership: Mutex<Arc<dyn crate::membership::MembershipStore>>,
    desired: Mutex<BTreeMap<String, crate::lifecycle::SessionSpec>>,
    pub(crate) work: Mutex<crate::delegation::Work>,
    pub(crate) work_path: Mutex<Option<std::path::PathBuf>>,
    pub(crate) work_writes: tokio::sync::Mutex<()>,
    pub(crate) supervisors: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl Directory {
    pub async fn install_archive(self:&Arc<Self>,store:Arc<crate::archive::Store>)->Result<(),Fault> {
        use misa_kernel::Store as _;
        {let mut state=self.state.lock().unwrap();if state.archive_installed{return Err(Fault::new("composition","Archive store already installed"));}state.archive_installed=true;}
        let mut changes=store.watch();let mut lifecycle=self.watch_work();let weak=Arc::downgrade(self);
        let initial=store.clone();let result=tokio::task::spawn_blocking(move||initial.conversations()).await.map_err(|error|Fault::new("archive",error.to_string()))?.map(|rows|Value::list(rows.iter().map(misa_kernel::Conversation::to_value))).map_err(|message|Fault::new("archive",message));
        {let mut state=self.state.lock().unwrap();state.archive=result;publish(&mut state);}
        let task=tokio::spawn(async move {loop {
            tokio::select! {changed=changes.changed()=>if changed.is_err(){return;},changed=lifecycle.changed()=>{if changed.is_err() || weak.upgrade().is_none_or(|directory|directory.is_closed()){return;}continue;}}
            let source=store.clone();let result=tokio::task::spawn_blocking(move||source.conversations()).await.map_err(|error|Fault::new("archive",error.to_string())).and_then(|value|value.map_err(|message|Fault::new("archive",message))).map(|rows|Value::list(rows.iter().map(misa_kernel::Conversation::to_value)));
            let Some(directory)=weak.upgrade() else{return;};if directory.is_closed(){return;}let mut state=directory.state.lock().unwrap();if state.archive!=result {state.archive=result;publish(&mut state);}
        }});self.supervisors.lock().unwrap().push(task);Ok(())
    }
    pub fn fresh() -> Result<Arc<Self>, Fault> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes)
            .map_err(|_| Fault::new("randomness", "Cannot create daemon incarnation"))?;
        Self::new(
            bytes
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
        )
    }

    pub fn new(incarnation: impl Into<String>) -> Result<Arc<Self>, Fault> {
        let scope = Scope {
            id: ScopeId::Daemon,
            incarnation: incarnation.into(),
        };
        scope.validate()?;
        let (changed, _) = watch::channel(0);
        Ok(Arc::new_cyclic(|me| Self {
            closed: std::sync::atomic::AtomicBool::new(false),
            scope,
            me: me.clone(),
            factory: Mutex::new(None),
            lifecycle: tokio::sync::Mutex::new(()),
            membership: Mutex::new(Arc::new(crate::membership::Memory::default())),
            desired: Mutex::new(BTreeMap::new()),
            work: Mutex::new(Default::default()),
            work_path: Mutex::new(None),
            work_writes: tokio::sync::Mutex::new(()),
            supervisors: Mutex::new(Vec::new()),
            state: Mutex::new(State {
                connections: BTreeMap::new(),
                archive: Err(Fault::unsupported("Archive store is not installed")),
                archive_installed:false,
                entries: BTreeMap::new(),
                position: 0,
                value: Value::list([]),
                changed,
            }),
        }))
    }

    pub(crate) fn scope_value(&self) -> Scope {
        self.scope.clone()
    }
    pub fn install_factory(&self, factory: Arc<crate::lifecycle::Factory>) -> Result<(), Fault> {
        let mut installed = self.factory.lock().expect("factory lock is never poisoned");
        if installed.is_some() {
            return Err(Fault::new(
                "composition",
                "Session factory already installed",
            ));
        }
        *installed = Some(factory);
        Ok(())
    }
    pub async fn install_membership(
        &self,
        storage: Arc<dyn crate::membership::MembershipStore>,
    ) -> Result<(), Fault> {
        let _guard = self.lifecycle.lock().await;
        let reader = storage.clone();
        let records = tokio::task::spawn_blocking(move || reader.load())
            .await
            .map_err(|error| Fault::new("membership_storage", error.to_string()))??;
        let mut desired = BTreeMap::new();
        for record in records {
            if desired.insert(record.id.clone(), record).is_some() {
                return Err(Fault::new(
                    "membership_storage",
                    "Duplicate desired session",
                ));
            }
        }
        *self.membership.lock().unwrap() = storage;
        *self.desired.lock().unwrap() = desired;
        Ok(())
    }
    async fn save_desired(
        &self,
        desired: &BTreeMap<String, crate::lifecycle::SessionSpec>,
    ) -> Result<(), Fault> {
        let storage = self.membership.lock().unwrap().clone();
        let records = desired.values().cloned().collect::<Vec<_>>();
        tokio::task::spawn_blocking(move || storage.save(&records))
            .await
            .map_err(|error| Fault::new("membership_storage", error.to_string()))?
    }
    pub(crate) async fn forget_bound_children(&self, ids: &[String]) -> Result<(), Fault> {
        let _guard = self.lifecycle.lock().await;
        let mut desired = self.desired.lock().unwrap().clone();
        for id in ids {
            desired.remove(id);
        }
        self.save_desired(&desired).await?;
        *self.desired.lock().unwrap() = desired;
        Ok(())
    }
    pub async fn restore(&self, context: &CallContext) -> Vec<(String, Fault)> {
        let desired = self
            .desired
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut failed = Vec::new();
        for mut spec in desired {
            spec.recovering = true;
            spec.conversation = Some(spec.conversation.clone().unwrap_or_else(|| spec.id.clone()));
            if let Err(fault) = self.open(context, spec.clone()).await {
                failed.push((spec.id, fault));
            }
        }
        failed
    }
    pub async fn open(
        &self,
        context: &CallContext,
        mut spec: crate::lifecycle::SessionSpec,
    ) -> Result<Arc<Runtime>, Fault> {
        let _guard = self.lifecycle.lock().await;
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Fault::new("closed_scope", "Daemon is shutting down"));
        }
        if spec.id.is_empty()
            || spec.id.len() > 128
            || spec.title.len() > 1024
            || spec
                .conversation
                .as_ref()
                .is_some_and(|v| v.is_empty() || v.len() > 256)
            || spec.provider.as_ref().is_some_and(|v| v.len() > 256)
            || spec.model.as_ref().is_some_and(|v| v.len() > 256)
        {
            return Err(Fault::new(
                "invalid_session",
                "Session metadata exceeds its declared bounds",
            ));
        }
        {
            let state = self.state.lock().unwrap();
            if state.entries.contains_key(&spec.id) {
                return Err(Fault::new(
                    "duplicate_session",
                    "Session identity is already open",
                ));
            }
            if state.entries.len() >= MAX_SESSIONS {
                return Err(Fault::new("busy", "Daemon session capacity reached"));
            }
            let conversation = spec.conversation.as_deref().unwrap_or(&spec.id);
            if state.entries.values().any(|entry| {
                entry
                    .runtime
                    .metadata()
                    .conversation
                    .as_deref()
                    .unwrap_or(entry.runtime.id())
                    == conversation
            }) {
                return Err(Fault::new(
                    "conversation_busy",
                    "Conversation already has an active writer",
                ));
            }
        }
        let factory = self
            .factory
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| Fault::new("unavailable", "Session creation is not configured"))?;
        let mut lifecycle_changed = self.watch_work();
        let prepared = factory(context.clone(), spec.clone());
        tokio::pin!(prepared);
        let runtime = loop {
            if self.is_closed() {
                return Err(Fault::new(
                    "closed_scope",
                    "Daemon stopped session creation",
                ));
            }
            tokio::select! {
                result=&mut prepared=>break result?,
                _=lifecycle_changed.changed()=>{},
            }
        };
        if runtime.is_started() {
            runtime.shutdown_complete().await;
            return Err(Fault::new(
                "composition",
                "Lifecycle factory must return a dormant owner",
            ));
        }
        if runtime.id() != spec.id {
            runtime.shutdown_complete().await;
            return Err(Fault::new(
                "composition",
                "Factory returned a different session identity",
            ));
        }
        spec.provider = Some(runtime.provider().into());
        spec.model = Some(runtime.model().into());
        spec.recovering = false;
        let mut desired = self.desired.lock().unwrap().clone();
        desired.insert(spec.id.clone(), spec);
        if let Err(fault) = self.save_desired(&desired).await {
            runtime.shutdown();
            return Err(fault);
        }
        if let Err(fault) = self.insert_inner(runtime.clone()) {
            runtime.shutdown();
            let previous = self.desired.lock().unwrap().clone();
            self.save_desired(&previous).await?;
            return Err(fault);
        }
        *self.desired.lock().unwrap() = desired;
        runtime.activate();
        Ok(runtime)
    }
    pub async fn close(&self, scope: &Scope) -> Result<bool, Fault> {
        let _guard = self.lifecycle.lock().await;
        let Some(runtime) = self.session(scope) else {
            return Ok(false);
        };
        let mut desired = self.desired.lock().unwrap().clone();
        desired.remove(runtime.id());
        self.save_desired(&desired).await?;
        *self.desired.lock().unwrap() = desired;
        runtime.shutdown_complete().await;
        Ok(self.remove(scope))
    }
    /// Process shutdown preserves desired membership for the next incarnation.
    pub fn shutdown(&self) {
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        publish(&mut self.state.lock().unwrap());
        let runtimes = self
            .state
            .lock()
            .unwrap()
            .entries
            .values()
            .map(|entry| entry.runtime.clone())
            .collect::<Vec<_>>();
        for runtime in runtimes {
            runtime.shutdown();
            self.remove(&runtime.scope());
        }
    }
    pub(crate) fn is_closed(&self) -> bool {
        self.closed.load(std::sync::atomic::Ordering::Acquire)
    }
    /// Fence admission, then await all owner capabilities and child supervisors.
    pub async fn shutdown_complete(&self) {
        self.closed
            .store(true, std::sync::atomic::Ordering::Release);
        publish(&mut self.state.lock().unwrap());
        let guard = self.lifecycle.lock().await;
        let runtimes = self.sessions();
        for runtime in &runtimes {
            runtime.shutdown();
            self.remove(&runtime.scope());
        }
        drop(guard);
        for runtime in runtimes {
            runtime.shutdown_complete().await;
        }
        let supervisors = std::mem::take(&mut *self.supervisors.lock().unwrap());
        for supervisor in supervisors {
            let _ = supervisor.await;
        }
    }
    pub fn sessions(&self) -> Vec<Arc<Runtime>> {
        self.state
            .lock()
            .unwrap()
            .entries
            .values()
            .map(|entry| entry.runtime.clone())
            .collect()
    }
    pub(crate) fn edit_work(&self) -> WorkEdit<'_> {
        WorkEdit {
            state: self.state.lock().unwrap(),
            work: self.work.lock().unwrap(),
            dirty: false,
        }
    }
    pub(crate) fn watch_work(&self) -> watch::Receiver<u64> {
        self.state.lock().unwrap().changed.subscribe()
    }
    fn definitions(&self) -> Vec<misa_proto::query::Definition> {
        let mut definitions = vec![
            misa_proto::directory::definition(),
            misa_proto::invocation::catalog_definition(),
            misa_proto::query::catalog_definition(),
        ];
        definitions.extend(crate::delegation::definitions());
        definitions.extend(crate::archive::definitions());
        definitions.push(connections_definition());
        definitions
    }
    fn member(&self, state: &State, member: &Member) -> Result<Content,Fault> {
        if matches!(member.query.id.as_str(),"daemon.conversations"|"completion.catalog"|"completion.search") {return match crate::archive::read(&state.archive,&member.query) {Ok(value)=>Ok(Content::Value(value)),Err(fault) if member.optional=>Ok(Content::Unavailable(fault)),Err(fault)=>Err(fault)};}
        Ok(Content::Value(match member.query.id.as_str() {
            "daemon.connections" => Value::list(state.connections.values().cloned()),
            misa_proto::invocation::CATALOG => {
                crate::lifecycle::encode(&crate::lifecycle::commands())
            }
            misa_proto::query::CATALOG => crate::lifecycle::encode(&self.definitions()),
            "daemon.work" => self.work.lock().unwrap().value(),
            "daemon.overview" => self.work.lock().unwrap().overview(&state.value),
            "operation.result" => self.work.lock().unwrap().result(
                member
                    .query
                    .args
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or_default(),
            ),
            _ => state.value.clone(),
        }))
    }

    pub fn selection(&self) -> Selection {
        misa_proto::directory::selection(self.scope.clone())
    }

    /// Installing a live runtime subscribes only to its cheap domain summary.
    /// Duplicate identities are refused; recreating a removed label uses a new
    /// runtime incarnation and cannot inherit an old observation's authority.
    pub fn insert(&self, runtime: Arc<Runtime>) -> Result<(), Fault> {
        let _guard = self
            .lifecycle
            .try_lock()
            .map_err(|_| Fault::new("busy", "Session lifecycle mutation is pending"))?;
        self.insert_inner(runtime)
    }
    fn insert_inner(&self, runtime: Arc<Runtime>) -> Result<(), Fault> {
        let summary = runtime
            .query_exports()
            .into_iter()
            .find(|definition| definition.id == misa_session::observation::SUMMARY)
            .ok_or_else(|| Fault::query("Session has no summary export"))?;
        let selection = Selection {
            scope: runtime.scope(),
            members: BTreeMap::from([(
                "summary".into(),
                Member {
                    query: Query::new(summary.id),
                    contract: summary.contract,
                    encoding: Encoding::Value,
                    optional: false,
                },
            )]),
        };
        let (mut observed, initial) = Runtime::observe(
            &runtime,
            Handle {
                id: 0,
                generation: 0,
            },
            selection,
            None,
        )?;
        let Publication::Snapshot { snapshot, .. } = initial else {
            return Err(Fault::query("Session summary is unavailable"));
        };
        let Some(Content::Value(summary)) = snapshot.members.get("summary") else {
            return Err(Fault::query("Session summary is invalid"));
        };
        let mut state = self
            .state
            .lock()
            .expect("directory state is never poisoned");
        if self.closed.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Fault::new("closed_scope", "Daemon is shutting down"));
        }
        let id = runtime.id().to_owned();
        if state.entries.contains_key(&id) {
            return Err(Fault::new(
                "duplicate_session",
                "Session identity is already open",
            ));
        }
        if state.entries.len() >= MAX_SESSIONS {
            return Err(Fault::new("busy", "Daemon session capacity reached"));
        }
        let weak = self.me.clone();
        let source_scope = runtime.scope();
        let task_id = id.clone();
        let task = tokio::spawn(async move {
            loop {
                if observed.changed().await.is_err() {
                    return;
                }
                let Some(publication) = observed.poll() else {
                    continue;
                };
                let Some(directory) = weak.upgrade() else {
                    return;
                };
                directory.receive(&task_id, &source_scope, publication);
            }
        })
        .abort_handle();
        state.entries.insert(
            id,
            Entry {
                runtime,
                source: snapshot.position,
                summary: summary.clone(),
                availability: "current",
                task,
            },
        );
        publish(&mut state);
        Ok(())
    }

    /// Removal releases summary interest; the lifecycle owner separately decides
    /// how to stop or preserve the runtime. This is not a session-close command.
    pub fn remove(&self, scope: &Scope) -> bool {
        let ScopeId::Session { id } = &scope.id else {
            return false;
        };
        let mut state = self
            .state
            .lock()
            .expect("directory state is never poisoned");
        if !state
            .entries
            .get(id)
            .is_some_and(|entry| entry.runtime.scope() == *scope)
        {
            return false;
        }
        state.entries.remove(id);
        publish(&mut state);
        true
    }

    pub fn session(&self, scope: &Scope) -> Option<Arc<Runtime>> {
        let ScopeId::Session { id } = &scope.id else {
            return None;
        };
        self.state
            .lock()
            .expect("directory state is never poisoned")
            .entries
            .get(id)
            .filter(|entry| entry.runtime.scope() == *scope)
            .map(|entry| entry.runtime.clone())
    }

    fn receive(&self, id: &str, scope: &Scope, publication: Publication) {
        let mut state = self
            .state
            .lock()
            .expect("directory state is never poisoned");
        let Some(entry) = state
            .entries
            .get_mut(id)
            .filter(|entry| entry.runtime.scope() == *scope)
        else {
            return;
        };
        let (position, content) = match publication {
            Publication::Snapshot { snapshot, .. } => (
                Some(snapshot.position),
                snapshot.members.get("summary").cloned(),
            ),
            Publication::Update { update, .. } => (
                Some(update.position),
                update.members.get("summary").and_then(|delta| match delta {
                    Delta::Replace { content } => Some(content.clone()),
                    _ => None,
                }),
            ),
            Publication::Recovered { recovered, .. } => (
                Some(recovered.position),
                recovered
                    .members
                    .get("summary")
                    .and_then(|recovery| match recovery {
                        Recovery::Replace { content } => Some(content.clone()),
                        _ => None,
                    }),
            ),
            Publication::Fault { .. } | Publication::Closed { .. } => (None, None),
        };
        match (position, content) {
            (Some(position), Some(Content::Value(summary))) if position >= entry.source => {
                entry.source = position;
                entry.summary = summary;
                entry.availability = "current";
            }
            (Some(_), _) => return,
            _ => entry.availability = "stale",
        }
        publish(&mut state);
    }

    fn validate(&self, selection: &Selection) -> Result<(), Fault> {
        selection.validate()?;
        if selection.scope != self.scope {
            return Err(Fault::new(
                "stale_scope",
                "Directory incarnation does not match",
            ));
        }
        for member in selection.members.values() {
            self.definitions()
                .iter()
                .find(|definition| definition.id == member.query.id)
                .ok_or_else(|| Fault::query("Daemon query is not exported"))?
                .validate(member)?;
        }
        Ok(())
    }
}

fn publish(state: &mut State) {
    state.value = Value::list(state.entries.iter().map(|(id, entry)| {
        Value::map([
            ("id", Value::str(id)),
            ("incarnation", Value::str(entry.runtime.scope().incarnation)),
            ("title", Value::str(entry.runtime.metadata().title)),
            ("availability", Value::str(entry.availability)),
            (
                "source_position",
                Value::Int(
                    i64::try_from(entry.source)
                        .expect("source publication fits signed data integer"),
                ),
            ),
            ("summary", entry.summary.clone()),
        ])
    }));
    state.position = state
        .position
        .checked_add(1)
        .expect("directory publication clock exhausted");
    state.changed.send_replace(state.position);
}

impl CommandOwner for Directory {
    fn scope(&self) -> Scope {
        self.scope.clone()
    }
    fn command(&self, _: &CallContext, id: &str) -> Option<Command> {
        crate::lifecycle::commands()
            .into_iter()
            .find(|command| command.id == id)
    }
    fn execute<'a>(&'a self, context: &'a CallContext, invocation: Invocation) -> Execution<'a> {
        Box::pin(async move { crate::lifecycle::execute(self, context, invocation).await })
    }
}

impl Owner for Directory {
    fn read(&self, _: &CallContext, selection: &Selection) -> Result<Snapshot, Fault> {
        self.validate(selection)?;
        let state = self
            .state
            .lock()
            .expect("directory state is never poisoned");
        Ok(Snapshot {
            position: state.position,
            members: selection
                .members
                .iter()
                .map(|(name, member)| Ok((name.clone(), self.member(&state, member)?)))
                .collect::<Result<_,Fault>>()?,
        })
    }
    fn observe(
        self: Arc<Self>,
        context: CallContext,
        handle: Handle,
        selection: Selection,
        resume: Option<Resume>,
    ) -> Result<(Box<dyn misa_protocol::owner::Observation>, Publication), Fault> {
        self.validate(&selection)?;
        if resume
            .as_ref()
            .is_some_and(|resume| resume.selection != selection)
        {
            return Err(Fault::query("Resume belongs to another selection"));
        }
        let state = self
            .state
            .lock()
            .expect("directory state is never poisoned");
        if resume
            .as_ref()
            .and_then(|resume| resume.publication)
            .is_some_and(|position| position > state.position)
        {
            return Err(Fault::query("Resume is ahead of directory"));
        }
        let changed = state.changed.subscribe();
        let snapshot = Snapshot {
            position: state.position,
            members: selection
                .members
                .iter()
                .map(|(name, member)| Ok((name.clone(), self.member(&state, member)?)))
                .collect::<Result<_,Fault>>()?,
        };
        drop(state);
        let position = snapshot.position;
        Ok((
            Box::new(Observed {
                directory: self,
                context,
                selection,
                handle,
                changed,
                position,
                faulted: false,
            }),
            Publication::Snapshot { handle, snapshot },
        ))
    }
}

struct Observed {
    directory: Arc<Directory>,
    context: CallContext,
    selection: Selection,
    handle: Handle,
    changed: watch::Receiver<u64>,
    position: u64,
    faulted: bool,
}
impl misa_protocol::owner::Observation for Observed {
    fn changed(&mut self) -> &mut watch::Receiver<u64> {
        &mut self.changed
    }
    fn poll(&mut self) -> Option<Publication> {
        let snapshot = match self.directory.read(&self.context, &self.selection) {
            Ok(snapshot) => snapshot,
            Err(reason) => {
                self.faulted = true;
                return Some(Publication::Fault {
                    handle: self.handle,
                    fault: reason,
                });
            }
        };
        if std::mem::take(&mut self.faulted) {
            self.position = snapshot.position;
            return Some(Publication::Snapshot { handle: self.handle, snapshot });
        }
        if snapshot.position == self.position {
            return None;
        }
        let from = self.position;
        self.position = snapshot.position;
        Some(Publication::Update {
            handle: self.handle,
            update: Update {
                from,
                position: snapshot.position,
                members: snapshot
                    .members
                    .into_iter()
                    .map(|(name, content)| (name, Delta::Replace { content }))
                    .collect(),
            },
        })
    }
}

/// Admission checks are supplied by transport before exposing this resolver.
/// Every lookup still binds the full scope incarnation, never just its label.
pub struct Routes(pub Arc<Directory>);
impl Resolver for Routes {
    fn connected(&self, context: &CallContext, client: &misa_proto::ClientInfo) -> Result<(), Fault> {
        if client.name.len() > 256 || client.version.len() > 256 {
            return Err(Fault::protocol("Client display metadata is too long"));
        }
        let mut state = self.0.state.lock().unwrap();
        if self.0.is_closed() { return Err(Fault::new("closed_scope", "Daemon is shutting down")); }
        if state.connections.len() >= 256 { return Err(Fault::new("busy", "Daemon connection limit reached")); }
        if state.connections.contains_key(&context.connection) { return Err(Fault::protocol("Connection identity reused")); }
        state.connections.insert(context.connection, Value::map([
            ("connection", Value::str(context.connection.to_string())),
            ("principal", Value::str(&context.principal)),
            ("name", Value::str(&client.name)),
            ("version", Value::str(&client.version)),
        ]));
        publish(&mut state);
        Ok(())
    }
    fn disconnected(&self, context: &CallContext) {
        let mut state = self.0.state.lock().unwrap();
        if state.connections.remove(&context.connection).is_some() { publish(&mut state); }
    }
    fn resolve(&self, _: &CallContext, scope: &Scope) -> Result<Arc<dyn Owner>, Fault> {
        if *scope == self.0.scope {
            return Ok(self.0.clone());
        }
        self.0
            .session(scope)
            .map(|runtime| runtime as Arc<dyn Owner>)
            .ok_or_else(|| Fault::new("unavailable_scope", "Scope is unavailable"))
    }
}

/// Authenticated transport connections, not clients attached to individual sessions.
pub fn connections_definition() -> misa_proto::query::Definition {
    use misa_proto::{query::{Definition, ResultContract}, schema::{Schema, Field}};
    Definition {
        id: "daemon.connections".into(), contract: "daemon.connections@1".into(), arguments: vec![],
        result: ResultContract::Data { schema: Schema::List { items: Box::new(Schema::Record {
            fields: ["connection", "principal", "name", "version"].into_iter().map(|name| (name.into(), Field { schema: Schema::String, optional: false })).collect(),
            allow_unknown: false,
        }) } },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::query::ResultContract;

    fn runtime(id: &str) -> Arc<Runtime> {
        Runtime::start(
            id,
            format!("Session {id}"),
            None,
            Arc::new(misa_kernel::LocalKernel::new(
                misa_kernel::ScriptedProvider::always("done"),
            )),
            "scripted",
            "test",
            Value::Null,
        )
    }
    fn context() -> CallContext {
        CallContext {
            principal: "paired".into(),
            connection: 1,
        }
    }
    fn rows(snapshot: &Snapshot) -> &[Value] {
        let Content::Value(value) = &snapshot.members["sessions"] else {
            panic!("directory must be data")
        };
        value.as_list().unwrap()
    }

    #[tokio::test]
    async fn directory_observes_summary_changes_and_routes_full_incarnations() {
        let directory = Directory::new("daemon-run").unwrap();
        let first = runtime("one");
        let second = runtime("two");
        directory.insert(first.clone()).unwrap();
        directory.insert(second.clone()).unwrap();
        let selected = directory.selection();
        let initial = directory.read(&context(), &selected).unwrap();
        assert_eq!(rows(&initial).len(), 2);
        if let Content::Value(value) = &initial.members["sessions"] {
            let ResultContract::Data { schema } = misa_proto::directory::definition().result else {
                panic!()
            };
            schema.validate(value).unwrap();
        }
        let routes = Routes(directory.clone());
        assert_eq!(
            routes.resolve(&context(), &first.scope()).unwrap().scope(),
            first.scope()
        );
        let (mut observed, _) = directory
            .clone()
            .observe(
                context(),
                Handle {
                    id: 1,
                    generation: 1,
                },
                selected.clone(),
                None,
            )
            .unwrap();
        assert!(matches!(first.execute(&context(), Invocation {
            id: 1, scope: first.scope(), command: "session.prompt".into(),
            input: Value::map([("text", Value::str("start"))]),
        }).await, misa_proto::invocation::Outcome::Accepted { .. }));
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            observed.changed().changed(),
        )
        .await
        .unwrap()
        .unwrap();
        let update = observed.poll().unwrap();
        assert!(
            matches!(update, Publication::Update { update, .. } if update.from == initial.position && update.position > initial.position)
        );
        let current = directory.read(&context(), &selected).unwrap();
        assert!(rows(&current).iter().any(|row| {
            row.get("id").and_then(Value::as_str) == Some("one")
                && row.get("source_position").and_then(Value::as_i64).unwrap()
                    > rows(&initial)[0]
                        .get("source_position")
                        .and_then(Value::as_i64)
                        .unwrap()
        }));
        assert!(directory.remove(&first.scope()));
        assert!(routes.resolve(&context(), &first.scope()).is_err());
        let replacement = runtime("one");
        assert_ne!(replacement.scope(), first.scope());
        directory.insert(replacement.clone()).unwrap();
        assert!(routes.resolve(&context(), &first.scope()).is_err());
        assert!(routes.resolve(&context(), &replacement.scope()).is_ok());
        assert!(
            !directory.remove(&first.scope()),
            "old close cannot remove recreated label"
        );
        assert!(routes.resolve(&context(), &second.scope()).is_ok());
    }

    #[tokio::test]
    async fn dropping_one_directory_observer_keeps_the_other_current() {
        let directory = Directory::new("daemon-run").unwrap();
        let selected = directory.selection();
        let (first, _) = directory
            .clone()
            .observe(
                context(),
                Handle {
                    id: 1,
                    generation: 1,
                },
                selected.clone(),
                None,
            )
            .unwrap();
        let (mut second, _) = directory
            .clone()
            .observe(
                context(),
                Handle {
                    id: 2,
                    generation: 1,
                },
                selected,
                None,
            )
            .unwrap();
        drop(first);
        directory.insert(runtime("new")).unwrap();
        assert!(second.poll().is_some());
        assert!(second.poll().is_none());
        assert!(directory.insert(runtime("new")).is_err());
    }
}
#[cfg(test)] mod archive_fault_tests {
 use super::*;
 #[tokio::test] async fn archive_failure_recovers_with_a_complete_coherent_snapshot(){
  let directory=Directory::new("archive-recovery").unwrap();
  let archive=crate::archive::Store::new(Arc::new(misa_kernel::MemoryStore::default()));directory.install_archive(archive).await.unwrap();
  let definition=crate::archive::definitions().into_iter().find(|definition|definition.id=="daemon.conversations").unwrap();
  let selection=Selection{scope:directory.scope(),members:BTreeMap::from([("archive".into(),definition.member(vec![Value::str(""),Value::Int(10)]).unwrap())])};
  let (mut watch,_)=directory.clone().observe(CallContext{principal:"test".into(),connection:1},Handle{id:1,generation:1},selection,None).unwrap();
  {let mut state=directory.state.lock().unwrap();state.archive=Err(Fault::new("storage","Temporarily unavailable"));publish(&mut state);}
  assert!(matches!(watch.poll(),Some(Publication::Fault{..})));
  {let mut state=directory.state.lock().unwrap();state.archive=Ok(Value::list([]));publish(&mut state);}
  assert!(matches!(watch.poll(),Some(Publication::Snapshot{..})),"recovery cannot resume sparse updates over failed required data");
  directory.shutdown_complete().await;
 }
}
