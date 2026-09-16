//! Directed user actions; accepted operations keep their domain-owned lifetime.
use crate::files::Files;
use misa_client::{
    daemons::Daemon,
    interaction::{Interaction, Prepared},
    interface,
};
use misa_kit::intent::Intent;
use misa_proto::{invocation::Outcome, observation::Selection, view::BlobRef};
use misa_value::Value;
use serde_json::{Value as Json, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};

pub fn prepare(interaction: &Interaction, intent: Intent) -> Result<Prepared, String> {
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
        _ => return Err("This action has no installed command".into()),
    }
    .map_err(|fault| fault.message)
}
fn report(title: &str, value: Value) -> Json {
    json!({"kind":"report","title":title,"view":misa_client::request::report(title,&value)})
}
pub async fn execute(
    daemon: &Daemon,
    interaction: &Interaction,
    prepared: Prepared,
) -> Result<Json, String> {
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
                    Duration::from_secs(15),
                )
                .await
                .map_err(|fault| fault.message)?;
            Ok(
                json!({"kind":"report","title":title,"view":interface::report(&result,"result",&title).map_err(|fault|fault.message)?}),
            )
        }
        Prepared::Invoke { command, input } => {
            let title = command.id.clone();
            let result = daemon
                .client
                .invoke(
                    interaction.interface.scope.clone(),
                    command,
                    input,
                    Duration::from_secs(20),
                )
                .await
                .map_err(|fault| fault.message)?;
            match result.outcome {
                Outcome::Completed { value } => Ok(if value == Value::Null {
                    json!({"kind":"notice","level":"info","text":"Done"})
                } else {
                    report(&title, value)
                }),
                Outcome::Rejected { fault } | Outcome::Indeterminate { fault } => {
                    Err(fault.message)
                }
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
                    let (level, text) = match watch.wait().await.outcome {
                        misa_client::operation::Terminal::Finished { state, .. } => {
                            ("info", format!("Operation {state}"))
                        }
                        misa_client::operation::Terminal::Expired => (
                            "error",
                            "Operation result expired; completion is unknown".into(),
                        ),
                        misa_client::operation::Terminal::Fault(fault) => ("error", fault.message),
                    };
                    Ok(json!({"kind":"notice","level":level,"text":text}))
                }
            }
        }
    }
}
pub async fn fetch(
    daemon: &Daemon,
    files: Arc<Files>,
    hash: String,
    name: Option<String>,
) -> Result<Json, String> {
    let lookup = files.clone();
    let lookup_hash = hash.clone();
    let cached = tokio::task::spawn_blocking(move || lookup.cached(&lookup_hash))
        .await
        .map_err(|error| error.to_string())??;
    let path = match cached {
        Some(path) => path,
        None => {
            let blob = daemon
                .blobs
                .get(&hash)
                .await?
                .ok_or("Blob no longer available")?;
            let cache_hash = hash.clone();
            tokio::task::spawn_blocking(move || files.cache(&cache_hash, &blob.bytes))
                .await
                .map_err(|error| error.to_string())??
        }
    };
    Ok(
        json!({"kind":if name.is_some(){"download"}else{"blob"},"hash":hash,"path":path,"name":name}),
    )
}
pub async fn save(
    daemon: &Daemon,
    interaction: &Interaction,
    files: Arc<Files>,
    node: String,
) -> Result<Json, String> {
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
            Duration::from_secs(15),
        )
        .await
        .map_err(|fault| fault.message)?;
    let value = match reply.outcome {
        Outcome::Completed { value } => value,
        Outcome::Rejected { fault } | Outcome::Indeterminate { fault } => return Err(fault.message),
        _ => return Err("Attachment resolution did not complete".into()),
    };
    let reference: BlobRef = interface::decode(&value).map_err(|fault| fault.message)?;
    let blob = daemon.blobs.download(&reference).await?;
    let hash = reference.hash.clone();
    let path = tokio::task::spawn_blocking(move || files.cache(&reference.hash, &blob.bytes))
        .await
        .map_err(|error| error.to_string())??;
    Ok(json!({"kind":"download","hash":hash,"path":path,"name":"attachment"}))
}
