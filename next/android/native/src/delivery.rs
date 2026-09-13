//! JNI carries the same bounded operations as the wire; only canonical facts are persisted.
use misa_proto::{SessionEvent, SessionMsg, sync::ClientView};
use serde_json::{Value, json};

pub fn receive(
    view: &mut ClientView,
    message: &SessionMsg,
    mut persist: impl FnMut(&ClientView),
) -> Result<Option<Value>, String> {
    if !view.receive(message)? {
        return Ok(None);
    }
    let event = match message {
        SessionMsg::View {
            view: tree,
            version,
            ..
        } => {
            persist(view);
            json!({"kind":"view", "view":tree, "version":version})
        }
        SessionMsg::Changes { changes, .. } => {
            if !changes.is_empty() {
                persist(view);
            }
            json!({"kind":"changes", "changes":changes})
        }
        SessionMsg::Streams { streams } => json!({"kind":"streams", "streams":streams}),
        SessionMsg::Event {
            event: SessionEvent::Stream { update },
            ..
        } => json!({"kind":"stream", "update":update}),
        _ => return Ok(None),
    };
    Ok(Some(event))
}

#[cfg(test)]
mod tests {
    use super::*;
    use misa_proto::{
        Node, SubId,
        sync::{Stream, StreamUpdate, Version},
    };
    fn run(history: usize) -> (Vec<usize>, usize) {
        let tree = Node::section("root")
            .id("root")
            .children((0..history).map(|i| {
                Node::text(
                    "message.user",
                    [misa_proto::view::Span::plain("old text".repeat(100))],
                )
                .id(format!("msg.{i}"))
            }));
        let mut view = ClientView::restore(
            Version {
                epoch: "epoch".into(),
                rev: 1,
            },
            tree,
        )
        .unwrap();
        let mut writes = 0;
        let mut sizes = Vec::new();
        let current = SessionMsg::Streams {
            streams: vec![Stream {
                id: "live.text".into(),
                role: "message.assistant".into(),
                text: String::new(),
            }],
        };
        sizes.push(
            receive(&mut view, &current, |_| writes += 1)
                .unwrap()
                .unwrap()
                .to_string()
                .len(),
        );
        for offset in 0..100 {
            let message = SessionMsg::Event {
                seq: offset as u64,
                event: SessionEvent::Stream {
                    update: StreamUpdate::Append {
                        id: "live.text".into(),
                        offset,
                        text: "x".into(),
                    },
                },
            };
            sizes.push(
                receive(&mut view, &message, |_| writes += 1)
                    .unwrap()
                    .unwrap()
                    .to_string()
                    .len(),
            );
        }
        (sizes, writes)
    }
    #[test]
    fn stream_bytes_and_snapshot_writes_are_independent_of_history_size() {
        let small = run(1);
        let large = run(1000);
        eprintln!(
            "101 stream deliveries: {} bytes, {} snapshot writes, for both 1 and 1000 historical nodes",
            small.0.iter().sum::<usize>(),
            small.1
        );
        assert_eq!(small, large);
        assert_eq!(small.1, 0);
        assert!(small.0.iter().all(|bytes| *bytes < 180));
    }
    #[test]
    fn canonical_snapshot_is_saved_once_and_emitted_without_an_overlay() {
        let mut view = ClientView::default();
        let mut writes = 0;
        let message = SessionMsg::View {
            id: SubId(1),
            version: Version {
                epoch: "epoch".into(),
                rev: 1,
            },
            view: Node::section("root").id("root"),
        };
        let event = receive(&mut view, &message, |_| writes += 1)
            .unwrap()
            .unwrap();
        assert_eq!(writes, 1);
        assert_eq!(event["kind"], "view");
    }
}
