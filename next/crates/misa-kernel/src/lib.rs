//! Facts and capabilities: the part that must hold even when a policy is wrong.
//!
//! This is the previous system's kernel, kept and made explicit. It owns:
//!
//! - **the conversation log**, append-only, and **the attempt ledger**, where a row is
//!   written before a call starts and settled when it ends. Together they are what makes
//!   a resumed conversation the branch that was actually written and an unfinished call a
//!   fact rather than a guess. Both live behind [`store::Store`], so the memory
//!   implementation a test uses and the SQLite one a daemon uses keep the same rules.
//! - **capability**: running a provider, running a tool, an HTTP request, a credential,
//!   a blob, the clock. A policy describes an effect and this is what executes it.
//!
//! It owns no policy and no presentation. It does not know what a turn is, when to
//! compact, or what a transcript looks like. Its whole vocabulary is: append a fact, read
//! facts back, run this thing, tell me what happened.
//!
//! # The trust boundary
//!
//! Every path out of this process goes through a capability that a policy cannot reach
//! around:
//!
//! - a credential's bytes are read by [`http`] and become a header; they are never in a
//!   database, a view, a log, or an event;
//! - a blob is named by a content hash, so a name cannot be a path;
//! - a provider is a [`provider::Provider`], and the composition decides which ones
//!   exist;
//! - a tool is a [`Tool`], and it is where the filesystem and the process boundary are.
//!
//! That is the previous system's rule — every crossing is data, and capability is not
//! policy — with the boundary drawn in Rust instead of Lua.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use misa_value::Value;
use tokio::sync::mpsc;

pub mod blobs;
pub mod credentials;
pub mod http;
pub mod oauth;
/// The services a daemon can talk to, and what each one needs.
pub mod presets;
pub mod provider;
pub mod store;
pub mod tools;

pub use blobs::Blobs;
pub use credentials::{Credentials, OAuth, Secret};
pub use presets::{Auth, Effort, Preset};
pub use http::{Credential, Http, Request as HttpRequest, Response as HttpResponse};
pub use provider::{
    AnthropicMessages, BrokenProvider, OpenAiChat, OpenAiResponses, Provider, ScriptedProvider, Turn,
};
pub use store::{Attempt, Conversation, Entry, MemoryStore, SqliteStore, Store};
/// A durable fact, in the order it was recorded.
pub use store::Entry as LogEntry;

/// One map with one key replaced, as a new value.
///
/// `Value::map` takes `&'static str` keys, which is right for the places that build a map
/// from a literal and wrong for the places that amend one they were handed. This is the
/// second kind: three lines, no cleverness, and it copies the map rather than mutating it,
/// which is what makes a value shared between two snapshots stay two snapshots.
fn set(value: &Value, key: &str, replacement: Value) -> Value {
    let mut entries = value.as_map().cloned().unwrap_or_default();
    entries.insert(key.to_string(), replacement);
    Value::Map(Arc::new(entries))
}

/// Bytes as base64, for a provider that takes an image inside a message.
fn base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
/// What a session asks the kernel to do.
///
/// Stated as data, because that is how a policy expresses an effect: the loop validates
/// the kind, and this is what an accepted kind is translated into.
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Ask a provider for a completion, streaming as it arrives.
    ProviderCall {
        id: String,
        provider: String,
        model: String,
        messages: Value,
        tools: Value,
        /// Generation options the session decided on: `reasoning_effort`, `temperature`,
        /// `max_tokens`. The adapter translates them into what the service calls them.
        settings: Value,
    },
    /// Run one tool call.
    ToolRun {
        id: String,
        call_id: String,
        name: String,
        args: Value,
    },
    /// Append a fact to the conversation log.
    Append {
        conversation: String,
        kind: String,
        data: Value,
    },
    /// Read a conversation back, in order.
    Load {
        conversation: String,
        after: i64,
        limit: usize,
    },
    /// List the conversations this daemon knows about.
    Conversations {
        id: String,
    },
    /// Start an attempt. A provider call does this implicitly; a policy may also record
    /// one for work it is about to do itself.
    AttemptStarted {
        id: String,
        conversation: Option<String>,
        parent: Option<String>,
        provider: String,
        model: String,
        kind: String,
    },
    /// Settle an attempt.
    AttemptSettled {
        id: String,
        status: String,
        input_tokens: i64,
        output_tokens: i64,
        cost_micros: i64,
    },
    /// An HTTP request. `credential` names a slot; the bytes never reach policy.
    Http {
        id: String,
        request: http::Request,
    },
    /// Set, delete, or list credentials.
    Credential {
        id: String,
        action: CredentialAction,
    },
    /// Store bytes and name them by content.
    BlobPut {
        id: String,
        bytes: Vec<u8>,
        media: Option<String>,
    },
    /// Read a file into the blob store, so a session can attach or display it.
    BlobFile {
        id: String,
        path: String,
    },
    /// Read a blob back.
    BlobLoad {
        id: String,
        hash: String,
    },
    /// Ask a provider which models it has.
    ///
    /// The one request that is not a completion, and the one that lets a picker offer what a
    /// service actually serves instead of what a release happened to name.
    DiscoverModels {
        id: String,
        provider: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum CredentialAction {
    Set { slot: String, account: String, value: String },
    Delete { slot: String },
    List,
}

/// What the kernel reports back. Each one becomes an event in the session's loop.
///
/// This is the completion-event side of "effects are data": a policy that wanted an
/// answer asked for one, and this is the answer arriving as a fact.
#[derive(Clone, Debug, PartialEq)]
pub enum KernelEvent {
    /// One chunk of a streamed completion.
    ProviderDelta { id: String, text: String },
    /// One chunk of a service's own reasoning, which is not the answer.
    ///
    /// It travels on its own event rather than as part of [`KernelEvent::ProviderDelta`]
    /// because a client has to be able to draw thinking differently from text — collapsed,
    /// dimmed, or not at all — and a session that merged them would have decided that for
    /// every client at once.
    ProviderThinking { id: String, text: String },
    ProviderFinished {
        id: String,
        ok: bool,
        text: String,
        /// What the service reasoned before answering, when it reports one. Empty for the
        /// services that do not, which is most of them.
        thinking: String,
        /// Tool calls the model asked for, as `[{id, name, args}]`.
        tool_calls: Value,
        input_tokens: i64,
        output_tokens: i64,
        error: String,
    },
    /// The models a service says it has, in answer to a discovery request.
    Models {
        id: String,
        ok: bool,
        /// `[{id, label}]`, sorted and deduplicated.
        models: Value,
        message: String,
    },
    ToolFinished {
        id: String,
        call_id: String,
        ok: bool,
        text: String,
    },
    Appended { conversation: String, seq: i64 },
    Loaded { conversation: String, entries: Vec<Entry> },
    Conversations { id: String, headers: Value },
    AttemptRecorded { id: String, name: String },
    HttpFinished {
        id: String,
        ok: bool,
        status: u16,
        body: String,
    },
    Credential {
        id: String,
        ok: bool,
        message: String,
        /// Slot and account pairs. Never a value.
        slots: Value,
    },
    Blob {
        id: String,
        ok: bool,
        hash: String,
        len: i64,
        media: String,
        message: String,
    },
    BlobBytes {
        id: String,
        ok: bool,
        hash: String,
        bytes: Vec<u8>,
        media: String,
        message: String,
    },
    /// A command the shell tool started has finished, possibly long after the call that
    /// started it answered. Nothing asked for this event, which is why it is the one kind the
    /// kernel produces on its own schedule.
    ProcessFinished {
        pid: u32,
        command: String,
        /// The exit status, or `None` when a signal ended it.
        exit: Option<i32>,
        /// Whether the kernel stopped it at its deadline rather than the command ending.
        killed: bool,
        seconds: f64,
        log: String,
    },
    /// The kernel could not do what was asked. Never fatal to the session.
    Failed { id: String, message: String },
}

impl KernelEvent {
    /// A short name, for logs and for the event kind it becomes.
    pub fn kind(&self) -> &'static str {
        match self {
            KernelEvent::ProviderDelta { .. } => "provider.delta",
            KernelEvent::ProviderThinking { .. } => "provider.thinking",
            KernelEvent::ProviderFinished { .. } => "provider.finished",
            KernelEvent::Models { .. } => "models",
            KernelEvent::ToolFinished { .. } => "tool.finished",
            KernelEvent::Appended { .. } => "log.appended",
            KernelEvent::Loaded { .. } => "log.loaded",
            KernelEvent::Conversations { .. } => "log.listed",
            KernelEvent::AttemptRecorded { .. } => "attempt.recorded",
            KernelEvent::HttpFinished { .. } => "http.finished",
            KernelEvent::Credential { .. } => "credential",
            KernelEvent::Blob { .. } => "blob.stored",
            KernelEvent::BlobBytes { .. } => "blob.loaded",
            KernelEvent::ProcessFinished { .. } => "process.finished",
            KernelEvent::Failed { .. } => "failed",
        }
    }
}

/// A capability boundary.
///
/// Every method is the kernel's, not a policy's. A policy cannot reach the network, the
/// filesystem, or a secret except by asking for one of these, which is what makes the
/// trust boundary a boundary rather than a convention.
#[async_trait]
pub trait Kernel: Send + Sync {
    async fn execute(&self, request: Request, out: &mpsc::UnboundedSender<KernelEvent>);

    /// Events the kernel produces without being asked.
    ///
    /// A request is answered where it was made; a background command finishing is not
    /// something anybody asked for at the moment it happens, and the session that started it
    /// has to be told. Nothing else here reports unsolicited, which is why this is one method
    /// rather than a bus, and why the default is nothing at all.
    fn events(&self) -> Option<mpsc::UnboundedReceiver<KernelEvent>> {
        None
    }
}

/// One model call, as a provider adapter needs to see it.
#[derive(Clone, Debug)]
pub struct ProviderRequest {
    pub model: String,
    pub messages: Value,
    pub tools: Value,
    /// What the session decided about this request, as data: `reasoning_effort`,
    /// `temperature`, `max_tokens`, and whatever else a session sets. An adapter knows which
    /// of them this service accepts and what it calls them; it does not invent any.
    pub settings: Value,
}

#[derive(Clone, Debug, Default)]
pub struct Answer {
    pub text: String,
    /// What a service reasoned before it answered, when it streams one. Empty when it does
    /// not, which is nearly every service that does not charge for thinking aloud.
    pub thinking: String,
    /// `[{id, name, args}]`
    pub tool_calls: Value,
    pub input_tokens: i64,
    pub output_tokens: i64,
}

/// A tool, as the kernel runs it.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    /// Arguments as data in, text out. A tool's failure is a result, not an error: the
    /// model is told what happened and decides what to do, exactly as the previous system
    /// decided when it settled on one close-out sentence.
    async fn run(&self, args: &Value) -> Result<String, String>;
}

/// How a daemon is composed.
///
/// Everything here is a decision the *composition* makes, not the kernel: which store,
/// which providers, which tools, where credentials live.
pub struct Composition {
    pub store: Arc<dyn Store>,
    pub credentials: Arc<Credentials>,
    pub blobs: Arc<Blobs>,
    pub http: Option<Arc<Http>>,
    pub providers: Vec<Arc<dyn Provider>>,
    /// The provider a model with no explicit one uses.
    pub default_provider: String,
    /// A search backend, when one is configured.
    pub search: Option<SearchBackend>,
    /// Where the shell tool writes what it runs. A directory rather than a stream, because a
    /// command's output outlives the call that started it.
    pub shell: PathBuf,
}

/// A configured web search.
#[derive(Clone, Debug)]
pub struct SearchBackend {
    pub kind: SearchKind,
    /// The base url, so a test can point it at a local server.
    pub base_url: String,
    /// The credential slot, when the backend wants one.
    pub credential: Option<String>,
    /// How many results to ask for.
    pub limit: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchKind {
    Brave,
    Tavily,
    /// A self-hosted SearXNG, which needs no credential. The default, because a search
    /// tool that cannot run without an account is a search tool most people cannot use.
    Searxng,
}

impl SearchKind {
    pub fn from_id(id: &str) -> Option<SearchKind> {
        match id {
            "brave" => Some(SearchKind::Brave),
            "tavily" => Some(SearchKind::Tavily),
            "searxng" => Some(SearchKind::Searxng),
            _ => None,
        }
    }

    pub fn id(&self) -> &'static str {
        match self {
            SearchKind::Brave => "brave",
            SearchKind::Tavily => "tavily",
            SearchKind::Searxng => "searxng",
        }
    }
}

/// The kernel, with every capability it was composed with.
pub struct Daemon {
    store: Arc<dyn Store>,
    credentials: Arc<Credentials>,
    blobs: Arc<Blobs>,
    http: Option<Arc<Http>>,
    providers: Vec<Arc<dyn Provider>>,
    default_provider: String,
    tools: Vec<Arc<dyn Tool>>,
    /// Where a tool that keeps running after it answers reports what happened.
    events: mpsc::UnboundedSender<KernelEvent>,
    /// The other end of `events`, handed to the session exactly once.
    listener: std::sync::Mutex<Option<mpsc::UnboundedReceiver<KernelEvent>>>,
    /// Where the shell tool writes what it runs.
    shell: PathBuf,
}

impl Daemon {
    /// A daemon with everything in memory and one provider.
    ///
    /// What a test composes, and what a session with no data directory gets: no store to
    /// open, nothing to clean up, and the same rules as the durable one.
    pub fn new(provider: Arc<dyn Provider>) -> Daemon {
        let (events, listener) = mpsc::unbounded_channel();
        let shell = tools::default_shell_dir();
        Daemon {
            store: Arc::new(MemoryStore::new()),
            credentials: Arc::new(Credentials::in_memory()),
            blobs: Arc::new(Blobs::in_memory()),
            http: None,
            providers: vec![provider],
            default_provider: "scripted".into(),
            tools: tools::shipped(&shell, &events),
            events,
            listener: std::sync::Mutex::new(Some(listener)),
            shell,
        }
    }

    /// A daemon that keeps what it learns.
    ///
    /// `root` holds the log, the ledger, and the blobs. Credentials default to the state
    /// directory and can be pointed elsewhere with `MISA_CREDENTIALS`.
    pub fn open(root: &Path) -> Result<Daemon, String> {
        let credentials = Arc::new(Credentials::at(&Credentials::default_path())?);
        let http = Arc::new(Http::new(credentials.clone())?);
        let (events, listener) = mpsc::unbounded_channel();
        let shell = root.join("shell");
        Ok(Daemon {
            store: Arc::new(SqliteStore::open(&root.join("conversations.sqlite3"))?),
            blobs: Arc::new(Blobs::at(&root.join("blobs"))?),
            credentials,
            http: Some(http),
            providers: vec![ScriptedProvider::new([
                Turn::say("this daemon has no provider of its own configured"),
            ])],
            default_provider: "scripted".into(),
            tools: tools::shipped(&shell, &events),
            events,
            listener: std::sync::Mutex::new(Some(listener)),
            shell,
        })
    }

    /// Compose from parts.
    pub fn compose(composition: Composition) -> Daemon {
        let (events, listener) = mpsc::unbounded_channel();
        let mut tools = tools::shipped(&composition.shell, &events);
        // The search tool exists when a backend is configured, and not before: a tool a
        // model can call but that always fails is worse than one that is absent.
        if let (Some(backend), Some(http)) = (&composition.search, &composition.http) {
            tools.push(Arc::new(search::WebSearch::new(http.clone(), backend.clone())));
        }
        Daemon {
            store: composition.store,
            credentials: composition.credentials,
            blobs: composition.blobs,
            http: composition.http,
            default_provider: composition.default_provider,
            providers: composition.providers,
            tools,
            events,
            listener: std::sync::Mutex::new(Some(listener)),
            shell: composition.shell,
        }
    }

    /// Where the shell tool writes what it runs.
    ///
    /// Rebuilt rather than mutated in place: the log directory is the shell tool's whole
    /// configuration, and a tool list with the old one still in it would be two shells.
    pub fn with_shell(mut self, dir: impl Into<PathBuf>) -> Daemon {
        let dir = dir.into();
        self.tools.retain(|tool| tool.name() != "shell");
        self.tools.push(Arc::new(tools::Shell::at(&dir, self.events.clone())));
        self.shell = dir;
        self
    }

    pub fn with_store(mut self, store: Arc<dyn Store>) -> Daemon {
        self.store = store;
        self
    }

    pub fn with_http(mut self, http: Arc<Http>) -> Daemon {
        self.http = Some(http);
        self
    }

    pub fn with_credentials(mut self, credentials: Arc<Credentials>) -> Daemon {
        self.credentials = credentials;
        self
    }

    pub fn with_blobs(mut self, blobs: Arc<Blobs>) -> Daemon {
        self.blobs = blobs;
        self
    }

    pub fn with_provider(mut self, provider: Arc<dyn Provider>) -> Daemon {
        self.default_provider = provider.id().to_string();
        self.providers.push(provider);
        self
    }

    pub fn with_default_provider(mut self, id: impl Into<String>) -> Daemon {
        self.default_provider = id.into();
        self
    }

    pub fn with_tool(mut self, tool: Arc<dyn Tool>) -> Daemon {
        self.tools.push(tool);
        self
    }

    pub fn with_search(mut self, backend: SearchBackend) -> Daemon {
        if let Some(http) = &self.http {
            self.tools.push(Arc::new(search::WebSearch::new(http.clone(), backend)));
        }
        self
    }

    pub fn credentials(&self) -> &Arc<Credentials> {
        &self.credentials
    }

    pub fn blobs(&self) -> &Arc<Blobs> {
        &self.blobs
    }

    /// Where the shell tool writes what it runs.
    pub fn shell_dir(&self) -> &Path {
        &self.shell
    }

    pub fn store(&self) -> &Arc<dyn Store> {
        &self.store
    }

    pub fn http(&self) -> Option<&Arc<Http>> {
        self.http.as_ref()
    }

    /// The provider ids this daemon can reach.
    pub fn provider_ids(&self) -> Vec<String> {
        self.providers.iter().map(|provider| provider.id().to_string()).collect()
    }

    pub fn tool_names(&self) -> Vec<String> {
        self.tools.iter().map(|tool| tool.name().to_string()).collect()
    }

    /// Every entry this daemon holds. For a test or a diagnostic.
    pub fn entries(&self) -> Vec<Entry> {
        self.store.conversations().map(|conversations| {
            conversations
                .into_iter()
                .flat_map(|conversation| self.store.load(&conversation.id, 0, 100_000).unwrap_or_default())
                .collect()
        }).unwrap_or_default()
    }

    pub fn attempts(&self) -> Vec<Value> {
        self.store
            .attempts(None)
            .map(|rows| rows.into_iter().map(|row| row.to_value()).collect())
            .unwrap_or_default()
    }

    fn provider(&self, id: &str) -> Option<Arc<dyn Provider>> {
        self.providers
            .iter()
            .find(|provider| provider.id() == id)
            .cloned()
            .or_else(|| {
                // A model that names no provider, or one that is not composed, uses the
                // default rather than failing: a session with one provider should not have
                // to know its name.
                (id.is_empty() || id == self.default_provider)
                    .then(|| self.providers.iter().find(|provider| provider.id() == self.default_provider).cloned())
                    .flatten()
            })
    }

    fn tool(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.iter().find(|tool| tool.name() == name).cloned()
    }

    /// Give each attachment in a message the bytes it names.
    ///
    /// A session carries an attachment as a content hash, because that is what keeps a
    /// transcript small and a log portable. A provider wants bytes inside the message, which
    /// is what both wire formats take and what this produces: the session's own message
    /// shape, with each attachment either carrying its bytes or saying it cannot.
    ///
    /// Resolved per request rather than once, because a blob can arrive after the message
    /// that names it did, and it happens here rather than in a session because this is the
    /// one place that holds both the message and the store.
    ///
    /// An attachment whose bytes are gone is *marked*, never dropped: the model is told that
    /// an attachment was there and that it cannot be read, which is the same thing the
    /// person is told, and an answer about a picture nobody can see is worse than one that
    /// says the picture is missing.
    fn with_attachments(&self, messages: Value) -> Value {
        let Some(messages) = messages.as_list() else {
            return messages;
        };
        Value::list(messages.iter().map(|message| {
            let Some(attachments) = message.get("attachments").and_then(Value::as_list) else {
                return message.clone();
            };
            if attachments.is_empty() {
                return message.clone();
            }
            let resolved = Value::list(attachments.iter().map(|attachment| {
                let hash = attachment.get("hash").and_then(Value::as_str).unwrap_or_default();
                match self.blobs.get(hash) {
                    Some(bytes) => {
                        // The media type comes from the store when the message did not carry
                        // one: the store sniffed the bytes, and a message only knows what a
                        // filename suggested.
                        let media = attachment
                            .get("media")
                            .and_then(Value::as_str)
                            .filter(|media| !media.is_empty())
                            .map(str::to_string)
                            .or_else(|| self.blobs.media(hash))
                            .unwrap_or_else(|| "application/octet-stream".to_string());
                        let encoded = Value::str(base64(&bytes));
                        let with_media = set(attachment, "media", Value::str(media));
                        set(&with_media, "data", encoded)
                    }
                    None => set(attachment, "missing", Value::Bool(true)),
                }
            }));
            set(message, "attachments", resolved)
        }))
    }

    fn now(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis() as i64)
            .unwrap_or(0)
    }
}

#[async_trait]
impl Kernel for Daemon {
    fn events(&self) -> Option<mpsc::UnboundedReceiver<KernelEvent>> {
        self.listener.lock().expect("the listener is taken once").take()
    }

    async fn execute(&self, request: Request, out: &mpsc::UnboundedSender<KernelEvent>) {
        match request {
            Request::ProviderCall { id, provider, model, messages, tools, settings } => {
                let Some(adapter) = self.provider(&provider) else {
                    let _ = out.send(KernelEvent::ProviderFinished {
                        id,
                        ok: false,
                        text: String::new(),
                        thinking: String::new(),
                        tool_calls: Value::list([]),
                        input_tokens: 0,
                        output_tokens: 0,
                        error: format!("this daemon has no provider `{provider}`"),
                    });
                    return;
                };
                let messages = self.with_attachments(messages);
                let request = ProviderRequest { model, messages, tools, settings };
                match adapter.stream(&request, &id, out).await {
                    Ok(answer) => {
                        let _ = out.send(KernelEvent::ProviderFinished {
                            id,
                            ok: true,
                            text: answer.text,
                            thinking: answer.thinking,
                            tool_calls: answer.tool_calls,
                            input_tokens: answer.input_tokens,
                            output_tokens: answer.output_tokens,
                            error: String::new(),
                        });
                    }
                    Err(error) => {
                        let _ = out.send(KernelEvent::ProviderFinished {
                            id,
                            ok: false,
                            text: String::new(),
                            thinking: String::new(),
                            tool_calls: Value::list([]),
                            input_tokens: 0,
                            output_tokens: 0,
                            error,
                        });
                    }
                }
            }
            Request::ToolRun { id, call_id, name, args } => {
                let result = match self.tool(&name) {
                    Some(tool) => tool.run(&args).await,
                    // An unknown tool is a result the model can act on, not a fault that
                    // stops the turn.
                    None => Err(format!("no tool named `{name}`")),
                };
                let (ok, text) = match result {
                    Ok(text) => (true, text),
                    Err(text) => (false, text),
                };
                let _ = out.send(KernelEvent::ToolFinished { id, call_id, ok, text });
            }
            Request::DiscoverModels { id, provider } => {
                let message = |message: String| KernelEvent::Models {
                    id: id.clone(),
                    ok: false,
                    models: Value::list([]),
                    message,
                };
                let Some(adapter) = self.provider(&provider) else {
                    let _ = out.send(message(format!("this daemon has no provider `{provider}`")));
                    return;
                };
                let Some(http) = &self.http else {
                    let _ = out.send(message("this daemon was composed without http".into()));
                    return;
                };
                // The adapter is asked rather than a table, because where a service lists its
                // models is a property of the address it was composed against: a proxy lists
                // its own.
                match provider::discover(adapter.as_ref(), http).await {
                    Ok(models) => {
                        let listed = Value::list(models.iter().map(|model| {
                            Value::map([("id", Value::str(&model.id)), ("label", Value::str(&model.label))])
                        }));
                        let _ = out.send(KernelEvent::Models {
                            id,
                            ok: true,
                            models: listed,
                            message: format!("{} models from {provider}", models.len()),
                        });
                    }
                    Err(problem) => {
                        let _ = out.send(message(problem));
                    }
                }
            }
            Request::Append { conversation, kind, data } => {
                let store = self.store.clone();
                let at = self.now();
                // The branch is needed for the event afterwards and the closure owns what
                // it is given, so the name is cloned once into the worker.
                let branch = conversation.clone();
                match tokio::task::spawn_blocking(move || store.append(&branch, &kind, &data, at)).await {
                    Ok(Ok(seq)) => {
                        let _ = out.send(KernelEvent::Appended { conversation, seq });
                    }
                    Ok(Err(message)) => {
                        let _ = out.send(KernelEvent::Failed { id: conversation, message });
                    }
                    Err(err) => {
                        let _ = out.send(KernelEvent::Failed { id: conversation, message: err.to_string() });
                    }
                }
            }
            Request::Load { conversation, after, limit } => {
                let store = self.store.clone();
                let branch = conversation.clone();
                match tokio::task::spawn_blocking(move || store.load(&branch, after, limit)).await {
                    Ok(Ok(entries)) => {
                        let _ = out.send(KernelEvent::Loaded { conversation, entries });
                    }
                    Ok(Err(message)) => {
                        let _ = out.send(KernelEvent::Failed { id: conversation, message });
                    }
                    Err(err) => {
                        let _ = out.send(KernelEvent::Failed { id: conversation, message: err.to_string() });
                    }
                }
            }
            Request::Conversations { id } => {
                let store = self.store.clone();
                match tokio::task::spawn_blocking(move || store.conversations()).await {
                    Ok(Ok(headers)) => {
                        let _ = out.send(KernelEvent::Conversations {
                            id,
                            headers: Value::list(headers.iter().map(Conversation::to_value).collect::<Vec<_>>()),
                        });
                    }
                    Ok(Err(message)) => {
                        let _ = out.send(KernelEvent::Failed { id, message });
                    }
                    Err(err) => {
                        let _ = out.send(KernelEvent::Failed { id, message: err.to_string() });
                    }
                }
            }
            Request::AttemptStarted { id, conversation, parent, provider, model, kind } => {
                let store = self.store.clone();
                let started = self.now();
                let attempt = Attempt {
                    name: id.clone(),
                    conversation,
                    parent,
                    kind,
                    provider,
                    model,
                    status: "started".into(),
                    started_ms: started,
                    ..Attempt::default()
                };
                match tokio::task::spawn_blocking(move || store.attempt_start(&attempt)).await {
                    Ok(Ok(name)) => {
                        let _ = out.send(KernelEvent::AttemptRecorded { id, name });
                    }
                    Ok(Err(message)) => {
                        let _ = out.send(KernelEvent::Failed { id, message });
                    }
                    Err(err) => {
                        let _ = out.send(KernelEvent::Failed { id, message: err.to_string() });
                    }
                }
            }
            Request::AttemptSettled { id, status, input_tokens, output_tokens, cost_micros } => {
                let store = self.store.clone();
                let at = self.now();
                // A settle that fails is not reported: the attempt row is already the
                // durable fact, and a session that cannot settle one should keep running.
                let _ = tokio::task::spawn_blocking(move || {
                    store.attempt_settle(&id, &status, input_tokens, output_tokens, cost_micros, at)
                })
                .await;
            }
            Request::Http { id, request } => {
                let Some(http) = self.http.clone() else {
                    let _ = out.send(KernelEvent::HttpFinished {
                        id,
                        ok: false,
                        status: 0,
                        body: "this daemon has no HTTP capability".into(),
                    });
                    return;
                };
                match http.send(&request).await {
                    Ok(response) => {
                        let _ = out.send(KernelEvent::HttpFinished {
                            id,
                            ok: response.ok(),
                            status: response.status,
                            body: response.text(),
                        });
                    }
                    Err(message) => {
                        let _ = out.send(KernelEvent::HttpFinished { id, ok: false, status: 0, body: message });
                    }
                }
            }
            Request::Credential { id, action } => {
                let (ok, message) = match action {
                    CredentialAction::Set { slot, account, value } => match self.credentials.set(&slot, &account, &value) {
                        Ok(()) => (true, format!("stored a credential for `{slot}`")),
                        Err(message) => (false, message),
                    },
                    CredentialAction::Delete { slot } => match self.credentials.delete(&slot) {
                        Ok(true) => (true, format!("removed the credential for `{slot}`")),
                        Ok(false) => (true, format!("there was no credential for `{slot}`")),
                        Err(message) => (false, message),
                    },
                    CredentialAction::List => (true, String::new()),
                };
                let slots = Value::list(
                    self.credentials
                        .slots()
                        .into_iter()
                        .map(|(slot, account)| {
                            Value::map([("slot", Value::str(slot)), ("account", Value::str(account))])
                        })
                        .collect::<Vec<_>>(),
                );
                let _ = out.send(KernelEvent::Credential { id, ok, message, slots });
            }
            Request::BlobPut { id, bytes, media } => match self.blobs.put(&bytes, media.as_deref()) {
                Ok(reference) => {
                    let _ = out.send(KernelEvent::Blob {
                        id,
                        ok: true,
                        hash: reference.hash,
                        len: reference.len as i64,
                        media: reference.media.unwrap_or_default(),
                        message: String::new(),
                    });
                }
                Err(message) => {
                    let _ = out.send(KernelEvent::Blob {
                        id,
                        ok: false,
                        hash: String::new(),
                        len: 0,
                        media: String::new(),
                        message,
                    });
                }
            },
            Request::BlobFile { id, path } => match tokio::fs::read(&path).await {
                Ok(bytes) => match self.blobs.put(&bytes, None) {
                    Ok(reference) => {
                        let _ = out.send(KernelEvent::Blob {
                            id,
                            ok: true,
                            hash: reference.hash,
                            len: reference.len as i64,
                            media: reference.media.unwrap_or_default(),
                            message: String::new(),
                        });
                    }
                    Err(message) => {
                        let _ = out.send(KernelEvent::Blob {
                            id,
                            ok: false,
                            hash: String::new(),
                            len: 0,
                            media: String::new(),
                            message,
                        });
                    }
                },
                Err(err) => {
                    let _ = out.send(KernelEvent::Blob {
                        id,
                        ok: false,
                        hash: String::new(),
                        len: 0,
                        media: String::new(),
                        message: format!("could not read {path}: {err}"),
                    });
                }
            },
            Request::BlobLoad { id, hash } => match self.blobs.get(&hash) {
                Some(bytes) => {
                    let media = self.blobs.media(&hash).unwrap_or_default();
                    let _ = out.send(KernelEvent::BlobBytes {
                        id,
                        ok: true,
                        hash,
                        bytes,
                        media,
                        message: String::new(),
                    });
                }
                None => {
                    let _ = out.send(KernelEvent::BlobBytes {
                        id,
                        ok: false,
                        hash,
                        bytes: Vec::new(),
                        media: String::new(),
                        message: "no such blob".into(),
                    });
                }
            },
        }
    }
}

/// The name the in-memory composition is used under in tests.
pub type LocalKernel = Daemon;

#[cfg(test)]
mod tests {
    use super::*;

    async fn drain(rx: &mut mpsc::UnboundedReceiver<KernelEvent>) -> Vec<KernelEvent> {
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        events
    }

    fn kernel() -> Daemon {
        Daemon::new(ScriptedProvider::always("hi"))
    }

    #[tokio::test]
    async fn a_background_command_answers_at_once_and_reports_on_the_kernels_own_channel() {
        // The whole path, because the seam is where mistakes hide: the tool call comes back
        // immediately with a pid and a log, and the end of the command arrives on the one
        // channel nothing asked for.
        let kernel = kernel();
        let mut reports = kernel.events().expect("a kernel reports on its own schedule");
        let (tx, mut rx) = mpsc::unbounded_channel();
        kernel
            .execute(
                Request::ToolRun {
                    id: "r1".into(),
                    call_id: "r1.call.1".into(),
                    name: "shell".into(),
                    args: Value::map([
                        ("command", Value::str("sleep 0.3; echo finished")),
                        ("background", Value::Bool(true)),
                    ]),
                },
                &tx,
            )
            .await;
        match &drain(&mut rx).await[0] {
            KernelEvent::ToolFinished { ok, text, .. } => {
                assert!(ok, "{text}");
                assert!(text.contains("started in the background"), "{text}");
                assert!(text.contains("pid "), "{text}");
            }
            other => panic!("a tool call answered with `{}`", other.kind()),
        }

        let report = tokio::time::timeout(std::time::Duration::from_secs(10), reports.recv())
            .await
            .expect("a report")
            .expect("the channel is open");
        match report {
            KernelEvent::ProcessFinished { exit, command, log, .. } => {
                assert_eq!(exit, Some(0));
                assert!(command.contains("sleep 0.3"), "{command}");
                let text = std::fs::read_to_string(&log).expect("the log");
                assert!(text.contains("finished"), "{text}");
            }
            other => panic!("reported `{}`", other.kind()),
        }
    }

    #[tokio::test]
    async fn the_log_appends_in_order_and_never_rewrites() {
        let kernel = kernel();
        let (tx, mut rx) = mpsc::unbounded_channel();
        for text in ["one", "two", "three"] {
            kernel
                .execute(
                    Request::Append { conversation: "c1".into(), kind: "message".into(), data: Value::str(text) },
                    &tx,
                )
                .await;
        }
        let events = drain(&mut rx).await;
        assert_eq!(events.len(), 3);
        assert_eq!(kernel.entries().len(), 3);
        assert_eq!(kernel.entries()[2].data.as_str(), Some("three"));
    }

    #[tokio::test]
    async fn a_load_returns_what_came_after_the_cursor() {
        let kernel = kernel();
        let (tx, mut rx) = mpsc::unbounded_channel();
        for text in ["one", "two", "three"] {
            kernel
                .execute(
                    Request::Append { conversation: "c1".into(), kind: "message".into(), data: Value::str(text) },
                    &tx,
                )
                .await;
        }
        kernel.execute(Request::Load { conversation: "c1".into(), after: 1, limit: 10 }, &tx).await;
        let events = drain(&mut rx).await;
        match events.last().expect("a load event") {
            KernelEvent::Loaded { entries, .. } => {
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].data.as_str(), Some("two"));
            }
            other => panic!("expected a load, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_conversation_list_names_each_branch_by_its_first_message() {
        let kernel = kernel();
        let (tx, mut rx) = mpsc::unbounded_channel();
        kernel
            .execute(
                Request::Append {
                    conversation: "c1".into(),
                    kind: "message".into(),
                    data: Value::map([("role", Value::str("user")), ("text", Value::str("a haiku please"))]),
                },
                &tx,
            )
            .await;
        kernel.execute(Request::Conversations { id: "list".into() }, &tx).await;
        let events = drain(&mut rx).await;
        match events.last().expect("a listing") {
            KernelEvent::Conversations { headers, .. } => {
                let first = &headers.as_list().expect("a list")[0];
                assert_eq!(first.get("id").and_then(Value::as_str), Some("c1"));
                assert_eq!(first.get("title").and_then(Value::as_str), Some("a haiku please"));
            }
            other => panic!("expected a listing, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_attempt_is_recorded_before_it_is_settled() {
        let kernel = kernel();
        let (tx, mut rx) = mpsc::unbounded_channel();
        kernel
            .execute(
                Request::AttemptStarted {
                    id: "r1".into(),
                    conversation: Some("c1".into()),
                    parent: None,
                    provider: "scripted".into(),
                    model: "m".into(),
                    kind: "turn".into(),
                },
                &tx,
            )
            .await;
        let events = drain(&mut rx).await;
        assert!(matches!(events.last(), Some(KernelEvent::AttemptRecorded { .. })));
        assert_eq!(kernel.attempts()[0].get("status").and_then(Value::as_str), Some("started"));

        kernel
            .execute(
                Request::AttemptSettled {
                    id: "r1".into(),
                    status: "ok".into(),
                    input_tokens: 10,
                    output_tokens: 20,
                    cost_micros: 5,
                },
                &tx,
            )
            .await;
        // The settle runs on a blocking worker, so give it a moment.
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let row = &kernel.attempts()[0];
        assert_eq!(row.get("status").and_then(Value::as_str), Some("ok"));
        assert_eq!(row.get("output_tokens").and_then(Value::as_i64), Some(20));
    }

    #[tokio::test]
    async fn a_provider_that_fails_is_reported_not_raised() {
        let kernel = Daemon::new(BrokenProvider::new("broken", "no route to host"));
        let (tx, mut rx) = mpsc::unbounded_channel();
        kernel
            .execute(
                Request::ProviderCall {
                    id: "r1".into(),
                    provider: "broken".into(),
                    model: "m".into(),
                    messages: Value::list([]),
                    tools: Value::list([]),
                    settings: Value::map([]),
                },
                &tx,
            )
            .await;
        let events = drain(&mut rx).await;
        match events.last() {
            Some(KernelEvent::ProviderFinished { ok, error, .. }) => {
                assert!(!ok);
                assert_eq!(error, "no route to host");
            }
            other => panic!("expected a failed completion, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_provider_this_daemon_does_not_have_is_reported_rather_than_guessed() {
        let kernel = Daemon::new(ScriptedProvider::always("hi")).with_default_provider("scripted");
        let (tx, mut rx) = mpsc::unbounded_channel();
        kernel
            .execute(
                Request::ProviderCall {
                    id: "r1".into(),
                    provider: "anthropic".into(),
                    model: "m".into(),
                    messages: Value::list([]),
                    tools: Value::list([]),
                    settings: Value::map([]),
                },
                &tx,
            )
            .await;
        let events = drain(&mut rx).await;
        match events.last() {
            Some(KernelEvent::ProviderFinished { ok, error, .. }) => {
                assert!(!ok);
                assert!(error.contains("anthropic"), "{error}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_tool_that_does_not_exist_is_a_result_the_model_can_act_on() {
        let kernel = kernel();
        let (tx, mut rx) = mpsc::unbounded_channel();
        kernel
            .execute(
                Request::ToolRun {
                    id: "r1".into(),
                    call_id: "r1.call.1".into(),
                    name: "nonexistent".into(),
                    args: Value::Null,
                },
                &tx,
            )
            .await;
        let events = drain(&mut rx).await;
        match &events[0] {
            KernelEvent::ToolFinished { ok, text, .. } => {
                assert!(!ok);
                assert!(text.contains("nonexistent"));
            }
            other => panic!("expected a result, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_blob_goes_in_by_content_and_comes_back_out() {
        let kernel = kernel();
        let (tx, mut rx) = mpsc::unbounded_channel();
        kernel
            .execute(
                Request::BlobPut {
                    id: "b1".into(),
                    bytes: vec![0x89, b'P', b'N', b'G', 1, 2],
                    media: None,
                },
                &tx,
            )
            .await;
        let events = drain(&mut rx).await;
        let hash = match &events[0] {
            KernelEvent::Blob { ok, hash, media, .. } => {
                assert!(ok);
                assert_eq!(media, "image/png");
                hash.clone()
            }
            other => panic!("expected a blob, got {other:?}"),
        };
        kernel.execute(Request::BlobLoad { id: "b2".into(), hash: hash.clone() }, &tx).await;
        let events = drain(&mut rx).await;
        match &events[0] {
            KernelEvent::BlobBytes { ok, bytes, media, .. } => {
                assert!(ok);
                assert_eq!(bytes, &vec![0x89, b'P', b'N', b'G', 1, 2]);
                assert_eq!(media, "image/png");
            }
            other => panic!("expected bytes, got {other:?}"),
        }
        // A name that is not a hash is refused.
        kernel
            .execute(Request::BlobLoad { id: "b3".into(), hash: "../../etc/passwd".into() }, &tx)
            .await;
        let events = drain(&mut rx).await;
        assert!(matches!(&events[0], KernelEvent::BlobBytes { ok: false, .. }));
    }

    #[tokio::test]
    async fn a_credential_is_stored_by_slot_and_listed_without_its_value() {
        let kernel = kernel();
        let (tx, mut rx) = mpsc::unbounded_channel();
        kernel
            .execute(
                Request::Credential {
                    id: "a".into(),
                    action: CredentialAction::Set {
                        slot: "anthropic".into(),
                        account: "work".into(),
                        value: "sk-ant-0123456789".into(),
                    },
                },
                &tx,
            )
            .await;
        kernel.execute(Request::Credential { id: "b".into(), action: CredentialAction::List }, &tx).await;
        let events = drain(&mut rx).await;
        match events.last() {
            Some(KernelEvent::Credential { slots, .. }) => {
                let slot = &slots.as_list().expect("a list")[0];
                assert_eq!(slot.get("slot").and_then(Value::as_str), Some("anthropic"));
                assert_eq!(slot.get("account").and_then(Value::as_str), Some("work"));
                // The listing an event carries has no value in it.
                assert!(slot.get("value").is_none());
            }
            other => panic!("expected a listing, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_daemon_with_no_http_capability_says_so_rather_than_hanging() {
        let kernel = kernel();
        let (tx, mut rx) = mpsc::unbounded_channel();
        kernel
            .execute(
                Request::Http { id: "h1".into(), request: http::Request::get("http://example.invalid") },
                &tx,
            )
            .await;
        let events = drain(&mut rx).await;
        match &events[0] {
            KernelEvent::HttpFinished { ok, body, .. } => {
                assert!(!ok);
                assert!(body.contains("no HTTP capability"), "{body}");
            }
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_daemon_composed_in_memory_and_one_on_disk_agree_about_the_tool_they_advertise() {
        // A tool the kernel has and a policy does not declare is dead weight; a tool a
        // policy declares and the kernel does not have is a model being lied to. This
        // checks the intersection the session composes from.
        let names = kernel().tool_names();
        for tool in crate::tools::shipped(&crate::tools::default_shell_dir(), &mpsc::unbounded_channel().0) {
            assert!(names.contains(&tool.name().to_string()), "`{}` is missing", tool.name());
        }
    }
}

pub mod search;
pub use search::WebSearch;
pub use search::{Hit, urlencode};
