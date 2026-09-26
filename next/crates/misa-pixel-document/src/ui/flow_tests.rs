use super::Command;
use super::document::DocumentStore;
use super::flow::FlowId;
use super::{Control, DocumentUi, DocumentUpdate};
use misa_pixel_ui::{FlowPosition, Op};
use misa_proto::sync::{Stream, StreamUpdate, ViewOp};
use misa_proto::view::{Field, FieldKind, Kind, Node, Span};
use misa_window_core::Key;

fn select(app: &mut DocumentUi, first: &str, first_column: usize, last: &str, last_column: usize) {
    let point = |text: &str, column: usize| {
        let row = app
            .interaction
            .rows()
            .iter()
            .find(|r| r.geometry.text == text)
            .expect("visible row");
        (row.x + row.edge(column) + 0.1, row.y + 1.0)
    };
    let (x, y) = point(first, first_column);
    let (end_x, end_y) = point(last, last_column);
    app.pointer(x, y, false);
    app.pointer(end_x, end_y, true);
}

#[test]
fn selection_is_not_a_frame_row_index_after_scroll() {
    let mut app = DocumentUi::new(
        Node::section("session")
            .id("session")
            .children((0..100).map(|i| {
                Node::text("text", [Span::plain(format!("item {i}"))]).id(format!("item.{i}"))
            })),
        super::tests::test_metrics(),
    );
    app.viewport.anchor(FlowId::Node("item.0".into()), 0.0, 0.0);
    app.frame(320, 90);
    select(&mut app, "item 0", 0, "item 0", 4);
    assert_eq!(app.selected_text(), "item");
    app.viewport
        .anchor(FlowId::Node("item.75".into()), 0.0, 0.0);
    let scene = app.frame(320, 90);
    assert_eq!(app.key(Key::Copy), vec![Command::Copy("item".into())]);
    assert!(
        !scene.ops.iter().any(|op| matches!(op, Op::ClipRect { .. })),
        "unrelated owners must not be highlighted"
    );
}

#[test]
fn soft_wrap_selection_uses_source_offsets_not_line_numbers() {
    let mut app = DocumentUi::new(
        Node::text("text", [Span::plain("alpha beta gamma")]).id("body"),
        super::tests::test_metrics(),
    );
    app.frame(95, 200);
    select(&mut app, "alpha ", 0, "beta ", 4);
    assert_eq!(app.selected_text(), "alpha beta");
    let scene = app.frame(320, 200);
    assert_eq!(app.key(Key::Copy), vec![Command::Copy("alpha beta".into())]);
    assert!(scene.ops.iter().any(|op| matches!(op, Op::ClipRect { ops, .. } if ops.iter().any(|op| matches!(op, Op::Text { text, .. } if text == "alpha beta")))));
}

#[test]
fn table_cell_selection_tracks_source_across_interleaved_reflow() {
    let table = Node::new(
        "table",
        Kind::Table {
            head: vec![vec![Span::plain("header")], vec![Span::plain("other")]],
            rows: vec![vec![
                vec![Span::plain("alpha beta gamma")],
                vec![Span::plain("end")],
            ]],
            align: vec![],
        },
    )
    .id("table");
    let mut app = DocumentUi::new(table, super::tests::test_metrics());
    app.frame(170, 240);
    select(&mut app, "beta ", 0, "beta ", 4);
    assert_eq!(app.selected_text(), "beta");
    let scene = app.frame(400, 240);
    assert_eq!(app.key(Key::Copy), vec![Command::Copy("beta".into())]);
    assert!(scene.ops.iter().any(|op| matches!(op, Op::ClipRect { ops, .. } if ops.iter().any(|op| matches!(op, Op::Text { text, .. } if text == "beta")))));
}

#[test]
fn wrapped_table_cell_copies_contiguous_source_and_measures_like_placement() {
    let table = Node::new(
        "table",
        Kind::Table {
            head: vec![vec![Span::plain("heading")]],
            rows: vec![vec![vec![Span::plain("alpha beta gamma")]]],
            align: vec![],
        },
    )
    .id("table");
    let mut app = DocumentUi::new(table.clone(), super::tests::test_metrics());
    app.frame(100, 240);
    select(&mut app, "alpha ", 0, "gamma", 5);
    assert_eq!(
        app.key(Key::Copy),
        vec![Command::Copy("alpha beta gamma".into())]
    );

    let mut measured = DocumentUi::new(table, super::tests::test_metrics());
    let theme = misa_render::Theme::dark();
    let mut builder = super::layout::LayoutBuilder::new(
        &measured.document,
        &mut measured.drafts,
        &mut measured.interaction,
        &mut measured.retained,
        &mut measured.overlays,
        measured.metrics.as_ref(),
        false,
    );
    assert_eq!(
        builder.measure_owner("table", 60.0, &theme),
        Some(app.retained.cached("table").height)
    );
    assert_eq!(
        builder.retained.cached("table").geometry.rows().len(),
        app.retained.cached("table").geometry.rows().len()
    );
    assert!(builder.interaction.hits().is_empty());
}

#[test]
fn explicit_breaks_keep_source_positions_after_rewrap() {
    let mut app = DocumentUi::new(
        Node::text("text", [Span::plain("alpha beta\ngamma delta")]).id("body"),
        super::tests::test_metrics(),
    );
    app.frame(350, 200);
    select(&mut app, "gamma delta", 0, "gamma delta", 5);
    let before = app.interaction.selection();
    app.frame(95, 200);
    assert_eq!(app.interaction.selection(), before);
    assert_eq!(app.key(Key::Copy), vec![Command::Copy("gamma".into())]);
}

#[test]
fn selection_rewraps_and_survives_hidden_disclosure_then_retires_on_replacement() {
    let body = Node::text("text", [Span::plain("alpha beta gamma delta epsilon")]).id("body");
    let view = Node::new(
        "details",
        Kind::Collapsible {
            summary: vec![Span::plain("details")],
        },
    )
    .id("details")
    .child(body);
    let mut app = DocumentUi::new(view, super::tests::test_metrics());
    app.activate(Control::Disclosure("details".into()));
    app.frame(400, 220);
    select(
        &mut app,
        "alpha beta gamma delta epsilon",
        6,
        "alpha beta gamma delta epsilon",
        10,
    );
    assert_eq!(app.selected_text(), "beta");
    app.frame(125, 220);
    assert_eq!(app.key(Key::Copy), vec![Command::Copy("beta".into())]);
    app.activate(Control::Disclosure("details".into()));
    app.frame(125, 220);
    assert_eq!(app.selected_text(), "beta");
    app.activate(Control::Disclosure("details".into()));
    app.frame(400, 220);
    assert_eq!(app.selected_text(), "beta");
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Replace {
            id: "body".into(),
            node: Node::text("text", [Span::plain("other")]).id("body"),
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    app.frame(400, 220);
    assert!(app.selected_text().is_empty());
    assert!(app.key(Key::Copy).is_empty());
}

#[test]
fn replaced_queue_item_retires_composite_selection() {
    let mut app = DocumentUi::new(
        Node::section("queue")
            .id("queue")
            .child(Node::text("queue.item", [Span::plain("queued")]).id("item")),
        super::tests::test_metrics(),
    );
    app.frame(320, 180);
    select(&mut app, "queued", 0, "queued", 6);
    assert_eq!(app.key(Key::Copy), vec![Command::Copy("queued".into())]);
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Replace {
            id: "item".into(),
            node: Node::text("queue.item", [Span::plain("updated")]).id("item"),
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert!(app.selected_text().is_empty());
    assert!(app.key(Key::Copy).is_empty());
}

#[test]
fn clicking_disclosure_anchors_its_header_not_a_later_visible_message() {
    let details = Node::new(
        "details",
        Kind::Collapsible {
            summary: vec![Span::plain("Open details")],
        },
    )
    .id("details")
    .child(Node::text("text", [Span::plain("expanded ".repeat(150))]).id("long"));
    let mut app = DocumentUi::new(
        Node::section("session")
            .id("session")
            .children((0..20).map(|n| {
                Node::text("text", [Span::plain(format!("prior {n}"))]).id(format!("prior.{n}"))
            }))
            .child(details)
            .child(Node::text("text", [Span::plain("later message")]).id("later")),
        super::tests::test_metrics(),
    );
    app.frame(320, 180);
    let disclosure = Control::Disclosure("details".into());
    let (x, before_y) = app
        .control_center(&disclosure)
        .expect("disclosure in followed tail");
    let later_y = app
        .viewport
        .visible()
        .iter()
        .find(|p| p.id == FlowId::Node("later".into()))
        .unwrap()
        .y;
    app.pointer(x, before_y, false);
    app.frame(320, 180);
    let (_, after_y) = app
        .control_center(&disclosure)
        .expect("clicked header stays visible");
    assert!(
        (after_y - before_y).abs() < 0.01,
        "header moved from {before_y} to {after_y}"
    );
    assert!(
        app.viewport
            .visible()
            .iter()
            .any(|p| p.id == FlowId::Node("details".into()))
    );
    assert!(
        app.viewport
            .visible()
            .iter()
            .all(|p| p.id != FlowId::Node("later".into()) || p.y > later_y)
    );
}

#[test]
fn context_menu_disclosure_anchors_right_clicked_header_on_enter() {
    let details = Node::new(
        "details",
        Kind::Collapsible {
            summary: vec![Span::plain("Open details")],
        },
    )
    .id("details")
    .child(Node::text("text", [Span::plain("expanded ".repeat(150))]).id("long"));
    let mut app = DocumentUi::new(
        Node::section("session")
            .id("session")
            .children((0..20).map(|n| {
                Node::text("text", [Span::plain(format!("prior {n}"))]).id(format!("prior.{n}"))
            }))
            .child(details)
            .child(Node::text("text", [Span::plain("later message")]).id("later")),
        super::tests::test_metrics(),
    );
    app.frame(320, 180);
    let disclosure = Control::Disclosure("details".into());
    let (x, before_y) = app
        .control_center(&disclosure)
        .expect("header in followed tail");
    assert!(matches!(app.viewport.position, FlowPosition::FollowTail));
    app.drive(
        misa_window_core::Event::ContextMenu { x, y: before_y },
        std::time::Duration::ZERO,
    );
    assert!(app.menu.is_some());
    app.drive(
        misa_window_core::Event::Key(Key::Down),
        std::time::Duration::ZERO,
    );
    app.drive(
        misa_window_core::Event::Key(Key::Enter { newline: false }),
        std::time::Duration::ZERO,
    );
    assert!(app.interaction.is_expanded("details"));
    app.frame(320, 180);
    let (_, after_y) = app
        .control_center(&disclosure)
        .expect("header remains visible");
    assert!(
        (after_y - before_y).abs() < 0.01,
        "header moved from {before_y} to {after_y}"
    );
}

#[test]
fn removed_owner_cannot_copy_stale_selection() {
    let mut app = DocumentUi::new(
        Node::section("session")
            .id("session")
            .child(Node::text("text", [Span::plain("gone")]).id("gone"))
            .child(Node::text("text", [Span::plain("survives")]).id("survives")),
        super::tests::test_metrics(),
    );
    app.frame(320, 180);
    select(&mut app, "gone", 0, "gone", 4);
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Remove { id: "gone".into() }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    app.frame(320, 180);
    assert!(app.selected_text().is_empty());
    assert!(app.key(Key::Copy).is_empty());
}

#[test]
fn multi_owner_selection_retains_order_and_retires_if_either_owner_changes() {
    let mut app = DocumentUi::new(
        Node::section("session").id("session").children([
            Node::text("text", [Span::plain("first")]).id("first"),
            Node::text("text", [Span::plain("second")]).id("second"),
        ]),
        super::tests::test_metrics(),
    );
    app.frame(320, 180);
    select(&mut app, "first", 2, "second", 3);
    assert_eq!(app.selected_text(), "rst\nsec");
    app.frame(150, 180);
    assert_eq!(app.key(Key::Copy), vec![Command::Copy("rst\nsec".into())]);
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Replace {
            id: "first".into(),
            node: Node::text("text", [Span::plain("changed")]).id("first"),
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert!(app.key(Key::Copy).is_empty());
}

#[test]
fn ten_thousand_transcript_tail_frames_visit_only_visible_owners() {
    let view = Node::section("session").id("session").child(
        Node::section("transcript")
            .id("transcript")
            .children((0..10_000).map(|i| {
                if i == 9_999 {
                    Node::new(
                        "details",
                        Kind::Collapsible {
                            summary: vec![Span::plain("details")],
                        },
                    )
                    .id(format!("message.{i}"))
                    .child(Node::text("text", [Span::plain("hidden content")]).id("body"))
                } else {
                    Node::text("message", [Span::plain(format!("row {i}"))])
                        .id(format!("message.{i}"))
                }
            })),
    );
    let mut app = DocumentUi::new(view, super::tests::test_metrics());
    let frame = app.frame(640, 240);
    assert!(frame.ops.len() < 32);
    assert!(
        app.retained.owner_counts().0 < 32,
        "tail measurement visited history"
    );
    assert!(
        app.retained.owner_counts().1 < 32,
        "tail placement visited history"
    );
    assert!(!app.retained.contains("message.0"));
    assert!(!app.retained.contains("session"));
    assert!(!app.retained.contains("transcript"));

    app.frame(640, 240);
    assert_eq!(app.retained.owner_counts().0, 0);
    assert!(app.retained.owner_counts().1 < 32);
    app.scroll(-120.0);
    assert!(
        (1..32).contains(&app.retained.owner_counts().0),
        "backward wheel must measure only newly intersecting owners"
    );
    app.frame(640, 240);
    assert!(app.retained.owner_counts().0 < 32);
    assert!(app.retained.owner_counts().1 < 32);

    let anchor = app.viewport.visible()[0].id.clone();
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Insert {
            parent: "transcript".into(),
            before: Some("message.0".into()),
            node: Node::text("message", [Span::plain("older")]).id("older"),
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    app.frame(640, 240);
    assert!(app.viewport.visible().iter().any(|p| p.id == anchor));
    assert!(app.retained.owner_counts().0 < 32);

    app.frame(580, 240);
    assert!(app.retained.owner_counts().0 < 32, "resize visited history");
    assert!(app.retained.owner_counts().1 < 32);
    app.viewport
        .anchor(FlowId::Node("message.9999".into()), 0.0, 30.0);
    app.frame(580, 240);
    app.activate(Control::Disclosure("message.9999".into()));
    app.frame(580, 240);
    assert!(
        (1..32).contains(&app.retained.owner_counts().0),
        "disclosure must remeasure its atomic owner, not history"
    );
    assert!(app.retained.owner_counts().1 < 32);
    assert!(matches!(app.viewport.position, FlowPosition::Anchor { .. }));
}

fn transcript(prefix: &str, count: usize) -> Node {
    Node::section("session")
        .id("session")
        .children((0..count).map(|i| {
            Node::text("message", [Span::plain(format!("{prefix} row {i}"))])
                .id(format!("{prefix}.{i}"))
        }))
}

#[test]
fn reset_without_surviving_reading_ids_uses_ordinal_not_tail() {
    for observed in [false, true] {
        let mut app = DocumentUi::new(transcript("old", 120), super::tests::test_metrics());
        app.viewport
            .anchor(FlowId::Node("old.55".into()), 4.0, 32.0);
        app.frame(400, 140);
        let (local_y, screen_y) = match &app.viewport.position {
            FlowPosition::Anchor {
                local_y, screen_y, ..
            } => (*local_y, *screen_y),
            _ => panic!("expected anchored reading position"),
        };
        let replacement = transcript("new", 120);
        if observed {
            app.observed(&DocumentUpdate::Reset {
                tree: &replacement,
                streams: &[],
            })
            .unwrap();
        } else {
            app.set_view(replacement);
        }
        assert_eq!(
            app.viewport.position,
            FlowPosition::Anchor {
                id: FlowId::Node("new.55".into()),
                local_y,
                screen_y,
            }
        );
        app.frame(400, 140);
        let placed = app
            .viewport
            .visible()
            .iter()
            .find(|p| p.id == FlowId::Node("new.55".into()))
            .unwrap();
        assert!((placed.y + local_y - screen_y).abs() < 0.01);
        assert!(
            app.retained.owner_counts().0 < 32,
            "reset frame measured all history"
        );
        app.frame(400, 140);
        assert_eq!(
            app.retained.owner_counts().0,
            0,
            "idle frame measured history"
        );
    }
}

#[test]
fn reset_clamps_reading_ordinal_to_last_owner_and_short_content_to_top() {
    let mut app = DocumentUi::new(transcript("old", 120), super::tests::test_metrics());
    app.viewport
        .anchor(FlowId::Node("old.55".into()), 3.0, 32.0);
    app.frame(400, 140);
    app.set_view(transcript("new", 1));
    assert!(
        matches!(&app.viewport.position, FlowPosition::Anchor { id: FlowId::Node(id), .. } if id == "new.0")
    );
    app.frame(400, 140);
    assert!(app.viewport.visible().iter().any(|p| p.id == FlowId::Top));
    assert!(
        app.viewport
            .visible()
            .iter()
            .any(|p| p.id == FlowId::Node("new.0".into()))
    );
    assert!(!matches!(app.viewport.position, FlowPosition::FollowTail));
}

#[test]
fn reset_prefers_nearest_surviving_visible_reading_owner() {
    let mut app = DocumentUi::new(transcript("old", 120), super::tests::test_metrics());
    app.viewport
        .anchor(FlowId::Node("old.55".into()), 4.0, 32.0);
    app.frame(400, 140);
    let survivor = app
        .viewport
        .visible()
        .iter()
        .find(|p| p.id == FlowId::Node("old.56".into()))
        .unwrap()
        .clone();
    let replacement = Node::section("session")
        .id("session")
        .children((0..120).map(|i| {
            if i == 56 {
                Node::text("message", [Span::plain("survivor")]).id("old.56")
            } else {
                Node::text("message", [Span::plain("new")]).id(format!("new.{i}"))
            }
        }));
    app.observed(&DocumentUpdate::Reset {
        tree: &replacement,
        streams: &[],
    })
    .unwrap();
    assert_eq!(
        app.viewport.position,
        FlowPosition::Anchor {
            id: survivor.id,
            local_y: 0.0,
            screen_y: survivor.y,
        }
    );
}

#[test]
fn suppressed_stream_tail_navigation_and_idle_frames_do_not_probe_hidden_owners() {
    let view = Node::section("session").id("session").children([
        Node::section("hidden").id("b"),
        Node::section("hidden").id("y"),
    ]);
    let mut streams = Vec::with_capacity(10_002);
    for owner in ["b", "y"] {
        for i in 0..5_000 {
            streams.push(Stream {
                id: format!("{owner}.{i:05}"),
                role: "message.assistant.thinking".into(),
                text: "x".into(),
            });
        }
    }
    for id in ["a.text", "z.text"] {
        streams.push(Stream {
            id: id.into(),
            role: "message.assistant".into(),
            text: "visible".into(),
        });
    }
    let mut app = DocumentUi::new(view.clone(), super::tests::test_metrics());
    app.observed(&DocumentUpdate::Reset {
        tree: &view,
        streams: &streams,
    })
    .unwrap();
    let checks = app.document.visibility_checks();
    for _ in 0..100 {
        assert_eq!(app.document.first_stream().as_deref(), Some("a.text"));
        assert_eq!(app.document.last_stream().as_deref(), Some("z.text"));
        assert_eq!(
            app.document.next_stream("a.text").as_deref(),
            Some("z.text")
        );
        assert_eq!(
            app.document.previous_stream("z.text").as_deref(),
            Some("a.text")
        );
        assert_eq!(app.document.next_stream("z.text"), None);
        assert_eq!(app.document.previous_stream("a.text"), None);
        assert!(!app.document.has_stream("b.00000"));
    }
    assert_eq!(
        app.document.visibility_checks(),
        checks,
        "queries scanned suppressed streams"
    );
    for _ in 0..3 {
        app.frame(640, 240);
        assert_eq!(
            app.document.visibility_checks(),
            checks,
            "idle frame scanned suppressed streams"
        );
    }
    let checks_before_remove = app.document.visibility_checks();
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Remove { id: "b".into() }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert_eq!(
        app.document.next_stream("a.text").as_deref(),
        Some("b.00000")
    );
    assert_eq!(
        app.document.previous_stream("z.text").as_deref(),
        Some("b.04999")
    );
    assert!(!app.document.has_stream("y.00000"));
    let checks = app.document.visibility_checks();
    assert_eq!(
        checks - checks_before_remove,
        1,
        "only the changed owner is checked"
    );
    assert_eq!(
        app.document.next_stream("b.04999").as_deref(),
        Some("z.text")
    );
    assert_eq!(app.document.visibility_checks(), checks);
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Insert {
            parent: "session".into(),
            before: Some("y".into()),
            node: Node::section("hidden").id("b"),
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert_eq!(app.document.visible_streams(), ["a.text", "z.text"]);
    assert!(!app.document.has_stream("b.04999"));
}

#[test]
fn stream_visibility_tracks_nested_insert_replace_remove_current_append_end_and_reset() {
    let root = Node::section("session").id("session");
    let mut doc = DocumentStore::new(root.clone());
    let streams = ["agent.thinking", "agent.text", "agent.other", "z.text"].map(|id| Stream {
        id: id.into(),
        role: if id.ends_with("thinking") {
            "message.assistant.thinking"
        } else {
            "message.assistant"
        }
        .into(),
        text: "initial".into(),
    });
    doc.observe(&DocumentUpdate::Reset {
        tree: &root,
        streams: &streams,
    })
    .unwrap();
    assert_eq!(
        doc.visible_streams(),
        ["agent.thinking", "agent.text", "agent.other", "z.text"]
    );
    assert_eq!(doc.first_stream().as_deref(), Some("agent.thinking"));
    assert_eq!(
        doc.next_stream("agent.thinking").as_deref(),
        Some("agent.text")
    );
    assert_eq!(
        doc.previous_stream("agent.text").as_deref(),
        Some("agent.thinking")
    );

    let nested = Node::section("wrapper")
        .id("wrapper")
        .child(Node::section("agent").id("agent"));
    doc.observe(&DocumentUpdate::Changed {
        tree: &[ViewOp::Insert {
            parent: "session".into(),
            before: None,
            node: nested,
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert_eq!(doc.visible_streams(), ["z.text"]);
    assert_eq!(doc.previous_stream("z.text"), None);
    doc.observe(&DocumentUpdate::Changed {
        tree: &[ViewOp::Replace {
            id: "wrapper".into(),
            node: Node::section("wrapper").id("wrapper"),
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert_eq!(
        doc.visible_streams(),
        ["agent.thinking", "agent.text", "agent.other", "z.text"]
    );
    doc.observe(&DocumentUpdate::Changed {
        tree: &[ViewOp::Replace {
            id: "wrapper".into(),
            node: Node::section("wrapper")
                .id("wrapper")
                .child(Node::section("agent").id("agent")),
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert_eq!(doc.visible_streams(), ["z.text"]);
    doc.observe(&DocumentUpdate::Changed {
        tree: &[ViewOp::Remove {
            id: "wrapper".into(),
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert_eq!(
        doc.visible_streams(),
        ["agent.thinking", "agent.text", "agent.other", "z.text"]
    );

    doc.observe(&DocumentUpdate::Changed {
        tree: &[],
        live: &[StreamUpdate::Current {
            stream: Stream {
                id: "agent.thinking".into(),
                role: "message.assistant.thinking".into(),
                text: String::new(),
            },
        }],
        reset_live: false,
    })
    .unwrap();
    assert_eq!(doc.first_stream().as_deref(), Some("agent.text"));
    doc.observe(&DocumentUpdate::Changed {
        tree: &[],
        live: &[StreamUpdate::Append {
            id: "agent.thinking".into(),
            offset: 0,
            text: "thinking".into(),
        }],
        reset_live: false,
    })
    .unwrap();
    assert_eq!(doc.first_stream().as_deref(), Some("agent.thinking"));
    doc.observe(&DocumentUpdate::Changed {
        tree: &[],
        live: &[StreamUpdate::End {
            id: "agent.text".into(),
        }],
        reset_live: false,
    })
    .unwrap();
    assert_eq!(
        doc.next_stream("agent.thinking").as_deref(),
        Some("agent.other")
    );
    doc.observe(&DocumentUpdate::Reset {
        tree: &root,
        streams: &[],
    })
    .unwrap();
    assert_eq!(doc.first_stream(), None);
    assert_eq!(doc.last_stream(), None);
    assert!(doc.visible_streams().is_empty());
}

#[test]
fn flow_frames_do_not_keep_streams_after_owner_insert_or_lose_them_after_remove() {
    let root = Node::section("session").id("session");
    let mut app = DocumentUi::new(root.clone(), super::tests::test_metrics());
    app.observed(&DocumentUpdate::Reset {
        tree: &root,
        streams: &[Stream {
            id: "agent.text".into(),
            role: "message.assistant".into(),
            text: "live".into(),
        }],
    })
    .unwrap();
    let shown = |app: &mut DocumentUi| {
        app.frame(640, 480);
        app.viewport
            .visible()
            .iter()
            .any(|placed| placed.id == FlowId::Stream("agent.text".into()))
    };
    assert!(shown(&mut app));
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Insert {
            parent: "session".into(),
            before: None,
            node: Node::text("message", [Span::plain("settled")]).id("agent"),
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert!(!shown(&mut app));
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Remove { id: "agent".into() }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert!(shown(&mut app));
}

#[test]
fn embedded_list_field_edit_invalidates_exact_owner_height() {
    let field = Node::new(
        "form",
        Kind::Fields {
            fields: vec![Field {
                id: "body".into(),
                label: "Body".into(),
                value: String::new(),
                hint: None,
                kind: FieldKind::Block,
                read_only: false,
                secret: false,
            }],
        },
    )
    .id("embedded");
    let list = Node::new(
        "list",
        Kind::List {
            ordered: false,
            items: vec![vec![field]],
            markers: vec![],
        },
    )
    .id("list");
    let mut app = DocumentUi::new(
        Node::section("session").id("session").child(list),
        super::tests::test_metrics(),
    );
    app.frame(640, 480);
    let height = app
        .viewport
        .visible()
        .iter()
        .find(|p| p.id == FlowId::Node("list".into()))
        .unwrap()
        .height;
    app.focus_control(Some(Control::Field {
        node: "embedded".into(),
        field: "body".into(),
    }));
    app.drive(
        misa_window_core::Event::Text("one\ntwo\nthree\nfour\nfive".into()),
        std::time::Duration::ZERO,
    );
    app.frame(640, 480);
    let changed = app
        .viewport
        .visible()
        .iter()
        .find(|p| p.id == FlowId::Node("list".into()))
        .unwrap()
        .height;
    assert!(changed > height);
}

#[test]
fn stream_projection_keeps_its_structural_spacing_without_a_cached_parent() {
    let mut app = DocumentUi::new(
        Node::section("session")
            .id("session")
            .child(Node::section("transcript").id("transcript")),
        super::tests::test_metrics(),
    );
    let view = app.document.snapshot();
    app.observed(&DocumentUpdate::Reset {
        tree: &view,
        streams: &[misa_proto::sync::Stream {
            id: "live.text".into(),
            role: "message.assistant".into(),
            text: "hello".into(),
        }],
    })
    .unwrap();
    app.frame(640, 480);
    let ids: Vec<_> = app
        .viewport
        .visible()
        .iter()
        .map(|p| p.id.clone())
        .collect();
    assert_eq!(
        ids,
        [
            FlowId::Top,
            FlowId::Stream("live.text".into()),
            FlowId::StreamClose,
            FlowId::Close("transcript".into()),
            FlowId::Close("session".into()),
            FlowId::Bottom,
        ]
    );
    assert_eq!(app.viewport.visible()[2].height, 5.0);
    assert!(!app.retained.contains("streams"));
    assert!(!app.retained.contains("transcript"));
    assert!(!app.retained.contains("session"));
}
