use super::super::flow::FlowId;
use super::super::{DocumentUpdate, FieldKind, Value, layout, tests as ui_tests};
use super::*;
use misa_proto::sync::{Stream, StreamUpdate};
use misa_proto::view::{BlobRef, Definition, Field, Span};

fn parity(app: &mut DocumentUi, id: FlowId) {
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
                builder
                    .measure_owner(
                        match &id {
                            FlowId::Node(id) | FlowId::Stream(id) => id,
                            _ => unreachable!(),
                        },
                        width,
                        &theme,
                    )
                    .unwrap()
            };
            let snapshot = HeightSnapshot::capture(app, &id, width).unwrap();
            assert_eq!(
                measure(snapshot, ui_tests::test_metrics()),
                expected,
                "{id:?}, width={width}, light={light}"
            );
        }
    }
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
    let empty = HeightSnapshot::capture(&app, &FlowId::Node("empty".into()), 140.0).unwrap();
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
    let snapshot = HeightSnapshot::capture(&app, &FlowId::Node("owner".into()), 140.0).unwrap();
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
    let snapshot = HeightSnapshot::capture(&app, &FlowId::Node("owner".into()), 140.0).unwrap();
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
    let snapshot = HeightSnapshot::capture(&app, &FlowId::Node("picture".into()), 140.0).unwrap();
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
        HeightSnapshot::capture(&app, &FlowId::Top, 140.0).err(),
        Some(SnapshotError::NotAtomic)
    );
    assert_eq!(
        HeightSnapshot::capture(&app, &FlowId::Node("session".into()), 140.0).err(),
        Some(SnapshotError::MissingOwner)
    );
    assert_eq!(
        HeightSnapshot::capture(&app, &FlowId::Stream("missing".into()), 140.0).err(),
        Some(SnapshotError::MissingOwner)
    );
    let empty_group =
        HeightSnapshot::capture(&app, &FlowId::Node("streams".into()), 140.0).unwrap();
    assert_eq!(measure(empty_group, ui_tests::test_metrics()), 5.0);
}
