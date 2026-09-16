//! Installed daemon lifecycle commands and exact created-owner reconciliation.
//! Frontends own scheduling, forms and navigation; outcomes retain uncertainty.
use crate::{daemons::Daemon, interface::Interface};
use misa_proto::{
    Fault,
    directory::Entry,
    invocation::Outcome,
    observation::{Scope, ScopeId},
};
use misa_value::Value;
use std::time::Duration;
pub async fn invoke(daemon: &Daemon, command: &str, input: Value) -> Result<Outcome, Fault> {
    let interface = Interface::load(&daemon.client, daemon.client.welcome().scope).await?;
    let definition = interface
        .commands
        .get(command)
        .ok_or_else(|| Fault::unsupported("Daemon command is unavailable"))?;
    Ok(daemon
        .client
        .invoke(
            interface.scope.clone(),
            definition.clone(),
            input,
            Duration::from_secs(30),
        )
        .await?
        .outcome)
}
/// Call only for a validated Completed result from session create/resume. A
/// reused logical ID cannot attach a later unrelated owner incarnation.
pub async fn opened(daemon: &Daemon, value: &Value) -> Result<Entry, Fault> {
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| Fault::query("Created session result lacks ID"))?;
    let incarnation = value
        .get("incarnation")
        .and_then(Value::as_str)
        .ok_or_else(|| Fault::query("Created session result lacks incarnation"))?;
    let mut changes = daemon.watch();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(snapshot) = daemon.sessions()
                && matches!(snapshot.status, misa_protocol::observation::Status::Current)
                && let Some(entry) = snapshot
                    .sessions
                    .into_iter()
                    .find(|entry| entry.id == id && entry.incarnation == incarnation)
            {
                return Ok(entry);
            }
            changes.changed().await.map_err(|_| {
                Fault::new("closed", "Daemon directory closed after session opened")
            })?;
        }
    })
    .await
    .map_err(|_| {
        Fault::new(
            "reconcile_timeout",
            format!(
                "Session {id} opened, but its directory update timed out; reconcile before retrying"
            ),
        )
    })?
}
pub fn close_input(scope: &Scope) -> Result<Value, Fault> {
    scope.validate()?;
    let ScopeId::Session { id } = &scope.id else {
        return Err(Fault::query("Only session owners can be closed"));
    };
    Ok(Value::map([
        ("id", Value::str(id)),
        ("incarnation", Value::str(&scope.incarnation)),
    ]))
}
/// Finite archive lookup also works when the daemon has no live sessions.
pub async fn conversations(
    daemon: &Daemon,
    prefix: &str,
    limit: u32,
) -> Result<misa_proto::preparation::Candidates, Fault> {
    let interface = Interface::load(&daemon.client, daemon.client.welcome().scope).await?;
    let member = interface.query(
        misa_proto::preparation::SEARCH,
        vec![
            Value::str("conversations"),
            Value::str(prefix),
            Value::Int(i64::from(limit)),
        ],
    )?;
    let result = daemon
        .client
        .read(
            misa_proto::observation::Selection {
                scope: interface.scope,
                members: std::collections::BTreeMap::from([("conversations".into(), member)]),
            },
            Duration::from_secs(10),
        )
        .await?;
    crate::interface::decode(crate::interface::data(&result, "conversations")?)
}
