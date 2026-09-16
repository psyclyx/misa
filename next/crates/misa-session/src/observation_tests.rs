use std::{collections::BTreeMap, sync::Arc};

use misa_proto::{Query, observation::*, sync::StreamUpdate, wire::Intent};
use misa_protocol::observation::{MemberState, Replica};
use misa_reframe::Event;
use misa_value::Value;

use crate::{
    Runtime,
    observation::{CONVERSATION, SUMMARY},
};

fn runtime() -> Arc<Runtime> {
    Runtime::start(
        "observed",
        "Observed",
        None,
        Arc::new(misa_kernel::LocalKernel::new(
            misa_kernel::ScriptedProvider::always("unused"),
        )),
        "scripted",
        "test",
        Value::Null,
    )
}

fn selection(runtime: &Runtime) -> Selection {
    let exports = runtime.query_exports();
    Selection {
        scope: runtime.scope(),
        members: [("conversation", CONVERSATION), ("summary", SUMMARY)]
            .into_iter()
            .map(|(name, id)| {
                let definition = exports
                    .iter()
                    .find(|definition| definition.id == id)
                    .unwrap();
                (
                    name.into(),
                    Member {
                        query: Query::new(id),
                        contract: definition.contract.clone(),
                        encoding: definition.result.encoding(),
                        optional: false,
                    },
                )
            })
            .collect(),
    }
}

fn ack(runtime: &Runtime, seq: i64, role: &str, text: &str) {
    assert!(
        runtime
            .dispatch(
                Event::new("kernel/log.appended")
                    .with("conversation", Value::str("observed"))
                    .with("seq", Value::Int(seq))
                    .with("kind", Value::str("message"))
                    .with(
                        "data",
                        Value::map([
                            ("seq", Value::Int(seq)),
                            ("role", Value::str(role)),
                            ("text", Value::str(text)),
                            ("state", Value::str("done")),
                            ("attachments", Value::list([])),
                            ("calls", Value::list([])),
                        ])
                    )
            )
            .is_empty()
    );
}

fn start(runtime: &Runtime) -> String {
    assert!(
        runtime
            .intent(Intent::Prompt {
                text: "prompt".into(),
                attachments: vec![]
            })
            .is_empty()
    );
    ack(runtime, 1, "user", "prompt");
    runtime
        .state
        .lock()
        .unwrap()
        .state
        .db()
        .get("session")
        .unwrap()
        .get("pending")
        .unwrap()
        .get("request")
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

fn append(runtime: &Runtime, request: &str, text: &str) {
    assert!(
        runtime
            .dispatch(
                Event::new("kernel/provider.delta")
                    .with("id", Value::str(request))
                    .with("text", Value::str(text))
            )
            .is_empty()
    );
}

// Controlled acknowledgments run synchronously, before the spawned kernel runs.
#[tokio::test]
async fn composed_observation_settles_canonical_and_live_in_one_publication() {
    let runtime = runtime();
    let selection = selection(&runtime);
    let handle = Handle {
        id: 1,
        generation: 1,
    };
    let (mut observed, initial) = runtime.observe(handle, selection.clone(), None).unwrap();
    let mut replica = Replica::new(handle, selection).unwrap();
    replica.apply(initial).unwrap();
    let request = start(&runtime);
    replica.apply(observed.poll().unwrap()).unwrap();
    append(&runtime, &request, "é🙂");
    let Publication::Update { update, .. } = observed.poll().unwrap() else {
        panic!("expected update")
    };
    assert!(
        !update.members.contains_key("summary"),
        "token does not change domain summary"
    );
    assert!(
        matches!(&update.members["conversation"], Delta::Document { changes, streams }
        if changes.is_empty() && matches!(&streams[..], [StreamUpdate::Append { offset: 0, text, .. }] if text == "é🙂"))
    );
    replica
        .apply(Publication::Update { handle, update })
        .unwrap();
    ack(&runtime, 2, "assistant", "é🙂");
    let settled = observed.poll().unwrap();
    assert!(
        matches!(&settled, Publication::Update { update, .. } if matches!(&update.members["conversation"],
        Delta::Document { changes, streams } if !changes.is_empty() && streams.iter().any(|update| matches!(update, StreamUpdate::End { .. }))))
    );
    replica.apply(settled).unwrap();
    let MemberState::Document(document) = &replica.current().unwrap()["conversation"] else {
        panic!()
    };
    assert!(document.tree().contains("msg.2"));
    assert!(document.live().is_empty());
    assert!(observed.poll().is_none());
}

#[tokio::test]
async fn long_stream_recovery_retains_canonical_replay_and_resets_live() {
    let runtime = runtime();
    let request = start(&runtime);
    let selection = selection(&runtime);
    let handle = Handle {
        id: 2,
        generation: 1,
    };
    let (observed, initial) = runtime.observe(handle, selection.clone(), None).unwrap();
    let mut replica = Replica::new(handle, selection.clone()).unwrap();
    replica.apply(initial).unwrap();
    let checkpoint = replica.checkpoint(false).unwrap();
    let original_version = checkpoint.resume.documents["conversation"].clone();
    drop(observed);
    for _ in 0..600 {
        append(&runtime, &request, "é");
    }
    let handle = Handle {
        id: 2,
        generation: 2,
    };
    let mut restored = Replica::restore(handle, selection.clone(), checkpoint).unwrap();
    let (mut observed, recovered) = runtime
        .observe(handle, selection, Some(restored.resume()))
        .unwrap();
    assert!(
        matches!(&recovered, Publication::Recovered { recovered, .. } if matches!(&recovered.members["conversation"],
        Recovery::Document { from, changes, streams } if from == &original_version && changes.is_empty() && streams.iter().any(|s| s.text.len() == 1200)))
    );
    restored.apply(recovered).unwrap();
    append(&runtime, &request, "🙂");
    restored.apply(observed.poll().unwrap()).unwrap();
    let MemberState::Document(document) = &restored.current().unwrap()["conversation"] else {
        panic!()
    };
    assert_eq!(document.live()["msg.2.text"].text.len(), 1204);
    assert_eq!(document.version(), &original_version);
}

#[tokio::test]
async fn selection_validates_exports_arguments_and_incarnation() {
    let runtime = runtime();
    let mut selected = selection(&runtime);
    selected
        .members
        .get_mut("summary")
        .unwrap()
        .query
        .args
        .push(Value::Int(1));
    assert!(runtime.read_selection(&selected).is_err());
    selected = selection(&runtime);
    selected.scope.incarnation = "previous".into();
    assert!(runtime.read_selection(&selected).is_err());
    selected = selection(&runtime);
    selected.members = BTreeMap::from([(
        "private".into(),
        Member {
            query: Query::new("session.status"),
            contract: "session.status@1".into(),
            encoding: Encoding::Value,
            optional: false,
        },
    )]);
    assert!(runtime.read_selection(&selected).is_err());
}

#[tokio::test]
async fn subscribers_are_independent_and_stale_attempts_do_not_publish() {
    let runtime = runtime();
    let request = start(&runtime);
    let selected = selection(&runtime);
    let (first, _) = runtime
        .observe(
            Handle {
                id: 1,
                generation: 1,
            },
            selected.clone(),
            None,
        )
        .unwrap();
    let (mut second, _) = runtime
        .observe(
            Handle {
                id: 2,
                generation: 1,
            },
            selected,
            None,
        )
        .unwrap();
    drop(first);
    append(&runtime, "previous-attempt", "discard");
    assert!(second.poll().is_none());
    append(&runtime, &request, "retained");
    assert!(second.poll().is_some());
}
