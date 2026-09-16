//! A daemon: a kernel, the sessions composed over it, and an iroh endpoint.
//!
//! ```sh
//! # the scripted provider, in memory, which needs nothing
//! misa-daemon --session demo
//! # a real provider, a durable log, and web search
//! misa-daemon --data-dir ~/.local/state/misa \
//!     --provider openai --base-url https://api.openai.com/v1 --api openai \
//!     --model gpt-5 --search searxng --search-url http://localhost:8080
//! # a subscription is authorized by a device code, and the token is stored for you
//! misa-daemon login openai-codex
//! # a service that takes a key is stored from a client's login panel, because a key on
//! # a command line is a key in a process listing
//! misa-daemon --plugin ~/plugins/guest.component.wasm   # a policy plugin, in wasm
//! ```
//!
//! It prints a ticket for each session it opens. Nothing about the session — not the model,
//! not the loop, not the views — is decided here: this binary selects a composition and
//! gives it a kernel. That is the whole of what a daemon is.
//!
//! # Who may attach
//!
//! By default anybody who knows a ticket, which the daemon says out loud when it starts.
//! `--allow <endpoint id>`, given once or more, restricts it to that list, and both the
//! session connection and the blob connection are checked against it: a peer that may read a
//! transcript may fetch the images in it, and a peer that may not read one may not.

use std::path::PathBuf;
use std::sync::Arc;

use misa_kernel::{
    AnthropicMessages, Blobs, Credentials, Daemon, Http, Kernel, LocalKernel, OpenAiChat, OpenAiResponses,
    Provider, ScriptedProvider, SearchBackend, SearchKind, Turn,
};
use misa_session::Runtime;
use misa_value::Value;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

struct Options {
    session: String,
    sessions: Vec<String>,
    data_dir: Option<PathBuf>,
    relay: bool,
    provider: String,
    base_url: Option<String>,
    /// Which wire shape, when the service is not one this daemon knows by name. `auto` takes
    /// it from the table.
    api: String,
    model: String,
    /// Print the services this daemon knows and exit, which is how somebody finds out that
    /// their provider is already there.
    list_providers: bool,
    /// Admit anybody with the ticket, rather than requiring a paired key.
    ///
    /// Off by default: a daemon that serves whatever can name a session is a daemon that
    /// should not be reachable, and the ticket is a name. Somebody who wants that says so.
    open: bool,
    /// Show a fresh pairing code and exit, for a daemon that is already running elsewhere.
    pair: bool,
    /// Endpoint ids to admit without pairing, for a script or a fixed client.
    allow: Vec<String>,
    search: Option<String>,
    search_url: String,
    once: bool,
    tool_approval: String,
    /// Wasm components to load as policy plugins, in the order given.
    ///
    /// Repeatable, and paths rather than names: what a daemon may run is a decision somebody
    /// makes where the daemon is, and a plugin's id is its own to declare.
    plugins: Vec<PathBuf>,
    /// `login <provider>`: authorize an account by a device code and store the
    /// token, then exit. A subcommand rather than a flag because it is a thing
    /// somebody does, once, and not a mode the daemon runs in.
    login: Option<String>,
}

/// The wire shapes this daemon has an adapter for, and every spelling a flag may name them by.
///
/// One table rather than a check beside the selection, because two lists of the same thing is
/// how a flag comes to be accepted and then not understood — and the other way round:
/// `--api openai.responses` was refused for a while by a check that named two shapes while the
/// daemon had three.
const APIS: &[(&str, &str)] = &[
    ("auto", "auto"),
    ("openai", "openai.chat"),
    ("openai.chat", "openai.chat"),
    ("anthropic", "anthropic.messages"),
    ("anthropic.messages", "anthropic.messages"),
    ("openai.responses", "openai.responses"),
];

/// The shape a `--api` flag names, or `None` for one this daemon cannot serve.
fn canonical_api(flag: &str) -> Option<&'static str> {
    APIS.iter().find(|(spelling, _)| *spelling == flag).map(|(_, shape)| *shape)
}

fn parse() -> Result<Options, String> {
    parse_from(std::env::args().skip(1))
}

/// [`parse`] over an argument list, so that a command line is testable without a process.
fn parse_from(mut arguments: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options {
        session: "demo".into(),
        sessions: vec![],
        data_dir: None,
        relay: true,
        provider: "scripted".into(),
        base_url: None,
        api: "auto".into(),
        model: "scripted-1".into(),
        list_providers: false,
        open: false,
        pair: false,
        allow: Vec::new(),
        search: None,
        search_url: "http://localhost:8080".into(),
        once: false,
        tool_approval: "allow".into(),
        plugins: Vec::new(),
        login: None,
    };
    while let Some(argument) = arguments.next() {
        let mut next = |name: &str| arguments.next().ok_or_else(|| format!("{name} needs a value"));
        match argument.as_str() {
            "--session" | "-s" => {
                let session = next("--session")?;
                if session.is_empty() { return Err("Session names must not be empty".into()); }
                if options.sessions.is_empty() { options.session = session.clone(); }
                if !options.sessions.contains(&session) { options.sessions.push(session); }
            }
            "--data-dir" | "-d" => options.data_dir = Some(PathBuf::from(next("--data-dir")?)),
            "--no-relay" => options.relay = false,
            "--provider" => options.provider = next("--provider")?,
            "--base-url" => options.base_url = Some(next("--base-url")?),
            "--api" => options.api = next("--api")?,
            "--model" => options.model = next("--model")?,
            "--providers" | "--list-providers" => options.list_providers = true,
            "--search" => options.search = Some(next("--search")?),
            "--search-url" => options.search_url = next("--search-url")?,
            "--once" => options.once = true,
            "--tool-approval" => {
                let policy = next("--tool-approval")?;
                if !matches!(policy.as_str(), "allow" | "ask" | "deny") {
                    return Err("--tool-approval expects allow, ask, or deny".into());
                }
                options.tool_approval = policy;
            }
            "--plugin" => options.plugins.push(PathBuf::from(next("--plugin")?)),
            // The one bare word this command line takes: `login <provider>` is
            // something a person does, and a `--login` flag would be the same thing
            // spelled less like what it is.
            "login" => options.login = Some(next("login")?),
            "--open" => options.open = true,
            "--pair" => options.pair = true,
            // Repeatable, because it is a list: a person who has one client to admit by name
            // usually has two.
            "--allow" => options.allow.push(next("--allow")?),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    if options.search.as_deref().is_some_and(|id| SearchKind::from_id(id).is_none()) {
        return Err(format!(
            "`{}` is not a search backend; there is brave, tavily, searxng",
            options.search.unwrap_or_default()
        ));
    }
    if canonical_api(&options.api).is_none() {
        return Err(format!(
            "`{}` is not an api this daemon speaks; there is auto, openai/openai.chat, \
             anthropic/anthropic.messages, and openai.responses",
            options.api
        ));
    }
    Ok(options)
}

/// The adapter for a provider, from the table this daemon knows by name.
///
/// A service that is not in the table is still usable when a base url was given — there are three
/// wire shapes and which one a service speaks is a flag — but it gets the oldest spelling of
/// everything, because that is what a service nobody described here accepts: no reasoning effort,
/// no thinking back, `max_tokens`.
fn adapter_for(provider: &str, options: &Options, http: Arc<Http>) -> Result<Arc<dyn Provider>, String> {
    let preset = misa_kernel::presets::preset(provider);
    if preset.is_none() && options.base_url.is_none() {
        return Err(format!(
            "`{provider}` is not a provider this daemon knows. Pass --base-url for a service of \
             your own, and --providers to see the ones it does:\n{}",
            misa_kernel::presets::ids().join(" ")
        ));
    }
    // `unwrap_or` and not a panic on the impossible arm: `parse` already refused a flag that
    // names no shape, and nothing in this system panics on input.
    let api = match canonical_api(&options.api).unwrap_or("openai.chat") {
        "auto" => preset.map_or("openai.chat", |preset| preset.api).to_string(),
        shape => shape.to_string(),
    };
    let custom = options.base_url.clone();
    Ok(match api.as_str() {
        "anthropic.messages" => {
            let mut messages = match preset {
                Some(preset) => AnthropicMessages::from_preset(preset, http),
                // `unwrap_or_default` is right here: the branch above refused a provider with
                // no preset and no base url, so one of the two exists.
                None => AnthropicMessages::new(provider, custom.clone().unwrap_or_default(), http)
                    .credentialed(provider),
            };
            if let Some(base) = custom {
                messages = messages.with_base_url(base);
            }
            Arc::new(messages)
        }
        // The responses api is not chat completions with a flag, and a daemon that
        // sent one to the other's endpoint would get a 404 from a service that is
        // perfectly happy.
        "openai.responses" => {
            let mut responses = match preset {
                Some(preset) => OpenAiResponses::from_preset(preset, http),
                None => OpenAiResponses::new(provider, custom.clone().unwrap_or_default(), http).credentialed(provider),
            };
            if let Some(base) = custom {
                responses = responses.with_base_url(base);
            }
            Arc::new(responses)
        }
        _ => {
            let mut chat = match preset {
                Some(preset) => OpenAiChat::from_preset(preset, http),
                None => OpenAiChat::new(provider, custom.clone().unwrap_or_default(), http).credentialed(provider),
            };
            if let Some(base) = custom {
                chat = chat.with_base_url(base);
            }
            Arc::new(chat)
        }
    })
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = parse()?;
    if options.list_providers {
        println!("{}", misa_kernel::presets::describe());
        return Ok(());
    }

    // A credential is read from a store, not from a command line: an argument is visible in
    // a process listing and in a shell's history, and a token in either is a token leaked.
    let credentials = Arc::new(Credentials::at(&Credentials::default_path())?);
    let http = Arc::new(Http::new(credentials.clone())?);

    // `login <provider>` is its own path out: it authorizes an account, stores the
    // token, and stops. Nothing about a session is involved, and making somebody
    // start one to log in would be a daemon whose credential story needed a client.
    if let Some(provider) = &options.login {
        login(provider, &credentials).await?;
        return Ok(());
    }

    // Which provider, and — separately — whether the facts are durable. Two questions, because
    // they are not the same question: a daemon with a data directory and the scripted provider
    // is a durable session with no account, which is what somebody trying this out has, and a
    // daemon that needed a credential to answer a script would be a daemon that looked broken
    // for exactly the reason the scripted provider exists.
    let mut kernel: Daemon = match options.provider.as_str() {
        "scripted" => LocalKernel::new(ScriptedProvider::new([
            Turn::call("echo", Value::str("a demonstration"), Turn::say("that is all I have.")),
            Turn::say("that is all I have."),
            Turn::say("still here."),
        ])),
        // A real service, from the table or from a base url. The credential slot is the
        // provider's id: a key for `openai` is stored from a client's login panel, and a
        // subscription like `openai-codex` is authorized by a device flow — started here by
        // the `login` subcommand, or by `/login openai-codex` from any client.
        provider => LocalKernel::new(adapter_for(provider, &options, http.clone())?)
            .with_default_provider(provider),
    };
    if let Some(data_dir) = &options.data_dir {
        // The durable store, composed in: the same kernel with a different store, a directory
        // for blobs, and a directory for what the shell runs.
        kernel = kernel
            .with_store(Arc::new(misa_kernel::SqliteStore::open(&data_dir.join("conversations.sqlite3"))?))
            .with_blobs(Arc::new(Blobs::at(&data_dir.join("blobs"))?))
            .with_shell(data_dir.join("shell"));
    }
    kernel = kernel.with_credentials(credentials.clone());
    if options.search.is_some() || options.data_dir.is_some() {
        kernel = kernel.with_http(http.clone());
    }
    if let Some(id) = &options.search {
        let kind = SearchKind::from_id(id).ok_or("unknown search backend")?;
        kernel = kernel.with_search(SearchBackend {
            kind,
            base_url: options.search_url.clone(),
            credential: match kind {
                // SearXNG is self-hosted and needs no account, which is why it is the one
                // that needs nothing configured.
                SearchKind::Searxng => None,
                other => Some(other.id().to_string()),
            },
            limit: 8,
        });
    }
    // The blob store comes out of the kernel before it is boxed: the daemon serves the same
    // store the kernel writes, because a session's images have to be reachable from a client
    // and not only from the process that produced them.
    let blobs = kernel.blobs().clone();
    let store = kernel.store().clone();
    let kernel: Arc<dyn Kernel> = Arc::new(kernel);

    // Who may attach. Decided once, here, and applied to both connections a daemon serves:
    // a session and the bytes its views point at are the same trust decision.
    //
    // The default is a paired key rather than a ticket, because a ticket is a *name*: anybody
    // who reads one over a shoulder can use it, and a session is not a public thing. `--open`
    // is what a person means when they say "just let me in", and it says so out loud.
    let admission = Arc::new(if options.open {
        misa_transport::admission::Admission::open()
    } else {
        let store = match &options.data_dir {
            Some(dir) => misa_transport::admission::Paired::at(dir.join("paired"))?,
            // A daemon with no data directory keeps nothing, which is what a temporary daemon
            // is: pairing works while it runs and is gone when it stops.
            None => misa_transport::admission::Paired::in_memory(),
        };
        let mut admission = misa_transport::admission::Admission::paired(store);
        for peer in options.allow.clone() {
            admission = admission.also(peer);
        }
        admission
    });

    let identity = options.data_dir.as_ref().map(|dir| misa_transport::identity::load(&dir.join("daemon.identity"))).transpose()?;
    let endpoint = misa_transport::iroh::bind(identity, options.relay).await?;

    let sessions = misa_transport::iroh::Sessions::new();
    let directory = misa_daemon::directory::Directory::fresh().map_err(|fault| fault.message)?;
    {
        let kernel = kernel.clone();
        let provider = options.provider.clone();
        let model = options.model.clone();
        let paths = options.plugins.clone();
        let approval = options.tool_approval.clone();
        let owner = Arc::downgrade(&directory);
        directory.install_factory(Arc::new(move |_, spec: misa_daemon::lifecycle::SessionSpec| {
            let owner = owner.clone();
            let config = Value::map([("tool_approval", Value::str(&approval)), ("parent_attempt", spec.parent_attempt.as_ref().map(Value::str).unwrap_or(Value::Null))]);
            let (kernel, store, provider, model, paths) = (kernel.clone(), store.clone(), provider.clone(), model.clone(), paths.clone());
            Box::pin(async move { tokio::task::spawn_blocking(move || {
                let conversation = spec.conversation.as_deref().unwrap_or(&spec.id);
                let exists = !store.load(conversation, 0, 1).map_err(|message| misa_proto::Fault::new("storage", message))?.is_empty();
                if spec.conversation.is_some() && !exists && !spec.recovering { return Err(misa_proto::Fault::new("missing_conversation", "Stored conversation is unavailable")); }
                if spec.conversation.is_none() && exists { return Err(misa_proto::Fault::new("existing_conversation", "Use resume for a stored conversation")); }
                let contribution = plugins(&paths).map_err(|error| misa_proto::Fault::new("composition", error.to_string()))?;
                let contribution = misa_daemon::delegation::install(contribution, owner);
                Ok(Runtime::prepare_with(spec.id, spec.title, spec.conversation, kernel,
                    spec.provider.unwrap_or(provider), spec.model.unwrap_or(model), config, contribution))
            }).await.map_err(|error| misa_proto::Fault::new("composition", error.to_string()))? })
        })).map_err(|fault| fault.message)?;
    }
    if let Some(data_dir) = &options.data_dir {
        directory.install_membership(Arc::new(misa_daemon::membership::File::at(data_dir.join("active-sessions.cbor")))).await.map_err(|fault| fault.message)?;
        directory.install_work_log(data_dir.join("delegated-work.cbor")).await.map_err(|fault|fault.message)?;
    }
    let host = misa_protocol::invocation::CallContext { principal: endpoint.id().to_string(), connection: 0 };
    for (id, fault) in directory.restore(&host).await { eprintln!("cannot restore {id}: {}", fault.message); }
    let names = if options.sessions.is_empty() { vec![options.session.clone()] } else { options.sessions.clone() };
    for name in &names {
        if directory.sessions().iter().any(|runtime| runtime.id() == name) { continue; }
        directory.open(&host, misa_daemon::lifecycle::SessionSpec {
            id: name.clone(), title: format!("{} ({})", name, options.provider), conversation: Some(name.clone()),
            provider: Some(options.provider.clone()), model: Some(options.model.clone()), parent_attempt: None, recovering: true,
        }).await.map_err(|fault| fault.message)?;
    }
    for runtime in directory.sessions() { sessions.insert(runtime); }

    // The ticket is printed before anything waits on the network. An endpoint's identity
    // exists as soon as it is bound, and a daemon that says nothing until it has found a
    // relay is a daemon that looks broken on a machine with no route to one — which is
    // exactly the machine somebody runs it on first.
    for name in &names { println!("{}", misa_transport::iroh::ticket(&endpoint, name)); }
    if options.data_dir.is_some() {
        eprintln!("data in {}", options.data_dir.as_ref().expect("just checked").display());
    }
    eprintln!(
        "credentials: {}",
        match credentials.slots().len() {
            0 => "none stored (a provider that needs one will say so)".to_string(),
            count => format!("{count} slots"),
        }
    );
    // The node string a ticket carries, not the endpoint object: what an invitation and a
    // command line both need is something a client can be told, and the transport type stays
    // inside the transport.
    let node = misa_transport::iroh::node_of(&endpoint);
    // A daemon that admits anybody should say so out loud, and a daemon that admits only paired
    // keys should say how many and how to add one. Either way it is one line, and it names the
    // command that changes it.
    eprintln!("admission: {}", admission.describe());
    // A code, printed as a QR and as text. Shown when a daemon requires pairing and nobody has
    // paired yet, or whenever somebody asks for one: the code is how a client that has never
    // met this daemon becomes one that may attach.
    if !options.open && (admission.peers().is_empty() || options.pair) {
        show_invitation(&admission, &node, &options.session);
    }
    if options.once {
        return Ok(());
    }
    // The router exists before anything waits on the network, and that order is the whole
    // point: waiting to be online is what makes a dial from *another* machine reach this one,
    // and it must not be on the path to answering a client on this one. A machine with no
    // route out never becomes online, so a daemon that waited first would print a ticket and
    // then ignore every peer that used it — on exactly the machine somebody tries first.
    let scoped = misa_transport::scoped_server::Handler {
        daemon: endpoint.id().to_string(),
        scope: misa_protocol::invocation::CommandOwner::scope(directory.as_ref()),
        resolver: Arc::new(misa_daemon::directory::Routes(directory.clone())),
        admission: admission.clone(),
    };
    let router = misa_transport::server::serve_with_scopes(endpoint.clone(), sessions, Arc::new(blobs::Store(blobs)), admission.clone(), scoped);
    #[cfg(unix)]
    let _local = misa_transport::local::advertise(&node, &options.session, admission.clone())?;
    let online = endpoint.clone();
    tokio::spawn(async move {
        online.online().await;
    });
    // Standard input is a small console, and it is the one a daemon running in the foreground
    // actually has: a `pair` line shows a new code, `peers` lists the keys that may attach,
    // and `revoke <id>` forgets one. All three are decisions a person makes while the daemon
    // runs, which is why they are here rather than in a flag.
    let console = console(admission.clone(), node.clone(), options.session.clone());
    tokio::pin!(console);
    use std::io::IsTerminal as _;
    let interactive = std::io::stdin().is_terminal();
    tokio::select! {
        result = tokio::signal::ctrl_c() => result?,
        () = &mut console => {
            // Services commonly have /dev/null as stdin.
            if !interactive { tokio::signal::ctrl_c().await?; }
        },
    }
    directory.shutdown_complete().await;
    let _ = router.shutdown().await;
    Ok(())
}

/// Authorize an account by a device code, and store the token it produces.
///
/// The whole of `misa-daemon login <provider>`. A service that takes a key has no
/// flow here, and the answer says so — and names the ones that do — rather than
/// parking a terminal waiting for a code nobody is going to show.
async fn login(provider: &str, credentials: &Arc<Credentials>) -> Result<(), Box<dyn std::error::Error>> {
    let Some(flow) = misa_kernel::presets::oauth(provider) else {
        let with_flows: Vec<&str> = misa_kernel::presets::ids()
            .into_iter()
            .filter(|id| misa_kernel::presets::oauth(id).is_some())
            .collect();
        return Err(format!(
            "`{provider}` has no device flow; it takes a key, which the daemon's login panel stores. \
             The services with a flow are: {}",
            with_flows.join(" ")
        )
        .into());
    };
    eprintln!("authorizing `{provider}`; a code is about to be shown");
    let token = misa_kernel::oauth::login(
        &flow,
        |prompt| {
            eprintln!();
            eprintln!("  open {}", prompt.url);
            eprintln!("  and enter {}", prompt.code);
            eprintln!();
        },
        || false,
    )
    .await?;
    credentials.set_oauth(
        provider,
        misa_kernel::OAuth {
            account: token.account.clone(),
            access: token.access.clone(),
            refresh: token.refresh.clone(),
            expires_ms: token.expires_ms,
            token_url: flow.token_url.to_string(),
            client_id: flow.client_id.to_string(),
        },
    )?;
    eprintln!(
        "stored a token for `{provider}` (account `{}`); it will renew itself from here",
        if token.account.is_empty() { "unknown" } else { &token.account }
    );
    Ok(())
}

/// Load the plugins this daemon was told to run, as what a session registers.
///
/// Validation happens here, at install, and against the session's own list of accepted effects:
/// a plugin that asks for something this composition cannot do is refused *now*, with a message
/// that names it and the kind, rather than failing inside somebody's transaction later. Its
/// declared state roots become roots of the session's database, which is what makes its patches
/// land — a patch may only create the *last* key of its path, so somebody has to make the root
/// first, and the composition is the only layer that knows what it loaded.
fn plugins(paths: &[PathBuf]) -> Result<misa_session::Contribution, String> {
    let mut contribution = misa_session::Contribution::new();
    if paths.is_empty() {
        return Ok(contribution);
    }
    for path in paths {
        let bytes = std::fs::read(path).map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let plugin = Arc::new(
            misa_plugin::Plugin::load(&bytes)
                .map_err(|fault| format!("{}: {} ({})", path.display(), fault.message, fault.code))?,
        );
        plugin.configure(&[]).map_err(|fault| {
            format!("`{}` could not be configured: {}", plugin.descriptor().id, fault.message)
        })?;
        plugin.validate(&misa_session::AcceptedEffects).map_err(|fault| {
            format!("`{}` is not something this daemon can run: {}", plugin.descriptor().id, fault.message)
        })?;

        let descriptor = plugin.descriptor().clone();
        plugin.authorize_reads(&descriptor.roots).map_err(|fault| fault.message)?;
        eprintln!(
            "plugin `{}` {}: handling {:?}, answering {:?}, roots {:?}",
            descriptor.id, descriptor.version, descriptor.events, descriptor.queries, descriptor.roots
        );
        for (kind, handler) in plugin.handlers() {
            contribution = contribution.with_handler(kind, misa_plugin::PLUGIN_PRIORITY, handler);
        }
        for (name, subscription) in plugin.subscriptions() {
            contribution = contribution.with_subscription(name, subscription);
        }
        for definition in &descriptor.queries {
            contribution = contribution.export_query(definition.export());
        }
        for command in &descriptor.commands {
            contribution = contribution.with_command(misa_session::commands::CommandRegistration::event(
                &command.id, command.input.clone(), &command.event,
            ));
        }
        for binding in &descriptor.bindings { contribution = contribution.with_binding(binding.clone()); }
        for tool in &descriptor.tools { contribution = contribution.with_tool(tool.clone()); }
        // Every presentation is selected independently by each client.
        for presentation in &descriptor.presentations {
            contribution = contribution.with_presentation(presentation.clone());
        }
        for root in &descriptor.roots {
            // Empty, because what a plugin keeps in its own root is its own business and its
            // first patch is what fills it. A name the session already owns is refused in here.
            contribution = contribution.with_root(root, Value::map([])).map_err(|fault| {
                format!("`{}` asked for a state root it may not have: {}", descriptor.id, fault.message)
            })?;
        }
    }
    Ok(contribution)
}

/// Show a pairing code: as a QR a camera can read, and as text a person can type.
///
/// Both, because both are real: a phone reads the square, and a terminal on another machine
/// gets the line. What is encoded is one string — the ticket and the code — so scanning and
/// typing carry exactly the same information, and the client needs nothing else to connect.
fn show_invitation(admission: &misa_transport::admission::Admission, node: &str, session: &str) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as i64)
        .unwrap_or(0);
    let invitation = admission.invite(misa_transport::admission::INVITATION_TTL_MS, now);
    let pairing = misa_proto::Pairing::new(
        misa_proto::Ticket { node: node.to_string(), session: session.to_string() },
        invitation.code(),
    );
    let text = pairing.to_string();
    match qr(&text) {
        Ok(square) => eprintln!("{square}"),
        Err(error) => eprintln!("(no qr code: {error})"),
    }
    eprintln!("pair this client by scanning the code, or by running:");
    eprintln!("    misa {text}           # any client takes it in place of a ticket");
    eprintln!(
        "it is good for {} minutes, and pairs one client.",
        invitation.seconds_left(now) / 60
    );
}

/// The pairing string as a QR code, drawn in the terminal.
///
/// Half-blocks, because a terminal cell is twice as tall as it is wide: two rows of the code
/// per line of text, which is the difference between a square a camera can read and a
/// rectangle it cannot.
fn qr(text: &str) -> Result<String, String> {
    use qrcode::render::unicode;
    let code = qrcode::QrCode::new(text.as_bytes()).map_err(|err| err.to_string())?;
    Ok(code
        .render::<unicode::Dense1x2>()
        .dark_color(unicode::Dense1x2::Light)
        .light_color(unicode::Dense1x2::Dark)
        .build())
}

/// The daemon's console: one line, one decision.
async fn console(admission: Arc<misa_transport::admission::Admission>, node: String, session: String) {
    // Tokio's stdin uses a blocking pool read that runtime shutdown waits for.
    // A dedicated thread does not hold shutdown hostage while waiting for input.
    let (sender, mut lines) = tokio::sync::mpsc::channel(16);
    std::thread::spawn(move || {
        use std::io::BufRead as _;
        for line in std::io::stdin().lock().lines() {
            let Ok(line) = line else { break; };
            if sender.blocking_send(line).is_err() { break; }
        }
    });
    while let Some(line) = lines.recv().await {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }
        let (verb, rest) = match line.split_once(char::is_whitespace) {
            Some((verb, rest)) => (verb.to_string(), rest.trim().to_string()),
            None => (line.clone(), String::new()),
        };
        match verb.as_str() {
            "pair" => show_invitation(&admission, &node, &session),
            "peers" => {
                let peers = admission.peers();
                if peers.is_empty() {
                    eprintln!("no key has been paired yet");
                }
                for peer in peers {
                    eprintln!("{}  {}  added {}", peer.id, peer.label, peer.added_ms);
                }
            }
            "revoke" => match admission.revoke(&rest) {
                Ok(true) => eprintln!("forgot {rest}"),
                Ok(false) => eprintln!("no key `{rest}` was paired"),
                Err(error) => eprintln!("could not forget it: {error}"),
            },
            "admission" => eprintln!("{}", admission.describe()),
            other => eprintln!("there is no command `{other}`; there is pair, peers, revoke, admission"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A command line, as the argument list `parse` works over.
    fn arguments(line: &str) -> std::vec::IntoIter<String> {
        line.split_whitespace().map(str::to_string).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn every_shape_this_daemon_has_an_adapter_for_is_a_flag_it_accepts() {
        // The flag and the selection read one table, and this is what keeps them one:
        // `--api openai.responses` was refused by a check that named two shapes while
        // `adapter_for` had three, which is a daemon that cannot be pointed at a service it
        // can speak to.
        for shape in ["openai.chat", "anthropic.messages", "openai.responses"] {
            let parsed = parse_from(arguments(&format!(
                "--provider custom --base-url http://x --api {shape}"
            )))
            .unwrap_or_else(|error| panic!("`{shape}` was refused: {error}"));
            assert_eq!(parsed.api, shape);
        }
        // The short spellings name the same shapes, and `auto` defers to the table.
        assert_eq!(canonical_api("openai"), Some("openai.chat"));
        assert_eq!(canonical_api("anthropic"), Some("anthropic.messages"));
        assert_eq!(canonical_api("auto"), Some("auto"));
    }

    #[test]
    fn an_api_this_daemon_cannot_speak_is_refused_by_name() {
        let error = parse_from(arguments("--api chat")).err().expect("no adapter speaks `chat`");
        assert!(error.contains("openai.responses"), "the refusal should name the shapes: {error}");
    }

    #[test]
    fn a_plugin_is_a_path_given_once_or_more() {
        // Repeatable because it is a list, and a path because what a daemon runs is a decision
        // somebody makes where the daemon is.
        let parsed = parse_from(arguments("--plugin a.wasm --plugin b.wasm")).expect("a command line");
        assert_eq!(parsed.plugins, vec![PathBuf::from("a.wasm"), PathBuf::from("b.wasm")]);
        assert!(parse_from(arguments("--plugin")).is_err(), "a plugin with no path");
        let none = parse_from(arguments("--session demo")).expect("a command line");
        assert!(none.plugins.is_empty(), "a daemon with no plugins runs none");
    }

    #[test]
    fn login_names_a_provider_and_a_key_is_never_an_argument() {
        // The command line has no way to carry a secret: it names what to authorize, and the
        // value comes from a device flow — or, for a service with a key, from a client's login
        // panel, which is the whole reason there is no `--token`.
        let parsed = parse_from(arguments("login openai-codex")).expect("a login");
        assert_eq!(parsed.login.as_deref(), Some("openai-codex"));
        assert!(parse_from(arguments("login")).is_err(), "a login with nothing to authorize");
    }
}

mod blobs;
