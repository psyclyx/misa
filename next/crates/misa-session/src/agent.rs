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
//!                          independent assistant stream
//!                                                                     ▼
//! kernel/provider.delta ──▶ append to stream current ──▶ wire Stream
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

use crate::{Level, SessionEvent};
use misa_proto::view::{Field, FieldKind, Node};
use misa_reframe::fields;
use misa_reframe::{Effect, Event, Fault, Registry, Tx};
use misa_value::{Op, Value};
use std::collections::BTreeSet;

use misa_kernel::KernelEvent;

use crate::now_ms;
use crate::views;

/// Every handler and subscription the shipped agent loop installs.
pub fn registry() -> Registry {
    crate::completions::subscriptions(crate::usage::subscriptions(views::subscriptions(
        Registry::new(),
    )))
    .on_fn("session/started", 0, "agent.discovery", on_started)
    .on_fn("owner/notice", 0, "agent.notice", |tx, event| {
        let level = match fields::event_text(event, "level").as_str() {
            "error" => Level::Error,
            "warn" => Level::Warn,
            _ => Level::Info,
        };
        notice(tx, level, fields::event_text(event, "text"))
    })
    .on_fn(
        "discovery/models.refresh",
        0,
        "agent.models.refresh",
        |tx, _| refresh_models(tx),
    )
    .on_fn(
        "discovery/credentials.refresh",
        0,
        "agent.credentials.refresh",
        |tx, _| refresh_credentials(tx),
    )
    .on_fn(
        "discovery/usage.refresh",
        0,
        "agent.usage.refresh",
        |tx, _| refresh_usage(tx),
    )
    .on_fn(
        "discovery/conversations.refresh",
        0,
        "agent.conversations.refresh",
        |tx, _| {
            refresh_conversations(tx);
            Ok(())
        },
    )
    .on_fn("intent/prompt", 0, "agent.prompt", on_prompt)
    .on_fn("intent/interrupt", 0, "agent.interrupt", on_interrupt)
    .on_fn("intent/cancel", 0, "agent.cancel", on_cancel)
    .on_fn(
        "operation/prompt.cancel",
        0,
        "agent.prompt.cancel",
        on_prompt_cancel,
    )
    .on_fn("intent/command", 0, "agent.command", on_command)
    .on_fn(
        "intent/effort.cycle",
        0,
        "agent.effort.cycle",
        on_effort_cycle,
    )
    .on_fn("intent/action", 0, "agent.action", on_action)
    .on_fn("intent/queue.edit", 0, "agent.queue.edit", on_queue_take)
    // Kept as an internal compatibility route for already-installed clients;
    // the shipped view does not advertise a clear button because the reference
    // queue has no such affordance.
    .on_fn("intent/queue.take", 0, "agent.queue.take", on_queue_take)
    .on_fn("intent/queue.steer", 0, "agent.queue.steer", on_queue_steer)
    .on_fn("intent/queue.clear", 0, "agent.queue.clear", on_queue_clear)
    .on_fn(
        "kernel/conversations",
        0,
        "agent.conversations",
        on_conversations,
    )
    .on_fn("kernel/log.loaded", 0, "agent.loaded", on_loaded)
    .on_fn("kernel/log.appended", 0, "agent.appended", on_appended)
    .on_fn("kernel/credential", 0, "agent.credential", on_credential)
    .on_fn(
        "kernel/credential.prompt",
        0,
        "agent.credential.prompt",
        on_credential_prompt,
    )
    .on_fn("kernel/blob", 0, "agent.blob", on_blob)
    .on_fn(
        "kernel/process.finished",
        0,
        "agent.process.finished",
        on_process_finished,
    )
    .on_fn("queue/next", 0, "agent.queue.next", on_queue_next)
    .on_fn("agent/step", 0, "agent.step", on_step)
    .on_fn("agent/tools", 0, "agent.tools", on_tools)
    .on_fn("kernel/usage", 0, "agent.usage", on_usage)
    .on_fn("kernel/models", 0, "agent.models", on_models)
    .on_fn("kernel/provider.finished", 0, "agent.finished", on_finished)
    .on_fn(
        "kernel/tool.finished",
        0,
        "agent.tool.finished",
        on_tool_finished,
    )
    .on_fn("kernel/failed", 0, "agent.failed", on_kernel_failed)
}

/// The event a kernel report becomes.
///
/// The kernel's vocabulary and the loop's are deliberately different names, and this
/// function is the whole of the translation between them. It is the seam a second kernel
/// implementation would have to satisfy.
pub fn event_for(event: KernelEvent) -> Event {
    match event {
        KernelEvent::ProviderDelta { id, text } => Event::new("kernel/provider.delta")
            .with("id", Value::str(id))
            .with("text", Value::str(text)),
        // A service reasoning aloud, which is not the answer and is kept apart from it all the
        // way to the view.
        KernelEvent::ProviderThinking { id, text } => Event::new("kernel/provider.thinking")
            .with("id", Value::str(id))
            .with("text", Value::str(text)),
        KernelEvent::Usage {
            id,
            provider,
            facts,
        } => Event::new("kernel/usage")
            .with("id", Value::str(id))
            .with("provider", Value::str(provider))
            .with("facts", facts),
        KernelEvent::Models {
            id,
            ok,
            models,
            message,
        } => Event::new("kernel/models")
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
            provider_state,
            input_tokens,
            output_tokens,
            error,
        } => Event::new("kernel/provider.finished")
            .with("id", Value::str(id))
            .with("ok", Value::Bool(ok))
            .with("text", Value::str(text))
            .with("thinking", Value::str(thinking))
            .with("tool_calls", tool_calls)
            .with("provider_state", provider_state)
            .with("input_tokens", Value::Int(input_tokens))
            .with("output_tokens", Value::Int(output_tokens))
            .with("error", Value::str(error)),
        KernelEvent::ToolFinished {
            id,
            call_id,
            ok,
            text,
        } => Event::new("kernel/tool.finished")
            .with("id", Value::str(id))
            .with("call_id", Value::str(call_id))
            .with("ok", Value::Bool(ok))
            .with("text", Value::str(text)),
        KernelEvent::Conversations { id, headers } => Event::new("kernel/conversations")
            .with("id", Value::str(id))
            .with("headers", headers),
        // A conversation read back: its entries in the order they were appended, each with the
        // kind that says what it is. This is the one event that *rebuilds* a transcript rather
        // than adding to one, which is why it is not noted and dropped like the rest.
        KernelEvent::Loaded {
            conversation,
            entries,
        } => Event::new("kernel/log.loaded")
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
        KernelEvent::Credential {
            id,
            ok,
            message,
            slot,
            slots,
            model_providers,
        } => Event::new("kernel/credential")
            .with("id", Value::str(id))
            .with("ok", Value::Bool(ok))
            .with("message", Value::str(message))
            .with("slot", slot.map(Value::str).unwrap_or(Value::Null))
            .with("slots", slots)
            .with("model_providers", model_providers),
        // The one kernel event a person has to act on: a code and where to type it. It is
        // carried as the two strings it is, because what to do with them — a panel, a
        // notification, a link — is the session's decision and not the kernel's.
        KernelEvent::CredentialPrompt {
            id,
            provider,
            url,
            code,
        } => Event::new("kernel/credential.prompt")
            .with("id", Value::str(id))
            .with("provider", Value::str(provider))
            .with("url", Value::str(url))
            .with("code", Value::str(code)),
        KernelEvent::Blob {
            id,
            ok,
            hash,
            len,
            media,
            message,
        } => Event::new("kernel/blob")
            .with("id", Value::str(id))
            .with("ok", Value::Bool(ok))
            .with("hash", Value::str(hash))
            .with("len", Value::Int(len))
            .with("media", Value::str(media))
            .with("message", Value::str(message)),
        KernelEvent::BlobBytes {
            id,
            ok,
            media,
            message,
            ..
        } => Event::new("kernel/blob.bytes")
            .with("id", Value::str(id))
            .with("ok", Value::Bool(ok))
            .with("media", Value::str(media))
            .with("message", Value::str(message)),
        // The one report nothing asked for at the moment it arrives: a command that was left
        // running has finished, and the session has to decide what to say about it.
        KernelEvent::ProcessFinished {
            pid,
            command,
            exit,
            killed,
            seconds,
            log,
        } => Event::new("kernel/process.finished")
            .with("pid", Value::Int(pid as i64))
            .with("command", Value::str(command))
            .with(
                "exit",
                exit.map(|code| Value::Int(code as i64))
                    .unwrap_or(Value::Null),
            )
            .with("killed", Value::Bool(killed))
            .with("seconds", Value::Float(seconds))
            .with("log", Value::str(log)),
        KernelEvent::HttpFinished {
            id,
            ok,
            status,
            body,
        } => Event::new("kernel/http.finished")
            .with("id", Value::str(id))
            .with("ok", Value::Bool(ok))
            .with("status", Value::Int(status as i64))
            .with("body", Value::str(body)),
        KernelEvent::Failed { id, message } => Event::new("kernel/failed")
            .with("id", Value::str(id))
            .with("message", Value::str(message)),
        // Durable acknowledgment is the only point at which a fact joins canonical state.
        KernelEvent::AppendFailed {
            conversation,
            kind,
            data,
            message,
        } => Event::new("kernel/log.failed")
            .with("conversation", Value::str(conversation))
            .with("kind", Value::str(kind))
            .with("data", data)
            .with("message", Value::str(message)),
        KernelEvent::Appended {
            conversation,
            seq,
            kind,
            data,
        } => Event::new("kernel/log.appended")
            .with("conversation", Value::str(conversation))
            .with("seq", Value::Int(seq))
            .with("kind", Value::str(kind))
            .with("data", data),
        KernelEvent::AttemptRecorded { .. } => Event::new("kernel/noted").with_value(Value::Null),
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
fn on_interrupt(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let text = fields::event_text(event, "prompt");
    let attachments = submitted_attachments(tx, event);
    if text.trim().is_empty() && attachments.as_list().is_none_or(|items| items.is_empty()) {
        return Err(Fault::handler("a prompt with no text or attachments"));
    }
    let operation = crate::operations::admit_prompt(tx, event)?;
    let status = tx.text("session.status");
    if status == "idle" {
        return begin_turn(tx, &text, attachments, operation);
    }
    let id = next_id(tx, "session.queue_seq")?;
    let mut queue = vec![Value::map([
        ("text", Value::str(text)),
        ("attachments", attachments),
        ("state", Value::str("waiting")),
        ("id", Value::Int(id)),
        ("interrupt", Value::Bool(true)),
        ("operation", operation),
    ])];
    queue.extend(
        tx.get("session.queue")
            .and_then(Value::as_list)
            .unwrap_or(&[])
            .iter()
            .cloned(),
    );
    tx.set("session.queue", Value::list(queue))?;
    // A completed answer already being journalled must settle once. Its log reply
    // drains the priority prompt; cancelling it here would append a second answer.
    if matches!(status.as_str(), "recording" | "tools") {
        let active = tx.get("session.operation").cloned().unwrap_or(Value::Null);
        crate::operations::cancel_prompt_inputs(tx, &active)?;
        return Ok(());
    }
    let pending = tx.get("session.pending").is_some();
    on_cancel(tx, event)?;
    if !pending {
        tx.dispatch(Event::new("queue/next"));
    }
    Ok(())
}

fn on_prompt(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let text = fields::event_text(event, "text");
    let attachments = submitted_attachments(tx, event);
    if text.trim().is_empty() && attachments.as_list().is_none_or(|items| items.is_empty()) {
        return Err(Fault::handler("a prompt with no text or attachments"));
    }
    let operation = crate::operations::admit_prompt(tx, event)?;
    if tx.text("session.status") != "idle" {
        let waiting = queue_len(tx) + 1;
        let queue_id = next_id(tx, "session.queue_seq")?;
        tx.push(
            "session.queue",
            Value::map([
                ("text", Value::str(&text)),
                ("attachments", attachments),
                ("state", Value::str("waiting")),
                ("id", Value::Int(queue_id)),
                ("operation", operation),
            ]),
        )?;
        notice(tx, Level::Info, format!("queued ({waiting} waiting)"))?;
        return Ok(());
    }
    begin_turn(tx, &text, attachments, operation)
}

/// Start a turn: record the user message, mark the session thinking, ask for a step.
///
/// One function because a submission and a queue drain both begin a turn, and two copies
/// of this would be two places for the ordering to drift.
fn begin_turn(
    tx: &mut Tx<'_>,
    text: &str,
    attachments: Value,
    operation: Value,
) -> Result<(), Fault> {
    crate::operations::prompt_state(tx, &operation, "running", None)?;
    tx.set("session.operation", operation.clone())?;
    let turn = tx.int("session.turn") + 1;
    let message = Value::map([
        ("seq", Value::Int(turn)),
        ("role", Value::str("user")),
        ("operation", operation),
        ("text", Value::str(text)),
        ("state", Value::str("done")),
        // UTC milliseconds on the wire; the client owns local-time formatting.
        ("at_ms", Value::Int(now_ms())),
        ("attachments", attachments),
    ]);
    tx.set("session.turn", Value::Int(turn))?;

    // A submitted prompt is spent: the draft's attachments belong to the turn, not to the
    // next one.
    tx.set("session.attachments", Value::list([]))?;
    tx.set("session.status", Value::str("recording"))?;
    tx.fx(log_effect(tx, "message", message));
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
    let text = head
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut remaining = tx
        .get("session.queue")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .iter()
        .skip(1)
        .cloned()
        .collect::<Vec<_>>();
    // Consume cancellation marks for the prior active turn atomically with dequeue.
    if head
        .get("interrupt")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        for entry in &mut remaining {
            if let Value::Map(fields) = entry {
                std::sync::Arc::make_mut(fields).remove("interrupt");
            }
        }
    }
    tx.set("session.queue", Value::list(remaining))?;
    begin_turn(
        tx,
        &text,
        head.get("attachments")
            .cloned()
            .unwrap_or_else(|| Value::list([])),
        head.get("operation").cloned().unwrap_or(Value::Null),
    )
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
    let entry = tx
        .get(&format!("session.queue[{}]", last - 1))
        .cloned()
        .unwrap_or(Value::Null);
    let text = entry
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    crate::operations::prompt_state(
        tx,
        &entry.get("operation").cloned().unwrap_or(Value::Null),
        "cancelled",
        None,
    )?;
    tx.delete(&format!("session.queue[{}]", last - 1))?;
    tx.fx(Effect::new("wire.event").with("event", session_event(&SessionEvent::Recover { text })));
    Ok(())
}

/// Steer the oldest queued prompt into the active turn.
///
/// Reuse the admitted operation and queued attachments. Creating a fresh prompt
/// here would leave the original operation pending and make the queue action
/// observably different from Alt-Enter.
fn on_queue_steer(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let Some(head) = tx
        .get("session.queue")
        .and_then(Value::as_list)
        .and_then(|queue| queue.first())
        .cloned()
    else {
        notice(tx, Level::Info, "nothing is queued")?;
        return Ok(());
    };
    let remaining = tx
        .get("session.queue")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .iter()
        .skip(1)
        .cloned()
        .collect::<Vec<_>>();
    let text = head
        .get("text")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let attachments = head
        .get("attachments")
        .cloned()
        .unwrap_or_else(|| Value::list([]));
    let operation = head.get("operation").cloned().unwrap_or(Value::Null);
    let status = tx.text("session.status");
    if status == "idle" {
        tx.set("session.queue", Value::list(remaining))?;
        return begin_turn(tx, &text, attachments, operation);
    }
    let priority = Value::map([
        ("text", Value::str(&text)),
        ("attachments", attachments),
        ("state", Value::str("waiting")),
        ("id", head.get("id").cloned().unwrap_or(Value::Int(0))),
        ("interrupt", Value::Bool(true)),
        ("operation", operation),
    ]);
    tx.set(
        "session.queue",
        Value::list(
            std::iter::once(priority)
                .chain(remaining)
                .collect::<Vec<_>>(),
        ),
    )?;
    if matches!(status.as_str(), "recording" | "tools") {
        let active = tx.get("session.operation").cloned().unwrap_or(Value::Null);
        crate::operations::cancel_prompt_inputs(tx, &active)?;
        return Ok(());
    }
    let pending = tx.get("session.pending").is_some();
    on_cancel(tx, event)?;
    if !pending {
        tx.dispatch(Event::new("queue/next"));
    }
    Ok(())
}

fn on_queue_clear(tx: &mut Tx<'_>, _event: &Event) -> Result<(), Fault> {
    let queue = tx
        .get("session.queue")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .to_vec();
    for entry in queue {
        crate::operations::prompt_state(
            tx,
            &entry.get("operation").cloned().unwrap_or(Value::Null),
            "cancelled",
            None,
        )?;
    }
    tx.set("session.queue", Value::list([]))?;
    Ok(())
}

fn queue_len(tx: &Tx<'_>) -> usize {
    tx.get("session.queue")
        .and_then(Value::as_list)
        .map(<[Value]>::len)
        .unwrap_or(0)
}

fn current_attachments(tx: &Tx<'_>) -> Value {
    tx.get("session.attachments")
        .cloned()
        .unwrap_or_else(|| Value::list([]))
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
    for attachment in draft
        .as_list()
        .into_iter()
        .flatten()
        .chain(named.as_list().into_iter().flatten())
    {
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

fn on_prompt_cancel(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let id = event.get("operation").cloned().unwrap_or(Value::Null);
    if tx.get("session.operation") == Some(&id) {
        return on_cancel(tx, event);
    }
    let queue = tx
        .get("session.queue")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .iter()
        .filter(|entry| entry.get("operation") != Some(&id))
        .cloned()
        .collect::<Vec<_>>();
    tx.set("session.queue", Value::list(queue))?;
    crate::operations::prompt_state(tx, &id, "cancelled", None)
}

fn on_cancel(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    if tx.text("session.status") == "idle" {
        return Ok(());
    }
    let operation = tx.get("session.operation").cloned().unwrap_or(Value::Null);
    if let Some(index) = tx
        .get("prompt_operations")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .iter()
        .position(|record| record.get("id") == Some(&operation))
    {
        let base = format!("prompt_operations[{index}]");
        if tx.text(&format!("{base}.state")) == "cancelling" {
            return Ok(());
        }
        let generation = tx.int(&format!("{base}.generation")) + 1;
        tx.set(&format!("{base}.generation"), Value::Int(generation))?;
        crate::operations::prompt_state(tx, &operation, "cancelling", None)?;
    }
    crate::operations::cancel_prompt_inputs(tx, &operation)?;
    if let Some(seq) = tx
        .get("session.pending")
        .and_then(|pending| pending.get("seq"))
        .and_then(Value::as_i64)
    {
        tx.fx(log_effect(
            tx,
            "message",
            Value::map([
                ("seq", Value::Int(seq)),
                ("role", Value::str("assistant")),
                ("state", Value::str("cancelled")),
                (
                    "operation",
                    tx.get("session.operation").cloned().unwrap_or(Value::Null),
                ),
                ("text", fields::event_value(event, "text")),
                ("thinking", fields::event_value(event, "thinking")),
                ("calls", Value::list([])),
                ("attachments", Value::list([])),
            ]),
        ));
        tx.set("session.status", Value::str("recording"))?;
    } else {
        crate::operations::settle_prompt(tx, "cancelled", None)?;
        tx.set("session.status", Value::str("idle"))?;
    }
    tx.delete("session.pending")?;
    tx.delete("session.compacting")?;
    tx.fx(notice_effect(Level::Warn, "cancelled"));
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
    let declared = crate::catalog::commands()
        .into_iter()
        .find(|command| command.id == name);
    let Some(declared) = declared else {
        let known: Vec<String> = crate::catalog::commands()
            .into_iter()
            .map(|command| format!("/{}", command.id))
            .collect();
        notice(
            tx,
            Level::Warn,
            format!("no command named `/{name}`; there is {}", known.join(", ")),
        )?;
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
            tx.set("session.status", Value::str("idle"))?;
            tx.set("session.attachments", Value::list([]))?;
            tx.delete("session.pending")?;
            tx.delete("panel")?;
            tx.fx(Effect::new("kernel.log.append")
                .with("conversation", Value::str(&conversation))
                .with("kind", Value::str("reset"))
                .with(
                    "data",
                    Value::map([(
                        "reason",
                        Value::str(argument(&args, "reason").unwrap_or("cleared")),
                    )]),
                ));
        }
        "model" => {
            let requested = argument(&args, "model").unwrap_or_default();
            let (provider, id) = model_reference(requested, &tx.text("session.provider"));
            let known = crate::catalog::model(&id).filter(|model| model.provider == provider);
            let catalogue = tx
                .get("session.catalogue")
                .and_then(|catalogue| catalogue.get("models"))
                .and_then(Value::as_list)
                .map(<[Value]>::to_vec)
                .unwrap_or_default();
            let discovered = catalogue.iter().any(|model| {
                model.get("provider").and_then(Value::as_str) == Some(provider.as_str())
                    && model.get("id").and_then(Value::as_str) == Some(id.as_str())
            });
            if known.is_none() && !discovered {
                return Err(Fault::new(
                    "model.unknown",
                    format!("no model named `{requested}`"),
                ));
            }
            tx.set("session.provider", Value::str(&provider))?;
            tx.set("session.model", Value::str(&id))?;
            // Effort is a property of the model, so selecting a model that does not take one
            // clears it rather than leaving a setting nothing reads.
            let discovered_value = Value::list(catalogue);
            let effort_levels = crate::catalog::effort_levels(&discovered_value, &provider, &id);
            if effort_levels.is_empty() {
                tx.set("session.effort", Value::Null)?;
            } else if let Some(default) =
                crate::catalog::default_effort(&discovered_value, &provider, &id)
            {
                tx.set("session.effort", Value::str(default))?;
            }
        }
        "effort" => {
            let level = argument(&args, "level").unwrap_or_default();
            let provider = tx.text("session.provider");
            let model = tx.text("session.model");
            let discovered = tx
                .get("session.catalogue")
                .and_then(|catalogue| catalogue.get("models"))
                .cloned()
                .unwrap_or(Value::Null);
            let levels = crate::catalog::effort_levels(&discovered, &provider, &model);
            if !levels.iter().any(|candidate| candidate == level) {
                return Err(Fault::new(
                    "effort.unknown",
                    format!("`{level}` is not an effort; there is {}", levels.join(", ")),
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
        "status" => {
            let provider = argument(&args, "provider").unwrap_or_default();
            tx.set("session.status_provider", Value::str(provider))?;
            open_status(tx, provider)?;
            tx.fx(Effect::new("kernel.credential").with_value(Value::map([
                ("id", Value::str("status")),
                ("action", Value::str("list")),
            ])));
        }
        "usage" => {
            open_usage(tx, None)?;
            if tx.get("session.usage_request").is_some() {
                tx.set("session.usage_again", Value::Bool(true))?;
            } else {
                refresh_usage(tx)?;
            }
        }
        "login" => {
            let slot = argument(&args, "provider").unwrap_or_default().to_string();
            let account = args
                .get("account")
                .and_then(Value::as_str)
                .filter(|account| !account.is_empty())
                .unwrap_or("default")
                .to_string();
            if slot.is_empty() {
                return Err(Fault::argument(&name, "provider", Some("providers")));
            }
            if misa_kernel::presets::preset_for_slot(&slot)
                .is_some_and(|preset| preset.api == "claude.cli")
            {
                // Claude authentication belongs to Claude Code, not to a token
                // field. Opening the generic key panel here would invite a secret
                // that this adapter never reads and leave the user apparently logged
                // in while requests still fail.
                notice(
                    tx,
                    Level::Info,
                    "Claude uses Claude Code authentication; run `claude` on the daemon host to sign in",
                )?;
                return Ok(());
            }
            // A service that hands out tokens rather than keys has nothing for anybody to
            // paste, so `/login` asks the kernel to start its device flow and the code comes
            // back as a panel. That is the difference between a subscription being usable
            // from this session and it being usable only from the daemon's own command line.
            if misa_kernel::presets::oauth(&slot).is_some() {
                if let Some(previous) = tx.get("session.oauth_request").and_then(Value::as_str) {
                    tx.fx(Effect::new("kernel.credential").with_value(Value::map([
                        ("id", Value::str(previous)),
                        ("action", Value::str("cancel_oauth")),
                        ("request", Value::str(previous)),
                    ])));
                }
                let sequence = tx.int("session.oauth_sequence") + 1;
                let request = format!("oauth:{}:{sequence}", tx.text("session.id"));
                tx.set("session.oauth_sequence", Value::Int(sequence))?;
                tx.set("session.oauth_request", Value::str(&request))?;
                panel(
                    tx,
                    "authorize",
                    &format!("Authorize `{slot}`"),
                    "Requesting a device code",
                    vec![],
                    vec![],
                    vec![("credential.cancel".into(), "Cancel authorization".into())],
                )?;

                tx.fx(Effect::new("kernel.credential").with_value(Value::map([
                    ("id", Value::str(&request)),
                    (
                        "action",
                        Value::str(if account == "default" {
                            "oauth"
                        } else {
                            "oauth_account"
                        }),
                    ),
                    ("provider", Value::str(&slot)),
                    ("account", Value::str(&account)),
                ])));
                notice(
                    tx,
                    Level::Info,
                    format!("authorizing `{slot}` — a code is about to be shown"),
                )?;
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
            tx.set("panel.account", Value::str(&account))?;
        }
        "logout" => {
            let slot = argument(&args, "provider").unwrap_or("").to_string();
            let account = args
                .get("account")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if slot.is_empty() {
                notice(tx, Level::Warn, "say which provider to forget")?;
            } else {
                let effect = if account.is_empty() {
                    Value::map([
                        ("id", Value::str("logout")),
                        ("action", Value::str("delete")),
                        ("slot", Value::str(&slot)),
                    ])
                } else {
                    Value::map([
                        ("id", Value::str("logout")),
                        ("action", Value::str("delete_account")),
                        ("slot", Value::str(&slot)),
                        ("account", Value::str(&account)),
                    ])
                };
                tx.fx(Effect::new("kernel.credential").with_value(effect));
            }
        }
        "account" => {
            let slot = argument(&args, "provider").unwrap_or_default();
            let account = args
                .get("account")
                .and_then(Value::as_str)
                .unwrap_or_default();
            tx.fx(Effect::new("kernel.credential").with_value(Value::map([
                ("id", Value::str("account")),
                ("action", Value::str("select")),
                ("slot", Value::str(slot)),
                ("account", Value::str(account)),
            ])));
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
            notice(
                tx,
                Level::Warn,
                format!("`/{other}` is declared but not handled"),
            )?;
        }
    }
    Ok(())
}

/// Advance the current model's effort as a session-owned transition. The client
/// binding requests this operation; it does not mirror or mutate session state.
fn on_effort_cycle(tx: &mut Tx<'_>, _event: &Event) -> Result<(), Fault> {
    let discovered = tx
        .get("session.catalogue")
        .and_then(|catalogue| catalogue.get("models"))
        .cloned()
        .unwrap_or(Value::Null);
    let levels = crate::catalog::effort_levels(
        &discovered,
        &tx.text("session.provider"),
        &tx.text("session.model"),
    );
    if levels.is_empty() {
        return Err(Fault::new(
            "effort.unknown",
            "the selected model has no effort setting",
        ));
    }
    let current = tx.text("session.effort");
    let next = match levels.iter().position(|level| level == &current) {
        Some(index) => &levels[(index + 1) % levels.len()],
        None => &levels[0],
    };
    tx.set("session.effort", Value::str(next))?;
    notice(tx, Level::Info, format!("effort: {next}"))
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

/// Model choices are qualified so one picker can contain every provider the session knows.
/// Keep accepting an unqualified value for old drafts and direct callers; it belongs to the
/// active provider.
fn model_reference(requested: &str, active_provider: &str) -> (String, String) {
    requested
        .split_once('/')
        .map(|(provider, model)| (provider.to_string(), model.to_string()))
        .unwrap_or_else(|| (active_provider.to_string(), requested.to_string()))
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

fn refresh_conversations(tx: &mut Tx<'_>) {
    tx.fx(Effect::new("kernel.log.list").with("id", Value::str("discovery.conversations")));
}
fn refresh_credentials(tx: &mut Tx<'_>) -> Result<(), Fault> {
    tx.fx(Effect::new("kernel.credential").with_value(Value::map([
        ("id", Value::str("discovery.credentials")),
        ("action", Value::str("list")),
    ])));
    Ok(())
}
fn on_started(tx: &mut Tx<'_>, _event: &Event) -> Result<(), Fault> {
    refresh_conversations(tx);
    // Credentials are a daemon capability, not session configuration. Ask for the
    // redacted slot inventory once, then discover every provider that can use one.
    // This also means a freshly opened session sees an existing key without making
    // the person log in again.
    let provider = tx.text("session.provider");
    if provider != "scripted" {
        refresh_credentials(tx)?;
        refresh_usage(tx)?;
    }
    Ok(())
}
fn refresh_models(tx: &mut Tx<'_>) -> Result<(), Fault> {
    let provider = tx.text("session.provider");
    refresh_models_for(tx, &provider)
}
fn refresh_models_for(tx: &mut Tx<'_>, provider: &str) -> Result<(), Fault> {
    refresh_models_for_many(tx, &[provider])
}
fn refresh_models_for_many(tx: &mut Tx<'_>, providers: &[&str]) -> Result<(), Fault> {
    let mut sequence = tx.int("session.models_sequence");
    let mut requests = tx
        .get("session.models_requests")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .to_vec();
    let mut effects = Vec::with_capacity(providers.len());
    for provider in providers {
        sequence += 1;
        let id = format!("models.{sequence}");
        requests
            .retain(|request| request.get("provider").and_then(Value::as_str) != Some(*provider));
        requests.push(Value::map([
            ("id", Value::str(&id)),
            ("provider", Value::str(provider)),
        ]));
        effects.push((id, *provider));
    }
    tx.set("session.models_sequence", Value::Int(sequence))?;
    tx.set("session.models_requests", Value::list(requests))?;
    for (id, provider) in effects {
        tx.fx(Effect::new("kernel.models.discover")
            .with("id", Value::str(id))
            .with("provider", Value::str(provider)));
    }
    Ok(())
}
fn provider_for_credential_slot(slot: &str) -> Option<&'static str> {
    misa_kernel::presets::preset_for_slot(slot)
        .filter(|preset| !preset.models_path.is_empty())
        .map(|preset| preset.id)
}
fn refresh_models_for_slots_and_public(
    tx: &mut Tx<'_>,
    slots: &Value,
    public: &Value,
) -> Result<(), Fault> {
    let mut providers = slots
        .as_list()
        .unwrap_or(&[])
        .iter()
        .filter_map(|row| row.get("slot").and_then(Value::as_str))
        .filter_map(provider_for_credential_slot)
        .collect::<BTreeSet<_>>();
    providers.extend(
        public
            .as_list()
            .unwrap_or(&[])
            .iter()
            .filter_map(Value::as_str),
    );
    refresh_models_for_many(tx, &providers.into_iter().collect::<Vec<_>>())
}
fn refresh_usage(tx: &mut Tx<'_>) -> Result<(), Fault> {
    let sequence = tx.int("session.usage_sequence") + 1;
    let id = format!("usage.{sequence}");
    tx.set("session.usage_sequence", Value::Int(sequence))?;
    tx.set("session.usage_request", Value::str(&id))?;
    tx.fx(Effect::new("kernel.usage").with_value(Value::map([
        ("id", Value::str(id)),
        ("provider", Value::str(tx.text("session.provider"))),
    ])));
    Ok(())
}
fn on_usage(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    if tx.get("session.usage_request").is_none()
        || tx.text("session.usage_request") != fields::event_text(event, "id")
    {
        return Ok(());
    }
    tx.delete("session.usage_request")?;
    tx.set(
        "session.usage",
        event.data.get("facts").cloned().unwrap_or(Value::Null),
    )?;
    tx.set(
        "session.usage_provider",
        Value::str(fields::event_text(event, "provider")),
    )?;
    if tx.text("panel.id") == "usage" {
        open_usage(tx, Some(fields::event_value(event, "facts")))?;
    }
    if tx
        .get("session.usage_again")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        tx.delete("session.usage_again")?;
        refresh_usage(tx)?;
    }
    Ok(())
}
fn open_usage(tx: &mut Tx<'_>, incoming: Option<Value>) -> Result<(), Fault> {
    let attempts = tx.get("attempts").and_then(Value::as_list).unwrap_or(&[]);
    let report = Value::map([
        ("session", crate::usage::ledger(attempts)),
        (
            "last_request",
            attempts.last().cloned().unwrap_or(Value::Null),
        ),
        (
            "quota",
            incoming
                .or_else(|| tx.get("session.usage").cloned())
                .unwrap_or(Value::Null),
        ),
    ]);
    let rows = crate::usage::report_rows(&report)
        .into_iter()
        .map(|(label, role, value)| usage_row(&label, role, value))
        .collect::<Vec<_>>();
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
    Value::map([
        ("label", Value::str(label)),
        ("role", Value::str(role)),
        ("value", value),
    ])
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
        tx.delete("panel.account")?;
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
                        .map(|(label, value)| {
                            Value::map([("label", Value::str(label)), ("value", Value::str(value))])
                        })
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
                        .map(|(id, label)| {
                            Value::map([("id", Value::str(id)), ("label", Value::str(label))])
                        })
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
    "attachment.save",
    "panel.close",
    "panel.submit",
    "queue.edit",
    "queue.steer",
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
        .is_some_and(|actions| {
            actions
                .iter()
                .any(|declared| declared.as_str() == Some(action))
        })
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
                .and_then(|fields| {
                    fields
                        .iter()
                        .find(|field| field.get("id").and_then(Value::as_str) == Some("prompt"))
                })
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
        "queue.edit" | "queue.take" => {
            tx.dispatch(Event::new("intent/queue.edit"));
            Ok(())
        }
        "queue.steer" => on_queue_steer(tx, event),
        "queue.clear" => {
            tx.dispatch(Event::new("intent/queue.clear"));
            Ok(())
        }
        "attachment.save" => Err(Fault::handler(
            "Saving requires the validated requesting client",
        )),
        "credential.cancel" => {
            if let Some(request) = tx.get("session.oauth_request").and_then(Value::as_str) {
                tx.fx(Effect::new("kernel.credential").with_value(Value::map([
                    ("id", Value::str(request)),
                    ("action", Value::str("cancel_oauth")),
                    ("request", Value::str(request)),
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
                ("account", Value::str(tx.text("panel.account"))),
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
    tx.set("conversations", headers)?;
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
    if tx.text("session.oauth_request") != fields::event_text(event, "id") {
        return Ok(());
    }

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
        vec![
            ("credential.cancel".into(), "Cancel authorization".into()),
            ("panel.close".into(), "Dismiss".into()),
        ],
    )
}

fn on_appended(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    if fields::event_text(event, "conversation") != tx.text("session.conversation") {
        return Ok(());
    }
    let kind = fields::event_text(event, "kind");
    let data = fields::event_value(event, "data");
    let interrupted = tx
        .get("session.queue")
        .and_then(Value::as_list)
        .and_then(|queue| queue.first())
        .and_then(|head| head.get("interrupt"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    for (path, op) in
        crate::journal::patches(tx.db(), &kind, &data, fields::event_int(event, "seq"))
    {
        tx.patch(&path.to_string(), op)?;
    }
    if kind == crate::contribution::PATCH_KIND {
        if data.get("write").is_some() {
            if tx.get("session.plugin_write") != Some(&data) {
                return Ok(());
            }
            tx.set("session.plugin_write", Value::Null)?;
            crate::command_operations::settle(tx, &data, "succeeded")?;
        }
        let records = data
            .get("patches")
            .and_then(Value::as_list)
            .map(<[Value]>::to_vec)
            .unwrap_or_else(|| vec![data.clone()]);
        for record in records {
            if let Some((path, op)) = crate::contribution::recorded(&record) {
                if let Some(root) = Op::root_of(&path) {
                    if tx
                        .get("session.plugin_defaults")
                        .and_then(|defaults| defaults.get(root))
                        .is_some()
                    {
                        tx.patch(&path.to_string(), op)?;
                    }
                }
            }
        }
    }
    match kind.as_str() {
        "message" => match data.get("role").and_then(Value::as_str) {
            Some("user") if interrupted => {
                crate::operations::settle_prompt(tx, "cancelled", None)?;
                tx.set("session.status", Value::str("idle"))?;
                tx.dispatch(Event::new("queue/next"));
            }
            Some("user") if tx.text("session.status") == "recording" => {
                tx.set("session.status", Value::str("thinking"))?;
                tx.dispatch(Event::new("agent/step"));
            }
            Some("assistant") => {
                tx.delete("session.pending")?;
                tx.set("session.running_tools", Value::list([]))?;
                let has_calls = data
                    .get("calls")
                    .and_then(Value::as_list)
                    .is_some_and(|calls| !calls.is_empty());
                let result = match data.get("state").and_then(Value::as_str) {
                    Some("cancelled") => "cancelled",
                    Some("failed") => "failed",
                    _ if has_calls => "running",
                    _ => "succeeded",
                };
                crate::operations::settle_prompt(
                    tx,
                    result,
                    data.get("seq").and_then(Value::as_i64),
                )?;
                tx.set(
                    "session.status",
                    Value::str(if has_calls { "tools" } else { "idle" }),
                )?;
                tx.dispatch(Event::new(if has_calls {
                    "agent/tools"
                } else {
                    "queue/next"
                }));
            }
            _ => {}
        },
        "tool_result" => {
            tx.dispatch(Event::new("agent/tools"));
        }
        "reset" => {
            tx.set("session.status", Value::str("idle"))?;
            tx.dispatch(Event::new("queue/next"));
        }
        _ => {}
    }
    Ok(())
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
    let starting = tx.text("session.status") == "loading";
    if let Some(restored) = event.get("restored_operations") {
        tx.set(
            "prompt_operations",
            restored
                .get("prompts")
                .cloned()
                .unwrap_or_else(|| Value::list([])),
        )?;
        tx.set(
            "operations",
            restored
                .get("operations")
                .cloned()
                .unwrap_or_else(|| Value::list([])),
        )?;
        tx.set(
            "input_requests",
            restored
                .get("requests")
                .cloned()
                .unwrap_or_else(|| Value::list([])),
        )?;
        tx.delete("session.operation")?;
    }
    let conversation = fields::event_text(event, "conversation");
    let entries = fields::event_value(event, "entries");
    let commands = event
        .get("restored_operations")
        .and_then(|restored| restored.get("commands"))
        .cloned()
        .unwrap_or_else(|| crate::command_operations::restore(entries.as_list().unwrap_or(&[])));
    tx.set(crate::command_operations::ROOT, commands)?;
    let base = Value::map([("messages", Value::list([])), ("attempts", Value::list([]))]);
    let folded = crate::journal::fold(base, entries.as_list().unwrap_or(&[]));
    let messages = folded
        .get("messages")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .to_vec();
    let max_seq = messages
        .iter()
        .filter_map(|message| message.get("seq").and_then(Value::as_i64))
        .max()
        .unwrap_or(0);
    tx.set(
        "attempts",
        folded.get("attempts").cloned().unwrap_or(Value::list([])),
    )?;
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
    tx.set("session.turn", Value::Int(max_seq))?;
    if !starting {
        tx.delete("session.queue")?;
    }
    tx.delete("panel")?;
    if !starting {
        notice(
            tx,
            Level::Info,
            if previous == conversation {
                format!("reloaded {loaded} messages")
            } else {
                format!("resumed `{conversation}`: {loaded} messages")
            },
        )?;
    }
    if starting {
        tx.dispatch(Event::new("queue/next"));
    }
    if replayed > 0 {
        notice(
            tx,
            Level::Info,
            format!("replayed {replayed} patches a composition recorded"),
        )?;
    }
    if failed > 0 {
        notice(
            tx,
            Level::Warn,
            format!(
                "{failed} recorded patches could not be replayed, so some plugin state is missing"
            ),
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
    let defaults = tx
        .get("session.plugin_defaults")
        .and_then(Value::as_map)
        .cloned()
        .unwrap_or_default();
    for (root, value) in &defaults {
        database = misa_value::apply_one(
            &database,
            &misa_value::Path::parse(root).unwrap(),
            &Op::Set(value.clone()),
        )
        .expect("declared root defaults apply");
        touched.push(root.clone());
    }
    let mut replayed = 0;
    let mut failed = 0;
    let expanded = entries
        .iter()
        .flat_map(|entry| {
            if let Some(records) = entry
                .get("data")
                .and_then(|data| data.get("patches"))
                .and_then(Value::as_list)
            {
                records
                    .iter()
                    .map(|record| {
                        Value::map([
                            ("kind", Value::str(crate::contribution::PATCH_KIND)),
                            ("data", record.clone()),
                        ])
                    })
                    .collect::<Vec<_>>()
            } else {
                vec![entry.clone()]
            }
        })
        .collect::<Vec<_>>();
    for entry in &expanded {
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
        if !defaults.contains_key(root) {
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
    if oauth && tx.text("session.oauth_request") != id {
        return Ok(());
    }
    if oauth {
        tx.delete("session.oauth_request")?;
    }

    let ok = event.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let slot = event
        .get("slot")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let message = fields::event_text(event, "message");
    if !message.is_empty() {
        notice(tx, if ok { Level::Info } else { Level::Error }, message)?;
    }
    let slots = fields::event_value(event, "slots");
    let public_model_providers = fields::event_value(event, "model_providers");
    tx.set("session.credentials", slots.clone())?;
    if id == "status" {
        let provider = tx.text("session.status_provider");
        tx.delete("session.status_provider")?;
        open_status(tx, &provider)?;
        return Ok(());
    }
    if id == "discovery.credentials" {
        if ok {
            refresh_models_for_slots_and_public(tx, &slots, &public_model_providers)?;
        }
        return Ok(());
    }
    // The panel root holds the slot, so removing the root removes both. This is not a
    // formality: an authorization can finish long after somebody dismissed the panel, and a
    // delete of a *child* of a root that is not there fails the whole transaction — which is
    // how a report whose panel had already gone became a fault and no notice at all.
    if !oauth || tx.text("panel.id") == "authorize" {
        tx.delete("panel")?;
    }
    if ok && let Some(provider) = provider_for_credential_slot(&slot) {
        // Credentials change the provider's capability. Model discovery follows
        // that fact; it is not a separate user-facing command.
        refresh_models_for(tx, provider)?;
    }
    if ok && slot == tx.text("session.provider") {
        refresh_usage(tx)?;
    }
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
    let killed = event
        .get("killed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
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
    let seq = tx.int("session.turn") + 1;
    tx.set("session.turn", Value::Int(seq))?;
    tx.fx(log_effect(
        tx,
        "message",
        Value::map([
            ("seq", Value::Int(seq)),
            ("role", Value::str("system")),
            ("text", Value::str(&line)),
            ("state", Value::str("done")),
            ("attachments", Value::list([])),
        ]),
    ));
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
    if current_attachments(tx)
        .as_list()
        .unwrap_or(&[])
        .iter()
        .any(|attachment| attachment.get("hash").and_then(Value::as_str) == Some(hash.as_str()))
    {
        return Ok(());
    }
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
    let what = if media.is_empty() {
        "a file".to_string()
    } else {
        media
    };
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

    let index = message_count(tx);
    let seq = tx.int("session.turn") + 1;
    tx.set("session.turn", Value::Int(seq))?;
    let started_ms = now_ms();
    tx.set(
        "session.pending",
        Value::map([
            ("request", Value::str(&id)),
            ("seq", Value::Int(seq)),
            ("message", Value::Int(index as i64)),
            ("started_ms", Value::Int(started_ms)),
        ]),
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
        .with(
            "tools",
            tx.cofx()
                .config
                .get("installed_tools")
                .cloned()
                .unwrap_or_else(tools),
        ));
    Ok(())
}

/// What a service said its models are.
///
/// Stored rather than merged: the catalogue is a *fact about a service* with a time on it, and
/// the session's own catalog is enrichment on top. A person asking twice gets an answer twice,
/// because a model list changes while a daemon runs.
fn on_models(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let id = fields::event_text(event, "id");
    let Some(request) = tx
        .get("session.models_requests")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .iter()
        .find(|request| request.get("id").and_then(Value::as_str) == Some(id.as_str()))
    else {
        return Ok(());
    };
    let provider = request
        .get("provider")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let requests = tx
        .get("session.models_requests")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .iter()
        .filter(|request| request.get("id").and_then(Value::as_str) != Some(id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    tx.set("session.models_requests", Value::list(requests))?;
    let ok = event.get("ok").and_then(Value::as_bool).unwrap_or(false);
    if !ok {
        return Ok(());
    }
    let incoming = fields::event_value(event, "models");
    let previous = tx
        .get("session.catalogue")
        .and_then(|catalogue| catalogue.get("models"))
        .and_then(Value::as_list)
        .unwrap_or(&[]);
    let mut models = previous
        .iter()
        .filter(|model| model.get("provider").and_then(Value::as_str) != Some(provider.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    models.extend(incoming.as_list().unwrap_or(&[]).iter().map(|model| {
        let mut row = model.as_map().cloned().unwrap_or_default();
        row.insert("provider".into(), Value::str(&provider));
        Value::Map(std::sync::Arc::new(row))
    }));
    tx.set(
        "session.catalogue",
        Value::map([
            ("at", Value::Int(now_ms())),
            ("models", Value::list(models)),
        ]),
    )?;
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
        notice(
            tx,
            Level::Warn,
            "the summary did not arrive; nothing was replaced",
        )?;
        return Ok(());
    }
    let replaced = tx
        .get("messages")
        .and_then(Value::as_list)
        .map(<[Value]>::len)
        .unwrap_or(0);
    let seq = tx.int("session.turn") + 1;
    tx.set("session.turn", Value::Int(seq))?;
    let message = Value::map([
        ("seq", Value::Int(seq)),
        ("role", Value::str("assistant")),
        (
            "text",
            Value::str(format!("[summary of {replaced} messages]\n\n{summary}")),
        ),
        ("state", Value::str("done")),
        ("calls", Value::list([])),
        ("attachments", Value::list([])),
    ]);
    tx.set("session.status", Value::str("recording"))?;
    tx.fx(log_effect(
        tx,
        "reset",
        Value::map([
            ("reason", Value::str("compacted")),
            ("replaced", Value::Int(replaced as i64)),
            ("message", message),
        ]),
    ));
    notice(tx, Level::Info, format!("compacted {replaced} messages"))?;
    tx.dispatch(Event::new("queue/next"));
    Ok(())
}

fn on_finished(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let id = fields::event_text(event, "id");
    if tx
        .get("session.pending")
        .and_then(|pending| pending.get("side"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        return on_summary(tx, event);
    }
    let Some((_, seq)) = active_message(tx, &id) else {
        return Ok(());
    };
    let ok = event.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let text = fields::event_text(event, "text");
    let error = fields::event_text(event, "error");
    let thinking = fields::event_text(event, "thinking");
    let input_tokens = fields::event_int(event, "input_tokens");
    let output_tokens = fields::event_int(event, "output_tokens");
    let provider_state = fields::event_value(event, "provider_state");
    let model = tx.text("session.model");
    let provider = tx.text("session.provider");
    let discovered = tx
        .get("session.catalogue")
        .and_then(|catalogue| catalogue.get("models"))
        .cloned()
        .unwrap_or(Value::Null);
    let finished_ms = now_ms();
    let started_ms = tx
        .get("session.pending")
        .and_then(|pending| pending.get("started_ms"))
        .and_then(Value::as_i64);
    let elapsed_ms = started_ms.map(|started| finished_ms.saturating_sub(started));
    let cost = crate::catalog::discovered_cost_micros_at(
        &discovered,
        &provider,
        &model,
        input_tokens,
        output_tokens,
        0,
        0,
        started_ms,
    );
    tx.fx(Effect::new("kernel.attempt.settled")
        .with("id", Value::str(&id))
        .with("status", Value::str(if ok { "ok" } else { "error" }))
        .with("input_tokens", Value::Int(input_tokens))
        .with("output_tokens", Value::Int(output_tokens))
        .with("cost_micros", Value::Int(cost)));
    let attempt = Value::map([
        (
            "operation",
            tx.get("session.operation").cloned().unwrap_or(Value::Null),
        ),
        (
            "parent",
            tx.cofx()
                .config
                .get("parent_attempt")
                .cloned()
                .unwrap_or(Value::Null),
        ),
        (
            "name",
            Value::str(format!("{}:{id}", tx.text("session.incarnation"))),
        ),
        ("kind", Value::str("turn")),
        ("provider", Value::str(&provider)),
        ("model", Value::str(&model)),
        ("status", Value::str(if ok { "ok" } else { "error" })),
        ("input_tokens", Value::Int(input_tokens)),
        ("output_tokens", Value::Int(output_tokens)),
        ("cost_micros", Value::Int(cost)),
        ("finished_ms", Value::Int(finished_ms)),
    ]);
    let attempt = {
        let mut attempt = attempt.as_map().cloned().unwrap_or_default();
        if let Some(started_ms) = started_ms {
            attempt.insert("started_ms".into(), Value::Int(started_ms));
        }
        if let Some(elapsed_ms) = elapsed_ms {
            attempt.insert("elapsed_ms".into(), Value::Int(elapsed_ms));
        }
        Value::Map(std::sync::Arc::new(attempt))
    };
    let calls = if ok {
        normalise_calls(&fields::event_value(event, "tool_calls"))
    } else {
        vec![]
    };
    let mut message = Value::map([
        ("seq", Value::Int(seq)),
        ("role", Value::str("assistant")),
        ("text", Value::str(text)),
        ("error", Value::str(error)),
        (
            "operation",
            tx.get("session.operation").cloned().unwrap_or(Value::Null),
        ),
        ("state", Value::str(if ok { "done" } else { "failed" })),
        ("thinking", Value::str(thinking)),
        ("calls", Value::list(calls)),
        ("attachments", Value::list([])),
        ("attempt", attempt),
    ]);
    if provider_state
        .as_list()
        .is_some_and(|states| !states.is_empty())
    {
        let mut fields = message.as_map().cloned().unwrap_or_default();
        fields.insert("provider_state".into(), provider_state);
        message = Value::Map(std::sync::Arc::new(fields));
    }
    tx.fx(log_effect(tx, "message", message));
    tx.set("session.status", Value::str("recording"))?;
    Ok(())
}

fn priority_interrupt(tx: &Tx<'_>) -> bool {
    tx.get("session.queue")
        .and_then(Value::as_list)
        .and_then(|queue| queue.first())
        .and_then(|head| head.get("interrupt"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn on_tools(tx: &mut Tx<'_>, _event: &Event) -> Result<(), Fault> {
    if tx.text("session.status") != "tools" {
        return Ok(());
    }
    let Some(index) = last_assistant(tx.db()) else {
        return Ok(());
    };
    let calls = calls_at(tx.db(), index);
    let mut running = tx
        .get("session.running_tools")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .to_vec();
    for call in &calls {
        if call.get("status").and_then(Value::as_str) != Some("pending") {
            continue;
        }
        let call_id = call
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        if running.iter().any(|id| id.as_str() == Some(&call_id)) {
            continue;
        }
        running.push(Value::str(&call_id));
        if priority_interrupt(tx) {
            // Reserve this call until its journal acknowledgment, just like a running
            // effect, so another tools event cannot append the cancellation twice.
            tx.fx(log_effect(
                tx,
                "tool_result",
                Value::map([
                    ("call", Value::str(&call_id)),
                    ("ok", Value::Bool(false)),
                    ("text", Value::str("cancelled before starting")),
                ]),
            ));
            continue;
        }
        tx.fx(Effect::new("kernel.tool.run")
            .with(
                "operation",
                tx.get("session.operation").cloned().unwrap_or(Value::Null),
            )
            .with("id", Value::str(&call_id))
            .with("call_id", Value::str(&call_id))
            .with(
                "name",
                Value::str(call.get("name").and_then(Value::as_str).unwrap_or_default()),
            )
            .with("args", call.get("args").cloned().unwrap_or(Value::Null)));
    }
    tx.set("session.running_tools", Value::list(running))?;
    if calls.iter().all(|call| {
        matches!(
            call.get("status").and_then(Value::as_str),
            Some("ok" | "error")
        )
    }) {
        let interrupted = priority_interrupt(tx);
        if interrupted {
            crate::operations::settle_prompt(tx, "cancelled", None)?;
        }
        tx.set(
            "session.status",
            Value::str(if interrupted { "idle" } else { "thinking" }),
        )?;
        tx.dispatch(Event::new(if interrupted {
            "queue/next"
        } else {
            "agent/step"
        }));
    }
    Ok(())
}

fn on_tool_finished(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    if event
        .get("operation")
        .and_then(Value::as_str)
        .is_some_and(|operation| {
            tx.get("session.operation").and_then(Value::as_str) != Some(operation)
        })
    {
        return Ok(());
    }
    let call_id = fields::event_text(event, "call_id");
    let ok = event.get("ok").and_then(Value::as_bool).unwrap_or(false);
    let text = fields::event_text(event, "text");
    let Some(index) = last_assistant(tx.db()) else {
        return Ok(());
    };
    let calls = calls_at(tx.db(), index);
    let Some(_position) = calls
        .iter()
        .position(|call| call.get("id").and_then(Value::as_str) == Some(call_id.as_str()))
    else {
        return Ok(());
    };

    tx.fx(log_effect(
        tx,
        "tool_result",
        Value::map([
            ("call", Value::str(&call_id)),
            ("ok", Value::Bool(ok)),
            ("text", Value::str(&text)),
        ]),
    ));

    Ok(())
}

fn on_kernel_failed(tx: &mut Tx<'_>, event: &Event) -> Result<(), Fault> {
    let id = fields::event_text(event, "id");
    let pending = tx
        .get("session.pending")
        .and_then(|value| value.get("request"))
        .and_then(Value::as_str);
    let running_tool = tx
        .get("session.running_tools")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .iter()
        .any(|value| value.as_str() == Some(&id));
    if id != tx.text("session.conversation") && pending != Some(id.as_str()) && !running_tool {
        // Discovery and other unrelated requests cannot terminate a prompt.
        return notice(tx, Level::Error, fields::event_text(event, "message"));
    }
    crate::operations::settle_prompt(tx, "failed", None)?;
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
    tx.get("messages")
        .and_then(Value::as_list)
        .map(<[Value]>::len)
        .unwrap_or(0)
}

fn messages(db: &Value) -> Vec<Value> {
    db.get("messages")
        .and_then(Value::as_list)
        .map(<[Value]>::to_vec)
        .unwrap_or_default()
}

/// The index of the message a request is streaming into, and its sequence.
fn active_message(tx: &Tx<'_>, id: &str) -> Option<(usize, i64)> {
    let pending = tx.get("session.pending")?;
    if pending.get("request").and_then(Value::as_str) != Some(id) {
        return None;
    }
    let index = pending.get("message").and_then(Value::as_i64)? as usize;
    let seq = pending.get("seq").and_then(Value::as_i64)?;
    Some((index, seq))
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
        let role = message
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match role {
            "user" => out.push(Value::map([
                ("role", Value::str("user")),
                ("text", message.get("text").cloned().unwrap_or(Value::Null)),
                (
                    "attachments",
                    message
                        .get("attachments")
                        .cloned()
                        .unwrap_or_else(|| Value::list([])),
                ),
            ])),
            // A note the session wrote to the model: a background command finishing, and
            // whatever else the loop learns on its own. Both providers read a `system` message.
            "system" => out.push(Value::map([
                ("role", Value::str("system")),
                ("text", message.get("text").cloned().unwrap_or(Value::Null)),
            ])),
            "assistant" => {
                let mut assistant = Value::map([
                    ("role", Value::str("assistant")),
                    ("text", message.get("text").cloned().unwrap_or(Value::Null)),
                    // A service that wants its own reasoning back on a tool-call turn gets it;
                    // every other adapter ignores the field, and the message carries it either
                    // way because it is part of what the model said.
                    (
                        "thinking",
                        message
                            .get("thinking")
                            .cloned()
                            .unwrap_or_else(|| Value::str("")),
                    ),
                    (
                        "tool_calls",
                        message
                            .get("calls")
                            .cloned()
                            .unwrap_or_else(|| Value::list([])),
                    ),
                ]);
                if let Some(state) = message.get("provider_state") {
                    let mut fields = assistant.as_map().cloned().unwrap_or_default();
                    fields.insert("provider_state".into(), state.clone());
                    assistant = Value::Map(std::sync::Arc::new(fields));
                }
                out.push(assistant);
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
    // Notices are an ephemeral recent-diagnostics ring, not transcript history.
    let mut text = text.into();
    if text.len() > 8192 {
        let mut end = 8192;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        text.truncate(end);
    }
    let recent = tx.get("notices").and_then(Value::as_list).unwrap_or(&[]);
    if recent.len() >= 64 {
        tx.set(
            "notices",
            Value::list(recent[recent.len() - 63..].iter().cloned()),
        )?;
    }
    let id = next_id(tx, "session.notice_seq")?;
    tx.push(
        "notices",
        Value::map([
            ("id", Value::Int(id)),
            ("level", Value::str(level.as_str())),
            ("text", Value::str(&text)),
        ]),
    )?;
    tx.fx(notice_effect(level, text));
    Ok(())
}

fn next_id(tx: &mut Tx<'_>, path: &str) -> Result<i64, Fault> {
    let previous = tx
        .patches()
        .iter()
        .rev()
        .find_map(|(at, op)| {
            if at.to_string() == path {
                if let Op::Set(value) = op {
                    return value.as_i64();
                }
            }
            None
        })
        .unwrap_or_else(|| tx.int(path));
    tx.set(path, Value::Int(previous + 1))?;
    Ok(previous + 1)
}

fn notice_effect(level: Level, text: impl Into<String>) -> Effect {
    Effect::new("wire.event").with(
        "event",
        session_event(&SessionEvent::Notice {
            level,
            text: text.into(),
        }),
    )
}

/// A session event, on its way into an effect's data.
fn session_event(event: &impl serde::Serialize) -> Value {
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
            hint: None,
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
}

/// A one-line constructor, so the composer reads as what it is.
struct FieldSummary;

impl FieldSummary {
    fn fields(fields: Vec<Field>) -> misa_proto::view::Kind {
        misa_proto::view::Kind::Fields { fields }
    }
}

fn open_status(tx: &mut Tx<'_>, provider: &str) -> Result<(), Fault> {
    status_panel(tx, provider)
}

fn status_panel(tx: &mut Tx<'_>, provider: &str) -> Result<(), Fault> {
    let accounts = tx
        .get("session.credentials")
        .and_then(Value::as_list)
        .unwrap_or(&[])
        .iter()
        .filter(|row| row.get("slot").and_then(Value::as_str) == Some(provider))
        .map(|row| {
            let account = row
                .get("account")
                .and_then(Value::as_str)
                .unwrap_or("default");
            let active = row.get("active").and_then(Value::as_bool).unwrap_or(false);
            (
                "account",
                format!("{account}{}", if active { " (active)" } else { "" }),
            )
        })
        .collect::<Vec<_>>();
    let rows = if accounts.is_empty() {
        vec![("state", "logged out".into())]
    } else {
        accounts
    };
    panel(
        tx,
        "status",
        &format!("Status for `{provider}`"),
        "",
        rows,
        Vec::new(),
        vec![("panel.close".into(), "Close".into())],
    )
}
