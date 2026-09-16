//! One browser presentation instance over shared daemon/client lifetimes.
use super::{Region, Remote, Source};
use misa_client::{
    daemons::Daemon,
    interaction::{Interaction, Prepared},
    interface::Interface,
};
use misa_proto::{Intent, invocation::Outcome, observation::Selection};
#[cfg(test)]
use misa_value::Value;
use std::{collections::BTreeMap, sync::Arc, time::Duration};

#[cfg(test)]
async fn connect(daemon: &Arc<Daemon>, session_id: &str) -> Result<Arc<Remote>, String> {
    connect_with(daemon, session_id, Default::default()).await
}
pub(crate) async fn connect_with(daemon: &Arc<Daemon>, session_id: &str, initial: misa_client::composition::Preferences) -> Result<Arc<Remote>, String> {
    let mut identity = [0u8; 16];
    getrandom::fill(&mut identity).map_err(|error| format!("Unable to create presentation identity: {error}"))?;
    let instance = identity.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
    let snapshot = daemon.sessions().map_err(|fault| fault.message)?;
    if !matches!(snapshot.status, misa_protocol::observation::Status::Current) {
        return Err("Daemon directory is not current".into());
    }
    let entry = snapshot
        .sessions
        .into_iter()
        .find(|entry| entry.id == session_id)
        .ok_or("Session is no longer available")?;
    let interface = Interface::load(&daemon.client, entry.scope())
        .await
        .map_err(|fault| fault.message)?;
    let selected = initial.reconcile(&interface.presentations, &super::presentations::capabilities(), &["conversation", "status"]);
    let members = selected.members;
    let mut readers: BTreeMap<_, _> = members.keys().map(|id| (id.clone(), misa_client::document::Reader::new(id))).collect();
    let scope = interface.scope.clone();
    let interaction = Arc::new(
        Interaction::load(&daemon.client, interface)
            .await
            .map_err(|fault| fault.message)?,
    );
    let mut observation = if members.is_empty() { None } else { Some(daemon
        .client
        .observe(
            Selection {
                scope,
                members,
            },
            None,
        )
        .await
        .map_err(|fault| fault.message)?) };
    let region = Region::new();
    if observation.is_none() { region.set("<p>No supported content is selected. Choose a supported variant in Presentations.</p>".into()); }
    let activity = super::activity::start(&daemon.client, &interaction.interface, region.clone()).await.map_err(|fault| fault.message)?;
    let stream = region.clone();
    let preferences = Arc::new(std::sync::Mutex::new(initial));
    let saved_preferences = preferences.clone();
    let (presentation_changes, mut changes) = tokio::sync::mpsc::channel::<super::presentations::Change>(1);
    let task = tokio::spawn(async move {
        loop {
            let updates = observation.as_ref().map(|observation| misa_client::document::capture_many(observation, readers.iter_mut().map(|(id, reader)| (id.as_str(), reader)))).unwrap_or_default();
            if !updates.is_empty() {
                if let Err(error) = stream.observed_documents(updates) {
                    stream.set(format!("<p role=\"alert\">{}</p>", super::escape(&error)));
                    for (id, reader) in &mut readers { *reader = misa_client::document::Reader::new(id); }
                    continue;
                }
            }
            tokio::select! {
                changed = async { match observation.as_mut() { Some(observation) => observation.changed().await, None => std::future::pending().await } } => { if changed.is_err() { break; } }
                change = changes.recv() => {
                    let Some(change) = change else { break; };
                    let mut next_readers: BTreeMap<_, _> = change.members.iter().map(|id| (id.clone(), misa_client::document::Reader::new(id))).collect();
                    let updates = misa_client::document::capture_many(&change.observation, next_readers.iter_mut().map(|(id, reader)| (id.as_str(), reader)));
                    let result = stream.replace_documents(updates);
                    if result.is_ok() {
                        observation = Some(change.observation);
                        readers = next_readers;
                        *saved_preferences.lock().unwrap() = change.preferences;
                    }
                    let _ = change.reply.send(result);
                }
            }
        }
    });
    Ok(Arc::new(Remote {
        closed: tokio::sync::watch::channel(false).0,
        activity,
        claimed: std::sync::atomic::AtomicBool::new(false),
        presentation_changes,
        presentation_gate: tokio::sync::Mutex::new(()),
        preferences,
        region,
        instance,
        session: entry,
        daemon: daemon.clone(),
        interaction,
        task: task.abort_handle(),
        blobs: Some(Arc::new(Source::Remote(daemon.blobs.clone()))),
        pending: Arc::new(std::sync::Mutex::new(vec![])),
    }))
}

pub fn prepare(interaction: &Interaction, intent: Intent) -> Result<Prepared, String> {
    match intent {
        Intent::Prompt { text, attachments } => interaction.prompt(text, attachments, false),
        Intent::Interrupt { text, attachments } => interaction.prompt(text, attachments, true),
        Intent::Cancel { target } => interaction.cancel(target),
        Intent::Command { name, args } => interaction.shortcut(&name, args),
        Intent::Action { action, fields, .. } => {
            let model = misa_client::form::Form::action(&interaction.interface, &action).map_err(|fault| fault.message)?;
            let (command, input) = model.prepare(&fields.into_iter().map(|field| (field.id, field.value)).collect()).map_err(|fault| fault.message)?;
            interaction.invoke(&command, input)
        },
        Intent::Complete { .. } => return Err("Completion uses the finite query interface".into()),
    }
    .map_err(|fault| fault.message)
}

pub fn submitted(
    remote: &Remote,
    form: &std::collections::HashMap<String, String>,
    attachments: &[misa_proto::view::BlobRef],
) -> Result<Intent, String> {
    if form.get("action").map(String::as_str) != Some(super::COMPOSER) {
        return Ok(super::intent_from_form(form));
    }
    let text = form.get("prompt").map(String::as_str).unwrap_or("");
    let commands = declarations(&remote.interaction);
    use misa_kit::intent::Parsed;
    match misa_kit::intent::parse(text, &commands) {
        Parsed::Prompt(text) => Ok(Intent::Prompt {
            text,
            attachments: attachments.to_vec(),
        }),
        Parsed::Command { name, args } => Ok(Intent::Command { name, args }),
        Parsed::Empty if !attachments.is_empty() => Ok(Intent::Prompt {
            text: String::new(),
            attachments: attachments.to_vec(),
        }),
        Parsed::Empty => Err("Enter a message first".into()),
        Parsed::Unknown { name } => Err(format!("Unknown command: /{name}")),
        Parsed::Needs {
            command, argument, ..
        } => Err(format!("/{command} needs {argument}")),
    }
}

pub async fn invoke(remote: &Remote, prepared: Prepared) -> Result<Outcome, String> {
    if *remote.closed.borrow() { return Ok(Outcome::Rejected { fault: misa_proto::Fault::new("closed", "Presentation instance closed") }); }
    let Prepared::Invoke { command, input } = prepared else {
        return Err("This action is a read-only report".into());
    };
    let permit = match remote.activity.capacity.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return Ok(Outcome::Rejected { fault: misa_proto::Fault::new("busy", "Too many pending operations; wait for one to finish") }),
    };
    let delivery = match remote.activity.accepted.reserve().await {
        Ok(delivery) => delivery,
        Err(_) => return Ok(Outcome::Rejected { fault: misa_proto::Fault::new("closed", "Presentation instance closed") }),
    };
    let outcome = remote
        .daemon
        .client
        .invoke(
            remote.interaction.interface.scope.clone(),
            command,
            input,
            Duration::from_secs(20),
        )
        .await
        .map_err(|fault| fault.message)?
        .outcome;
    if let Outcome::Accepted { operation } = &outcome {
        let document = operation.scope == remote.interaction.interface.scope && remote.interaction.interface.queries.contains_key("operation.presentation");
        let watch = misa_client::operation::Watch::open(&remote.daemon.client, &remote.interaction.interface, operation.clone(), document).await
            .unwrap_or_else(|fault| misa_client::operation::Watch::failed(operation.clone(), fault));
        delivery.send(super::activity::Accepted { watch, permit });
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_protocol::invocation::CommandOwner;

    #[tokio::test]
    async fn scoped_web_instance_observes_and_invokes_without_a_legacy_connection() {
        use misa_proto::{invocation::{Command, Binding, ActionBinding}, schema::{Schema, Field}};
        let actions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let captured = actions.clone();
        let contribution = misa_session::Contribution::new().with_command(misa_session::commands::CommandRegistration::new(
            Command { preparation: Default::default(), id: "example.feed".into(), input: Schema::Record { fields: BTreeMap::from([
                ("amount".into(), Field { schema: Schema::Int, optional: false }),
                ("label".into(), Field { schema: Schema::String, optional: false }),
                ("pet".into(), Field { schema: Schema::String, optional: false }),
            ]), allow_unknown: false }, result: Schema::Value },
            move |_, _, invocation| { captured.lock().unwrap().push(invocation.input.clone()); Outcome::Completed { value: Value::Null } },
        )).with_binding(Binding { id: "example.feed".into(), binding: ActionBinding { command: "example.feed".into(), bound: BTreeMap::from([("pet".into(), Value::str("owned"))]), inputs: BTreeMap::from([("quantity".into(), "amount".into()), ("label".into(), "label".into())]) } });
        let form_values = Arc::new(std::sync::Mutex::new(Vec::new()));
        let collected = form_values.clone();
        let form_schema = Schema::Record { fields: BTreeMap::from([("count".into(), Field { schema: Schema::Int, optional: false })]), allow_unknown: false };
        let contribution = contribution.with_command(misa_session::commands::CommandRegistration::new(
            Command { preparation: Default::default(), id: "example.form".into(), input: Schema::Record { fields: BTreeMap::from([("value".into(), Field { schema: form_schema.clone(), optional: false })]), allow_unknown: false }, result: Schema::Value },
            move |_, _, invocation| { collected.lock().unwrap().push(invocation.input.clone()); Outcome::Completed { value: Value::Null } },
        ));
        let runtime = misa_session::Runtime::start_with(
            "web",
            "Web",
            None,
            Arc::new(misa_kernel::LocalKernel::new(
                misa_kernel::ScriptedProvider::new([
                    misa_kernel::Turn::say("scoped web reply"),
                    misa_kernel::Turn::call("echo", Value::str("approved web tool"), misa_kernel::Turn::say("tool completed")),
                    misa_kernel::Turn::say("tool completed"),
                ]),
            )),
            "scripted",
            "scripted-1",
            Value::map([("tool_approval", Value::str("ask"))]),
            contribution,
        );
        let directory = misa_daemon::directory::Directory::new("web-daemon").unwrap();
        directory.insert(runtime.clone()).unwrap();
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
        let daemons = misa_client::daemons::Daemons::new(
            endpoint.clone(),
            misa_proto::ClientInfo::new("web-test", "1"),
        );
        let daemon = daemons.connect(server.addr()).await.unwrap();
        let mut changes = daemon.watch();
        tokio::time::timeout(Duration::from_secs(5), async {
            while !matches!(
                daemon.sessions().unwrap().status,
                misa_protocol::observation::Status::Current
            ) {
                changes.changed().await.unwrap();
            }
        })
        .await
        .unwrap();
        let remote = connect(&daemon, "web").await.unwrap();
        let other = connect(&daemon, "web").await.unwrap();
        assert!(
            prepare(
                &remote.interaction,
                Intent::Command {
                    name: "not-installed".into(),
                    args: Value::Null
                }
            )
            .is_err()
        );
        use tower::ServiceExt;
        let mut uploading = connect(&daemon, "web").await.unwrap();
        Arc::get_mut(&mut uploading).unwrap().blobs = Some(Arc::new(Source::Local(Arc::new(misa_kernel::Blobs::in_memory()))));
        let attachment = || axum::http::Request::builder().method("POST").uri("/attach")
            .header("content-type", "multipart/form-data; boundary=web-upload")
            .body(axum::body::Body::from(format!("--web-upload\r\nContent-Disposition: form-data; name=\"file\"; filename=\"notes.txt\"\r\nContent-Type: text/plain\r\n\r\n{}\r\n--web-upload--\r\n", "x".repeat(2 * 1024 * 1024 + 1)))).unwrap();
        for _ in 0..2 {
            let uploaded = super::super::remote_router(uploading.clone()).oneshot(attachment()).await.unwrap();
            assert_eq!(uploaded.status(), axum::http::StatusCode::SEE_OTHER, "valid blobs above the HTTP framework default body limit must upload");
        }
        assert_eq!(uploading.pending.lock().unwrap().len(), 1, "repeated upload does not duplicate staging");
        {
            let mut pending = uploading.pending.lock().unwrap();
            pending.clear();
            pending.extend((0..32).map(|id| misa_proto::view::BlobRef { hash: format!("{id:064x}"), len: 1, media: None }));
        }
        assert_eq!(super::super::remote_router(uploading.clone()).oneshot(attachment()).await.unwrap().status(), axum::http::StatusCode::BAD_REQUEST);
        assert_eq!(uploading.pending.lock().unwrap().len(), 32);
        uploading.close();
        assert_eq!(super::super::remote_router(uploading.clone()).oneshot(attachment()).await.unwrap().status(), axum::http::StatusCode::GONE);
        drop(uploading);
        let obsolete = connect_with(&daemon, "web", misa_client::composition::Preferences(BTreeMap::from([
            ("conversation".into(), misa_client::composition::Choice::Variant("removed".into())),
            ("status".into(), misa_client::composition::Choice::Hidden),
        ]))).await.unwrap();
        assert!(obsolete.region.get().contains("No supported content"));
        assert!(super::super::presentations::controls(&obsolete).contains("removed (unavailable)"));
        let repaired = super::super::remote_router(obsolete.clone()).oneshot(axum::http::Request::builder().method("POST").uri("/presentations")
            .header("content-type", "application/x-www-form-urlencoded").body(axum::body::Body::from("id=conversation&choice=auto")).unwrap()).await.unwrap();
        assert_eq!(repaired.status(), axum::http::StatusCode::SEE_OTHER);
        assert!(obsolete.region.get().contains("data-presentation=\"conversation\""));
        assert!(!obsolete.region.get().contains("data-presentation=\"status\""));
        drop(obsolete);
        let request = |prompt: &str| {
            axum::http::Request::builder()
                .method("POST")
                .uri("/intent")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("accept", "application/json")
                .body(axum::body::Body::from(format!(
                    "action=composer.submit&prompt={prompt}"
                )))
                .unwrap()
        };
        let refused = super::super::remote_router(remote.clone())
            .oneshot(request("%2Fmissing"))
            .await
            .unwrap();
        assert_eq!(refused.status(), axum::http::StatusCode::BAD_REQUEST);
        let prepare_model = super::super::remote_router(remote.clone()).oneshot(request("%2Fmodel")).await.unwrap();
        let status = prepare_model.status();
        let body = axum::body::to_bytes(prepare_model.into_body(), 65536).await.unwrap();
        assert_eq!(status, axum::http::StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let prepared: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(prepared["preparation"], true);
        let html = prepared["report"].as_str().unwrap();
        assert!(html.contains("field.model") && html.contains("scripted-1") && html.contains("data-source=\"models\""));
        let providers = super::super::remote_router(remote.clone()).oneshot(axum::http::Request::builder().uri("/command?shortcut=login").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(providers.status(), axum::http::StatusCode::OK);
        let html = String::from_utf8(axum::body::to_bytes(providers.into_body(), 65536).await.unwrap().to_vec()).unwrap();
        assert!(html.contains("field.provider") && html.contains("groq"));
        let filtered = super::super::remote_router(remote.clone()).oneshot(axum::http::Request::builder().uri("/completions?source=providers&q=groq").body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(filtered.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(filtered.into_body(), 65536).await.unwrap();
        let candidates: misa_proto::preparation::Candidates = serde_json::from_slice(&body).unwrap();
        assert_eq!(candidates.items.len(), 1);
        assert_eq!(candidates.items[0].value, "groq");
        assert!(!remote.region.get().contains("scoped web reply"), "preparation does not execute a prompt");
        let accepted = super::super::remote_router(remote.clone())
            .oneshot(request("hello"))
            .await
            .unwrap();
        assert_eq!(accepted.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(accepted.into_body(), 4096)
            .await
            .unwrap();
        let accepted: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(accepted["ok"], true);
        assert_eq!(accepted["outcome"]["status"], "accepted");
        let operation: misa_proto::invocation::OperationRef =
            serde_json::from_value(accepted["outcome"]["operation"].clone()).unwrap();
        assert_eq!(operation.scope, remote.interaction.interface.scope);
        assert!(!operation.id.is_empty());
        let report = super::super::remote_router(remote.clone())
            .oneshot(request("%2Fstatus"))
            .await
            .unwrap();
        assert_eq!(report.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(report.into_body(), 65536)
            .await
            .unwrap();
        assert!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap()["report"]
                .as_str()
                .unwrap()
                .contains("scripted")
        );
        tokio::time::timeout(Duration::from_secs(5), async {
            while !remote.region.get().contains("scoped web reply")
                || !other.region.get().contains("scoped web reply")
            {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .unwrap();
        let configure = |choice: &str| axum::http::Request::builder().method("POST").uri("/presentations")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(axum::body::Body::from(format!("id=status&choice={choice}"))).unwrap();
        let gate = remote.presentation_gate.lock().await;
        let busy = super::super::remote_router(remote.clone()).oneshot(configure("hide")).await.unwrap();
        assert_eq!(busy.status(), axum::http::StatusCode::CONFLICT);
        drop(gate);
        let invalid = super::super::remote_router(remote.clone()).oneshot(configure("variant%3Amissing")).await.unwrap();
        assert_eq!(invalid.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(remote.region.get().contains("data-presentation=\"status\""));
        // Credential detail is a private finite read; visibility and drafts are
        // local, while resolution updates every observer of the shared work.
        let login = super::super::remote_router(remote.clone()).oneshot(axum::http::Request::builder().method("POST").uri("/perform").header("content-type", "application/x-www-form-urlencoded").header("accept", "application/json").body(axum::body::Body::from("kind=command&action_id=credentials.authorize&field.provider=groq")).unwrap()).await.unwrap();
        assert_eq!(login.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(login.into_body(), 65536).await.unwrap();
        let login: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let Outcome::Accepted { operation: credential } = serde_json::from_value(login["outcome"].clone()).unwrap() else { panic!("credential operation must be accepted"); };
        tokio::time::timeout(Duration::from_secs(5), async {
            while !remote.region.activity_html().contains("Respond") || !other.region.activity_html().contains("Respond") {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        }).await.unwrap();
        let form = |path: &str, fields: &str| axum::http::Request::builder().method("POST").uri(path)
            .header("content-type", "application/x-www-form-urlencoded")
            .body(axum::body::Body::from(fields.to_owned())).unwrap();
        let prepare_action = super::super::remote_router(remote.clone()).oneshot(axum::http::Request::builder().method("POST").uri("/intent").header("content-type", "application/x-www-form-urlencoded").header("accept", "application/json").body(axum::body::Body::from("action=example.feed")).unwrap()).await.unwrap();
        assert_eq!(prepare_action.status(), axum::http::StatusCode::OK);
        let body = axum::body::to_bytes(prepare_action.into_body(), 65536).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(body["report"].as_str().unwrap().contains("field.quantity"));
        assert!(actions.lock().unwrap().is_empty(), "opening local preparation cannot invoke a command");
        let invalid = super::super::remote_router(remote.clone()).oneshot(form("/perform", "action_id=example.feed&field.quantity=wrong&field.label=snack")).await.unwrap();
        assert_eq!(invalid.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(actions.lock().unwrap().is_empty());
        let valid = super::super::remote_router(remote.clone()).oneshot(form("/perform", "action_id=example.feed&field.quantity=3&field.label=snack&field.pet=forged")).await.unwrap();
        assert_eq!(valid.status(), axum::http::StatusCode::SEE_OTHER);
        assert_eq!(*actions.lock().unwrap(), vec![Value::map([("amount", Value::Int(3)), ("label", Value::str("snack")), ("pet", Value::str("owned"))])]);
        let Outcome::Accepted { operation: custom } = runtime.request_input(
            &misa_protocol::invocation::CallContext { principal: endpoint.id().to_string(), connection: 1 },
            misa_proto::input::Form { title: "Choose a feeding count".into(), input: form_schema, fields: BTreeMap::from([("count".into(), misa_proto::input::Field { label: "Number of portions".into() })]) },
            ActionBinding { command: "example.form".into(), bound: BTreeMap::new(), inputs: BTreeMap::from([("value".into(), "value".into())]) }, 60_000,
        ) else { panic!("custom request must be accepted"); };
        let opened = super::super::remote_router(remote.clone()).oneshot(form("/request", &format!("id={}", custom.id))).await.unwrap();
        assert_eq!(opened.status(), axum::http::StatusCode::OK);
        let html = axum::body::to_bytes(opened.into_body(), 65536).await.unwrap();
        let html = std::str::from_utf8(&html).unwrap();
        assert!(html.contains("Number of portions") && html.contains("name=\"field.count\"") && html.contains("type=\"number\"") && html.contains("formnovalidate"));
        assert!(!html.contains("app.js"));
        let invalid = super::super::remote_router(remote.clone()).oneshot(form("/respond", &format!("id={}&generation=1&action=resolve&field.count=wrong", custom.id))).await.unwrap();
        assert_eq!(invalid.status(), axum::http::StatusCode::BAD_REQUEST);
        assert!(form_values.lock().unwrap().is_empty());
        let resolved = super::super::remote_router(other.clone()).oneshot(form("/respond", &format!("id={}&generation=1&action=resolve&field.count=4", custom.id))).await.unwrap();
        assert_eq!(resolved.status(), axum::http::StatusCode::OK);
        tokio::time::timeout(Duration::from_secs(5), async {
            while form_values.lock().unwrap().is_empty() { tokio::task::yield_now().await; }
        }).await.unwrap();
        assert_eq!(*form_values.lock().unwrap(), vec![Value::map([("value", Value::map([("count", Value::Int(4))]))])]);
        let duplicate = super::super::remote_router(remote.clone()).oneshot(form("/respond", &format!("id={}&generation=1&action=resolve&field.count=8", custom.id))).await.unwrap();
        assert_eq!(duplicate.status(), axum::http::StatusCode::BAD_REQUEST);
        let current = remote.daemon.client.read(Selection { scope: credential.scope.clone(), members: BTreeMap::from([("request".into(), remote.interaction.interface.query("operation.request", vec![Value::str(&credential.id)]).unwrap())]) }, Duration::from_secs(5)).await.unwrap();
        let model = misa_client::request::Model::parse(misa_client::interface::data(&current, "request").unwrap(), &remote.interaction.interface).unwrap().unwrap();
        let opened = super::super::remote_router(remote.clone()).oneshot(form("/request", &format!("id={}", credential.id))).await.unwrap();
        assert_eq!(opened.status(), axum::http::StatusCode::OK);
        assert_eq!(opened.headers()["cache-control"], "no-store");
        let html = String::from_utf8(axum::body::to_bytes(opened.into_body(), 65536).await.unwrap().to_vec()).unwrap();
        assert!(html.contains("type=\"password\""));
        assert!(!html.contains("localStorage"));
        assert!(!html.contains("app.js"), "secret forms never use composer persistence");
        let watched = super::super::remote_router(remote.clone()).oneshot(axum::http::Request::builder().uri(format!("/request/events?id={}&generation={}", credential.id, model.generation)).body(axum::body::Body::empty()).unwrap()).await.unwrap();
        assert_eq!(watched.headers()["cache-control"], "no-store");
        let mut validity = watched.into_body().into_data_stream();
        use futures::StreamExt;
        tokio::time::timeout(Duration::from_secs(5), async {
            while !String::from_utf8_lossy(&validity.next().await.unwrap().unwrap()).contains("current") {}
        }).await.unwrap();
        let stale = super::super::remote_router(remote.clone()).oneshot(form("/respond", &format!("id={}&generation={}&action=submit&value=never-echo-this-secret", credential.id, model.generation + 1))).await.unwrap();
        assert_eq!(stale.status(), axum::http::StatusCode::BAD_REQUEST);
        let html = String::from_utf8(axum::body::to_bytes(stale.into_body(), 65536).await.unwrap().to_vec()).unwrap();
        assert!(html.contains("Response not sent"));
        assert!(!html.contains("never-echo-this-secret"));
        let cancelled = super::super::remote_router(other.clone()).oneshot(form("/respond", &format!("id={}&generation={}&action=cancel", credential.id, model.generation))).await.unwrap();
        assert_eq!(cancelled.status(), axum::http::StatusCode::OK);
        tokio::time::timeout(Duration::from_secs(5), async {
            while !String::from_utf8_lossy(&validity.next().await.unwrap().unwrap()).contains("resolved") {}
        }).await.unwrap();
        drop(validity);
        tokio::time::timeout(Duration::from_secs(5), async {
            while remote.region.activity_html().contains("Respond") || other.region.activity_html().contains("Respond") || !remote.region.activity_html().contains("Finished: cancelled") {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        }).await.unwrap();
        let closed = super::super::remote_router(remote.clone()).oneshot(form("/request", &format!("id={}", credential.id))).await.unwrap();
        assert_eq!(closed.status(), axum::http::StatusCode::BAD_REQUEST);
        let prompted = super::super::remote_router(remote.clone()).oneshot(request("please+run+the+tool")).await.unwrap();
        assert_eq!(prompted.status(), axum::http::StatusCode::OK);
        let approval = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let values = remote.daemon.client.read(Selection { scope: remote.interaction.interface.scope.clone(), members: BTreeMap::from([("requests".into(), remote.interaction.interface.query("requests.summary", vec![]).unwrap())]) }, Duration::from_secs(5)).await.unwrap();
                if let Some(value) = misa_client::interface::data(&values, "requests").unwrap().as_list().unwrap().iter().find(|value| value.get("kind").and_then(Value::as_str) == Some("tool_approval") && value.get("state").and_then(Value::as_str) == Some("awaiting_input")) { break value.clone(); }
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        }).await.unwrap();
        let id = approval.get("id").and_then(Value::as_str).unwrap();
        let generation = approval.get("generation").and_then(Value::as_i64).unwrap();
        let opened = super::super::remote_router(other.clone()).oneshot(form("/request", &format!("id={id}"))).await.unwrap();
        let html = String::from_utf8(axum::body::to_bytes(opened.into_body(), 65536).await.unwrap().to_vec()).unwrap();
        assert!(html.contains("Allow tool") && html.contains("Deny tool"));
        let approved = super::super::remote_router(other.clone()).oneshot(form("/respond", &format!("id={id}&generation={generation}&action=approve"))).await.unwrap();
        assert_eq!(approved.status(), axum::http::StatusCode::OK);
        let duplicate = super::super::remote_router(remote.clone()).oneshot(form("/respond", &format!("id={id}&generation={generation}&action=deny"))).await.unwrap();
        assert_eq!(duplicate.status(), axum::http::StatusCode::BAD_REQUEST);
        tokio::time::timeout(Duration::from_secs(5), async {
            while !remote.region.activity_html().contains("tool completed") || !other.region.get().contains("tool completed") {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
        }).await.unwrap();
        let hidden = super::super::remote_router(remote.clone()).oneshot(configure("hide")).await.unwrap();
        assert_eq!(hidden.status(), axum::http::StatusCode::SEE_OTHER);
        assert!(!remote.region.get().contains("data-presentation=\"status\""));
        assert!(remote.region.get().contains("scoped web reply"));
        assert!(other.region.get().contains("data-presentation=\"status\""), "selection is instance-local");
        let shown = super::super::remote_router(remote.clone()).oneshot(configure("auto")).await.unwrap();
        assert_eq!(shown.status(), axum::http::StatusCode::SEE_OTHER);
        assert!(remote.region.get().contains("data-presentation=\"status\""));
        remote
            .pending
            .lock()
            .unwrap()
            .push(misa_proto::view::BlobRef {
                hash: "0".repeat(64),
                len: 1,
                media: None,
            });
        assert!(
            other.pending.lock().unwrap().is_empty(),
            "presentation instances do not share pending attachments"
        );
        drop(remote);
        assert!(other.region.get().contains("scoped web reply"));
        drop(other);
        drop(daemons);
        endpoint.close().await;
        router.shutdown().await.unwrap();
    }
}

/// Adapt installed shortcuts to the local composer parser; these are not session metadata.
pub(crate) fn declarations(interaction: &Interaction) -> Vec<misa_proto::wire::Command> {
    interaction.shortcuts.iter().map(|shortcut| misa_proto::wire::Command { id: shortcut.id.clone(), label: shortcut.label.clone(), description: shortcut.description.clone(), args: shortcut.args.clone() }).collect()
}
