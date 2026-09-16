use super::*;
use crate::Contribution;
use misa_proto::{Node, Query, sync::StreamUpdate};
use misa_protocol::observation::{MemberState, Replica};
use misa_reframe::{Event, read_query};

fn document(id: &str, text: &str) -> Value {
    crate::wire::render(
        &Node::new(
            "test.document",
            misa_proto::view::Kind::Status { text: text.into() },
        )
        .id(id),
    )
}

fn runtime() -> Arc<Runtime> {
    let mut contribution = Contribution::new()
        .with_root(
            "guest",
            Value::map([
                ("left", document("left", "Left")),
                ("right", document("right", "Right")),
            ]),
        )
        .unwrap();
    for name in ["left", "right"] {
        contribution = contribution
            .with_subscription(
                format!("test.{name}"),
                read_query(move |db, _| {
                    db.get("guest")
                        .and_then(|guest| guest.get(name))
                        .cloned()
                        .unwrap_or(Value::Null)
                }),
            )
            .export_query(Definition {
                id: format!("test.{name}"),
                arguments: vec![],
                contract: format!("test.{name}@1"),
                result: ResultContract::Document {},
            })
            .with_presentation(misa_proto::presentation::Presentation {
                id: format!("test.{name}"),
                title: name.into(),
                variants: vec![misa_proto::presentation::Variant {
                    id: "basic".into(),
                    requirements: vec![],
                    member: Member {
                        query: Query::new(format!("test.{name}")),
                        contract: format!("test.{name}@1"),
                        encoding: Encoding::Document,
                        optional: false,
                    },
                }],
            });
    }
    Runtime::start_with(
        "observed-docs",
        "Documents",
        None,
        Arc::new(misa_kernel::LocalKernel::new(
            misa_kernel::ScriptedProvider::always("unused"),
        )),
        "scripted",
        "test",
        Value::Null,
        contribution,
    )
}

#[tokio::test]
async fn preparation_catalogs_resolve_without_shared_panel_mutation() {
    let runtime = runtime();
    let panel = runtime.state.lock().unwrap().state.db().get("panel").cloned();
    let read = |id: &str, args: Vec<Value>| {
        let definition = runtime.query_exports().into_iter().find(|definition| definition.id == id).unwrap();
        let selection = Selection { scope: runtime.scope(), members: BTreeMap::from([("value".into(), Member {
            query: Query { id: id.into(), args }, contract: definition.contract,
            encoding: definition.result.encoding(), optional: false,
        })]) };
        let snapshot = runtime.read_selection(&selection).unwrap();
        match snapshot.members["value"].clone() { Content::Value(value) => value, _ => panic!("expected data") }
    };
    let sources: Vec<misa_proto::preparation::Source> = crate::wire::parse(&read(misa_proto::preparation::SOURCES, vec![])).unwrap();
    for source in sources { read(&source.member.query.id, source.member.query.args); }
    let shortcuts: Vec<misa_proto::preparation::Shortcut> = crate::wire::parse(&read(misa_proto::preparation::SHORTCUTS, vec![])).unwrap();
    assert!(shortcuts.iter().any(|shortcut| shortcut.id == "model"));
    for shortcut in shortcuts {
        match shortcut.target {
            misa_proto::preparation::Target::Read { member } => {
                let encoding = member.encoding;
                let snapshot = runtime.read_selection(&Selection { scope: runtime.scope(), members: BTreeMap::from([("report".into(), member)]) }).unwrap();
                assert!(matches!((&snapshot.members["report"], encoding),
                    (Content::Value(_), Encoding::Value) | (Content::Document(_), Encoding::Document)));
            }
            misa_proto::preparation::Target::Command { command } => assert!(runtime.command_registry.contains_key(&command)),
        }
    }
    assert_eq!(runtime.state.lock().unwrap().state.db().get("panel").cloned(), panel);
}

#[tokio::test]
async fn model_discovery_ignores_superseded_reports_and_accepts_discovered_ids() {
    let runtime = runtime();
    runtime.dispatch(Event::new("discovery/models.refresh"));
    runtime.dispatch(Event::new("discovery/models.refresh"));
    let report = |id: &str, model: &str| Event::new("kernel/models")
        .with("id", Value::str(id)).with("ok", Value::Bool(true))
        .with("models", Value::list([Value::map([("id", Value::str(model))])])) ;
    runtime.dispatch(report("models.1", "stale-model"));
    assert!(runtime.state.lock().unwrap().state.db().get("session").unwrap().get("catalogue").is_none());
    runtime.dispatch(report("models.2", "newly-served-model"));
    assert!(runtime.intent(crate::Intent::Command { name: "model".into(), args: Value::str("newly-served-model") }).is_empty());
    assert_eq!(runtime.state.lock().unwrap().state.db().get("session").unwrap().get("model").and_then(Value::as_str), Some("newly-served-model"));
}

#[tokio::test]
async fn presentation_catalog_is_an_ordinary_export_and_resolves_its_query_contracts() {
    let runtime = runtime();
    let definition = misa_proto::presentation::definition();
    let selection = Selection {
        scope: runtime.scope(),
        members: BTreeMap::from([(
            "catalog".into(),
            Member {
                query: Query::new(&definition.id),
                contract: definition.contract,
                encoding: definition.result.encoding(),
                optional: false,
            },
        )]),
    };
    let snapshot = runtime.read_selection(&selection).unwrap();
    let Content::Value(value) = &snapshot.members["catalog"] else {
        panic!()
    };
    let catalog: Vec<misa_proto::presentation::Presentation> = crate::wire::parse(value).unwrap();
    assert!(misa_proto::view::find(&runtime.view().unwrap(), "indicators").is_none(), "optional status was embedded in the canonical conversation");
    assert_eq!(
        catalog
            .iter()
            .map(|entry| entry.id.as_str())
            .collect::<Vec<_>>(),
        ["conversation", "status", "test.left", "test.right"]
    );
    let member = catalog.iter().find(|entry| entry.id == "test.left").unwrap().select(&[]).unwrap().member.clone();
    let document = runtime
        .read_selection(&Selection {
            scope: runtime.scope(),
            members: BTreeMap::from([("selected".into(), member)]),
        })
        .unwrap();
    assert!(
        matches!(&document.members["selected"], Content::Document(document) if document.tree.id == "left")
    );
}

fn selected(runtime: &Runtime, optional_left: bool, conversation: bool) -> Selection {
    let mut members: BTreeMap<_, _> = ["left", "right"]
        .into_iter()
        .map(|name| {
            (
                name.into(),
                Member {
                    query: Query::new(format!("test.{name}")),
                    contract: format!("test.{name}@1"),
                    encoding: Encoding::Document,
                    optional: name == "left" && optional_left,
                },
            )
        })
        .collect();
    if conversation {
        members.insert(
            "conversation".into(),
            Member {
                query: Query::new(CONVERSATION),
                contract: "conversation.presentation@1".into(),
                encoding: Encoding::Document,
                optional: false,
            },
        );
    }
    Selection {
        scope: runtime.scope(),
        members,
    }
}

fn set(runtime: &Runtime, name: &str, value: Value) {
    let faults = runtime.dispatch(
        Event::new("kernel/log.appended")
            .with("conversation", Value::str("observed-docs"))
            .with("seq", Value::Int(1))
            .with("kind", Value::str(crate::contribution::PATCH_KIND))
            .with(
                "data",
                Value::map([
                    ("path", Value::str(format!("guest.{name}"))),
                    ("patch", misa_value::Op::Set(value).to_value()),
                ]),
            ),
    );
    assert!(faults.is_empty(), "{faults:?}");
}

fn noise(runtime: &Runtime) {
    runtime
        .state
        .lock()
        .unwrap()
        .publications
        .commit(vec![StreamUpdate::Append {
            id: "irrelevant".into(),
            offset: 0,
            text: "token".into(),
        }]);
}

#[tokio::test]
async fn contributed_documents_are_distinct_from_each_other_and_the_conversation() {
    let runtime = runtime();
    let snapshot = runtime
        .read_selection(&selected(&runtime, false, true))
        .unwrap();
    for name in ["left", "right"] {
        let Content::Document(document) = &snapshot.members[name] else {
            panic!()
        };
        assert_eq!(document.tree.id, name);
        assert!(document.streams.is_empty());
    }
    let Content::Document(conversation) = &snapshot.members["conversation"] else {
        panic!()
    };
    assert_eq!(conversation.tree.id, "session");
}

#[tokio::test]
async fn optional_invalid_document_deduplicates_and_recovers_without_token_parsing() {
    let runtime = runtime();
    let selection = selected(&runtime, true, false);
    let handle = Handle {
        id: 3,
        generation: 1,
    };
    let (mut observation, initial) = runtime.observe(handle, selection.clone(), None).unwrap();
    let mut replica = Replica::new(handle, selection).unwrap();
    replica.apply(initial).unwrap();
    let parsed = DOCUMENT_PARSES.with(|count| count.get());
    for _ in 0..50 {
        noise(&runtime);
        assert!(observation.poll().is_none());
    }
    assert_eq!(
        DOCUMENT_PARSES.with(|count| count.get()),
        parsed,
        "unchanged documents are not parsed or materialized for tokens"
    );
    set(&runtime, "left", Value::Int(7));
    let faulted = observation.poll().unwrap();
    assert!(matches!(&faulted, Publication::Update { update, .. }
        if matches!(&update.members["left"], Delta::Replace { content: Content::Unavailable(_) }) && !update.members.contains_key("right")));
    replica.apply(faulted).unwrap();
    let parsed = DOCUMENT_PARSES.with(|count| count.get());
    for _ in 0..50 {
        noise(&runtime);
        assert!(observation.poll().is_none());
    }
    assert_eq!(
        DOCUMENT_PARSES.with(|count| count.get()),
        parsed,
        "unchanged invalid documents retain their validated fault"
    );
    set(&runtime, "left", document("left", "Recovered"));
    replica.apply(observation.poll().unwrap()).unwrap();
    assert!(matches!(
        &replica.current().unwrap()["left"],
        MemberState::Document(_)
    ));
}

#[tokio::test]
async fn required_fault_preserves_cursor_and_recovers_the_whole_selection() {
    let runtime = runtime();
    let selection = selected(&runtime, false, false);
    let handle = Handle {
        id: 4,
        generation: 1,
    };
    let (mut observation, initial) = runtime.observe(handle, selection.clone(), None).unwrap();
    let mut replica = Replica::new(handle, selection).unwrap();
    replica.apply(initial).unwrap();
    let before = observation.resume.clone();
    set(&runtime, "left", Value::Bool(false));
    assert!(matches!(
        observation.poll(),
        Some(Publication::Fault { .. })
    ));
    assert_eq!(observation.resume, before);
    set(
        &runtime,
        "right",
        document("right", "Changed while faulted"),
    );
    assert!(observation.poll().is_none());
    assert_eq!(observation.resume, before);
    set(&runtime, "left", document("left", "Recovered"));
    let recovered = observation.poll().unwrap();
    assert!(
        matches!(&recovered, Publication::Recovered { recovered, .. } if recovered.members.len() == 2)
    );
    replica.apply(recovered).unwrap();
    let MemberState::Document(right) = &replica.current().unwrap()["right"] else {
        panic!()
    };
    assert!(
        matches!(&right.tree().snapshot().kind, misa_proto::view::Kind::Status { text } if text == "Changed while faulted")
    );
}

#[tokio::test]
async fn future_publication_resume_is_rejected_before_observation_registration() {
    let runtime = runtime();
    let selection = selected(&runtime, false, false);
    let resume = Resume {
        selection: selection.clone(),
        publication: Some(u64::MAX),
        documents: BTreeMap::new(),
    };
    assert!(
        runtime
            .observe(
                Handle {
                    id: 5,
                    generation: 1
                },
                selection,
                Some(resume)
            )
            .is_err()
    );
}
