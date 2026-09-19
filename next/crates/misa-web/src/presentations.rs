//! Local choices and bounded replacement of one browser instance's observation.
use crate::Remote;
use axum::{
    extract::{Form, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use misa_client::{
    composition::{Choice, Preferences},
    driver::Observation,
};
use misa_proto::observation::Selection;
use std::{collections::HashMap, sync::Arc, time::Duration};

pub const CAPABILITIES: &[&str] = &["semantic.meter@1"];
pub(crate) fn capabilities() -> Vec<String> {
    CAPABILITIES.iter().map(|value| (*value).into()).collect()
}
pub(crate) fn initial(encoded: Option<&str>) -> Result<Preferences, String> {
    let Some(encoded) = encoded else {
        return Ok(Preferences::default());
    };
    if encoded.len() > 32768 {
        return Err("Presentation preferences exceed limit".into());
    }
    let values: std::collections::BTreeMap<String, String> =
        serde_json::from_str(encoded).map_err(|_| "Invalid presentation preferences")?;
    if values.len() > 128 {
        return Err("Too many presentation preferences".into());
    }
    let mut result = Preferences::default();
    for (id, value) in values {
        let choice = match value.as_str() {
            "auto" => Choice::Auto,
            "hide" if id != "conversation" => Choice::Hidden,
            value if value.starts_with("variant:") && value.len() > 8 => {
                Choice::Variant(value[8..].into())
            }
            _ => return Err("Invalid presentation preference".into()),
        };
        result.0.insert(id, choice);
    }
    Ok(result)
}
pub(crate) struct Change {
    pub observation: Observation,
    pub members: Vec<String>,
    pub preferences: Preferences,
    pub reply: tokio::sync::oneshot::Sender<Result<(), String>>,
}

pub(crate) fn controls(remote: &Remote) -> String {
    let preferences = remote.preferences.lock().unwrap();
    let unavailable = preferences
        .reconcile(
            &remote.interaction.interface.presentations,
            &capabilities(),
            &["conversation", "status"],
        )
        .unavailable;
    let mut html = String::from(
        "<p><a href=\"./commands\">Commands</a> · <a href=\"./requests\">Pending requests</a> · <a href=\"./actions\">Actions</a></p><details><summary>Presentations</summary>",
    );
    for presentation in &remote.interaction.interface.presentations {
        let default = if ["conversation", "status"].contains(&presentation.id.as_str()) {
            Choice::Auto
        } else {
            Choice::Hidden
        };
        let current = preferences.0.get(&presentation.id).unwrap_or(&default);
        if let Some(fault) = unavailable.get(&presentation.id) {
            html.push_str(&format!(
                "<p role=\"status\">{}: {}</p>",
                crate::escape(&presentation.title),
                crate::escape(&fault.message)
            ));
        }
        html.push_str(&format!("<form data-presentation-choice method=\"post\" action=\"./presentations\"><input type=\"hidden\" name=\"id\" value=\"{}\"><label>{} <select name=\"choice\">", crate::escape(&presentation.id), crate::escape(&presentation.title)));
        let mut options = vec![("auto".to_owned(), "Automatic".to_owned(), Choice::Auto)];
        if presentation.id != "conversation" {
            options.push(("hide".into(), "Hidden".into(), Choice::Hidden));
        }
        for variant in &presentation.variants {
            if variant
                .requirements
                .iter()
                .all(|requirement| CAPABILITIES.contains(&requirement.as_str()))
            {
                options.push((
                    format!("variant:{}", variant.id),
                    variant.id.clone(),
                    Choice::Variant(variant.id.clone()),
                ));
            }
        }
        if let Choice::Variant(id) = current {
            if !options.iter().any(|(_, _, choice)| choice == current) {
                options.push((
                    format!("variant:{id}"),
                    format!("{id} (unavailable)"),
                    current.clone(),
                ));
            }
        }
        for (value, label, choice) in options {
            html.push_str(&format!(
                "<option value=\"{}\"{}>{}</option>",
                crate::escape(&value),
                if current == &choice { " selected" } else { "" },
                crate::escape(&label)
            ));
        }
        html.push_str("</select></label><button>Apply</button></form>");
    }
    html.push_str("</details>");
    html
}

pub(crate) async fn configure(
    State(remote): State<Arc<Remote>>,
    headers: HeaderMap,
    Form(fields): Form<HashMap<String, String>>,
) -> Response {
    let Ok(_permit) = remote.presentation_gate.try_lock() else {
        return (
            StatusCode::CONFLICT,
            "Another presentation change is pending",
        )
            .into_response();
    };
    let result = async {
        let id = fields.get("id").ok_or("Missing presentation")?;
        let choice = match fields.get("choice").map(String::as_str) {
            Some("auto") => Choice::Auto,
            Some("hide") if id != "conversation" => Choice::Hidden,
            Some(value) if value.starts_with("variant:") => Choice::Variant(value[8..].into()),
            _ => return Err("Invalid presentation choice".to_string()),
        };
        let mut preferences = remote.preferences.lock().unwrap().clone();
        let catalog = &remote.interaction.interface.presentations;
        preferences
            .set(catalog, &capabilities(), id, choice)
            .map_err(|fault| fault.message)?;
        let members = preferences
            .reconcile(catalog, &capabilities(), &["conversation", "status"])
            .members;
        let ids = members.keys().cloned().collect();
        let mut observation = remote
            .daemon
            .client
            .observe(
                Selection {
                    scope: remote.interaction.interface.scope.clone(),
                    members,
                },
                None,
            )
            .await
            .map_err(|fault| fault.message)?;
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                use misa_protocol::observation::Status;
                match observation.inspect(|replica, _| replica.status().clone()) {
                    Some(Status::Current) => return Ok(()),
                    Some(Status::Closed(fault) | Status::Stale(fault)) => return Err(fault.message),
                    _ => {}
                }
                observation
                    .changed()
                    .await
                    .map_err(|_| "Presentation observation closed".to_string())?;
            }
        })
        .await
        .map_err(|_| "Presentation did not become current".to_string())??;
        let (reply, done) = tokio::sync::oneshot::channel();
        remote
            .presentation_changes
            .try_send(Change {
                observation,
                members: ids,
                preferences,
                reply,
            })
            .map_err(|_| "Presentation instance is unavailable".to_string())?;
        done.await
            .map_err(|_| "Presentation instance closed".to_string())?
    }
    .await;
    if headers
        .get("accept")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("application/json"))
    {
        return match result {
            Ok(()) => axum::Json(serde_json::json!({"ok":true})).into_response(),
            Err(error) => (
                StatusCode::BAD_REQUEST,
                axum::Json(serde_json::json!({"ok":false,"error":error})),
            )
                .into_response(),
        };
    }
    match result {
        Ok(()) => Redirect::to("./").into_response(),
        Err(message) => (StatusCode::BAD_REQUEST, message).into_response(),
    }
}
