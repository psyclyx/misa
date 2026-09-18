//! Where a value can come from.
//!
//! Two shapes, and the difference is a decision about *cost*, not about capability:
//!
//! - A **resident** source is a query. It is small and slow-changing, so a client
//!   subscribes once and filters locally. Nothing is asked for while someone types,
//!   which is what makes a picker feel immediate, and it works with no round trip,
//!   no correlation, and no latency.
//! - An **on-demand** source is a finite query with a prefix and result bound.
//!   Reading it never starts an effect or mutates the session.
//!
//! What is *not* here: matching, ranking, frecency, key bindings, layout, and when to
//! open. Those are the client's, and `misa-kit`'s picker is one implementation of
//! them. A platform that wants a different picker — a browser's `<datalist>`, a
//! native autocomplete, a remote control's list — changes nothing here.
//!
//! # Why the command list is a source as well as a declaration
//!
//! So that `/co<TAB>` and a palette and a pull-down all read the same thing. The
//! exported preparation catalog tells a client which pickers are available;
//! the source exists so it has something to put in one, in the same shape as every
//! other source.

use misa_proto::view::Choice;
use misa_proto::view::{ChoiceMetadata, ChoicePeak, ChoicePricing};
use misa_proto::{Fault, Query};
use misa_reframe::{Inputs, Registry, read_query, try_derived_query};
use misa_value::Value;

use crate::catalog;

fn metadata_value(metadata: &ChoiceMetadata) -> Value {
    let mut fields = Vec::new();
    if let Some(context) = metadata.context_window {
        fields.push(("context_window", Value::Int(context)));
    }
    if !metadata.efforts.is_empty() {
        fields.push(("efforts", Value::list(metadata.efforts.iter().map(Value::str))));
    }
    if let Some(pricing) = &metadata.pricing {
        fields.push(("pricing", pricing_value(pricing)));
    }
    if let Some(peak) = &metadata.peak {
        fields.push(("peak", peak_value(peak)));
    }
    Value::map(fields)
}

fn pricing_value(pricing: &ChoicePricing) -> Value {
    let mut fields = Vec::new();
    for (name, value) in [
        ("input_micros_per_thousand", pricing.input_micros_per_thousand),
        ("output_micros_per_thousand", pricing.output_micros_per_thousand),
        ("cache_read_micros_per_thousand", pricing.cache_read_micros_per_thousand),
        ("cache_write_micros_per_thousand", pricing.cache_write_micros_per_thousand),
        ("request_micros", pricing.request_micros),
    ] {
        if let Some(value) = value {
            fields.push((name, Value::Int(value)));
        }
    }
    Value::map(fields)
}

fn peak_value(peak: &ChoicePeak) -> Value {
    Value::map([
        ("multiplier_ppm", Value::Int(peak.multiplier_ppm)),
        ("weekdays", Value::list(peak.weekdays.iter().copied().map(Value::Int))),
        (
            "windows",
            Value::list(peak.windows.iter().map(|window| {
                Value::map([
                    ("start_hour", Value::Int(window.start_hour)),
                    ("end_hour", Value::Int(window.end_hour)),
                ])
            })),
        ),
    ])
}

/// A fact about the conversation list, as its own query so a client can show a
/// count without asking for candidates.
use misa_proto::completion::CONVERSATIONS_QUERY;

/// Every resident source the shipped session installs.
///
/// Each source declares its exact exported member; clients never synthesize
/// query identifiers from source names.
pub fn subscriptions(registry: Registry) -> Registry {
    registry
        .subscription(
            "session.catalogue",
            read_query(|db, _query| {
                db.get("session")
                    .and_then(|session| session.get("catalogue"))
                    .cloned()
                    .unwrap_or(Value::Null)
            }),
        )
        .subscription(
            misa_proto::completion::MODELS_QUERY,
            read_query(|db, _query| {
                let session = db.get("session");
                let current = session
                    .and_then(|session| session.get("model"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let provider = session
                    .and_then(|session| session.get("provider"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let catalogue = session.and_then(|session| session.get("catalogue"));
                let listed = catalogue
                    .and_then(|catalogue| catalogue.get("models"))
                    .cloned()
                    .unwrap_or(Value::Null);
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
                            if let Some(metadata) = &choice.metadata {
                                row.push(("metadata", metadata_value(metadata)));
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
            misa_proto::completion::EFFORT_QUERY,
            read_query(|db, _query| {
                let model_id = db
                    .get("session")
                    .and_then(|session| session.get("model"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let provider = db
                    .get("session")
                    .and_then(|session| session.get("provider"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let current = db
                    .get("session")
                    .and_then(|session| session.get("effort"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let discovered = db
                    .get("session")
                    .and_then(|session| session.get("catalogue"))
                    .and_then(|catalogue| catalogue.get("models"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let levels = catalog::effort_levels(&discovered, provider, model_id);
                if levels.is_empty() {
                    Value::list([])
                } else {
                    Value::list(
                        levels
                            .iter()
                            .map(|level| {
                                Value::map([
                                    ("value", Value::str(level)),
                                    ("label", Value::str(level)),
                                    (
                                        "detail",
                                        Value::str(if level == current { "current" } else { "" }),
                                    ),
                                ])
                            })
                            .collect::<Vec<_>>(),
                    )
                }
            }),
        )
        .subscription(
            // The services the kernel knows by name. The list is the *daemon's*, not the
            // client's, because which services exist is a composition decision.
            misa_proto::completion::PROVIDERS_QUERY,
            read_query(|_db, _query| {
                Value::list(
                    misa_kernel::presets::auth_ids()
                        .into_iter()
                        .map(|id| {
                            // Provider ids are the public vocabulary. Display labels that
                            // paraphrase them make the value somebody types differ from the
                            // value the session receives, and the old picker never did that.
                            Value::map([("value", Value::str(id)), ("label", Value::str(id))])
                        })
                        .collect::<Vec<_>>(),
                )
            }),
        )
        .subscription(
            CONVERSATIONS_QUERY,
            read_query(|db, _query| db.get("conversations").cloned().unwrap_or(Value::list([]))),
        )
        .subscription(
            misa_proto::preparation::SOURCES,
            misa_reframe::derived_query([], |_| crate::wire::render(&sources())),
        )
        .subscription(
            misa_proto::preparation::SEARCH,
            try_derived_query(
                Inputs::Dynamic(std::sync::Arc::new(|query| {
                    let (source, _, _) = search_args(query)?;
                    match source_query(source) {
                        Some(id) => Ok(vec![Query::new(id)]),
                        None if source == misa_proto::completion::CONVERSATIONS => Ok(vec![
                            Query::new(CONVERSATIONS_QUERY),
                            Query::new("session.status"),
                        ]),
                        _ => Err(misa_reframe::Fault::query("Unknown completion source")),
                    }
                })),
                |inputs, query, _| {
                    let (source, prefix, limit) = search_args(query)?;
                    let (items, truncated) = if source == misa_proto::completion::CONVERSATIONS {
                        let db = Value::map([
                            ("conversations", inputs[0].clone()),
                            ("session", inputs[1].clone()),
                        ]);
                        on_demand(&db, source, prefix, limit)
                            .map_err(|fault| misa_reframe::Fault::new(fault.code, fault.message))?
                    } else {
                        let all: Vec<Choice> =
                            crate::wire::parse(&inputs[0]).map_err(misa_reframe::Fault::query)?;
                        let needle = prefix.to_lowercase();
                        let mut matches = all.into_iter().filter(|choice| {
                            choice.value.to_lowercase().contains(&needle)
                                || choice.label.to_lowercase().contains(&needle)
                        });
                        let items = matches.by_ref().take(limit as usize).collect();
                        (items, matches.next().is_some())
                    };
                    Ok(crate::wire::render(&misa_proto::preparation::Candidates {
                        items,
                        truncated,
                    }))
                },
            ),
        )
}

fn search_args(query: &Query) -> Result<(&str, &str, u32), misa_reframe::Fault> {
    if query.args.len() != 3 {
        return Err(misa_reframe::Fault::query(
            "Completion search needs source, prefix and limit",
        ));
    }
    let source = query.args[0]
        .as_str()
        .ok_or_else(|| misa_reframe::Fault::query("Completion source must be text"))?;
    let prefix = query.args[1]
        .as_str()
        .ok_or_else(|| misa_reframe::Fault::query("Completion prefix must be text"))?;
    let limit = query.args[2]
        .as_i64()
        .filter(|limit| *limit > 0 && *limit <= misa_proto::preparation::DEFAULT_CANDIDATES as i64)
        .ok_or_else(|| {
            misa_reframe::Fault::query("Completion limit is outside its bounded range")
        })? as u32;
    Ok((source, prefix, limit))
}

pub fn sources() -> Vec<misa_proto::preparation::Source> {
    catalog::sources()
}

pub fn exports() -> Vec<misa_proto::query::Definition> {
    use misa_proto::{
        query::{Definition, ResultContract},
        schema::{Field, Schema},
    };
    let choices = Schema::List {
        items: Box::new(Schema::Record {
            fields: [
                ("value", Schema::String, false),
                ("label", Schema::String, false),
                ("detail", Schema::String, true),
                ("metadata", Schema::Value, true),
            ]
            .into_iter()
            .map(|(name, schema, optional)| {
                (
                    name.into(),
                    Field { schema, optional },
                )
            })
            .collect(),
            allow_unknown: false,
        }),
    };
    let mut definitions = sources()
        .into_iter()
        .filter(|source| {
            source.kind == misa_proto::preparation::SourceKind::Resident
                && source.member.query.id != misa_proto::completion::COMMANDS_QUERY
        })
        .map(|source| Definition {
            id: source.member.query.id,
            arguments: vec![],
            contract: source.member.contract,
            result: ResultContract::Data {
                schema: choices.clone(),
            },
        })
        .collect::<Vec<_>>();
    definitions.push(Definition {
        id: misa_proto::preparation::SEARCH.into(),
        arguments: vec![Schema::String, Schema::String, Schema::Int],
        contract: "completion.search@1".into(),
        result: ResultContract::Data {
            schema: Schema::Record {
                fields: [
                    (
                        "items".into(),
                        Field {
                            schema: choices,
                            optional: false,
                        },
                    ),
                    (
                        "truncated".into(),
                        Field {
                            schema: Schema::Bool,
                            optional: false,
                        },
                    ),
                ]
                .into_iter()
                .collect(),
                allow_unknown: false,
            },
        },
    });
    definitions.push(Definition {
        id: misa_proto::preparation::SOURCES.into(),
        arguments: vec![],
        contract: "completion.catalog@1".into(),
        result: ResultContract::Data {
            schema: Schema::List {
                items: Box::new(Schema::Value),
            },
        },
    });
    definitions
}

pub(crate) fn command_definition() -> misa_proto::query::Definition {
    use misa_proto::{
        query::{Definition, ResultContract},
        schema::{Field, Schema},
    };
    Definition {
        id: misa_proto::completion::COMMANDS_QUERY.into(),
        arguments: vec![],
        contract: format!("{}@1", misa_proto::completion::COMMANDS_QUERY),
        result: ResultContract::Data {
            schema: Schema::List {
                items: Box::new(Schema::Record {
                    fields: [("value", false), ("label", false), ("detail", true)]
                        .into_iter()
                        .map(|(name, optional)| {
                            (
                                name.into(),
                                Field {
                                    schema: Schema::String,
                                    optional,
                                },
                            )
                        })
                        .collect(),
                    allow_unknown: false,
                }),
            },
        },
    }
}

/// Answer an on-demand source.
///
/// Only sources the session can answer without a capability do this synchronously.
/// A source that needs one — paths on the daemon's filesystem, a search backend —
/// goes through the kernel and answers later, with the same correlation and the
/// same shape; the reason it is not here is that a handler must not do IO, and that
/// rule is worth more than the convenience of answering inline.
pub fn on_demand(
    db: &Value,
    source: &str,
    prefix: &str,
    limit: u32,
) -> Result<(Vec<Choice>, bool), Fault> {
    let declared = catalog::sources()
        .into_iter()
        .find(|entry| entry.id == source);
    let Some(declared) = declared else {
        return Err(Fault::new(
            "source.unknown",
            format!("there is no completion source named `{source}`"),
        ));
    };
    if declared.kind != misa_proto::preparation::SourceKind::OnDemand {
        return Err(Fault::new(
            "source.resident",
            format!(
                "`{source}` is a source a client holds; subscribe to `{}`",
                declared.member.query.id
            ),
        ));
    }
    let limit = limit.clamp(1, misa_proto::preparation::DEFAULT_CANDIDATES) as usize;
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
        let title = entry
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or_default();
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
            label: if title.is_empty() {
                id.to_string()
            } else {
                title.to_string()
            },
            detail: Some(format!("{messages} messages")),
                metadata: None,
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
    source_query(source).map(Query::new).ok_or_else(|| {
        Fault::new(
            "source.unknown",
            format!("`{source}` is not a resident source"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_reframe::Loop;

    fn db() -> Value {
        let mut map = std::collections::BTreeMap::new();
        map.insert(
            "session".to_string(),
            Value::map([
                ("id", Value::str("c1")),
                ("conversation", Value::str("c1")),
                ("provider", Value::str("claude")),
                ("model", Value::str("claude-sonnet-5")),
                ("effort", Value::str("medium")),
                (
                    "catalogue",
                    Value::map([("provider", Value::str("claude")), ("models", Value::Null)]),
                ),
            ]),
        );
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
        let mut registry = subscriptions(Registry::new().subscription(
            "session.status",
            read_query(|db, _| db.get("session").cloned().unwrap_or(Value::Null)),
        ));
        for (definition, value) in
            crate::commands::catalogs(&crate::commands::install(&[]).unwrap(), &[]).unwrap()
        {
            registry = registry.subscription(
                &definition.id,
                misa_reframe::derived_query([], move |_| value.clone()),
            );
        }
        let registry = std::sync::Arc::new(registry);
        Loop::new(
            registry,
            std::sync::Arc::new(misa_reframe::AcceptsEverything),
            db(),
        )
    }

    #[test]
    fn finite_search_is_bounded_and_does_not_mutate_owner_state() {
        let mut owner = loop_with_completions();
        let query = Query::new(misa_proto::preparation::SEARCH)
            .arg(Value::str("conversations"))
            .arg(Value::str(""))
            .arg(Value::Int(1));
        let before = owner.db().clone();
        let value = owner.query(&query).unwrap();
        let answer: misa_proto::preparation::Candidates = crate::wire::parse(&value).unwrap();
        assert_eq!(answer.items.len(), 1);
        assert!(answer.truncated);
        assert!(before.same(owner.db()));
        for limit in [0, -1, i64::MAX] {
            let invalid = Query::new(misa_proto::preparation::SEARCH)
                .arg(Value::str("models"))
                .arg(Value::str(""))
                .arg(Value::Int(limit));
            assert!(owner.query(&invalid).is_err());
        }
    }

    #[test]
    fn a_resident_source_is_readable_and_an_on_demand_one_is_not() {
        let mut loop_ = loop_with_completions();
        let models = loop_.query(&query_for("models").unwrap()).expect("models");
        assert!(!misa_proto::preparation::candidates(&models).is_empty());
        assert!(query_for("conversations").is_err());
        assert!(query_for("nonsense").is_err());
    }

    #[test]
    fn provider_choices_are_ids_without_display_copy() {
        let mut loop_ = loop_with_completions();
        let providers = loop_.query(&query_for("providers").unwrap()).unwrap();
        let candidates = misa_proto::preparation::candidates(&providers);

        assert!(!candidates.is_empty());
        assert!(candidates.iter().any(|choice| choice.value == "brave"));
        assert!(candidates.iter().any(|choice| choice.value == "tavily"));
        assert!(candidates.iter().all(|choice| {
            choice.value == choice.label
                && choice
                    .detail
                    .as_deref()
                    .is_none_or(str::is_empty)
                && choice.value != "scripted"
        }));
    }

    #[test]
    fn the_model_source_carries_what_a_picker_needs_to_choose() {
        let mut loop_ = loop_with_completions();
        let models = loop_.query(&query_for("models").unwrap()).unwrap();
        let candidates = misa_proto::preparation::candidates(&models);
        let current = candidates
            .iter()
            .find(|candidate| candidate.value == "claude/claude-sonnet-5")
            .expect("the current model");
        assert!(
            current
                .detail
                .as_deref()
                .expect("a detail")
                .contains("current")
        );
        // A model's own facts, so a picker can show why one differs from another.
        assert!(
            current
                .detail
                .as_deref()
                .expect("a detail")
                .contains("context")
        );
        assert_eq!(
            current
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.context_window),
            Some(1_000_000)
        );
    }

    #[test]
    fn model_effort_levels_are_offered_when_the_model_takes_one() {
        let mut loop_ = loop_with_completions();
        assert_eq!(
            misa_proto::preparation::candidates(
                &loop_.query(&query_for("effort").unwrap()).unwrap()
            )
            .len(),
            4
        );
    }

    #[test]
    fn the_command_source_carries_the_slash_a_person_would_type() {
        let mut loop_ = loop_with_completions();
        let commands = loop_.query(&query_for("commands").unwrap()).unwrap();
        let candidates = misa_proto::preparation::candidates(&commands);
        assert!(
            candidates
                .iter()
                .any(|candidate| candidate.value == "/model")
        );
        let model = candidates
            .iter()
            .find(|candidate| candidate.value == "/model")
            .unwrap();
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
        assert!(
            truncated,
            "a client filtering locally was not told its list was partial"
        );
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
        assert!(
            fault.message.contains("completion.models"),
            "{}",
            fault.message
        );
        let fault = on_demand(&db(), "nope", "s", 10).unwrap_err();
        assert_eq!(fault.code, "source.unknown");
    }
}
