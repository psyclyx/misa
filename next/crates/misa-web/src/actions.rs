//! Local preparation for installed command bindings. Widgets follow schemas;
//! plugins do not require a domain-specific browser implementation.
use std::{collections::{BTreeMap, HashMap}, sync::Arc};
use axum::{extract::{State, Form}, http::{StatusCode, HeaderMap}, response::{IntoResponse, Redirect, Response}};
use misa_client::{form::Form as ActionForm, interaction::Prepared};
use misa_proto::{invocation::Outcome, schema::{Literal, Schema}};
use crate::{Remote, escape};

pub(crate) fn markup(model: &ActionForm) -> String {
    markup_for(model, false, &BTreeMap::new(), &BTreeMap::new())
}

pub(crate) struct Choices {
    pub source: String,
    pub candidates: misa_proto::preparation::Candidates,
}
pub(crate) fn markup_for(model: &ActionForm, command: bool, choices: &BTreeMap<String, Choices>, drafts: &BTreeMap<String, String>) -> String {
    let mut html = format!("<h2>{}</h2><form method=\"post\" action=\"./perform\"><input type=\"hidden\" name=\"kind\" value=\"{}\"><input type=\"hidden\" name=\"action_id\" value=\"{}\">", escape(&model.title), if command { "command" } else { "action" }, escape(&model.title));
    for (id, field) in &model.fields {
        let name = escape(&format!("field.{id}"));
        let draft = drafts.get(id).map(String::as_str).unwrap_or("");
        let value = escape(draft);
        let required = if field.optional { "" } else { " required" };
        html.push_str(&format!("<label>{}{} ", escape(id), if field.optional { " (optional)" } else { "" }));
        match &field.schema {
            Schema::String => {
                if let Some(choices) = choices.get(id) {
                    let list = format!("choices-{}", crate::next_id());
                    html.push_str(&format!("<input name=\"{name}\" value=\"{value}\"{required} list=\"{list}\" data-source=\"{}\" aria-describedby=\"{list}-status\"><datalist id=\"{list}\">", escape(&choices.source)));
                    for choice in &choices.candidates.items {
                        html.push_str(&format!("<option value=\"{}\">{}{}</option>", escape(&choice.value), escape(&choice.label), choice.detail.as_ref().map(|detail| format!(" · {}", escape(detail))).unwrap_or_default()));
                    }
                    html.push_str(&format!("</datalist><small id=\"{list}-status\">{}</small>", if choices.candidates.truncated { "More matches available; type to filter." } else { "" }));
                } else { html.push_str(&format!("<input name=\"{name}\" value=\"{value}\"{required}>")); }
            },
            Schema::Int | Schema::Number => html.push_str(&format!("<input name=\"{name}\" value=\"{value}\" type=\"number\" step=\"{}\"{required}>", if field.schema == Schema::Int { "1" } else { "any" })),
            Schema::Bool => html.push_str(&format!("<select name=\"{name}\"{required}><option value=\"\">Choose…</option><option value=\"true\"{}>Yes</option><option value=\"false\"{}>No</option></select>", if draft == "true" { " selected" } else { "" }, if draft == "false" { " selected" } else { "" })),
            Schema::Choice { values } => {
                html.push_str(&format!("<select name=\"{name}\"{required}><option value=\"\">Choose…</option>"));
                for value in values {
                    let (value, label) = match value {
                        Literal::Null => ("null".into(), "None".into()),
                        Literal::Bool(value) => (value.to_string(), value.to_string()),
                        Literal::Int(value) => (value.to_string(), value.to_string()),
                        Literal::String(value) => (serde_json::to_string(value).unwrap(), value.clone()),
                    };
                    html.push_str(&format!("<option value=\"{}\"{}>{}</option>", escape(&value), if draft == value { " selected" } else { "" }, escape(&label)));
                }
                html.push_str("</select>");
            }
            _ => html.push_str(&format!("<textarea name=\"{name}\"{required} aria-label=\"{} as JSON\">{value}</textarea><small>Enter a JSON value. Bytes use an array of numbers from 0 to 255.</small>", escape(id))),
        }
        html.push_str("</label>");
    }
    html.push_str("<button>Run action</button></form>");
    html
}

pub(crate) async fn list(State(remote): State<Arc<Remote>>) -> Response {
    let mut html = String::new();
    for id in remote.interaction.interface.actions.keys() {
        html.push_str(&format!("<form method=\"post\" action=\"./action\"><input type=\"hidden\" name=\"action_id\" value=\"{}\"><button>{}</button></form>", escape(id), escape(id)));
    }
    if html.is_empty() { html.push_str("<p>No actions available.</p>"); }
    crate::requests::page(StatusCode::OK, "Available actions", html)
}
pub(crate) async fn open(State(remote): State<Arc<Remote>>, Form(fields): Form<HashMap<String,String>>) -> Response {
    match ActionForm::action(&remote.interaction.interface, fields.get("action_id").map(String::as_str).unwrap_or("")) {
        Ok(model) => crate::requests::page(StatusCode::OK, &model.title, markup(&model)),
        Err(fault) => crate::requests::page(StatusCode::BAD_REQUEST, "Action unavailable", escape(&fault.message)),
    }
}
pub(crate) async fn perform(State(remote): State<Arc<Remote>>, headers: HeaderMap, Form(fields): Form<HashMap<String,String>>) -> Response {
    let json = headers.get("accept").and_then(|value| value.to_str().ok()).is_some_and(|value| value.contains("application/json"));
    let failed = |status, title: &str, message: &str| {
        if json { (status, axum::Json(serde_json::json!({"ok":false,"error":message}))).into_response() }
        else { crate::requests::page(status, title, escape(message)) }
    };
    let prepared = (|| {
        let id = fields.get("action_id").map(String::as_str).unwrap_or("");
        let model = match fields.get("kind").map(String::as_str) {
            None | Some("action") => ActionForm::action(&remote.interaction.interface, id)?,
            Some("command") => ActionForm::command(&remote.interaction.interface, id)?,
            _ => return Err(misa_proto::Fault::protocol("Unknown form kind")),
        };
        let drafts = fields.iter().filter_map(|(key,value)| key.strip_prefix("field.").map(|id| (id.into(), value.clone()))).collect::<BTreeMap<_, _>>();
        let (command, input) = model.prepare(&drafts)?;
        remote.interaction.invoke(&command, input)
    })();
    let prepared: Prepared = match prepared {
        Ok(prepared) => prepared,
        Err(fault) => return failed(StatusCode::BAD_REQUEST, "Action not sent", &fault.message),
    };
    match crate::remote::invoke(&remote, prepared).await {
        Ok(outcome @ (Outcome::Completed { .. } | Outcome::Accepted { .. })) => {
            let report = crate::outcome_report(&outcome);
            if json { axum::Json(serde_json::json!({"ok":true,"outcome":outcome,"report":report})).into_response() }
            else if let Some(report) = report { crate::requests::page(StatusCode::OK, "Result", report) }
            else { Redirect::to("./").into_response() }
        },
        Ok(Outcome::Rejected { fault }) => failed(StatusCode::BAD_REQUEST, "Action rejected", &fault.message),
        _ => failed(StatusCode::BAD_GATEWAY, "Action not confirmed", "Check the operation before retrying."),
    }
}
