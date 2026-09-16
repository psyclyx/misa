//! Android JNI adapts platform lifecycle and bounded commands to the shared scoped client.
//! Kotlin owns local presentation; Rust owns replicas, recovery and directed outcomes.
mod bridge;
mod commands;
mod delivery;
mod files;
mod session;
mod snapshot;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, OnceLock};

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::sys::{JNI_FALSE, JNI_TRUE, jboolean, jlong};
use jni::{JNIEnv, JavaVM};

type Command = serde_json::Value;

/// One live workspace: the sending half, owned by the runtime thread.
struct Live {
    commands: tokio::sync::mpsc::Sender<Command>,
    stop: tokio::sync::oneshot::Sender<()>,
    mailbox: Arc<delivery::Mailbox>,
}

fn connections() -> &'static Mutex<HashMap<i64, Live>> {
    static CONNECTIONS: OnceLock<Mutex<HashMap<i64, Live>>> = OnceLock::new();
    CONNECTIONS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn next_handle() -> i64 {
    static NEXT: AtomicI64 = AtomicI64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
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

/// Pull one bounded event on the surface's own schedule. Document transactions
/// are captured coherently from the current shared replica at this boundary.
#[unsafe(no_mangle)]
pub extern "system" fn Java_org_misa_app_Native_poll(
    env: JNIEnv<'_>,
    _class: JClass<'_>,
    handle: jlong,
) -> jni::sys::jstring {
    let mailbox = connections()
        .lock()
        .ok()
        .and_then(|live| live.get(&handle).map(|entry| entry.mailbox.clone()));
    mailbox
        .and_then(|mailbox| mailbox.next())
        .and_then(|event| env.new_string(event.to_string()).ok())
        .map_or(std::ptr::null_mut(), |text| text.into_raw())
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
    let (commands, inbox) = tokio::sync::mpsc::channel(32);
    let (stop, stopped) = tokio::sync::oneshot::channel();
    let handle = next_handle();
    let mailbox = Arc::new(delivery::Mailbox::new(Arc::new(move |mut event| {
        event["handle"] = serde_json::json!(handle);
        emit(&vm, &listener, event);
    })));
    if let Ok(mut live) = connections().lock() {
        if live.len() >= 16 {
            return 0;
        }
        live.insert(
            handle,
            Live {
                commands,
                stop,
                mailbox: mailbox.clone(),
            },
        );
    }
    let spawned = std::thread::Builder::new()
        .name("misa-android".into())
        .spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
            else {
                mailbox.startup_fault("Could not start the native runtime");
                if let Ok(mut live) = connections().lock() {
                    live.remove(&handle);
                }
                return;
            };
            runtime.block_on(async {
                tokio::select! {
                    _ = bridge::run(ticket, PathBuf::from(storage), inbox, mailbox) => {},
                    _ = stopped => {},
                }
            });
            if let Ok(mut live) = connections().lock() {
                live.remove(&handle);
            }
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
    if text.len() > 1024 * 1024 {
        return JNI_FALSE;
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return JNI_FALSE;
    };
    if !value.is_object() {
        return JNI_FALSE;
    }
    let command = value;
    let sent = connections()
        .lock()
        .ok()
        .and_then(|live| {
            live.get(&handle)
                .map(|live| live.commands.try_send(command).is_ok())
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
