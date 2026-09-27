use super::super::flow::FlowId;
use super::super::{DocumentUpdate, FieldKind, Value, layout, tests as ui_tests};
use super::*;
use misa_proto::sync::{Stream, StreamUpdate};
use misa_proto::view::{BlobRef, Definition, Field, Span};

fn capture(ui: &DocumentUi, id: &FlowId, width: f32) -> Result<OwnerSnapshot, SnapshotError> {
    let mut budget = SNAPSHOT_BUDGET;
    OwnerSnapshot::capture(ui, id, width, &mut budget)
}

fn parity(app: &mut DocumentUi, id: FlowId) {
    if let FlowId::Node(owner) = &id {
        if let Some(count) = app.document.row_count(owner) {
            for index in 0..count {
                parity(app, FlowId::Row(owner.clone(), index));
            }
            parity(app, FlowId::End(owner.clone()));
        }
    }
    for width in [140.0, 600.0] {
        for light in [false, true] {
            app.set_light(light);
            // Bypass cache between widths/themes; this is the same production path
            // FlowSource uses to populate the retained owner height.
            app.retained.clear();
            let theme = if light { Theme::light() } else { Theme::dark() };
            let expected = {
                let mut builder = layout::LayoutBuilder::new(
                    &app.document,
                    &mut app.drafts,
                    &mut app.interaction,
                    &mut app.retained,
                    &mut app.overlays,
                    app.metrics.as_ref(),
                    light,
                );
                builder.measure_flow(&id, width, &theme).unwrap()
            };
            let key = id.cache_key();
            let expected_ops = format!("{:?}", app.retained.cached(&key).ops);
            let snapshot = capture(app, &id, width).unwrap();
            let rendered = render(snapshot, ui_tests::test_metrics());
            assert_eq!(
                rendered.height, expected,
                "{id:?}, width={width}, light={light}"
            );
            // The cached display list must match too: a snapshot that measures
            // right but paints differently is still wrong.
            assert_eq!(
                format!("{:?}", rendered.retained.cached(&key).ops),
                expected_ops,
                "display list parity for {id:?}, width={width}, light={light}"
            );
        }
    }
}

#[test]
fn fragmented_row_snapshot_matches_decoded_image_and_edited_field() {
    let image = Node::new(
        "image",
        Kind::Image {
            blob: BlobRef {
                hash: "row-image".into(),
                len: 16,
                media: None,
            },
            alt: "preview".into(),
            width: 2,
            height: 2,
        },
    )
    .id("row.image");
    let field = Node::new(
        "form",
        Kind::Fields {
            fields: vec![Field {
                id: "body".into(),
                label: "Body".into(),
                value: "initial".into(),
                hint: None,
                kind: FieldKind::Block,
                read_only: false,
                secret: false,
            }],
        },
    )
    .id("row.field");
    let mut app = DocumentUi::new(
        Node::new(
            "list",
            Kind::List {
                ordered: false,
                items: vec![vec![image, field]],
                markers: vec![],
            },
        )
        .id("list"),
        ui_tests::test_metrics(),
    );
    let row = FlowId::Row("list".into(), 0);
    parity(&mut app, row.clone());
    app.image("row-image".into(), Arc::new(image::RgbaImage::new(2, 2)));
    app.frame(140, 240);
    app.focus_control(Some(super::super::Control::Field {
        node: "row.field".into(),
        field: "body".into(),
    }));
    app.drive(
        misa_window_core::Event::Text("\nsecond line".into()),
        std::time::Duration::ZERO,
    );
    app.frame(140, 240);
    parity(&mut app, row);
}

#[test]
fn table_tail_snapshot_uses_columns_from_offscreen_rows() {
    let mut app = DocumentUi::new(
        Node::new(
            "table",
            Kind::Table {
                head: vec![vec![Span::plain("head")]],
                rows: vec![
                    vec![
                        vec![Span::plain("one")],
                        vec![Span::plain("two")],
                        vec![Span::plain("three")],
                    ],
                    vec![vec![Span::plain(
                        "tail wraps when it gets a third of the width",
                    )]],
                ],
                align: vec![],
            },
        )
        .id("table"),
        ui_tests::test_metrics(),
    );
    assert_eq!(app.document.table_columns("table"), 3);
    parity(&mut app, FlowId::Row("table".into(), 2));
    app.frame(140, 220);
    let tail = app
        .viewport
        .visible()
        .iter()
        .find(|p| p.id == FlowId::Row("table".into(), 2))
        .unwrap();
    let snapshot = capture(&app, &tail.id, 100.0).unwrap();
    assert_eq!(measure(snapshot, ui_tests::test_metrics()), tail.height);
}

#[test]
fn every_kind_and_composite_has_exact_snapshot_height() {
    let spans = || {
        vec![Span::plain(
            "long line 界 with different glyph widths and word wrapping",
        )]
    };
    let kinds = [
        Kind::Section,
        Kind::Text { spans: spans() },
        Kind::Heading {
            level: 2,
            spans: spans(),
        },
        Kind::Quote,
        Kind::Rule,
        Kind::Code {
            lang: Some("rust".into()),
            text: "let x = 1;\nprintln!(\"long line\");".into(),
        },
        Kind::List {
            ordered: true,
            items: vec![vec![Node::text("plain", spans()).id("item")]],
            markers: vec![Some(true)],
        },
        Kind::Table {
            head: vec![spans(), spans()],
            rows: vec![vec![spans(), spans()]],
            align: vec![],
        },
        Kind::Definition {
            entries: vec![Definition {
                term: spans(),
                definitions: vec![spans(), spans()],
            }],
        },
        Kind::Fields {
            fields: vec![Field {
                id: "value".into(),
                label: "Value".into(),
                value: "initial".into(),
                hint: None,
                kind: FieldKind::Block,
                read_only: false,
                secret: false,
            }],
        },
        Kind::Collapsible { summary: spans() },
        Kind::Image {
            blob: BlobRef {
                hash: "missing".into(),
                len: 4,
                media: None,
            },
            alt: "alt text".into(),
            width: 8,
            height: 8,
        },
        Kind::Status {
            text: "working".into(),
        },
        Kind::Meter {
            label: "used".into(),
            value: 3.0,
            max: 4.0,
        },
        Kind::Fact {
            value: Value::from(42),
        },
    ];
    for (index, kind) in kinds.into_iter().enumerate() {
        let mut app = DocumentUi::new(
            Node::new("case", kind)
                .id("owner")
                .label("Label")
                .child(Node::text("plain", spans()).id("child")),
            ui_tests::test_metrics(),
        );
        if index == 9 {
            app.drafts
                .insert("owner", "value", "\nunsent\nmultiline\ndraft");
        }
        if index == 10 {
            app.interaction.toggle_disclosure("owner");
        }
        parity(&mut app, FlowId::Node("owner".into()));
    }
    for role in ["status.indicators", "message.group.footer", "queue"] {
        let mut app = DocumentUi::new(
            Node::section(role)
                .id("owner")
                .child(Node::text("queue.item", spans()).id("child")),
            ui_tests::test_metrics(),
        );
        parity(&mut app, FlowId::Node("owner".into()));
    }
    let mut app = DocumentUi::new(
        Node::section("status.indicators").id("empty"),
        ui_tests::test_metrics(),
    );
    let empty = capture(&app, &FlowId::Node("empty".into()), 140.0).unwrap();
    assert_eq!(measure(empty, ui_tests::test_metrics()), 0.0);
    parity(&mut app, FlowId::Node("empty".into()));
}

#[test]
fn nested_disclosures_secret_drafts_and_owner_isolation() {
    let secret = "not-to-be-queued-界\nsecret";
    let mut app = DocumentUi::new(
        Node::section("session").id("session").children([
            Node::new(
                "card",
                Kind::Collapsible {
                    summary: vec![Span::plain("outer")],
                },
            )
            .id("owner")
            .child(
                Node::new(
                    "inner",
                    Kind::Collapsible {
                        summary: vec![Span::plain("inner")],
                    },
                )
                .id("inner")
                .child(
                    Node::new(
                        "form",
                        Kind::Fields {
                            fields: vec![Field {
                                id: "password".into(),
                                label: "Secret".into(),
                                value: "server-secret".into(),
                                hint: None,
                                kind: FieldKind::Block,
                                read_only: false,
                                secret: true,
                            }],
                        },
                    )
                    .id("form"),
                ),
            ),
            Node::text("unrelated", [Span::plain("must not enter snapshot")]).id("sibling"),
        ]),
        ui_tests::test_metrics(),
    );
    app.drafts.insert("form", "password", secret);
    app.interaction.toggle_disclosure("owner");
    app.interaction.toggle_disclosure("inner");
    let snapshot = capture(&app, &FlowId::Node("owner".into()), 140.0).unwrap();
    let debug = format!("{:?}", snapshot.root);
    assert!(!debug.contains(secret));
    assert!(!debug.contains("server-secret"));
    assert!(!debug.contains("must not enter snapshot"));
    parity(&mut app, FlowId::Node("owner".into()));
    app.interaction.toggle_disclosure("inner");
    parity(&mut app, FlowId::Node("owner".into()));
}

#[test]
fn choice_boolean_readonly_and_secret_choice_drafts() {
    use misa_proto::view::Choice;
    let choice = || FieldKind::Choice {
        options: vec![
            Choice {
                value: "alpha".into(),
                label: "First".into(),
                detail: None,
                metadata: None,
            },
            Choice {
                value: "beta".into(),
                label: "Second".into(),
                detail: None,
                metadata: None,
            },
        ],
        selected: Some("alpha".into()),
    };
    let field = |id: &str, kind: FieldKind, secret: bool, read_only: bool| Field {
        id: id.into(),
        label: id.into(),
        value: "alpha".into(),
        hint: None,
        kind,
        secret,
        read_only,
    };
    let mut app = DocumentUi::new(
        Node::new(
            "form",
            Kind::Fields {
                fields: vec![
                    field("choice", choice(), false, false),
                    field("secret_choice", choice(), true, false),
                    field("bool", FieldKind::Bool, false, false),
                    field("readonly", FieldKind::Block, false, true),
                    field("inline", FieldKind::Inline, false, false),
                ],
            },
        )
        .id("owner"),
        ui_tests::test_metrics(),
    );
    app.drafts.cycle("owner", "choice", &app.document);
    app.drafts.insert("owner", "secret_choice", "hidden");
    app.drafts.insert("owner", "inline", "unsent draft");
    let snapshot = capture(&app, &FlowId::Node("owner".into()), 140.0).unwrap();
    assert!(!format!("{:?}", snapshot.root).contains("hidden"));
    parity(&mut app, FlowId::Node("owner".into()));
}

#[test]
fn decoded_and_missing_images_and_live_markdown_streams() {
    let blob = BlobRef {
        hash: "picture".into(),
        len: 12,
        media: None,
    };
    let view = Node::section("session").id("session").child(
        Node::section("transcript")
            .id("transcript")
            .label("Transcript")
            .child(
                Node::new(
                    "image",
                    Kind::Image {
                        blob,
                        alt: "alt".into(),
                        width: 40,
                        height: 90,
                    },
                )
                .id("picture"),
            ),
    );
    let mut app = DocumentUi::new(view.clone(), ui_tests::test_metrics());
    parity(&mut app, FlowId::Node("picture".into()));
    let image = Arc::new(image::RgbaImage::new(40, 90));
    app.image("picture".into(), image.clone());
    let snapshot = capture(&app, &FlowId::Node("picture".into()), 140.0).unwrap();
    assert!(Arc::ptr_eq(snapshot.images.get("picture").unwrap(), &image));
    parity(&mut app, FlowId::Node("picture".into()));
    let streams = [Stream { id: "turn.text".into(), role: "message.assistant.text".into(), text: "# Heading\n\nA long paragraph with *emphasis* and 界 glyphs\n\n- item one\n- item two".into() },
                   Stream { id: "turn.thinking".into(), role: "message.assistant.thinking".into(), text: "a\nb\nc\nd".into() }];
    app.observed(&DocumentUpdate::Reset {
        tree: &view,
        streams: &streams,
    })
    .unwrap();
    for id in [
        FlowId::Stream("turn.text".into()),
        FlowId::Stream("turn.thinking".into()),
        FlowId::Node("streams".into()),
        FlowId::Node("transcript".into()),
    ] {
        parity(&mut app, id);
    }
    app.interaction.toggle_disclosure("turn.thinking");
    parity(&mut app, FlowId::Stream("turn.thinking".into()));
    parity(&mut app, FlowId::Node("transcript".into()));
    app.observed(&DocumentUpdate::Changed {
        tree: &[],
        live: &[StreamUpdate::Append {
            id: "turn.text".into(),
            offset: streams[0].text.len(),
            text: "\n\n| a | b |\n| - | - |\n| 界 | longer cell |".into(),
        }],
        reset_live: false,
    })
    .unwrap();
    parity(&mut app, FlowId::Stream("turn.text".into()));
    parity(&mut app, FlowId::Node("transcript".into()));
}

#[test]
fn non_atomic_ids_fail_instead_of_guessing() {
    let app = DocumentUi::new(
        Node::section("session").id("session"),
        ui_tests::test_metrics(),
    );
    assert_eq!(
        capture(&app, &FlowId::Top, 140.0).err(),
        Some(SnapshotError::NotAtomic)
    );
    assert_eq!(
        capture(&app, &FlowId::Node("session".into()), 140.0).err(),
        Some(SnapshotError::MissingOwner)
    );
    assert_eq!(
        capture(&app, &FlowId::Stream("missing".into()), 140.0).err(),
        Some(SnapshotError::MissingOwner)
    );
    let empty_group = capture(&app, &FlowId::Node("streams".into()), 140.0).unwrap();
    assert_eq!(measure(empty_group, ui_tests::test_metrics()), 5.0);
}

#[test]
fn a_read_only_secret_choice_masks_the_value_the_painter_shows() {
    use misa_proto::view::Choice;
    // The painter defaults a Choice to its `selected` value, not `field.value`.
    // A masked read-only choice must carry exactly that many bullets.
    let selected = "a-selected-value-much-longer-than-value";
    let mut app = DocumentUi::new(
        Node::new(
            "form",
            Kind::Fields {
                fields: vec![Field {
                    id: "token".into(),
                    label: "Token".into(),
                    value: "short".into(),
                    hint: None,
                    kind: FieldKind::Choice {
                        options: vec![Choice {
                            value: selected.into(),
                            label: "Production account".into(),
                            detail: None,
                            metadata: None,
                        }],
                        selected: Some(selected.into()),
                    },
                    read_only: true,
                    secret: true,
                }],
            },
        )
        .id("owner"),
        ui_tests::test_metrics(),
    );
    let snapshot = capture(&app, &FlowId::Node("owner".into()), 600.0).unwrap();
    let debug = format!("{:?}", snapshot.root);
    assert!(!debug.contains(selected));
    assert!(!debug.contains("Production account"));
    assert!(debug.contains(&"•".repeat(selected.chars().count())));
    // Two widths: the wrong bullet count would wrap differently and diverge.
    parity(&mut app, FlowId::Node("owner".into()));
}

#[test]
fn snapshot_budgets_are_explicit_and_never_approximate() {
    let app = DocumentUi::new(
        Node::new(
            "card",
            Kind::Text {
                spans: vec![Span::plain("measured")],
            },
        )
        .id("owner"),
        ui_tests::test_metrics(),
    );
    let mut tiny = 8;
    assert_eq!(
        OwnerSnapshot::capture(&app, &FlowId::Node("owner".into()), 140.0, &mut tiny).err(),
        Some(SnapshotError::Budget)
    );
    let huge = Node::new(
        "card",
        Kind::Code {
            lang: None,
            text: "x".repeat(SNAPSHOT_BUDGET * 2),
        },
    )
    .id("owner");
    let app = DocumentUi::new(huge, ui_tests::test_metrics());
    let mut budget = SNAPSHOT_BUDGET;
    assert_eq!(
        OwnerSnapshot::capture(&app, &FlowId::Node("owner".into()), 140.0, &mut budget).err(),
        Some(SnapshotError::Oversize)
    );
    assert_eq!(budget, SNAPSHOT_BUDGET, "an oversize owner spends nothing");
}

#[test]
fn every_visible_stream_counts_before_any_is_cloned() {
    let view = Node::section("session").id("session").child(
        Node::section("transcript")
            .id("transcript")
            .label("Transcript"),
    );
    let streams: Vec<Stream> = (0..20)
        .map(|index| Stream {
            id: format!("turn.{index}.text"),
            role: "message.assistant.text".into(),
            text: "p".repeat(5_000),
        })
        .collect();
    let mut app = DocumentUi::new(view.clone(), ui_tests::test_metrics());
    app.observed(&DocumentUpdate::Reset {
        tree: &view,
        streams: &streams,
    })
    .unwrap();
    // The synthetic stream group carries every projection, so its atomic owner
    // must be refused instead of cloning tens of kilobytes per stream.
    let mut budget = SNAPSHOT_BUDGET;
    assert_eq!(
        OwnerSnapshot::capture(&app, &FlowId::Node("streams".into()), 140.0, &mut budget).err(),
        Some(SnapshotError::Oversize)
    );
    // One projection alone is still an ordinary owner.
    assert!(capture(&app, &FlowId::Stream("turn.0.text".into()), 140.0).is_ok());
}
