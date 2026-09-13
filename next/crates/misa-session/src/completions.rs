//! Where a value can come from.
//!
//! Two shapes, and the difference is a decision about *cost*, not about capability:
//!
//! - A **resident** source is a query. It is small and slow-changing, so a client
//!   subscribes once and filters locally. Nothing is asked for while someone types,
//!   which is what makes a picker feel immediate, and it works with no round trip,
//!   no correlation, and no latency.
//! - An **on-demand** source is an intent. It is large, dynamic, or a capability,
//!   so a client asks for a prefix and is answered with candidates it did not have.
//!
//! What is *not* here: matching, ranking, frecency, key bindings, layout, and when to
//! open. Those are the client's, and `misa-client`'s picker is one implementation of
//! them. A platform that wants a different picker — a browser's `<datalist>`, a
//! native autocomplete, a remote control's list — changes nothing here.
//!
//! # Why the command list is a source as well as a declaration
//!
//! So that `/co<TAB>` and a palette and a pull-down all read the same thing. The
//! declaration in `SessionInfo` exists so a client knows a picker is *meaningful*;
//! the source exists so it has something to put in one, in the same shape as every
//! other source.

use misa_proto::view::Choice;
use misa_proto::{Fault, Query};
use misa_reframe::{Registry, read_query};
use misa_value::Value;

use crate::catalog;

/// A fact about the conversation list, as its own query so a client can show a
/// count without asking for candidates.
pub const CONVERSATIONS_QUERY: &str = "session.conversations";

/// Every resident source the shipped session installs.
///
/// One query per source, named `completion.<id>` by `Source::query`, so a client
/// that holds a declaration can find the items with no further agreement.
pub fn subscriptions(registry: Registry) -> Registry {
    registry
        .subscription(
            "completion.models",
            read_query(|db, _query| {
                let session = db.get("session");
                let current = session
                    .and_then(|session| session.get("model"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let catalogue = session.and_then(|session| session.get("catalogue"));
                let provider = catalogue
                    .and_then(|catalogue| catalogue.get("provider"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let listed = catalogue.and_then(|catalogue| catalogue.get("models")).cloned().unwrap_or(Value::Null);
                Value::list(
                    catalog::choices(&listed, provider, current)
                        .into_iter()
                        .map(|choice| {
                            let mut row = vec![
                                ("value", Value::str(&choice.value)),
                                ("label", Value::str(&choice.label)),
                            ];
                            if let Some(detail) = &choice.detail {
                                row.push(("detail", Value::str(detail)));
                            }
                            Value::map(row)
                        })
                        .collect::<Vec<_>>(),
                )
            }),
        )
        .subscription(
            // Only the levels the current model takes: offering "high" to a model
            // that ignores it is telling a person a setting exists.
            "completion.effort",
            read_query(|db, _query| {
                let model_id = db
                    .get("session")
                    .and_then(|session| session.get("model"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let current = db
                    .get("session")
                    .and_then(|session| session.get("effort"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                match catalog::model(model_id) {
                    Some(model) if model.effort => Value::list(
                        catalog::EFFORTS
                            .iter()
                            .map(|level| {
                                Value::map([
                                    ("value", Value::str(*level)),
                                    ("label", Value::str(*level)),
                                    (
                                        "detail",
                                        Value::str(if *level == current { "current" } else { "" }),
                                    ),
                                ])
                            })
                            .collect::<Vec<_>>(),
                    ),
                    _ => Value::list([]),
                }
            }),
        )
        .subscription(
            // The value a client accepts is what a person would have typed, so a
            // command's candidate carries its slash. The declaration omits it
            // because a slash is punctuation and punctuation belongs to whoever is
            // writing the line.
            "completion.commands",
            read_query(|_db, _query| {
                Value::list(
                    catalog::commands()
                        .into_iter()
                        .map(|command| {
                            let args = command
                                .args
                                .iter()
                                .map(|arg| arg.label.clone())
                                .collect::<Vec<_>>()
                                .join(" ");
                            Value::map([
                                ("value", Value::str(format!("/{}", command.id))),
                                ("label", Value::str(format!("/{}", command.id))),
                                (
                                    "detail",
                                    Value::str(if args.is_empty() {
                                        command.description.clone()
                                    } else {
                                        format!("{} — {args}", command.description)
                                    }),
                                ),
                            ])
                        })
                        .collect::<Vec<_>>(),
                )
            }),
        )
        .subscription(
            // The services the kernel knows by name. The list is the *daemon's*, not the
            // client's, because which services exist is a composition decision.
            "completion.providers",
            read_query(|_db, _query| {
                Value::list(
                    misa_kernel::presets::ids()
                        .into_iter()
                        .map(|id| {
                            let preset = misa_kernel::presets::preset(id);
                            let mut row = vec![
                                ("value", Value::str(id)),
                                ("label", Value::str(preset.map(|preset| preset.label).unwrap_or(id))),
                            ];
                            // One line, and the one thing that decides what `/login` will do
                            // with it: a service with a device flow has no key to paste.
                            let detail = match preset {
                                Some(preset) if misa_kernel::presets::oauth(id).is_some() => {
                                    format!("{} · authorized by a device code", preset.note)
                                }
                                Some(preset) => preset.note.to_string(),
                                None => String::new(),
                            };
                            row.push(("detail", Value::str(detail)));
                            Value::map(row)
                        })
                        .collect::<Vec<_>>(),
                )
            }),
        )
        .subscription(
            CONVERSATIONS_QUERY,
            read_query(|db, _query| db.get("conversations").cloned().unwrap_or(Value::list([]))),
        )
}

/// Answer an on-demand source.
///
/// Only sources the session can answer without a capability do this synchronously.
/// A source that needs one — paths on the daemon's filesystem, a search backend —
/// goes through the kernel and answers later, with the same correlation and the
/// same shape; the reason it is not here is that a handler must not do IO, and that
/// rule is worth more than the convenience of answering inline.
pub fn on_demand(db: &Value, source: &str, prefix: &str, limit: u32) -> Result<(Vec<Choice>, bool), Fault> {
    let declared = catalog::sources().into_iter().find(|entry| entry.id == source);
    let Some(declared) = declared else {
        return Err(Fault::new(
            "source.unknown",
            format!("there is no completion source named `{source}`"),
        ));
    };
    if declared.kind != misa_proto::wire::SourceKind::OnDemand {
        return Err(Fault::new(
            "source.resident",
            format!("`{source}` is a source a client holds; subscribe to `{}`", declared.query()),
        ));
    }
    let limit = limit.clamp(1, misa_proto::wire::DEFAULT_CANDIDATES) as usize;
    let needle = prefix.to_lowercase();

    let all = db
        .get("conversations")
        .and_then(Value::as_list)
        .map(<[Value]>::to_vec)
        .unwrap_or_default();
    let mut candidates = Vec::new();
    let mut truncated = false;
    for entry in all {
        let id = entry.get("id").and_then(Value::as_str).unwrap_or_default();
        let title = entry.get("title").and_then(Value::as_str).unwrap_or_default();
        // A prefix matching either the name or the title, because a person looking
        // for a conversation remembers one or the other and asking them which is a
        // distinction the session invented.
        let matches = needle.is_empty()
            || id.to_lowercase().contains(&needle)
            || title.to_lowercase().contains(&needle);
        if !matches {
            continue;
        }
        if candidates.len() == limit {
            truncated = true;
            break;
        }
        let messages = entry.get("messages").and_then(Value::as_i64).unwrap_or(0);
        candidates.push(Choice {
            value: id.to_string(),
            label: if title.is_empty() { id.to_string() } else { title.to_string() },
            detail: Some(format!("{messages} messages")),
        });
    }
    // The current conversation is worth saying so about, since resuming it is a
    // no-op a person should not have to discover.
    let current = db
        .get("session")
        .and_then(|session| session.get("conversation"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    for candidate in candidates.iter_mut() {
        if candidate.value == current {
            candidate.detail = Some(match candidate.detail.take() {
                Some(detail) => format!("{detail} · current"),
                None => "current".into(),
            });
        }
    }
    Ok((candidates, truncated))
}

/// A subscription registered under an arbitrary id, for a source that names one.
pub fn source_query(source: &str) -> Option<String> {
    catalog::resident_query(source)
}

/// The query a client reads for a resident source, or a fault saying why not.
pub fn query_for(source: &str) -> Result<Query, Fault> {
    source_query(source)
        .map(Query::new)
        .ok_or_else(|| Fault::new("source.unknown", format!("`{source}` is not a resident source")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_reframe::Loop;

    fn db() -> Value {
        let mut map = std::collections::BTreeMap::new();
        map.insert("session".to_string(), Value::map([
            ("id", Value::str("c1")),
            ("conversation", Value::str("c1")),
            ("model", Value::str("scripted-1")),
            ("effort", Value::str("medium")),
        ]));
        map.insert(
            "conversations".to_string(),
            Value::list([
                Value::map([
                    ("id", Value::str("c1")),
                    ("title", Value::str("a haiku")),
                    ("messages", Value::Int(4)),
                ]),
                Value::map([
                    ("id", Value::str("c2")),
                    ("title", Value::str("refactor the parser")),
                    ("messages", Value::Int(12)),
                ]),
            ]),
        );
        Value::Map(std::sync::Arc::new(map))
    }

    fn loop_with_completions() -> Loop {
        let registry = std::sync::Arc::new(subscriptions(Registry::new()));
        Loop::new(registry, std::sync::Arc::new(misa_reframe::AcceptsEverything), db())
    }

    #[test]
    fn a_resident_source_is_readable_and_an_on_demand_one_is_not() {
        let mut loop_ = loop_with_completions();
        let models = loop_.query(&query_for("models").unwrap()).expect("models");
        assert!(!misa_proto::wire::candidates(&models).is_empty());
        assert!(query_for("conversations").is_err());
        assert!(query_for("nonsense").is_err());
    }

    #[test]
    fn the_model_source_carries_what_a_picker_needs_to_choose() {
        let mut loop_ = loop_with_completions();
        let models = loop_.query(&query_for("models").unwrap()).unwrap();
        let candidates = misa_proto::wire::candidates(&models);
        let current = candidates
            .iter()
            .find(|candidate| candidate.value == "scripted-1")
            .expect("the current model");
        assert!(current.detail.as_deref().expect("a detail").contains("current"));
        // A model's own facts, so a picker can show why one differs from another.
        assert!(current.detail.as_deref().expect("a detail").contains("context"));
    }

    #[test]
    fn a_models_effort_levels_are_only_offered_when_it_takes_one() {
        let mut loop_ = loop_with_completions();
        assert!(misa_proto::wire::candidates(&loop_.query(&query_for("effort").unwrap()).unwrap()).is_empty());
    }

    #[test]
    fn the_command_source_carries_the_slash_a_person_would_type() {
        let mut loop_ = loop_with_completions();
        let commands = loop_.query(&query_for("commands").unwrap()).unwrap();
        let candidates = misa_proto::wire::candidates(&commands);
        assert!(candidates.iter().any(|candidate| candidate.value == "/model"));
        let model = candidates.iter().find(|candidate| candidate.value == "/model").unwrap();
        assert!(model.detail.as_deref().unwrap().contains("Model"));
    }

    #[test]
    fn an_on_demand_source_is_searched_by_prefix_over_the_name_or_the_title() {
        let (by_id, _) = on_demand(&db(), "conversations", "c2", 10).unwrap();
        assert_eq!(by_id.len(), 1);
        assert_eq!(by_id[0].value, "c2");
        let (by_title, _) = on_demand(&db(), "conversations", "parser", 10).unwrap();
        assert_eq!(by_title[0].value, "c2");
        let (all, truncated) = on_demand(&db(), "conversations", "", 10).unwrap();
        assert_eq!(all.len(), 2);
        assert!(!truncated);
    }

    #[test]
    fn a_search_that_hits_more_than_the_limit_says_it_was_truncated() {
        let (candidates, truncated) = on_demand(&db(), "conversations", "", 1).unwrap();
        assert_eq!(candidates.len(), 1);
        assert!(truncated, "a client filtering locally was not told its list was partial");
    }

    #[test]
    fn the_current_conversation_says_so_rather_than_being_a_silent_no_op() {
        let (candidates, _) = on_demand(&db(), "conversations", "c1", 10).unwrap();
        assert!(candidates[0].detail.as_deref().unwrap().contains("current"));
    }

    #[test]
    fn asking_a_resident_source_on_demand_is_refused_with_the_query_to_use() {
        let fault = on_demand(&db(), "models", "s", 10).unwrap_err();
        assert_eq!(fault.code, "source.resident");
        assert!(fault.message.contains("completion.models"), "{}", fault.message);
        let fault = on_demand(&db(), "nope", "s", 10).unwrap_err();
        assert_eq!(fault.code, "source.unknown");
    }
}
