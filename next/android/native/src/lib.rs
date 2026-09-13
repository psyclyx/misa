//! The Android frontend's native half.
//!
//! A phone cannot run the desktop workspace, and it should not have to: a
//! frontend is a client, and everything a client needs is already a Rust
//! library. This crate is a thin JNI seam over [`misa_net::iroh`] and
//! [`misa_proto`] — the same transport, the same pairing, the same intent
//! vocabulary the terminal, browser, and pixel frontends use. The Kotlin side
//! owns the appearance (a `Compose` tree instead of cells or HTML) and nothing
//! else, which is the architecture's rule stated as a build script.
//!
//! # The shape of the seam
//!
//! One connection per handle. `connect` starts a runtime on its own thread,
//! pairs if the string carries a code, attaches, subscribes to the session's
//! view, and then loops: messages from the session become JSON strings handed to
//! a Kotlin listener, and intents from Kotlin go back over the same connection.
//! No policy crosses the seam, because there is none to cross: what to send is
//! the client's, and what it means is the session's.
//!
//! # Why JSON
//!
//! Both the view tree and every wire message are `serde` types, and the
//! protocol's own CBOR is not something Kotlin should have to grow a reader for.
//! JSON keeps the seam in one language-independent shape, and the Kotlin side
//! parses it with the platform's own `org.json` — no code generation, no
//! reflection, and no second definition of anything.

mod files;
mod snapshot;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{JNI_FALSE, JNI_TRUE, jboolean, jlong};
use jni::{JNIEnv, JavaVM};

use misa_proto::wire::{ClientInfo, Intent, Level, SessionEvent, SessionMsg};
use misa_proto::{Pairing, Query, SubId};

/// What a Kotlin caller asks the connection's owner to do.
enum Command {
    /// Send an intent the client already decided on.
    Intent(Intent),
    Fetch {
        hash: String,
    },
    Upload {
        path: PathBuf,
        name: String,
        media: String,
    },
}

/// One live connection: the sending half, owned by the runtime thread.
struct Live {
    commands: tokio::sync::mpsc::UnboundedSender<Command>,
    stop: tokio::sync::oneshot::Sender<()>,
}

fn connections() -> &'static Mutex<HashMap<i64, Live>> {
    static CONNECTIONS: OnceLock<Mutex<HashMap<i64, Live>>> = OnceLock::new();
    CONNECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_handle() -> i64 {
    static NEXT: AtomicI64 = AtomicI64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Where the session's views live in this connection.
fn view_query() -> Query {
    Query::new(misa_proto::VIEW_QUERY)
}

/// Deliver one event to the Kotlin listener as a JSON string.
///
/// Attaching "as a daemon" means the callback thread is not kept alive by the
/// runtime: the JVM detaches it when the thread ends, which is what the call
/// intends.
fn emit(vm: &JavaVM, listener: &GlobalRef, payload: serde_json::Value) {
    let Ok(mut env) = vm.attach_current_thread_as_daemon() else {
        return;
    };
    let Ok(text) = env.new_string(payload.to_string()) else {
        return;
    };
    let _ = env.call_method(
        listener,
        "onEvent",
        "(Ljava/lang/String;)V",
        &[JValue::Object(&text)],
    );
    // A listener that throws must not poison the next call, and there is nothing
    // useful to do with the exception here: the session keeps running either way.
    let _ = env.exception_clear();
    let _ = env.delete_local_ref(text);
}

fn string_at(env: &mut JNIEnv<'_>, value: &JString<'_>) -> Option<String> {
    env.get_string(value).ok().map(Into::into)
}

/// Connect to a daemon and start delivering events.
///
/// `ticket` is a ticket or a pairing string — the same string the terminal takes,
/// because "what the daemon printed" is one thing across every frontend.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_misa_app_Native_connect<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    ticket: JString<'local>,
    storage: JString<'local>,
    listener: JObject<'local>,
) -> jlong {
    let Some(ticket) = string_at(&mut env, &ticket) else {
        return 0;
    };
    let Some(storage) = string_at(&mut env, &storage) else {
        return 0;
    };
    let Ok(vm) = env.get_java_vm() else {
        return 0;
    };
    let Ok(listener) = env.new_global_ref(&listener) else {
        return 0;
    };
    let (commands, inbox) = tokio::sync::mpsc::unbounded_channel();
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let handle = next_handle();
    if let Ok(mut live) = connections().lock() {
        live.insert(handle, Live { commands, stop });
    }
    let spawned = std::thread::Builder::new()
        .name("misa-android".into())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_multi_thread().enable_all().build() else {
                emit(&vm, &listener, serde_json::json!({"kind":"fault", "code":"runtime", "message":"could not start the native runtime"}));
                if let Ok(mut live) = connections().lock() { live.remove(&handle); }
                return;
            };
            runtime.block_on(async {
                tokio::select! {
                    _ = session(vm, listener, ticket, PathBuf::from(storage), inbox) => {},
                    _ = stopped => {},
                }
            });
            if let Ok(mut live) = connections().lock() { live.remove(&handle); }
        });
    if spawned.is_err() {
        if let Ok(mut live) = connections().lock() {
            live.remove(&handle);
        }
        return 0;
    }
    handle
}

/// Send one intent, as JSON. `true` when it was handed to a live connection.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_misa_app_Native_send<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    handle: jlong,
    intent: JString<'local>,
) -> jboolean {
    let Some(text) = string_at(&mut env, &intent) else {
        return JNI_FALSE;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return JNI_FALSE;
    };
    let command = match value.get("local").and_then(|v| v.as_str()) {
        Some("fetch") => Command::Fetch {
            hash: value["hash"].as_str().unwrap_or_default().into(),
        },
        Some("upload") => Command::Upload {
            path: value["path"].as_str().unwrap_or_default().into(),
            name: value["name"].as_str().unwrap_or("attachment").into(),
            media: value["media"]
                .as_str()
                .unwrap_or("application/octet-stream")
                .into(),
        },
        Some(_) => return JNI_FALSE,
        None => match serde_json::from_value::<Intent>(value) {
            Ok(intent) => Command::Intent(intent),
            Err(_) => return JNI_FALSE,
        },
    };
    let sent = connections()
        .lock()
        .ok()
        .and_then(|live| {
            live.get(&handle)
                .map(|live| live.commands.send(command).is_ok())
        })
        .unwrap_or(false);
    if sent { JNI_TRUE } else { JNI_FALSE }
}

/// Close a connection and forget it.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_misa_app_Native_disconnect(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) {
    if let Ok(mut live) = connections().lock()
        && let Some(live) = live.remove(&handle)
    {
        let _ = live.stop.send(());
    }
}

/// The frontend's own name and version, for a diagnostic.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_misa_app_Native_version<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> jni::sys::jstring {
    env.new_string(env!("CARGO_PKG_VERSION"))
        .map(|text| text.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

/// The whole life of one connection.
async fn session(
    vm: JavaVM,
    listener: GlobalRef,
    ticket: String,
    storage: PathBuf,
    mut inbox: tokio::sync::mpsc::UnboundedReceiver<Command>,
) {
    let state = |state: &str, message: String| serde_json::json!({ "kind": "state", "state": state, "message": message });
    let (parsed, code) = match Pairing::given(&ticket) {
        Ok(parsed) => parsed,
        Err(message) => {
            emit(
                &vm,
                &listener,
                serde_json::json!({ "kind": "fault", "code": "ticket", "message": message }),
            );
            return;
        }
    };
    emit(
        &vm,
        &listener,
        state("connecting", format!("reaching {}", parsed.node)),
    );
    let files = match files::Files::new(storage).and_then(|files| Ok((files.identity()?, files))) {
        Ok((identity, files)) => (identity, Arc::new(files)),
        Err(message) => {
            emit(
                &vm,
                &listener,
                serde_json::json!({"kind":"fault", "code":"storage", "message":message}),
            );
            return;
        }
    };
    let (identity, files) = files;
    let snapshot_name = snapshot::name(&parsed);
    let mut view = match snapshot::load(&files, &snapshot_name) {
        Ok(view) => view,
        Err(error) => {
            emit(
                &vm,
                &listener,
                serde_json::json!({"kind":"notice", "level":"warn", "text":format!("could not restore the saved conversation: {error}")}),
            );
            Default::default()
        }
    };
    if let Some(tree) = view.rendered() {
        emit(
            &vm,
            &listener,
            serde_json::json!({"kind":"view", "view":tree}),
        );
    }
    let endpoint = match misa_net::iroh::bind(Some(identity), !parsed.node.contains('@')).await {
        Ok(endpoint) => endpoint,
        Err(message) => {
            emit(
                &vm,
                &listener,
                serde_json::json!({ "kind": "fault", "code": "transport", "message": message }),
            );
            return;
        }
    };
    let target = match misa_net::iroh::address_of(&parsed.node) {
        Ok(target) => target,
        Err(message) => {
            emit(
                &vm,
                &listener,
                serde_json::json!({ "kind": "fault", "code": "ticket", "message": message }),
            );
            return;
        }
    };
    // A code is spent once, before the ordinary connection: what is approved is
    // this endpoint's key, so the connection that follows is an ordinary client.
    if let Some(code) = &code {
        emit(
            &vm,
            &listener,
            state("pairing", "showing the daemon a pairing code".into()),
        );
        match misa_net::iroh::Client::pair(&endpoint, target.clone(), code, "the phone").await {
            Ok(message) => {
                emit(
                    &vm,
                    &listener,
                    serde_json::json!({ "kind": "paired", "message": message }),
                );
            }
            Err(message) => {
                emit(
                    &vm,
                    &listener,
                    serde_json::json!({ "kind": "fault", "code": "pairing", "message": message }),
                );
                return;
            }
        }
    }

    let info = ClientInfo::new("misa-android", env!("CARGO_PKG_VERSION"));
    let mut client =
        match misa_net::iroh::Client::connect(&endpoint, target.clone(), info, &parsed.session)
            .await
        {
            Ok(client) => client,
            Err(message) => {
                emit(
                    &vm,
                    &listener,
                    serde_json::json!({ "kind": "fault", "code": "attach", "message": message }),
                );
                return;
            }
        };
    // The declarations are what make this frontend as capable as any other: with
    // them the phone knows a command's arguments and where their values come
    // from, so it can build a picker with no round trip and no second truth.
    if let Some(session) = client.session() {
        emit(
            &vm,
            &listener,
            serde_json::json!({ "kind": "session", "session": session }),
        );
    }
    let subscribed = match view.version().cloned() {
        Some(version) => {
            client
                .subscribe_since(SubId(1), view_query(), version)
                .await
        }
        None => client.subscribe(SubId(1), view_query()).await,
    };
    if let Err(message) = subscribed {
        emit(
            &vm,
            &listener,
            serde_json::json!({ "kind": "fault", "code": "subscribe", "message": message }),
        );
    }
    emit(
        &vm,
        &listener,
        state("connected", format!("attached to {}", parsed.session)),
    );

    let blobs = misa_net::blob::Store::new(endpoint.clone(), target);
    let mut transfers = tokio::task::JoinSet::new();
    emit(
        &vm,
        &listener,
        serde_json::json!({ "kind":"ticket", "ticket":parsed.to_string() }),
    );
    let mut next_id = 1u64;
    loop {
        tokio::select! {
            message = client.next() => match message {
                Ok(Some(message @ (SessionMsg::View { .. } | SessionMsg::Changes { .. } | SessionMsg::Streams { .. } | SessionMsg::Event { event: SessionEvent::Stream { .. }, .. }))) => {
                    let canonical = matches!(&message, SessionMsg::View { .. } | SessionMsg::Changes { .. });
                    let mode = if matches!(&message, SessionMsg::Changes { .. }) { "changes" } else { "snapshot" };
                    match view.receive(&message) {
                        Ok(changed) => {
                            if canonical {
                                if let Err(error) = snapshot::save(&files, &snapshot_name, &view) {
                                    emit(&vm, &listener, serde_json::json!({"kind":"notice", "level":"error", "text":error}));
                                }
                                emit(&vm, &listener, serde_json::json!({"kind":"sync", "mode":mode}));
                            }
                            if changed && let Some(tree) = view.rendered() {
                                emit(&vm, &listener, serde_json::json!({"kind":"view", "view":tree}));
                            }
                        }
                        Err(error) => {
                            emit(&vm, &listener, serde_json::json!({"kind":"notice", "level":"warn", "text":error}));
                            if let Err(error) = client.subscribe(SubId(1), view_query()).await {
                                emit(&vm, &listener, serde_json::json!({"kind":"fault", "code":"subscribe", "message":error}));
                            }
                        }
                    }
                }
                Ok(Some(SessionMsg::Download { download, .. })) => {
                    if let Some(blob) = download.blob {
                        let blobs = blobs.clone(); let files = files.clone();
                        transfers.spawn(async move { fetch(blobs, files, blob.hash, Some(download.name)).await });
                    } else {
                        emit(&vm, &listener, serde_json::json!({"kind":"notice", "level":"error", "text":download.error}));
                    }
                }
                Ok(Some(SessionMsg::Welcome { session, .. })) => {
                    emit(&vm, &listener, serde_json::json!({ "kind": "session", "session": session }));
                }
                Ok(Some(SessionMsg::Event { event, .. })) => emit_event(&vm, &listener, event),
                Ok(Some(SessionMsg::Fault { fault, .. })) => {
                    emit(&vm, &listener, serde_json::json!({ "kind": "fault", "code": fault.code, "message": fault.message }));
                }
                Ok(Some(SessionMsg::QueryFault { fault, .. })) => {
                    emit(&vm, &listener, serde_json::json!({ "kind": "fault", "code": fault.code, "message": fault.message }));
                }
                Ok(Some(_)) => {}
                Ok(None) => {
                    emit(&vm, &listener, state("closed", "the daemon closed the connection".into()));
                    break;
                }
                Err(message) => {
                    emit(&vm, &listener, serde_json::json!({ "kind": "fault", "code": "transport", "message": message }));
                    break;
                }
            },
            result = transfers.join_next(), if !transfers.is_empty() => {
                match result {
                    Some(Ok(event)) => emit(&vm, &listener, event),
                    Some(Err(error)) => emit(&vm, &listener, serde_json::json!({"kind":"notice", "level":"error", "text":error.to_string()})),
                    None => {}
                }
            },
            command = inbox.recv() => match command {
                Some(Command::Fetch { hash }) => {
                    let blobs = blobs.clone(); let files = files.clone();
                    transfers.spawn(async move { fetch(blobs, files, hash, None).await });
                }
                Some(Command::Upload { path, name, media }) => {
                    let blobs = blobs.clone();
                    let upload = files::UploadFile(path);
                    transfers.spawn(async move {
                        let result = async {
                            let bytes = files::upload_bytes(&upload.0)?;
                            blobs.share(bytes, Some(&media)).await
                        }.await;
                        drop(upload);
                        match result {
                            Ok(blob) => serde_json::json!({"kind":"uploaded", "name":name, "blob":blob}),
                            Err(error) => serde_json::json!({"kind":"upload_failed", "text":error}),
                        }
                    });
                }
                Some(Command::Intent(intent)) => {
                    let id = next_id;
                    next_id += 1;
                    if let Err(message) = client.intent(id, intent).await {
                        emit(&vm, &listener, serde_json::json!({ "kind": "fault", "code": "transport", "message": message }));
                        break;
                    }
                }
                None => break,
            },
        }
    }
    transfers.abort_all();
    emit(&vm, &listener, serde_json::json!({ "kind": "closed" }));
}

/// Non-view session events presented by the frontend.
fn emit_event(vm: &JavaVM, listener: &GlobalRef, event: SessionEvent) {
    let payload = match event {
        SessionEvent::DownloadReady { .. } => return,
        SessionEvent::Stream { .. } => return,
        SessionEvent::Notice { level, text } => {
            serde_json::json!({ "kind": "notice", "level": level_word(level), "text": text })
        }
        SessionEvent::Status { text } => serde_json::json!({ "kind": "status", "text": text }),
        SessionEvent::Recover { text } => serde_json::json!({ "kind": "recover", "text": text }),
    };
    emit(vm, listener, payload);
}

fn level_word(level: Level) -> &'static str {
    match level {
        Level::Info => "info",
        Level::Warn => "warn",
        Level::Error => "error",
    }
}

async fn fetch(
    blobs: Arc<misa_net::blob::Store>,
    files: Arc<files::Files>,
    hash: String,
    name: Option<String>,
) -> serde_json::Value {
    let result = async {
        if let Some(path) = files.cached(&hash)? {
            return Ok(path);
        }
        let blob = blobs
            .get(&hash)
            .await?
            .ok_or("the blob is no longer available")?;
        files.cache(&hash, &blob.bytes)
    }
    .await;
    match result {
        Ok(path) => {
            serde_json::json!({"kind":if name.is_some() {"download"} else {"blob"}, "hash":hash, "path":path, "name":name})
        }
        Err(error) => serde_json::json!({"kind":"blob_failed", "hash":hash, "text":error}),
    }
}

/// Read a verified cached blob without opening a network connection.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_misa_app_Native_cached<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    storage: JString<'local>,
    hash: JString<'local>,
) -> jni::sys::jstring {
    let Some(storage) = string_at(&mut env, &storage) else {
        return std::ptr::null_mut();
    };
    let Some(hash) = string_at(&mut env, &hash) else {
        return std::ptr::null_mut();
    };
    let path = files::Files::new(storage)
        .and_then(|files| files.cached(&hash))
        .ok()
        .flatten();
    path.and_then(|path| env.new_string(path.to_string_lossy()).ok())
        .map_or(std::ptr::null_mut(), |text| text.into_raw())
}
