//! Command preparation and completion are finite reads. Opening a model/provider
//! chooser is local interaction, not a command that opens a shared panel.
use crate::{Remote, escape};
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use misa_client::{
    form::Form,
    interface::{data, decode},
};
use misa_proto::{
    Fault,
    observation::Selection,
    preparation::{Candidates, Target},
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::Duration,
};

pub(crate) async fn prepare(
    remote: &Remote,
    shortcut: &str,
    drafts: &BTreeMap<String, String>,
) -> Result<String, Fault> {
    let shortcut = remote
        .interaction
        .shortcuts
        .iter()
        .find(|entry| entry.id == shortcut)
        .ok_or_else(|| Fault::unsupported("Shortcut is unavailable"))?;
    match &shortcut.target {
        Target::Read { member } => {
            let result = remote
                .daemon
                .client
                .read(
                    Selection {
                        scope: remote.interaction.interface.scope.clone(),
                        members: BTreeMap::from([("report".into(), member.clone())]),
                    },
                    Duration::from_secs(20),
                )
                .await?;
            crate::read_report(&shortcut.label, &result, "report")
        }
        Target::Command { command } => {
            let model = Form::command(&remote.interaction.interface, command)?;
            let sources = shortcut
                .args
                .iter()
                .filter_map(|arg| {
                    arg.source
                        .as_ref()
                        .map(|source| (arg.name.clone(), source.clone()))
                })
                .collect::<BTreeMap<_, _>>();
            let members = sources
                .iter()
                .map(|(field, source)| {
                    remote
                        .interaction
                        .complete(source, "", misa_proto::preparation::DEFAULT_CANDIDATES)
                        .map(|member| (field.clone(), member))
                })
                .collect::<Result<BTreeMap<_, _>, _>>()?;
            let mut choices = BTreeMap::new();
            if !members.is_empty() {
                let values = remote
                    .daemon
                    .client
                    .read(
                        Selection {
                            scope: remote.interaction.interface.scope.clone(),
                            members,
                        },
                        Duration::from_secs(20),
                    )
                    .await?;
                for (field, source) in sources {
                    choices.insert(
                        field.clone(),
                        crate::actions::Choices {
                            source,
                            candidates: decode(data(&values, &field)?)?,
                        },
                    );
                }
            }
            Ok(crate::actions::markup_for(&model, true, &choices, drafts))
        }
    }
}

pub(crate) async fn list(State(remote): State<Arc<Remote>>) -> Response {
    let mut html = String::new();
    for shortcut in &remote.interaction.shortcuts {
        html.push_str(&format!("<form method=\"get\" action=\"./command\"><input type=\"hidden\" name=\"shortcut\" value=\"{}\"><button>{}</button> {}</form>", escape(&shortcut.id), escape(&shortcut.label), escape(&shortcut.description)));
    }
    html.push_str("<details><summary>More installed commands</summary>");
    for id in remote.interaction.interface.commands.keys() {
        if remote.interaction.shortcuts.iter().any(
            |shortcut| matches!(&shortcut.target, Target::Command { command } if command == id),
        ) {
            continue;
        }
        match Form::command(&remote.interaction.interface, id) {
            Ok(_) => html.push_str(&format!("<form method=\"get\" action=\"./command\"><input type=\"hidden\" name=\"command\" value=\"{}\"><button>{}</button></form>", escape(id), escape(id))),
            Err(fault) => html.push_str(&format!("<p><code>{}</code> · {}</p>", escape(id), escape(&fault.message))),
        }
    }
    html.push_str("</details>");
    crate::requests::page(StatusCode::OK, "Commands", html)
}
pub(crate) async fn open(
    State(remote): State<Arc<Remote>>,
    Query(fields): Query<HashMap<String, String>>,
) -> Response {
    let prepared = if let Some(command) = fields.get("command") {
        Form::command(&remote.interaction.interface, command).map(|model| {
            crate::actions::markup_for(&model, true, &BTreeMap::new(), &BTreeMap::new())
        })
    } else {
        prepare(
            &remote,
            fields.get("shortcut").map(String::as_str).unwrap_or(""),
            &BTreeMap::new(),
        )
        .await
    };
    match prepared {
        Ok(html) => crate::requests::page(
            StatusCode::OK,
            "Prepare command",
            format!("{html}<script src=\"./commands.js\"></script>"),
        ),
        Err(fault) => crate::requests::page(
            StatusCode::BAD_REQUEST,
            "Command unavailable",
            escape(&fault.message),
        ),
    }
}
pub(crate) async fn candidates(
    State(remote): State<Arc<Remote>>,
    Query(fields): Query<HashMap<String, String>>,
) -> Response {
    let source = fields.get("source").map(String::as_str).unwrap_or("");
    let prefix = fields.get("q").map(String::as_str).unwrap_or("");
    if prefix.len() > 2048 {
        return (StatusCode::BAD_REQUEST, "Search is too long").into_response();
    }
    let result = async {
        let member = remote.interaction.complete(
            source,
            prefix,
            misa_proto::preparation::DEFAULT_CANDIDATES,
        )?;
        let values = remote
            .daemon
            .client
            .read(
                Selection {
                    scope: remote.interaction.interface.scope.clone(),
                    members: BTreeMap::from([("choices".into(), member)]),
                },
                Duration::from_secs(20),
            )
            .await?;
        decode::<Candidates>(data(&values, "choices")?)
    }
    .await;
    match result {
        Ok(candidates) => axum::Json(candidates).into_response(),
        Err(fault) => (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error":fault.message})),
        )
            .into_response(),
    }
}
