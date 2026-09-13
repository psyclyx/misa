//! The agent loop, as policy.
//!
//! This module is the "middle layer" the architecture calls for: everything that knows
//! what a turn is, and nothing that knows what a turn looks like. It is written the way a
//! plugin would be written — handlers over an event vocabulary, asking for effects and
//! handling their completions — because the plan is for a composition to be able to
//! replace it, and the cheapest way to keep that door open is to walk through it from the
//! start.
//!
//! # The loop, in full
//!
//! ```text
//! intent/prompt ──▶ free? record the user message, status = thinking ─┐
//!                    busy? queue it and say so                        │
//!                                                                     ▼
//!                                                                agent/step
//!                          record an attempt, ask the provider, open a
//!                          streaming assistant message
//!                                                                     ▼
//! kernel/provider.delta ──▶ append to that message ──▶ wire TextDelta
//!                                                                     ▼
//! kernel/provider.finished ──▶ settle the attempt with what it cost
//!        │
//!        ├── no tool calls ──▶ status = idle ──▶ queue/next
//!        └── tool calls ─────▶ agent/tools ──▶ kernel/tool.finished
//!                                                     │ batch complete?
//!                                                     └──▶ agent/step
//! ```
//!
//! Four decisions in it are worth naming, because each one is a lesson the previous system
//! paid for:
//!
//! 1. **An unfinished attempt is a fact, not a retry.** A provider call that never
//!    answered leaves an attempt row in `started` and a message in `streaming`. Nothing
//!    re-issues it. A person decides.
//! 2. **A tool's failure is a result, not an exception.** The model is told what happened
//!    in the same shape a success arrives in, so it can re-check what it depends on
//!    instead of the session guessing whether a tool is retry-safe.
//! 3. **A stale completion is ignored, not applied.** Every kernel event names the request
//!    it answers, and a handler that does not recognise the name does nothing.
//! 4. **Typing while a model answers is normal.** A prompt submitted during a turn is
//!    queued, not refused and not an interruption, and it is drained when the turn ends.

use misa_proto::view::{Field, FieldKind, Node, Span};
use misa_proto::wire::{Level, SessionEvent};
use misa_reframe::fields;
use misa_reframe::{Effect, Event, Fault, Registry, Tx};
use misa_value::{Op, Value};

use misa_kernel::KernelEvent;

use crate::now_ms;
use crate::views;

/// Every handler and subscription the shipped agent loop installs.
pub fn registry() -> Registry {
    views::subscriptions(Registry::new())
        .on_fn("intent/prompt", 0, "agent.prompt", on_prompt)
        .on_fn("intent/cancel", 0, "agent.cancel", on_cancel)
        .on_fn("intent/command", 0, "agent.command", on_command)
        .on_fn("intent/action", 0, "agent.action", on_action)
        .on_fn("intent/queue.take", 0, "agent.queue.take", on_queue_take)
        .on_fn("intent/queue.clear", 0, "agent.queue.clear", on_queue_clear)
        .on_fn("kernel/conversations", 0, "agent.conversations", on_conversations)
        .on_fn("kernel/log.loaded", 0, "agent.loaded", on_loaded)
        .on_fn("kernel/credential", 0, "agent.credential", on_credential)
        .on_fn("kernel/credential.prompt", 0, "agent.credential.prompt", on_credential_prompt)
        .on_fn("kernel/blob", 0, "agent.blob", on_blob)
        .on_fn("kernel/process.finished", 0, "agent.process.finished", on_process_finished)
        .on_fn("queue/next", 0, "agent.queue.next", on_queue_next)
        .on_fn("agent/step", 0, "agent.step", on_step)
        .on_fn("agent/tools", 0, "agent.tools", on_tools)
        .on_fn("kernel/provider.delta", 0, "agent.delta", on_delta)
        .on_fn("kernel/provider.thinking", 0, "agent.thinking", on_thinking)
        .on_fn("kernel/usage", 0, "agent.usage", on_usage)
        .on_fn("kernel/models", 0, "agent.models", on_models)
        .on_fn("kernel/provider.finished", 0, "agent.finished", on_finished)
        .on_fn("kernel/tool.finished", 0, "agent.tool.finished", on_tool_finished)
        .on_fn("kernel/failed", 0, "agent.failed", on_kernel_failed)
}

/// The event a kernel report becomes.
///
/// The kernel's vocabulary and the loop's are deliberately different names, and this
/// function is the whole of the translation between them. It is the seam a second kernel
/// implementation would have to satisfy.
pub fn event_for(event: KernelEvent) -> Event {
    match event {
        KernelEvent::ProviderDelta { id, text } => {
            Event::new("kernel/provider.delta").with("id", Value::str(id)).with("text", Value::str(text))
        }
        // A service reasoning aloud, which is not the answer and is kept apart from it all the
        // way to the view.
        KernelEvent::ProviderThinking { id, text } => {
            Event::new("kernel/provider.thinking").with("id", Value::str(id)).with("text", Value::str(text))
        }
        KernelEvent::Usage { id, provider, facts } => Event::new("kernel/usage")
            .with("id", Value::str(id))
            .with("provider", Value::str(provider))
            .with("facts", facts),
        KernelEvent::Models { id, ok, models, message } => Event::new("kernel/models")
            .with("id", Value::str(id))
            .with("ok", Value::Bool(ok))
            .with("models", models)
            .with("message", Value::str(message)),
        KernelEvent::ProviderFinished {
            id,
            ok,
            text,
            thinking,
            tool_calls,
            input_tokens,
            output_tokens,
            error,
        } => Event::new("kernel/provider.finished")
            .with("id", Value::str(id))
            .with("ok", Value::Bool(ok))
            .with("text", Value::str(text))
            .with("thinking", Value::str(thinking))
            .with("tool_calls", tool_calls)
            .with("input_tokens", Value::Int(input_tokens))
            .with("output_tokens", Value::Int(output_tokens))
            .with("error", Value::str(error)),
        KernelEvent::ToolFinished { id, call_id, ok, text } => Event::new("kernel/tool.finished")
            .with("id", Value::str(id))
            .with("call_id", Value::str(call_id))
            .with("ok", Value::Bool(ok))
            .with("text", Value::str(text)),
        KernelEvent::Conversations { id, headers } => {
            Event::new("kernel/conversations").with("id", Value::str(id)).with("headers", headers)
        }
        // A conversation read back: its entries in the order they were appended, each with the
        // kind that says what it is. This is the one event that *rebuilds* a transcript rather
        // than adding to one, which is why it is not noted and dropped like the rest.
        KernelEvent::Loaded { conversation, entries } => Event::new("kernel/log.loaded")
            .with("conversation", Value::str(conversation))
            .with(
                "entries",
                Value::list(
                    entries
                        .into_iter()
                        .map(|entry| {
                            Value::map([
                                ("seq", Value::Int(entry.seq)),
                                ("kind", Value::str(entry.kind)),
                                ("data", entry.data),
                            ])
                        })
                        .collect::<Vec<_>>(),
                ),
            ),
        KernelEvent::Credential { id, ok, message, slots } => Event::new("kernel/credential")
            .with("id", Value::str(id))
            .with("ok", Value::Bool(ok))
            .with("message", Value::str(message))
            .with("slots", slots),
        // The one kernel event a person has to act on: a code and where to type it. It is
        // carried as the two strings it is, because what to do with them — a panel, a
        // notification, a link — is the session's decision and not the kernel's.
        KernelEvent::CredentialPrompt { id, provider, url, code } => Event::new("kernel/credential.prompt")
            .with("id", Value::str(id))
            .with("provider", Value::str(provider))
            .with("url", Value::str(url))
            .with("code", Value::str(code)),
        KernelEvent::Blob { id, ok, hash, len, media, message } => Event::new("kernel/blob")
            .with("id", Value::str(id))
            .with("ok", Value::Bool(ok))
            .with("hash", Value::str(hash))
            .with("len", Value::Int(len))
            .with("media", Value::str(media))
            .with("message", Value::str(message)),
        KernelEvent::BlobBytes { id, ok, media, message, .. } => Event::new("kernel/blob.bytes")
            .with("id", Value::str(id))
            .with("ok", Value::Bool(ok))
            .with("media", Value::str(media))
            .with("message", Value::str(message)),
        // The one report nothing asked for at the moment it arrives: a command that was left
        // running has finished, and the session has to decide what to say about it.
        KernelEvent::ProcessFinished { pid, command, exit, killed, seconds, log } => {
            Event::new("kernel/process.finished")
                .with("pid", Value::Int(pid as i64))
                .with("command", Value::str(command))
                .with("exit", exit.map(|code| Value::Int(code as i64)).unwrap_or(Value::Null))
                .with("killed", Value::Bool(killed))
                .with("seconds", Value::Float(seconds))
                .with("log", Value::str(log))
        }
        KernelEvent::HttpFinished { id, ok, status, body } => Event::new("kernel/http.finished")
            .with("id", Value::str(id))
            .with("ok", Value::Bool(ok))
            .with("status", Value::Int(status as i64))
            .with("body", Value::str(body)),
        KernelEvent::Failed { id, message } => {
            Event::new("kernel/failed").with("id", Value::str(id)).with("message", Value::str(message))
        }
        // A recorded fact changes nothing the loop decides, so it is not an event.
        KernelEvent::Appended { .. } | KernelEvent::AttemptRecorded { .. } => {
            Event::new("kernel/noted").with_value(Value::Null)
        }
    }
}

// ---------------------------------------------------------------------------
// Submitting, and the queue
// ---------------------------------------------------------------------------

/// A submission: a turn if the session is free, a queued prompt if it is not.
///
/// Queueing rather than refusing, because somebody typing while a model answers is the
/// normal case and not a mistake; and queueing rather than interrupting, because the
/// previous system's answer was also that the running turn finishes.
fn on_prompt(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let text = fields::event_text(event, "text");
    if text.trim().is_empty() {
        return Err(Fault::handler("a prompt with no text"));
    }
    let attachments = submitted_attachments(tx, event);
    if tx.text("session.status") != "idle" {
        let waiting = queue_len(tx) + 1;
        tx.push(
            "session.queue",
            Value::map([
                ("text", Value::str(&text)),
                ("attachments", attachments),
                ("state", Value::str("waiting")),
            ]),
        )?;
        notice(tx, Level::Info, format!("queued ({waiting} waiting)"))?;
        return Ok(());
    }
    begin_turn(tx, &text, attachments)
}

/// Start a turn: record the user message, mark the session thinking, ask for a step.
///
/// One function because a submission and a queue drain both begin a turn, and two copies
/// of this would be two places for the ordering to drift.
fn begin_turn(tx: &mut Tx<'_>, text: &str, attachments: Value) -> Result<(), Fault> {
    let turn = tx.int("session.turn") + 1;
    let message = Value::map([
        ("seq", Value::Int(turn)),
        ("role", Value::str("user")),
        ("text", Value::str(text)),
        ("state", Value::str("done")),
        ("attachments", attachments),
    ]);
    tx.set("session.turn", Value::Int(turn))?;
    tx.push("messages", message.clone())?;
    // A submitted prompt is spent: the draft's attachments belong to the turn, not to the
    // next one.
    tx.set("session.attachments", Value::list([]))?;
    tx.set("session.status", Value::str("thinking"))?;
    tx.fx(log_effect(tx, "message", message));
    tx.dispatch(Event::new("agent/step"));
    Ok(())
}

/// Drain one queued prompt, called when a turn ends.
fn on_queue_next(tx: &mut Tx<'_>, _event: &Event) -> Result<(), Fault> {
    if tx.text("session.status") != "idle" {
        return Ok(());
    }
    let Some(head) = tx
        .get("session.queue")
        .and_then(Value::as_list)
        .and_then(|queue| queue.first())
        .cloned()
    else {
        return Ok(());
    };
    let text = head.get("text").and_then(Value::as_str).unwrap_or_default().to_string();
    tx.delete("session.queue[0]")?;
    begin_turn(tx, &text, head.get("attachments").cloned().unwrap_or_else(|| Value::list([])))
}

/// Take a queued prompt back into the draft.
///
/// The only thing a session ever hands a client to put in its editor, and it is only ever
/// in answer to the client's own request.
fn on_queue_take(tx: &mut Tx<'_>, _event: &Event) -> Result<(), Fault> {
    let last = queue_len(tx);
    if last == 0 {
        notice(tx, Level::Info, "nothing is queued")?;
        return Ok(());
    }
    let entry = tx.get(&format!("session.queue[{}]", last - 1)).cloned().unwrap_or(Value::Null);
    let text = entry.get("text").and_then(Value::as_str).unwrap_or_default().to_string();
    tx.delete(&format!("session.queue[{}]", last - 1))?;
    tx.fx(Effect::new("wire.event").with(
        "event",
        session_event(&SessionEvent::Recover { text }),
    ));
    Ok(())
}

fn on_queue_clear(tx: &mut Tx<'_>, _event: &Event) -> Result<(), Fault> {
    tx.set("session.queue", Value::list([]))?;
    notice(tx, Level::Info, "queue cleared")?;
    Ok(())
}

fn queue_len(tx: &Tx<'_>) -> usize {
    tx.get("session.queue").and_then(Value::as_list).map(<[Value]>::len).unwrap_or(0)
}

fn current_attachments(tx: &Tx<'_>) -> Value {
    tx.get("session.attachments").cloned().unwrap_or_else(|| Value::list([]))
}

/// The attachments a turn carries: the ones the client named with this prompt, plus whatever
/// the draft holds.
///
/// Two sources because bytes arrive two ways — a client that read a file and uploaded it names
/// a hash it put in the store, and `/attach <path>` reads one where the daemon is running — and
/// one list because a message has one list of attachments.
///
/// Neither source is trusted, and neither has to be: a name is a name. A hash that is not a
/// lowercase hex hash names nothing the store can hold, so it is dropped here rather than
/// carried into a message; a hash the store does not have is kept, because that is a fact the
/// provider is told about in words rather than a lie by omission.
fn submitted_attachments(tx: &Tx<'_>, event: &Event) -> Value {
    let draft = current_attachments(tx);
    let named = fields::event_value(event, "attachments");
    let mut out: Vec<Value> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for attachment in draft.as_list().into_iter().flatten().chain(named.as_list().into_iter().flatten()) {
        let Some(hash) = attachment.get("hash").and_then(Value::as_str) else {
            continue;
        };
        if !misa_proto::blob::valid_hash(hash) || seen.iter().any(|seen| seen == hash) {
            continue;
        }
        seen.push(hash.to_string());
        out.push(attachment.clone());
    }
    Value::list(out)
}

fn on_cancel(tx: &mut Tx<'_>, _event: &Event) -> Result<(), Fault> {
    let status = tx.text("session.status");
    if status == "idle" {
        return Ok(());
    }
    // The attempt stays unfinished, on purpose: that is what makes "a request was made and
    // nothing came back" visible instead of looking like a fresh turn.
    if let Some(index) = pending_message(tx) {
        tx.set(&format!("messages[{index}].state"), Value::str("cancelled"))?;
    }
    tx.delete("session.pending")?;
    tx.delete("session.compacting")?;
    tx.set("session.status", Value::str("idle"))?;
    tx.fx(notice_effect(Level::Warn, "cancelled"));
    tx.dispatch(Event::new("queue/next"));
    Ok(())
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// A command, as the session declared it.
///
/// The declaration is the contract: a command that cannot run without an argument refuses
/// with `Fault::argument`, which names the source the client should open. That is the whole
/// reason a client can be useful without a round trip — the error path and the declaration
/// point at the same place.
fn on_command(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let name = fields::event_text(event, "name");
    let args = fields::event_value(event, "args");
    let declared = crate::catalog::commands().into_iter().find(|command| command.id == name);
    let Some(declared) = declared else {
        let known: Vec<String> = crate::catalog::commands()
            .into_iter()
            .map(|command| format!("/{}", command.id))
            .collect();
        notice(tx, Level::Warn, format!("no command named `/{name}`; there is {}", known.join(", ")))?;
        return Ok(());
    };

    for arg in &declared.args {
        if arg.required && argument(&args, &arg.name).is_none() {
            return Err(Fault::argument(&name, &arg.name, arg.source.as_deref()));
        }
    }

    match name.as_str() {
        "clear" => {
            let conversation = tx.text("session.conversation");
            tx.set("messages", Value::list([]))?;
            tx.set("session.turn", Value::Int(0))?;
            tx.set("session.status", Value::str("idle"))?;
            tx.set("session.attachments", Value::list([]))?;
            tx.delete("session.pending")?;
            tx.delete("panel")?;
            tx.fx(Effect::new("kernel.log.append")
                .with("conversation", Value::str(&conversation))
                .with("kind", Value::str("reset"))
                .with(
                    "data",
                    Value::map([("reason", Value::str(argument(&args, "reason").unwrap_or("cleared")))]),
                ));
            notice(tx, Level::Info, "cleared")?;
        }
        "model" => {
            let id = argument(&args, "model").unwrap_or_default();
            let Some(model) = crate::catalog::model(id) else {
                return Err(Fault::new("model.unknown", format!("no model named `{id}`")));
            };
            tx.set("session.model", Value::str(model.id))?;
            // Effort is a property of the model, so selecting a model that does not take one
            // clears it rather than leaving a setting nothing reads.
            if !model.effort {
                tx.set("session.effort", Value::Null)?;
            }
            notice(tx, Level::Info, format!("model: {}", model.label))?;
        }
        "effort" => {
            let level = argument(&args, "level").unwrap_or_default();
            if !crate::catalog::EFFORTS.contains(&level) {
                return Err(Fault::new(
                    "effort.unknown",
                    format!("`{level}` is not an effort; there is {}", crate::catalog::EFFORTS.join(", ")),
                ));
            }
            tx.set("session.effort", Value::str(level))?;
            notice(tx, Level::Info, format!("effort: {level}"))?;
        }
        "resume" => {
            let conversation = argument(&args, "conversation").unwrap_or_default();
            if conversation.is_empty() {
                // No argument: ask the daemon what there is, which is what the on-demand
                // source for conversations reads.
                tx.fx(Effect::new("kernel.log.list").with("id", Value::str("resume")));
            } else {
                tx.fx(Effect::new("kernel.log.load")
                    .with("conversation", Value::str(conversation))
                    .with("after", Value::Int(0))
                    .with("limit", Value::Int(2_000)));
            }
        }
        "compact" => {
            let messages = request_messages(tx.db());
            if messages.as_list().map(<[Value]>::len).unwrap_or(0) < 4 {
                notice(tx, Level::Info, "there is nothing worth summarising yet")?;
                return Ok(());
            }
            let request = tx.int("session.requests") + 1;
            tx.set("session.requests", Value::Int(request))?;
            let id = format!("r{request}");
            let provider = tx.text("session.provider");
            let model = tx.text("session.model");
            let conversation = tx.text("session.conversation");
            // A side request: a model call that is not a turn. It has its own attempt row
            // and it leaves the transcript alone until its answer arrives, which is what
            // makes a failed summary harmless.
            tx.set("session.compacting", Value::Bool(true))?;
            tx.set(
                "session.pending",
                Value::map([("request", Value::str(&id)), ("side", Value::Bool(true))]),
            )?;
            tx.fx(Effect::new("kernel.attempt.started")
                .with("id", Value::str(&id))
                .with("conversation", Value::str(&conversation))
                .with("provider", Value::str(&provider))
                .with("model", Value::str(&model))
                .with("kind", Value::str("side")));
            tx.fx(Effect::new("kernel.provider.call")
                .with("id", Value::str(&id))
                .with("provider", Value::str(&provider))
                .with("model", Value::str(&model))
                .with("messages", summary_request(&messages))
                .with("tools", Value::list([])));
            notice(tx, Level::Info, "summarising…")?;
        }
        "models" => {
            // Asking a service for its list is one request with one answer, and the answer
            // arrives as an event rather than here: a handler does not await.
            let provider = tx.text("session.provider");
            tx.fx(Effect::new("kernel.models.discover")
                .with("id", Value::str("models"))
                .with("provider", Value::str(&provider)));
            notice(tx, Level::Info, format!("asking `{provider}` for its models…"))?;
        }
        "status" => panel(
            tx,
            "status",
            "Session",
            "",
            vec![
                ("provider", tx.text("session.provider")),
                ("model", tx.text("session.model")),
                ("effort", tx.text("session.effort")),
                ("turns", tx.int("session.turn").to_string()),
                ("queued", queue_len(tx).to_string()),
                ("attachments", current_attachments(tx).as_list().map(<[Value]>::len).unwrap_or(0).to_string()),
                (
                    "credentials",
                    tx.get("session.credentials")
                        .and_then(Value::as_list)
                        .map(|slots| slots.len().to_string())
                        .unwrap_or_else(|| "unknown".into()),
                ),
            ],
            Vec::new(),
            vec![("panel.close".into(), "Close".into())],
        )?,
        "usage" => {
            open_usage(tx, None)?;
            if tx.get("session.usage_request").is_some() {
                tx.set("session.usage_again", Value::Bool(true))?;
            } else {
                refresh_usage(tx)?;
            }
        }
        "login" => {
            let slot = argument(&args, "provider").unwrap_or("scripted").to_string();
            // A service that hands out tokens rather than keys has nothing for anybody to
            // paste, so `/login` asks the kernel to start its device flow and the code comes
            // back as a panel. That is the difference between a subscription being usable
            // from this session and it being usable only from the daemon's own command line.
            if misa_kernel::presets::oauth(&slot).is_some() {
                if let Some(previous) = tx.get("session.oauth_request").and_then(Value::as_str) {
                    tx.fx(Effect::new("kernel.credential").with_value(Value::map([
                        ("id", Value::str(previous)), ("action", Value::str("cancel_oauth")), ("request", Value::str(previous)),
                    ])));
                }
                let sequence = tx.int("session.oauth_sequence") + 1;
                let request = format!("oauth:{}:{sequence}", tx.text("session.id"));
                tx.set("session.oauth_sequence", Value::Int(sequence))?;
                tx.set("session.oauth_request", Value::str(&request))?;
                panel(tx, "authorize", &format!("Authorize `{slot}`"), "Requesting a device code", vec![], vec![],
                    vec![("credential.cancel".into(), "Cancel authorization".into())])?;

                tx.fx(Effect::new("kernel.credential").with_value(Value::map([
                    ("id", Value::str(&request)),
                    ("action", Value::str("oauth")),
                    ("provider", Value::str(&slot)),
                ])));
                notice(tx, Level::Info, format!("authorizing `{slot}` — a code is about to be shown"))?;
                return Ok(());
            }
            // A secret is never an argument: it goes in a field, and the effect that stores
            // it is the only thing that ever sees it.
            panel(
                tx,
                "login",
                &format!("Credential for `{slot}`"),
                "The value is stored by the daemon and never reaches a policy or a view.",
                Vec::new(),
                vec![("value".to_string(), "Token".to_string())],
                vec![
                    ("panel.submit".into(), "Store".into()),
                    ("panel.close".into(), "Cancel".into()),
                ],
            )?;
            tx.set("panel.slot", Value::str(&slot))?;
        }
        "logout" => {
            let slot = argument(&args, "provider").unwrap_or("").to_string();
            if slot.is_empty() {
                notice(tx, Level::Warn, "say which provider to forget")?;
            } else {
                tx.fx(Effect::new("kernel.credential").with_value(Value::map([
                    ("id", Value::str("logout")),
                    ("action", Value::str("delete")),
                    ("slot", Value::str(&slot)),
                ])));
            }
        }
        "attach" | "image" => {
            let path = argument(&args, "path").unwrap_or("").to_string();
            if path.is_empty() {
                notice(tx, Level::Warn, format!("/{name} needs a path"))?;
            } else {
                tx.fx(Effect::new("kernel.blob.file").with_value(Value::map([
                    ("id", Value::str(&path)),
                    ("path", Value::str(&path)),
                ])));
            }
        }
        other => {
            notice(tx, Level::Warn, format!("`/{other}` is declared but not handled"))?;
        }
    }
    Ok(())
}

/// One named argument, as the client sent it.
fn argument<'a>(args: &'a Value, name: &str) -> Option<&'a str> {
    match args {
        // A single positional argument is what a person typed, so it is accepted without
        // the client having to invent a name for it.
        Value::Str(text) => Some(text),
        other => other.get(name).and_then(Value::as_str),
    }
}

/// The request a summary is asked for with.
///
/// A side request is not a turn: it never becomes a message, and a failure leaves the
/// transcript exactly as it was.
fn summary_request(messages: &Value) -> Value {
    Value::list([
        Value::map([
            ("role", Value::str("system")),
            (
                "text",
                Value::str(
                    "Summarise the conversation so far. Keep every decision, every file path, and \
                     every unresolved question. Be brief.",
                ),
            ),
        ]),
        Value::map([("role", Value::str("user")), ("text", messages.clone())]),
    ])
}

fn refresh_usage(tx: &mut Tx<'_>) -> Result<(), Fault> {
    let sequence = tx.int("session.usage_sequence") + 1;
    let id = format!("usage.{sequence}");
    tx.set("session.usage_sequence", Value::Int(sequence))?;
    tx.set("session.usage_request", Value::str(&id))?;
    tx.fx(Effect::new("kernel.usage")
        .with_value(Value::map([("id", Value::str(id)), ("provider", Value::str(tx.text("session.provider")))])));
    Ok(())
}
fn on_usage(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    if tx.get("session.usage_request").is_none() || tx.text("session.usage_request") != fields::event_text(event, "id")
    {
        return Ok(());
    }
    tx.delete("session.usage_request")?;
    tx.set("session.usage", event.data.get("facts").cloned().unwrap_or(Value::Null))?;
    if tx.text("panel.id") == "usage" {
        open_usage(tx, Some(fields::event_value(event, "facts")))?;
    }
    if tx.get("session.usage_again").and_then(Value::as_bool).unwrap_or(false) {
        tx.delete("session.usage_again")?;
        refresh_usage(tx)?;
    }
    Ok(())
}
fn open_usage(tx: &mut Tx<'_>, incoming: Option<Value>) -> Result<(), Fault> {
    let attempts = tx.get("attempts").and_then(Value::as_list).unwrap_or(&[]);
    let mut rows = vec![usage_row("Calls", "value.number", Value::Int(attempts.len() as i64))];
    for (key, label, role) in [
        ("input_tokens", "Input tokens", "value.tokens"),
        ("output_tokens", "Output tokens", "value.tokens"),
        ("cost_micros", "Spend", "value.money"),
    ] {
        let sum = attempts.iter().map(|a| a.get(key).and_then(Value::as_i64).unwrap_or(0)).sum();
        rows.push(usage_row(label, role, Value::Int(sum)));
    }
    let facts = incoming.or_else(|| tx.get("session.usage").cloned()).unwrap_or(Value::Null);
    if let Some(plan) = facts.get("plan").filter(|v| **v != Value::Null) {
        rows.push(usage_row("Plan", "value.text", plan.clone()));
    }
    if facts.get("unavailable").and_then(Value::as_bool).unwrap_or(true) {
        rows.push(usage_row("Provider quota", "value.text", Value::str("Unavailable")));
    }
    for window in facts.get("windows").and_then(Value::as_list).unwrap_or(&[]) {
        let label = window.get("label").and_then(Value::as_str).unwrap_or("Quota");
        for (key, suffix, role) in [
            ("used", "used", "value.number"),
            ("limit", "limit", "value.number"),
            ("remaining", "remaining", "value.number"),
            ("reset", "reset", "value.datetime"),
            ("reset_after_seconds", "reset after seconds", "value.number"),
        ] {
            if let Some(value) = window.get(key).filter(|v| **v != Value::Null) {
                let role = if ["used", "limit", "remaining"].contains(&key)
                    && window.get("unit").and_then(Value::as_str) == Some("percent")
                {
                    "value.percent"
                } else {
                    role
                };
                rows.push(usage_row(&format!("{label} · {suffix}"), role, value.clone()));
            }
        }
    }
    if let Some(count) = facts.get("reset_count").filter(|v| **v != Value::Null) {
        rows.push(usage_row("Quota resets available", "value.number", count.clone()));
    }
    for credit in facts.get("reset_credits").and_then(Value::as_list).unwrap_or(&[]) {
        let label = credit.get("label").and_then(Value::as_str).unwrap_or("Quota reset");
        for key in ["status", "granted", "expires"] {
            if let Some(value) = credit.get(key).filter(|v| **v != Value::Null) {
                rows.push(usage_row(
                    &format!("{label} · {key}"),
                    if key == "status" { "value.text" } else { "value.datetime" },
                    value.clone(),
                ));
            }
        }
    }
    if let Some(credits) = facts.get("credits") {
        for key in ["enabled", "currency", "limit", "used", "remaining", "unlimited", "has_credits"] {
            if let Some(value) = credits.get(key).filter(|v| **v != Value::Null) {
                rows.push(usage_row(&format!("Credits · {key}"), "value.number", value.clone()));
            }
        }
    }
    panel(
        tx,
        "usage",
        "Usage",
        "Session ledger and provider quota",
        vec![],
        vec![],
        vec![("panel.close".into(), "Close".into())],
    )?;
    tx.set("panel.rows", Value::list(rows))
}
fn usage_row(label: &str, role: &str, value: Value) -> Value {
    Value::map([("label", Value::str(label)), ("role", Value::str(role)), ("value", value)])
}

/// Open a panel: a title, some text, rows of facts, fields, and actions.
///
/// Deliberately not a "dialog": a panel is state the session keeps and the view shows. It
/// has no correlation token and no protected input, because the one interaction that needs
/// those — a credential — is a field and an action like everything else, and the secret
/// never leaves the daemon.
#[allow(clippy::too_many_arguments)]
fn panel(
    tx: &mut Tx<'_>,
    id: &str,
    title: &str,
    text: &str,
    rows: Vec<(&str, String)>,
    fields: Vec<(String, String)>,
    actions: Vec<(String, String)>,
) -> Result<(), Fault> {
    if tx.get("panel").is_some() {
        tx.delete("panel.slot")?;
    }
    tx.set(
        "panel",
        Value::map([
            ("id", Value::str(id)),
            ("title", Value::str(title)),
            ("text", Value::str(text)),
            (
                "rows",
                Value::list(
                    rows.into_iter()
                        .map(|(label, value)| Value::map([("label", Value::str(label)), ("value", Value::str(value))]))
                        .collect::<Vec<_>>(),
                ),
            ),
            (
                "fields",
                Value::list(
                    fields
                        .into_iter()
                        .map(|(id, label)| {
                            Value::map([
                                ("id", Value::str(id)),
                                ("label", Value::str(label)),
                                // A field a panel opens is a secret unless it is known to be
                                // something else: the only panel with a field today is the
                                // one for a credential.
                                ("secret", Value::Bool(true)),
                            ])
                        })
                        .collect::<Vec<_>>(),
                ),
            ),
            (
                "actions",
                Value::list(
                    actions
                        .into_iter()
                        .map(|(id, label)| Value::map([("id", Value::str(id)), ("label", Value::str(label))]))
                        .collect::<Vec<_>>(),
                ),
            ),
        ]),
    )?;
    // A panel is presentation state and is never journalled, which the manifest says and a
    // test checks.
    Ok(())
}

/// The affordances this session's own vocabulary is made of.
///
/// A composition that loaded something may not claim one of these: a plugin that declared
/// `panel.close` would have the session's own handler do its work, which is the shadowing this
/// list exists to make impossible. Named here so that "which actions are the session's" has one
/// answer, and pinned by a test that every one of them is handled below.
pub const ACTIONS: &[&str] = &[
    "composer.submit",
    "credential.cancel",
    "panel.close",
    "panel.submit",
    "queue.take",
    "queue.clear",
    "turn.cancel",
];

/// Whether a composition declared this action, and is therefore the one handling it.
///
/// The list is *state* rather than a field on this handler, because a handler reads the database
/// and nothing else — and because what a composition added is in the state it added. An action
/// nobody declared is still the fault it always was, which is how a client finds out that the
/// affordance it kept from an old view does not exist any more.
fn declared_action(tx: &Tx<'_>, action: &str) -> bool {
    tx.get("session.actions")
        .and_then(Value::as_list)
        .is_some_and(|actions| actions.iter().any(|declared| declared.as_str() == Some(action)))
}

/// Actions are the only thing a view offers, and this is the only place one is given a
/// meaning.
fn on_action(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let action = fields::event_text(event, "action");
    match action.as_str() {
        "composer.submit" => {
            let fields = fields::event_value(event, "fields");
            let text = fields
                .as_list()
                .and_then(|fields| fields.iter().find(|field| field.get("id").and_then(Value::as_str) == Some("prompt")))
                .and_then(|field| field.get("value"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if text.trim().is_empty() {
                notice(tx, Level::Info, "nothing to send")?;
                return Ok(());
            }
            tx.dispatch(Event::new("intent/prompt").with("text", Value::str(text)));
            Ok(())
        }
        // A queue dock's buttons are actions like any other, and they name the intent they
        // are asking for rather than being special-cased by a client.
        "queue.take" => {
            tx.dispatch(Event::new("intent/queue.take"));
            Ok(())
        }
        "queue.clear" => {
            tx.dispatch(Event::new("intent/queue.clear"));
            Ok(())
        }
        "credential.cancel" => {
            if let Some(request) = tx.get("session.oauth_request").and_then(Value::as_str) {
                tx.fx(Effect::new("kernel.credential").with_value(Value::map([
                    ("id", Value::str(request)), ("action", Value::str("cancel_oauth")), ("request", Value::str(request)),
                ])));
                if tx.text("panel.id") == "authorize" {
                    tx.set("panel.text", Value::str("Cancelling authorization"))?;
                    tx.set("panel.actions", Value::list([]))?;
                }
            }
            Ok(())
        }
        "panel.close" => {
            tx.delete("panel")?;
            Ok(())
        }
        "panel.submit" => {
            // The only path a secret takes: from a client's field, through this effect, into
            // the daemon's store.
            let id = tx.text("panel.id");
            let slot = tx.text("panel.slot");
            let fields = fields::event_value(event, "fields");
            let value = fields
                .as_list()
                .and_then(|fields| fields.first())
                .and_then(|field| field.get("value"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if id != "login" {
                tx.delete("panel")?;
                return Ok(());
            }
            if value.trim().is_empty() {
                notice(tx, Level::Warn, "there was no value to store")?;
                return Ok(());
            }
            tx.fx(Effect::new("kernel.credential").with_value(Value::map([
                ("id", Value::str("login")),
                ("action", Value::str("set")),
                ("slot", Value::str(&slot)),
                ("account", Value::str("default")),
                ("value", Value::str(value)),
            ])));
            Ok(())
        }
        "turn.cancel" => {
            tx.dispatch(Event::new("intent/cancel"));
            Ok(())
        }
        other => {
            // An action a composition declared belongs to whatever handler that composition
            // registered for `intent/action` — the same event this handler is answering. Nothing
            // here needs to know which one it was, or what it will do about it: that is what
            // routing by event kind means.
            if declared_action(tx, other) {
                return Ok(());
            }
            Err(Fault::handler(format!("no action named `{other}`")))
        }
    }
}

// ---------------------------------------------------------------------------
// Kernel reports that are facts
// ---------------------------------------------------------------------------

/// A conversation listing, stored so `/resume` and a picker can read it without asking
/// again.
fn on_conversations(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let headers = fields::event_value(event, "headers");
    let count = headers.as_list().map(<[Value]>::len).unwrap_or(0);
    tx.set("conversations", headers)?;
    notice(tx, Level::Info, format!("{count} conversations"))?;
    Ok(())
}

/// Somebody has to finish an authorization in a browser, so they are shown how.
///
/// A panel rather than a notice, because a code and an address are things a person copies
/// and comes back to, and because a panel is what every frontend already draws — which is
/// what makes a device flow startable from a terminal, a browser, and a phone alike rather
/// than from a daemon's console.
///
/// It is replaced by the outcome: the same `kernel.credential` event that follows closes it,
/// so an approved code never sits on a screen looking like something still to do.
fn on_credential_prompt(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    if tx.text("session.oauth_request") != fields::event_text(event, "id") { return Ok(()); }

    let provider = fields::event_text(event, "provider");
    let url = fields::event_text(event, "url");
    let code = fields::event_text(event, "code");
    panel(
        tx,
        "authorize",
        &format!("Authorize `{provider}`"),
        "Open the address, enter the code, and approve it. This session is polling until it is \
         approved, and the token it gets back is stored by the daemon.",
        vec![("code", code), ("address", url)],
        Vec::new(),
        vec![("credential.cancel".into(), "Cancel authorization".into()), ("panel.close".into(), "Dismiss".into())],
    )
}

/// A stored conversation, read back, which is what `/resume <id>` asked for.
///
/// The log is the truth, so the transcript is *rebuilt* from the entries rather than merged
/// with what is on screen: somebody who asked for another conversation asked to see that one.
/// A `reset` entry is honoured because it says an earlier part of the branch was cleared, and
/// a resumed conversation that opened with the messages compaction replaced would be a
/// transcript nobody ever had.
///
/// A message that was still streaming when the log was written is settled exactly as it was:
/// an unfinished turn is a fact about what happened, and nothing here re-issues it.
fn on_loaded(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let conversation = fields::event_text(event, "conversation");
    let entries = fields::event_value(event, "entries");
    let mut messages: Vec<Value> = Vec::new();
    for entry in entries.as_list().unwrap_or(&[]) {
        match entry.get("kind").and_then(Value::as_str).unwrap_or_default() {
            "reset" => messages.clear(),
            "message" => {
                if let Some(message) = entry.get("data").and_then(Value::as_map) {
                    let mut message = message.clone();
                    // A sequence is the id every node made from this message is keyed by, and
                    // two messages with no id are two nodes with the same id — a tree no client
                    // may be sent. An entry that carries none gets the log's own.
                    if !matches!(message.get("seq"), Some(Value::Int(_))) {
                        let seq = entry.get("seq").and_then(Value::as_i64).unwrap_or(messages.len() as i64);
                        message.insert("seq".to_string(), Value::Int(seq));
                    }
                    messages.push(Value::Map(std::sync::Arc::new(message)));
                }
            }
            // A kind this session does not know is a fact from a version that knew more, and
            // skipping it is the only honest thing to do with it.
            _ => {}
        }
    }
    // What a composition wrote into its own roots, applied again: its state is a fold of the
    // patches the log recorded, because nothing else can reproduce it. A patch that does not apply
    // is counted rather than ignored — a log and a composition that disagree is something a person
    // should hear about, and silence is how state goes missing.
    let (replayed, failed) = replay_patches(tx, entries.as_list().unwrap_or(&[]));
    let loaded = messages.len();
    let previous = tx.text("session.conversation");
    tx.set("session.conversation", Value::str(&conversation))?;
    tx.set("messages", Value::list(messages))?;
    // Nothing that was in flight is in flight any more, and a queue of prompts belongs to the
    // branch somebody was in: they asked to be somewhere else.
    tx.set("session.status", Value::str("idle"))?;
    tx.set("session.turn", Value::Int(loaded as i64))?;
    tx.delete("session.queue")?;
    tx.delete("panel")?;
    notice(
        tx,
        Level::Info,
        if previous == conversation {
            format!("reloaded {loaded} messages")
        } else {
            format!("resumed `{conversation}`: {loaded} messages")
        },
    )?;
    if replayed > 0 {
        notice(tx, Level::Info, format!("replayed {replayed} patches a composition recorded"))?;
    }
    if failed > 0 {
        notice(
            tx,
            Level::Warn,
            format!("{failed} recorded patches could not be replayed, so some plugin state is missing"),
        )?;
    }
    Ok(())
}

/// Fold what a composition recorded back into the roots it declared.
///
/// Returns how many patches were applied and how many could not be. A patch for a root this
/// composition does not have is *skipped* rather than counted as a failure: the log may be older
/// than the composition, and history it no longer has a place for is not a fault. Everything else —
/// a path that will not parse, an operation this version does not know — is counted, because a
/// session that quietly drops state is worse than one that says so.
///
/// The fold starts from the state the composition is running with, which is the values it declared
/// its roots to start at, so a resumed session and a replay agree by construction.
fn replay_patches(tx: &mut Tx<'_>, entries: &[Value]) -> (usize, usize) {
    // The fold runs over the database as a whole rather than over each root's own value, because
    // that is what a patch's path is relative to: `guest.turns[0]` names the first turn of the
    // `guest` root only because `guest` is a key in the database, and applying it to anything else
    // would be a second implementation of the rule this one is supposed to confirm.
    let mut database = tx.db().clone();
    let mut touched: Vec<String> = Vec::new();
    let mut replayed = 0;
    let mut failed = 0;
    for entry in entries {
        if entry.get("kind").and_then(Value::as_str) != Some(crate::contribution::PATCH_KIND) {
            continue;
        }
        let Some((path, op)) = entry.get("data").and_then(crate::contribution::recorded) else {
            failed += 1;
            continue;
        };
        let Some(root) = misa_value::Op::root_of(&path) else {
            failed += 1;
            continue;
        };
        // A root this composition does not declare: history from one that is not running.
        if tx.get(root).is_none() {
            continue;
        }
        match misa_value::apply_one(&database, &path, &op) {
            Ok(value) => {
                database = value;
                if !touched.iter().any(|name| name == root) {
                    touched.push(root.to_string());
                }
                replayed += 1;
            }
            Err(_) => failed += 1,
        }
    }
    // Written back once per root, because a root is the unit a composition declares and the unit a
    // patch here can name: the whole database is the kernel's and never a policy's to replace.
    for root in touched {
        let written = match database.get(&root) {
            Some(value) => tx.set(&root, value.clone()),
            // A recorded delete of a whole root leaves nothing to write, and leaving the root as
            // it was would be the session disagreeing with the log it just read.
            None => tx.delete(&root),
        };
        if written.is_err() {
            failed += 1;
        }
    }
    (replayed, failed)
}

/// What a credential change did.
fn on_credential(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let id = fields::event_text(event, "id");
    let oauth = id.starts_with("oauth:");
    if oauth && tx.text("session.oauth_request") != id { return Ok(()); }
    if oauth { tx.delete("session.oauth_request")?; }

    let ok = event.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let message = fields::event_text(event, "message");
    if !message.is_empty() {
        notice(tx, if ok { Level::Info } else { Level::Error }, message)?;
    }
    let slots = fields::event_value(event, "slots");
    tx.set("session.credentials", slots)?;
    // The panel root holds the slot, so removing the root removes both. This is not a
    // formality: an authorization can finish long after somebody dismissed the panel, and a
    // delete of a *child* of a root that is not there fails the whole transaction — which is
    // how a report whose panel had already gone became a fault and no notice at all.
    if !oauth || tx.text("panel.id") == "authorize" { tx.delete("panel")?; }
    Ok(())
}

/// A command that was left running has finished.
///
/// It becomes a line in the transcript rather than a notice, because both the person and the
/// model have to be able to see it later: the model is the one who has to decide what to do
/// about a build that failed while it was answering a different question, and the person is the
/// one who has to be able to tell why the model suddenly knew.
///
/// It does not start a turn of its own. A turn is something a person asks for, and a session
/// that answered every finished process with a provider call would be a session that spends
/// money behind somebody's back; the note is there for the next turn, whenever that is.
fn on_process_finished(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let pid = fields::event_int(event, "pid");
    let command = fields::event_text(event, "command");
    let killed = event.get("killed").and_then(Value::as_bool).unwrap_or(false);
    let exit = event.get("exit").and_then(Value::as_i64);
    let seconds = event.get("seconds").and_then(Value::as_f64).unwrap_or(0.0);
    let log = fields::event_text(event, "log");
    let ending = match (killed, exit) {
        (true, _) => "was stopped at its deadline".to_string(),
        (false, Some(code)) => format!("exited {code}"),
        (false, None) => "ended by a signal".to_string(),
    };
    let line = format!(
        "[background] `{command}` (pid {pid}) {ending} after {} · log {log}",
        seconds_text(seconds)
    );
    tx.push(
        "messages",
        Value::map([
            ("seq", Value::Int(message_count(tx) as i64 + 1)),
            ("role", Value::str("system")),
            ("text", Value::str(&line)),
            ("state", Value::str("done")),
            ("attachments", Value::list([])),
        ]),
    )?;
    Ok(())
}

/// A duration, as a person would say it.
fn seconds_text(seconds: f64) -> String {
    if seconds < 1.0 {
        format!("{}ms", (seconds * 1000.0).round() as i64)
    } else if seconds < 60.0 {
        format!("{seconds:.1}s")
    } else {
        format!("{}m{:.0}s", (seconds / 60.0).floor() as i64, seconds % 60.0)
    }
}

/// A file that became a blob, which is how an attachment arrives.
fn on_blob(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let ok = event.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if !ok {
        let message = fields::event_text(event, "message");
        notice(tx, Level::Error, format!("could not attach: {message}"))?;
        return Ok(());
    }
    let hash = fields::event_text(event, "hash");
    let media = fields::event_text(event, "media");
    let len = fields::event_int(event, "len");
    let source = fields::event_text(event, "id");
    tx.push(
        "session.attachments",
        Value::map([
            ("hash", Value::str(&hash)),
            ("media", Value::str(&media)),
            ("len", Value::Int(len)),
            ("source", Value::str(&source)),
        ]),
    )?;
    let what = if media.is_empty() { "a file".to_string() } else { media };
    notice(tx, Level::Info, format!("attached {what} ({len} bytes)"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// The loop
// ---------------------------------------------------------------------------

fn on_step(tx: &mut Tx<'_>, _event: &Event) -> Result<(), Fault> {
    if tx.text("session.status") != "thinking" {
        return Ok(());
    }
    let request = tx.int("session.requests") + 1;
    tx.set("session.requests", Value::Int(request))?;
    let id = format!("r{request}");

    // The first step of a session that has a real provider and no list yet asks that provider
    // what it serves, so a picker opened later has the service's own answer in it. It happens
    // once: the answer is remembered whether it worked or not, and `/models` is how somebody
    // asks again.
    let provider = tx.text("session.provider");
    if provider != "scripted" && tx.get("session.catalogue").is_none() {
        tx.fx(Effect::new("kernel.models.discover")
            .with("id", Value::str("models"))
            .with("provider", Value::str(&provider)));
    }

    let index = message_count(tx);
    let seq = tx.int("session.turn") + 1;
    tx.set("session.turn", Value::Int(seq))?;
    tx.push(
        "messages",
        Value::map([
            ("seq", Value::Int(seq)),
            ("role", Value::str("assistant")),
            ("text", Value::str("")),
            ("state", Value::str("streaming")),
            ("thinking", Value::str("")),
            ("calls", Value::list([])),
            ("attachments", Value::list([])),
        ]),
    )?;
    tx.set(
        "session.pending",
        Value::map([("request", Value::str(&id)), ("message", Value::Int(index as i64))]),
    )?;

    let provider = tx.text("session.provider");
    let model = tx.text("session.model");
    let effort = tx.text("session.effort");
    let mut settings = Value::map([]);
    if !effort.is_empty() {
        settings = Value::map([("reasoning_effort", Value::str(&effort))]);
    }
    // A ceiling on the answer, when the composition set one: an adapter knows which field this
    // service wants it in, and a session that names the field would be a session that knows a
    // provider's dialect.
    if let Some(max) = tx.cofx().config.get("max_tokens").and_then(Value::as_i64) {
        // One more setting, added rather than composed: `Value::map` takes literal keys, and
        // this one is a fact about the daemon's composition rather than about the loop.
        let mut entries = settings.as_map().cloned().unwrap_or_default();
        entries.insert("max_tokens".to_string(), Value::Int(max));
        settings = Value::Map(std::sync::Arc::new(entries));
    }
    tx.fx(Effect::new("kernel.attempt.started")
        .with("id", Value::str(&id))
        .with("conversation", Value::str(tx.text("session.conversation")))
        .with("provider", Value::str(&provider))
        .with("model", Value::str(&model))
        .with("kind", Value::str("turn")));
    tx.fx(Effect::new("kernel.provider.call")
        .with("id", Value::str(&id))
        .with("provider", Value::str(&provider))
        .with("model", Value::str(&model))
        .with("settings", settings)
        .with("messages", request_messages(tx.db()))
        .with("tools", tools()));
    Ok(())
}

fn on_delta(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let id = fields::event_text(event, "id");
    let text = fields::event_text(event, "text");
    let Some((index, seq)) = active_message(tx, &id) else {
        return Ok(());
    };
    tx.patch(&format!("messages[{index}].text"), Op::Append(Value::str(&text)))?;
    // The same text, pushed cheaply, so a client can show a growing answer without
    // re-reading a tree per token. A client that misses it converges on the next
    // subscription value.
    tx.fx(Effect::new("wire.event").with(
        "event",
        session_event(&SessionEvent::TextDelta { node: format!("msg.{seq}.text"), text }),
    ));
    Ok(())
}

/// Reasoning a service streamed, which the transcript shows apart from the answer.
///
/// It is patched into the message as it arrives rather than collected at the end, so a person
/// watching a slow model can see that it is thinking rather than that it is stuck. It travels
/// as a view refresh and not as a wire event of its own, which is exactly how the answer text
/// travels: a delta event is an optimisation, and a session with two of them is a session with
/// two things to keep in step.
fn on_thinking(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let id = fields::event_text(event, "id");
    let text = fields::event_text(event, "text");
    let Some((index, _)) = active_message(tx, &id) else {
        return Ok(());
    };
    tx.patch(&format!("messages[{index}].thinking"), Op::Append(Value::str(&text)))?;
    Ok(())
}

/// What a service said its models are.
///
/// Stored rather than merged: the catalogue is a *fact about a service* with a time on it, and
/// the session's own catalog is enrichment on top. A person asking twice gets an answer twice,
/// because a model list changes while a daemon runs.
fn on_models(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let ok = event.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let message = fields::event_text(event, "message");
    let provider = tx.text("session.provider");
    if !ok {
        notice(tx, Level::Warn, format!("could not list models: {message}"))?;
        return Ok(());
    }
    let models = fields::event_value(event, "models");
    let count = models.as_list().map(<[Value]>::len).unwrap_or(0);
    tx.set(
        "session.catalogue",
        Value::map([
            ("provider", Value::str(&provider)),
            ("at", Value::Int(now_ms())),
            ("models", models),
        ]),
    )?;
    notice(tx, Level::Info, format!("{count} models from {provider}"))?;
    Ok(())
}

/// A side request's answer: the summary replaces history, or nothing happens.
///
/// Guarded rather than blind: the replacement is refused unless there is a summary to
/// replace it with, which is the primitive the previous system called `replace history`,
/// and the reason a compaction can never silently drop what somebody said while it was
/// summarising.
fn on_summary(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let ok = event.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let summary = fields::event_text(event, "text");
    tx.delete("session.compacting")?;
    tx.delete("session.pending")?;
    if !ok || summary.trim().is_empty() {
        notice(tx, Level::Warn, "the summary did not arrive; nothing was replaced")?;
        return Ok(());
    }
    let conversation = tx.text("session.conversation");
    let replaced = tx.get("messages").and_then(Value::as_list).map(<[Value]>::len).unwrap_or(0);
    tx.set(
        "messages",
        Value::list([Value::map([
            ("seq", Value::Int(1)),
            ("role", Value::str("assistant")),
            ("text", Value::str(format!("[summary of {replaced} messages]\n\n{summary}"))),
            ("state", Value::str("done")),
            ("calls", Value::list([])),
            ("attachments", Value::list([])),
        ])]),
    )?;
    tx.set("session.turn", Value::Int(1))?;
    tx.set("session.status", Value::str("idle"))?;
    // The replacement is a fact, not a deletion: a shorter history means the branch was
    // replaced, and the log records that rather than quietly diverging.
    tx.fx(Effect::new("kernel.log.append")
        .with("conversation", Value::str(&conversation))
        .with("kind", Value::str("reset"))
        .with("data", Value::map([
            ("reason", Value::str("compacted")),
            ("replaced", Value::Int(replaced as i64)),
        ])));
    notice(tx, Level::Info, format!("compacted {replaced} messages"))?;
    tx.dispatch(Event::new("queue/next"));
    Ok(())
}

fn on_finished(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let id = fields::event_text(event, "id");
    // A side request is answered by its own handler, and it never touches the transcript
    // until the summary is in hand.
    if tx
        .get("session.pending")
        .and_then(|pending| pending.get("side"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return on_summary(tx, event);
    }
    let Some((index, seq)) = active_message(tx, &id) else {
        return Ok(());
    };
    let ok = event.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let text = fields::event_text(event, "text");
    let calls = fields::event_value(event, "tool_calls");

    tx.set(&format!("messages[{index}].state"), Value::str(if ok { "done" } else { "failed" }))?;
    tx.set(&format!("messages[{index}].text"), Value::str(&text))?;
    // The whole of what it reasoned, in case a delta was missed and because the message is
    // what gets replayed to a service that wants its own reasoning back.
    let thinking = fields::event_text(event, "thinking");
    if !thinking.is_empty() {
        tx.set(&format!("messages[{index}].thinking"), Value::str(&thinking))?;
    }

    let input_tokens = fields::event_int(event, "input_tokens");
    let output_tokens = fields::event_int(event, "output_tokens");
    let model = tx.text("session.model");
    let provider = tx.text("session.provider");
    let cost = crate::catalog::cost_micros(&model, input_tokens, output_tokens);
    tx.fx(Effect::new("kernel.attempt.settled")
        .with("id", Value::str(&id))
        .with("status", Value::str(if ok { "ok" } else { "error" }))
        .with("input_tokens", Value::Int(input_tokens))
        .with("output_tokens", Value::Int(output_tokens))
        .with("cost_micros", Value::Int(cost)));
    // The session keeps its own row as well, so an indicator has something to read without
    // a round trip to the ledger it just wrote.
    tx.push(
        "attempts",
        Value::map([
            ("name", Value::str(&id)),
            ("kind", Value::str("turn")),
            ("provider", Value::str(&provider)),
            ("model", Value::str(&model)),
            ("status", Value::str(if ok { "ok" } else { "error" })),
            ("input_tokens", Value::Int(input_tokens)),
            ("output_tokens", Value::Int(output_tokens)),
            ("cost_micros", Value::Int(cost)),
            ("finished_ms", Value::Int(now_ms())),
        ]),
    )?;
    tx.delete("session.pending")?;

    if !ok {
        let error = fields::event_text(event, "error");
        tx.set("session.status", Value::str("idle"))?;
        notice(tx, Level::Error, format!("the provider failed: {error}"))?;
        tx.dispatch(Event::new("queue/next"));
        return Ok(());
    }

    let calls = normalise_calls(&calls);
    tx.set(&format!("messages[{index}].calls"), Value::list(calls.clone()))?;
    // The journalled message is the one that finished, written from the values this
    // transaction has rather than read back out of the database: a read would see the text as
    // of the previous commit, which for a streamed answer is the text before the last delta.
    tx.fx(log_effect(
        tx,
        "message",
        Value::map([
            ("seq", Value::Int(seq)),
            ("role", Value::str("assistant")),
            ("text", Value::str(&text)),
            ("state", Value::str(if ok { "done" } else { "failed" })),
            ("thinking", Value::str(&thinking)),
            ("calls", Value::list(calls.clone())),
            ("attachments", Value::list([])),
        ]),
    ));

    if calls.is_empty() {
        tx.set("session.status", Value::str("idle"))?;
        tx.dispatch(Event::new("queue/next"));
    } else {
        tx.set("session.status", Value::str("tools"))?;
        tx.dispatch(Event::new("agent/tools"));
    }
    Ok(())
}

fn on_tools(tx: &mut Tx<'_>, _event: &Event) -> Result<(), Fault> {
    if tx.text("session.status") != "tools" {
        return Ok(());
    }
    let Some(index) = last_assistant(tx.db()) else {
        return Ok(());
    };
    let calls = calls_at(tx.db(), index);
    let mut dispatched = 0usize;
    for (position, call) in calls.iter().enumerate() {
        if call.get("status").and_then(Value::as_str) != Some("pending") {
            continue;
        }
        let call_id = call.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        tx.set(&format!("messages[{index}].calls[{position}].status"), Value::str("running"))?;
        tx.fx(Effect::new("kernel.tool.run")
            .with("id", Value::str(&call_id))
            .with("call_id", Value::str(&call_id))
            .with("name", Value::str(call.get("name").and_then(Value::as_str).unwrap_or_default()))
            .with("args", call.get("args").cloned().unwrap_or(Value::Null)));
        dispatched += 1;
    }
    if dispatched == 0 {
        // Nothing to run and nothing to wait for: the turn is over.
        tx.set("session.status", Value::str("idle"))?;
        tx.dispatch(Event::new("queue/next"));
    }
    Ok(())
}

fn on_tool_finished(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let call_id = fields::event_text(event, "call_id");
    let ok = event.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let text = fields::event_text(event, "text");
    let Some(index) = last_assistant(tx.db()) else {
        return Ok(());
    };
    let calls = calls_at(tx.db(), index);
    let Some(position) = calls
        .iter()
        .position(|call| call.get("id").and_then(Value::as_str) == Some(call_id.as_str()))
    else {
        return Ok(());
    };

    tx.set(
        &format!("messages[{index}].calls[{position}].status"),
        Value::str(if ok { "ok" } else { "error" }),
    )?;
    tx.set(&format!("messages[{index}].calls[{position}].result"), Value::str(&text))?;
    tx.fx(log_effect(
        tx,
        "tool_result",
        Value::map([
            ("call", Value::str(&call_id)),
            ("ok", Value::Bool(ok)),
            ("text", Value::str(&text)),
        ]),
    ));

    // The batch is done when no call is still pending or running. The call this event
    // settles has not been written yet, so its status is known here rather than read back.
    let settled_here = if ok { "ok" } else { "error" };
    let unsettled = calls.iter().enumerate().any(|(at, call)| {
        let status = if at == position {
            settled_here
        } else {
            call.get("status").and_then(Value::as_str).unwrap_or("pending")
        };
        matches!(status, "pending" | "running")
    });
    if !unsettled {
        tx.set("session.status", Value::str("thinking"))?;
        tx.dispatch(Event::new("agent/step"));
    }
    Ok(())
}

fn on_kernel_failed(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let message = fields::event_text(event, "message");
    tx.set("session.status", Value::str("idle"))?;
    tx.delete("session.pending")?;
    tx.delete("session.compacting")?;
    notice(tx, Level::Error, message)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers over the database
// ---------------------------------------------------------------------------

fn message_count(tx: &Tx<'_>) -> usize {
    tx.get("messages").and_then(Value::as_list).map(<[Value]>::len).unwrap_or(0)
}

fn messages(db: &Value) -> Vec<Value> {
    db.get("messages").and_then(Value::as_list).map(<[Value]>::to_vec).unwrap_or_default()
}

/// The index of the message a request is streaming into, and its sequence.
fn active_message(tx: &Tx<'_>, id: &str) -> Option<(usize, i64)> {
    let pending = tx.get("session.pending")?;
    if pending.get("request").and_then(Value::as_str) != Some(id) {
        return None;
    }
    let index = pending.get("message").and_then(Value::as_i64)? as usize;
    let seq = messages(tx.db()).get(index)?.get("seq").and_then(Value::as_i64)?;
    Some((index, seq))
}

fn pending_message(tx: &Tx<'_>) -> Option<usize> {
    tx.get("session.pending")
        .and_then(|pending| pending.get("message"))
        .and_then(Value::as_i64)
        .map(|index| index as usize)
}

fn last_assistant(db: &Value) -> Option<usize> {
    messages(db)
        .iter()
        .rposition(|message| message.get("role").and_then(Value::as_str) == Some("assistant"))
}

fn calls_at(db: &Value, index: usize) -> Vec<Value> {
    messages(db)
        .get(index)
        .and_then(|message| message.get("calls"))
        .and_then(Value::as_list)
        .map(<[Value]>::to_vec)
        .unwrap_or_default()
}

/// The provider's view of the conversation, rebuilt from the transcript.
///
/// Rebuilt rather than accumulated, because the log is the truth and a second
/// representation of history is exactly the reconciliation problem the previous system
/// spent a great deal of effort removing.
///
/// An attachment travels as its content hash. That is enough here: the kernel turns a hash
/// into bytes when it builds the request, because the kernel is the layer that holds the
/// store, and a session that fetched a blob would be a session with a second way to read the
/// disk.
fn request_messages(db: &Value) -> Value {
    let mut out = Vec::new();
    for message in messages(db) {
        let role = message.get("role").and_then(Value::as_str).unwrap_or_default();
        match role {
            "user" => out.push(Value::map([
                ("role", Value::str("user")),
                ("text", message.get("text").cloned().unwrap_or(Value::Null)),
                ("attachments", message.get("attachments").cloned().unwrap_or_else(|| Value::list([]))),
            ])),
            // A note the session wrote to the model: a background command finishing, and
            // whatever else the loop learns on its own. Both providers read a `system` message.
            "system" => out.push(Value::map([
                ("role", Value::str("system")),
                ("text", message.get("text").cloned().unwrap_or(Value::Null)),
            ])),
            "assistant" => {
                out.push(Value::map([
                    ("role", Value::str("assistant")),
                    ("text", message.get("text").cloned().unwrap_or(Value::Null)),
                    // A service that wants its own reasoning back on a tool-call turn gets it;
                    // every other adapter ignores the field, and the message carries it either
                    // way because it is part of what the model said.
                    ("thinking", message.get("thinking").cloned().unwrap_or_else(|| Value::str(""))),
                    ("tool_calls", message.get("calls").cloned().unwrap_or_else(|| Value::list([]))),
                ]));
                for call in message.get("calls").and_then(Value::as_list).unwrap_or(&[]) {
                    let Some(result) = call.get("result") else {
                        continue;
                    };
                    if result.is_null() {
                        continue;
                    }
                    out.push(Value::map([
                        ("role", Value::str("tool")),
                        ("call", call.get("id").cloned().unwrap_or(Value::Null)),
                        ("text", result.clone()),
                    ]));
                }
            }
            _ => {}
        }
    }
    Value::list(out)
}

/// What the session tells a provider it can do.
///
/// Read from the catalog rather than written here, so a tool is declared once and the same
/// declaration is what the kernel's implementations are checked against.
fn tools() -> Value {
    crate::catalog::tool_schemas()
}

fn normalise_calls(calls: &Value) -> Vec<Value> {
    calls
        .as_list()
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .map(|(index, call)| {
            Value::map([
                (
                    "id",
                    Value::str(
                        call.get("id")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                            .unwrap_or_else(|| format!("call.{index}")),
                    ),
                ),
                ("name", call.get("name").cloned().unwrap_or(Value::Null)),
                ("args", call.get("args").cloned().unwrap_or(Value::Null)),
                ("status", Value::str("pending")),
            ])
        })
        .collect()
}

fn log_effect(tx: &Tx<'_>, kind: &str, data: Value) -> Effect {
    Effect::new("kernel.log.append")
        .with("conversation", Value::str(tx.text("session.conversation")))
        .with("kind", Value::str(kind))
        .with("data", data)
}

/// Record a notice and push it to every attached client.
///
/// Two things, deliberately: the transcript keeps it so a client that attaches later sees
/// it, and the wire event delivers it now so a client already attached does not have to
/// wait for the next view. The notice is presentation state and never journalled — the
/// manifest says so, and a test enforces it.
fn notice(tx: &mut Tx<'_>, level: Level, text: impl Into<String>) -> Result<(), Fault> {
    let text = text.into();
    tx.push(
        "notices",
        Value::map([("level", Value::str(level.as_str())), ("text", Value::str(&text))]),
    )?;
    tx.fx(notice_effect(level, text));
    Ok(())
}

fn notice_effect(level: Level, text: impl Into<String>) -> Effect {
    Effect::new("wire.event").with(
        "event",
        session_event(&SessionEvent::Notice { level, text: text.into() }),
    )
}

/// A session event, on its way into an effect's data.
fn session_event(event: &SessionEvent) -> Value {
    let mut bytes = Vec::new();
    if ciborium::ser::into_writer(event, &mut bytes).is_err() {
        return Value::Null;
    }
    ciborium::de::from_reader(&bytes[..]).unwrap_or(Value::Null)
}

/// The view's composer, as the session declares it.
///
/// A field and an action: what a client draws for them is the client's business, and the
/// action's meaning is only known here.
pub fn composer() -> Node {
    Node::new(
        "composer",
        FieldSummary::fields(vec![Field {
            id: "prompt".into(),
            label: "Message".into(),
            value: String::new(),
            hint: Some("enter to send".into()),
            read_only: false,
            secret: false,
            kind: FieldKind::Block,
        }]),
    )
    .id("composer")
    .action(misa_proto::view::Action {
        id: "composer.submit".into(),
        on: misa_proto::view::ActionOn::Submit,
        label: Some("Send".into()),
        args: Value::Null,
    })
    .child(Node::text("composer.hint", [Span::plain("the session owns what happens next")]))
}

/// A one-line constructor, so the composer reads as what it is.
struct FieldSummary;

impl FieldSummary {
    fn fields(fields: Vec<Field>) -> misa_proto::view::Kind {
        misa_proto::view::Kind::Fields { fields }
    }
}
