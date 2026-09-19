//! Private request details are finite authorized reads. Forms and dismissal are
//! local documents; secret values are never echoed or stored in browser memory.
use crate::{Remote, escape};
use axum::{
    extract::{Form, Query, State},
    http::{StatusCode, header},
    response::{
        Html, IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use misa_client::{interface::data, request::Model};
use misa_proto::{invocation::Outcome, observation::Selection};
use misa_value::Value;
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::Duration,
};

pub(crate) fn page(status: StatusCode, title: &str, body: String) -> Response {
    (status, [(header::CACHE_CONTROL, "no-store")], Html(format!("<!doctype html><html><head><meta name=\"viewport\" content=\"width=device-width\"><title>{}</title><link rel=\"stylesheet\" href=\"./style.css\"></head><body><main><h1>{}</h1>{}<p><a href=\"./\">Return to session</a> · <a href=\"./requests\">Pending requests</a></p></main></body></html>", escape(title), escape(title), body))).into_response()
}
async fn read(remote: &Remote, query: &str, arguments: Vec<Value>) -> Result<Value, String> {
    let member = remote
        .interaction
        .interface
        .query(query, arguments)
        .map_err(|fault| fault.message)?;
    let result = remote
        .daemon
        .client
        .read(
            Selection {
                scope: remote.interaction.interface.scope.clone(),
                members: BTreeMap::from([("request".into(), member)]),
            },
            Duration::from_secs(20),
        )
        .await
        .map_err(|fault| fault.message)?;
    data(&result, "request")
        .cloned()
        .map_err(|fault| fault.message)
}
async fn model(remote: &Remote, id: &str) -> Result<Model, String> {
    let value = read(remote, "operation.request", vec![Value::str(id)]).await?;
    Model::parse(&value, &remote.interaction.interface)
        .map_err(|fault| fault.message)?
        .ok_or_else(|| "This request is resolved or no longer available".into())
}
pub(crate) async fn list(State(remote): State<Arc<Remote>>) -> Response {
    let summaries = async {
        let members = ["requests.summary", "operations.summary"]
            .into_iter()
            .map(|id| {
                remote
                    .interaction
                    .interface
                    .query(id, vec![])
                    .map(|member| (id.into(), member))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()
            .map_err(|fault| fault.message)?;
        remote
            .daemon
            .client
            .read(
                Selection {
                    scope: remote.interaction.interface.scope.clone(),
                    members,
                },
                Duration::from_secs(20),
            )
            .await
            .map_err(|fault| fault.message)
    }
    .await;
    let summaries = match summaries {
        Ok(value) => value,
        Err(error) => {
            return page(
                StatusCode::BAD_GATEWAY,
                "Requests unavailable",
                escape(&error),
            );
        }
    };
    let mut body = String::new();
    let mut seen = std::collections::BTreeSet::new();
    for request in ["requests.summary", "operations.summary"]
        .into_iter()
        .filter_map(|id| data(&summaries, id).ok())
        .flat_map(|value| value.as_list().unwrap_or(&[]))
    {
        if request.get("state").and_then(Value::as_str) != Some("awaiting_input") {
            continue;
        }
        let Some(id) = request.get("id").and_then(Value::as_str) else {
            continue;
        };
        if !seen.insert(id) {
            continue;
        }
        let title = request
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("Input request");
        body.push_str(&format!("<form method=\"post\" action=\"./request\"><input type=\"hidden\" name=\"id\" value=\"{}\"><button>{}</button></form>", escape(id), escape(title)));
    }
    if body.is_empty() {
        body.push_str("<p>No pending requests.</p>");
    }
    page(StatusCode::OK, "Pending requests", body)
}
pub(crate) async fn navigate(
    State(remote): State<Arc<Remote>>,
    Query(fields): Query<HashMap<String, String>>,
) -> Response {
    open(State(remote), Form(fields)).await
}
pub(crate) async fn open(
    State(remote): State<Arc<Remote>>,
    Form(fields): Form<HashMap<String, String>>,
) -> Response {
    let model = match model(&remote, fields.get("id").map(String::as_str).unwrap_or("")).await {
        Ok(model) => model,
        Err(error) => {
            return page(
                StatusCode::BAD_REQUEST,
                "Request unavailable",
                escape(&error),
            );
        }
    };
    if fields
        .get("generation")
        .is_some_and(|expected| expected.parse::<i64>().ok() != Some(model.generation))
    {
        return page(StatusCode::CONFLICT, "Request changed", "This request changed after the overview was read. Open the current request from Pending requests.".into());
    }
    let mut body = crate::render_scoped(&model.body, "private-request:");
    body.push_str(&format!("<p id=\"request-status\" role=\"status\"></p><form id=\"private-request\" method=\"post\" action=\"./respond\" autocomplete=\"off\"><input type=\"hidden\" name=\"id\" value=\"{}\"><input type=\"hidden\" name=\"generation\" value=\"{}\">", escape(&model.id), model.generation));
    if let Some(input) = &model.input {
        body.push_str(&format!(
            "<label>{}<input name=\"{}\" type=\"{}\" autocomplete=\"off\"></label>",
            escape(&input.label),
            escape(&input.id),
            if input.secret { "password" } else { "text" }
        ));
    }
    if let Some(form) = &model.form {
        if let misa_proto::schema::Schema::Record { fields, .. } = &form.input {
            let fields = fields
                .iter()
                .map(|(id, field)| (id.clone(), field.clone()))
                .collect::<Vec<_>>();
            let labels = form
                .fields
                .iter()
                .map(|(id, field)| (id.clone(), field.label.clone()))
                .collect();
            body.push_str(&crate::actions::fields(
                &fields,
                &labels,
                &BTreeMap::new(),
                &BTreeMap::new(),
            ));
        }
    }
    for action in &model.actions {
        body.push_str(&format!(
            "<button name=\"action\" value=\"{}\"{}>{}</button>",
            escape(&action.id),
            if action.id == "cancel" {
                " formnovalidate"
            } else {
                ""
            },
            escape(&action.label)
        ));
    }
    body.push_str("</form><p>Returning to the session hides this form. It does not cancel the operation.</p><script src=\"./request.js\"></script>");
    page(StatusCode::OK, &model.title, body)
}

/// A private observation carries only form validity to the browser. Secret
/// fields stay in this document and are cleared on resolution or loss of scope.
pub(crate) async fn events(
    State(remote): State<Arc<Remote>>,
    Query(fields): Query<HashMap<String, String>>,
) -> Response {
    let Some(id) = fields.get("id") else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let Some(generation) = fields
        .get("generation")
        .and_then(|value| value.parse::<i64>().ok())
    else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let member = match remote
        .interaction
        .interface
        .query("operation.request", vec![Value::str(id)])
    {
        Ok(member) => member,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };
    let observation = match remote
        .daemon
        .client
        .observe(
            Selection {
                scope: remote.interaction.interface.scope.clone(),
                members: BTreeMap::from([("request".into(), member)]),
            },
            None,
        )
        .await
    {
        Ok(observation) => observation,
        Err(_) => return StatusCode::BAD_GATEWAY.into_response(),
    };
    let closed = remote.closed.subscribe();
    let stream = futures::stream::unfold(
        (observation, true, remote, closed, false),
        move |(mut observation, first, remote, mut closed, ended)| async move {
            if ended {
                return None;
            }
            if !first && !*closed.borrow() {
                tokio::select! {
                    result = observation.changed() => { if result.is_err() { return None; } }
                    _ = closed.changed() => {}
                }
            }
            if *closed.borrow() {
                return Some((
                    Ok::<_, std::convert::Infallible>(Event::default().data("unavailable")),
                    (observation, false, remote, closed, true),
                ));
            }
            let status = observation
                .inspect(|replica, _| {
                    use misa_protocol::observation::{MemberState, Status};
                    match replica.status() {
                        Status::Awaiting => "opening",
                        Status::Current => {
                            match replica.current().and_then(|members| members.get("request")) {
                                Some(MemberState::Value(Value::Null)) => "resolved",
                                Some(MemberState::Value(value))
                                    if value.get("generation").and_then(Value::as_i64)
                                        == Some(generation) =>
                                {
                                    "current"
                                }
                                Some(MemberState::Value(_)) => "changed",
                                _ => "unavailable",
                            }
                        }
                        _ => "unavailable",
                    }
                })
                .unwrap_or("unavailable");
            Some((
                Ok::<_, std::convert::Infallible>(Event::default().data(status)),
                (observation, false, remote, closed, false),
            ))
        },
    );
    let mut response = Sse::new(stream)
        .keep_alive(KeepAlive::default())
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}
pub(crate) async fn respond(
    State(remote): State<Arc<Remote>>,
    Form(fields): Form<HashMap<String, String>>,
) -> Response {
    let prepared = async {
        let model = model(&remote, fields.get("id").map(String::as_str).unwrap_or("")).await?;
        if fields
            .get("generation")
            .and_then(|value| value.parse::<i64>().ok())
            != Some(model.generation)
        {
            return Err("The request changed; reopen its current form".to_string());
        }
        let drafts = if model.form.is_some() {
            fields
                .iter()
                .filter_map(|(id, value)| {
                    id.strip_prefix("field.")
                        .map(|id| (id.into(), value.clone()))
                })
                .collect()
        } else {
            model
                .input
                .as_ref()
                .and_then(|input| {
                    fields
                        .get(&input.id)
                        .map(|value| (input.id.clone(), value.clone()))
                })
                .into_iter()
                .collect()
        };
        model
            .prepare_drafts(
                fields.get("action").map(String::as_str).unwrap_or(""),
                &drafts,
                &remote.interaction.interface,
            )
            .map_err(|fault| fault.message)
    }
    .await;
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(error) => return page(StatusCode::BAD_REQUEST, "Response not sent", escape(&error)),
    };
    match crate::remote::invoke(&remote, prepared).await {
        Ok(Outcome::Completed { .. } | Outcome::Accepted { .. }) => page(StatusCode::OK, "Response accepted", "<p>The owner accepted your response.</p>".into()),
        Ok(Outcome::Rejected { fault }) => page(StatusCode::BAD_REQUEST, "Response rejected", escape(&fault.message)),
        Ok(Outcome::Indeterminate { .. }) | Err(_) => page(StatusCode::BAD_GATEWAY, "Response not confirmed", "<p>Check the current request before submitting again. Secret input has not been retained.</p>".into()),
    }
}
