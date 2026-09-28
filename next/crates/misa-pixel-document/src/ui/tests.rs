struct TestMetrics;
impl TextMetrics for TestMetrics {
    fn measure(&self, text: &str, _size: f32) -> f32 {
        text.chars()
            .map(|ch| match ch {
                'i' | 'l' | ' ' => 4.0,
                '界' | '終' => 18.0,
                'W' | 'M' => 13.0,
                _ => 9.0,
            })
            .sum()
    }
    fn advances(&self, text: &str, _size: f32) -> Vec<f32> {
        let mut widths = vec![0.0];
        for ch in text.chars() {
            widths.push(
                widths.last().unwrap()
                    + match ch {
                        'i' | 'l' | ' ' => 4.0,
                        '界' | '終' => 18.0,
                        'W' | 'M' => 13.0,
                        _ => 9.0,
                    },
            );
        }
        widths
    }
    fn line_metrics(&self, _size: f32) -> misa_pixel_ui::LineMetrics {
        misa_pixel_ui::LineMetrics {
            ascent: -15.0,
            descent: 4.0,
            leading: 2.0,
            line_height: 21.0,
        }
    }
}
pub(super) fn test_metrics() -> Arc<dyn TextMetrics> {
    Arc::new(TestMetrics)
}

fn scene_view() -> Node {
    Node::section("session").id("session").children([
        Node::section("message.user")
            .id("msg.1")
            .child(Node::text("message.user", [Span::plain("a question")]).id("question")),
        Node::new(
            "tool.result",
            Kind::Code {
                lang: Some("rust".into()),
                text: "let x = 1;".into(),
            },
        )
        .id("code"),
    ])
}

fn walk_ops(ops: &[Op], visit: &mut impl FnMut(&Op)) {
    for op in ops {
        visit(op);
        if let Op::Group { ops, .. } | Op::ClipRect { ops, .. } = op {
            walk_ops(ops, visit);
        }
    }
}

#[test]
fn every_kind_retains_the_same_owner_when_measured_without_placement() {
    use misa_proto::view::{BlobRef, Definition, Field};
    let span = || vec![Span::plain("a long measured line 界")];
    let child = || Node::text("plain", span()).id("nested");
    let kinds = [
        Kind::Section,
        Kind::Text { spans: span() },
        Kind::Heading {
            level: 1,
            spans: span(),
        },
        Kind::Quote,
        Kind::Rule,
        Kind::Code {
            lang: Some("rust".into()),
            text: "fn main() {}".into(),
        },
        Kind::List {
            ordered: true,
            items: vec![vec![child()]],
            markers: vec![None],
        },
        Kind::Table {
            head: vec![span()],
            rows: vec![vec![span()]],
            align: vec![],
        },
        Kind::Definition {
            entries: vec![Definition {
                term: span(),
                definitions: vec![span()],
            }],
        },
        Kind::Fields {
            fields: vec![Field {
                id: "value".into(),
                label: "Value".into(),
                value: "界".into(),
                hint: None,
                kind: FieldKind::Inline,
                read_only: false,
                secret: false,
            }],
        },
        Kind::Collapsible { summary: span() },
        Kind::Image {
            blob: BlobRef {
                hash: "missing".into(),
                len: 4,
                media: None,
            },
            alt: "image".into(),
            width: 2,
            height: 2,
        },
        Kind::Status {
            text: "working".into(),
        },
        Kind::Meter {
            label: "used".into(),
            value: 2.0,
            max: 4.0,
        },
        Kind::Fact {
            value: Value::from(42),
        },
    ];
    for (index, kind) in kinds.into_iter().enumerate() {
        let node = Node::new("test", kind)
            .id("owner")
            .label("Label")
            .child(child())
            .action(Action {
                id: "go".into(),
                on: ActionOn::Submit,
                label: None,
                args: Value::Null,
            });
        let mut standard = DocumentUi::new(node.clone(), test_metrics());
        standard.frame_at(640, 480, Duration::ZERO);
        let expected = standard.retained.cached("owner");
        let mut measured = DocumentUi::new(node, test_metrics());
        measured.retained.begin_frame(640, Duration::ZERO);
        let theme = Theme::dark();
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
            builder.measure_flow(&super::flow::FlowId::Node("owner".into()), 600.0, &theme),
            Some(expected.height),
            "kind {index}"
        );
        assert!(
            builder.interaction.hits().is_empty(),
            "measurement placed hits for kind {index}"
        );
        assert_eq!(
            format!("{:?}", builder.retained.cached("owner").ops),
            format!("{:?}", expected.ops),
            "kind {index}"
        );
        assert_eq!(
            builder.retained.cached("owner").geometry.rows().len(),
            expected.geometry.rows().len(),
            "kind {index}"
        );
        assert_eq!(
            builder.measure_flow(&super::flow::FlowId::Node("owner".into()), 600.0, &theme),
            Some(expected.height)
        );
        let retained_ops = builder.retained.cached("owner").ops.clone();
        drop(builder);
        measured.frame_at(640, 480, Duration::ZERO);
        assert!(
            Arc::ptr_eq(&retained_ops, &measured.retained.cached("owner").ops),
            "kind {index} laid out twice"
        );
        assert_eq!(
            format!("{:?}", measured.retained.cached("owner").ops),
            format!("{:?}", expected.ops)
        );
    }
    let mut empty = DocumentUi::new(
        Node::section("status.indicators").id("empty"),
        test_metrics(),
    );
    let theme = Theme::dark();
    let mut builder = super::layout::LayoutBuilder::new(
        &empty.document,
        &mut empty.drafts,
        &mut empty.interaction,
        &mut empty.retained,
        &mut empty.overlays,
        empty.metrics.as_ref(),
        false,
    );
    assert_eq!(
        builder.measure_flow(&super::flow::FlowId::Node("empty".into()), 600.0, &theme),
        Some(0.0)
    );
}

#[test]
fn measured_field_defers_viewport_updates_until_the_owner_is_placed() {
    let node = Node::new(
        "form",
        Kind::Fields {
            fields: vec![misa_proto::view::Field {
                id: "text".into(),
                label: "Text".into(),
                value: "short".into(),
                hint: None,
                kind: FieldKind::Inline,
                read_only: false,
                secret: false,
            }],
        },
    )
    .id("owner");
    let mut app = DocumentUi::new(node, test_metrics());
    app.retained.begin_frame(200, Duration::ZERO);
    let initial = FieldViewport {
        x: 10_000.0,
        line: 0,
    };
    app.drafts.set_viewport("owner", "text", initial);
    let theme = Theme::dark();
    let mut builder = super::layout::LayoutBuilder::new(
        &app.document,
        &mut app.drafts,
        &mut app.interaction,
        &mut app.retained,
        &mut app.overlays,
        app.metrics.as_ref(),
        false,
    );
    assert!(
        builder
            .measure_flow(&super::flow::FlowId::Node("owner".into()), 160.0, &theme)
            .is_some()
    );
    assert_eq!(builder.drafts.viewport("owner", "text"), initial);
    assert!(builder.interaction.hits().is_empty());
    drop(builder);
    app.frame_at(200, 480, Duration::ZERO);
    assert_eq!(app.drafts.viewport("owner", "text").x, 45.0);
}

#[test]
fn measured_composites_match_placed_owners_without_placing_hits() {
    let cases = [
        Node::section("status.indicators").id("owner").child(
            Node::new(
                "indicator.activity",
                Kind::Status {
                    text: "working".into(),
                },
            )
            .id("activity"),
        ),
        Node::section("message.group.footer").id("owner").child(
            Node::new(
                "fact",
                Kind::Fact {
                    value: Value::from(3),
                },
            )
            .id("fact"),
        ),
        Node::section("queue")
            .id("owner")
            .child(Node::text("queue.item", [Span::plain("queued")]).id("item")),
    ];
    for node in cases {
        let mut standard = DocumentUi::new(node.clone(), test_metrics());
        standard.frame_at(640, 480, Duration::ZERO);
        let mut measured = DocumentUi::new(node, test_metrics());
        let theme = Theme::dark();
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
            builder.measure_flow(&super::flow::FlowId::Node("owner".into()), 600.0, &theme),
            Some(standard.retained.cached("owner").height)
        );
        assert_eq!(
            format!("{:?}", builder.retained.cached("owner").ops),
            format!("{:?}", standard.retained.cached("owner").ops)
        );
        assert!(builder.interaction.hits().is_empty());
    }
}

#[test]
fn consecutive_frames_drop_quote_prefix_and_stale_hits() {
    let control = Control::Action {
        node: "quoted".into(),
        action: "open".into(),
    };
    let quoted = Node::new("markdown.quote", Kind::Quote).id("quote").child(
        Node::text("plain", [Span::plain("inside")])
            .id("quoted")
            .action(Action {
                id: "open".into(),
                on: ActionOn::Submit,
                label: None,
                args: Value::Null,
            }),
    );
    let mut app = DocumentUi::new(quoted, test_metrics());
    app.frame_at(400, 300, Duration::ZERO);
    assert!(
        app.interaction
            .rows()
            .iter()
            .any(|row| row.geometry.text.contains("inside"))
    );
    // A quote's rail is drawn beside the content; copied rows are content.
    assert!(
        app.interaction
            .rows()
            .iter()
            .all(|row| !row.geometry.text.contains('▏'))
    );
    assert!(app.control_center(&control).is_some());

    app.set_view(Node::text("plain", [Span::plain("outside")]).id("plain"));
    app.frame_at(400, 300, Duration::ZERO);
    assert!(
        app.interaction
            .rows()
            .iter()
            .any(|row| row.geometry.text == "outside")
    );
    assert!(
        app.interaction
            .rows()
            .iter()
            .all(|row| !row.geometry.text.contains('▏'))
    );
    assert!(app.control_center(&control).is_none());
}

#[test]
fn viewport_follows_new_output_but_report_wheel_and_offline_pin_do_not() {
    let view = |count| {
        Node::section("session")
            .id("session")
            .children((0..count).map(|i| {
                Node::text("message.user", [Span::plain(format!("row {i}"))]).id(format!("row.{i}"))
            }))
    };
    let mut app = DocumentUi::new(view(20), test_metrics());
    assert!(matches!(
        app.viewport.position,
        misa_pixel_ui::FlowPosition::FollowTail
    ));
    app.frame(400, 100);
    assert!(
        app.viewport
            .visible()
            .iter()
            .any(|p| p.id == super::flow::FlowId::Node("row.19".into()))
    );
    app.set_view(view(30));
    app.frame(400, 100);
    assert!(
        app.viewport
            .visible()
            .iter()
            .any(|p| p.id == super::flow::FlowId::Node("row.29".into()))
    );
    app.scroll(-50.0);
    let manual = app.viewport.visible()[0].id.clone();
    app.set_view(view(40));
    app.frame(400, 100);
    assert!(app.viewport.visible().iter().any(|p| p.id == manual));
    app.report("Local report".into(), Value::str("A report"));
    let position = app.viewport.position.clone();
    app.scroll(200.0);
    assert_eq!(app.viewport.position, position);
    app.key(Key::Escape);
    app.pin_to_top();
    app.frame(400, 100);
    assert!(
        app.viewport
            .visible()
            .iter()
            .any(|p| p.id == super::flow::FlowId::Top)
    );
    app.set_view(view(50));
    app.frame(400, 100);
    assert!(
        app.viewport
            .visible()
            .iter()
            .any(|p| p.id == super::flow::FlowId::Top)
    );
}

#[test]
fn frame_positions_nonempty_runs_and_uses_syntax_colours() {
    let mut app = DocumentUi::new(scene_view(), test_metrics());
    let scene = app.frame_at(800, 600, Duration::ZERO);
    assert_eq!((scene.width, scene.height), (800.0, 600.0));
    let mut runs = 0;
    let mut keyword = false;
    walk_ops(&scene.ops, &mut |op| match op {
        Op::Text {
            x, y, text, style, ..
        } => {
            assert!(*x >= 0.0 && *y >= 0.0);
            assert!(!text.is_empty());
            runs += 1;
            keyword |= *style == Theme::dark().token("keyword");
        }
        Op::Image { width, height, .. } | Op::Rect { width, height, .. } => {
            assert!(*width > 0.0 && *height > 0.0);
        }
        Op::Group { .. } | Op::ClipRect { .. } => {}
    });
    assert!(runs > 0);
    assert!(keyword, "no code run carried the keyword colour");
}

#[test]
fn narrow_code_clips_before_highlighting_but_keeps_fence_and_visible_colours() {
    let raw = "let x = 1; ".repeat(2000);
    let view = Node::new(
        "markdown.code",
        Kind::Code {
            lang: Some("rust".into()),
            text: format!("{raw}\nlet y = 2;"),
        },
    );
    let mut app = DocumentUi::new(view, test_metrics());
    let scene = app.frame(144, 180);
    assert_eq!(app.interaction.rows().len(), 3);
    assert_eq!(app.interaction.rows()[0].geometry.text, "rust");
    let visible = misa_pixel_ui::TextFlow::new(app.metrics.as_ref(), FONT_SIZE)
        .clip(&raw, 104.0)
        .0; // 144px frame minus the 40px document inset
    assert_eq!(app.interaction.rows()[1].geometry.text, visible);
    let last = misa_pixel_ui::TextFlow::new(app.metrics.as_ref(), FONT_SIZE)
        .clip("let y = 2;", 104.0)
        .0;
    assert_eq!(app.interaction.rows()[2].geometry.text, last);
    let theme = Theme::dark();
    assert_eq!(
        app.interaction.rows()[0].geometry.runs[0].0,
        theme.role("markdown.code.label")
    );
    assert!(
        app.interaction.rows()[1]
            .geometry
            .runs
            .iter()
            .any(|(style, text, _)| *style == theme.token("keyword") && text == "let")
    );
    assert!(
        app.interaction.rows()[1]
            .geometry
            .runs
            .iter()
            .any(|(style, text, _)| *style == theme.token("number") && text == "1")
    );
    assert!(any_op(
        &scene.ops,
        |op| matches!(op, Op::Text { text, style, .. }
        if text == "let" && *style == theme.token("keyword"))
    ));
    // The second line still uses its original byte offset, not the clipped
    // prefix length of the first line.
    assert!(
        app.interaction.rows()[2]
            .geometry
            .runs
            .iter()
            .any(|(style, text, _)| *style == theme.token("keyword") && text == "let")
    );
    app.key(Key::SelectAll);
    assert_eq!(
        app.selected_text(),
        format!(
            "rust\n{visible}\n{}",
            app.interaction.rows()[2].geometry.text
        )
    );
}

#[test]
fn frame_renders_quote_rule_meter_and_fact() {
    let view = Node::section("session").id("session").children([
        Node::new("markdown.quote", Kind::Quote)
            .id("quote")
            .child(Node::text("quote.body", [Span::plain("quoted")]).id("quote.body")),
        Node::new("markdown.rule", Kind::Rule).id("rule"),
        Node::new(
            "value.meter",
            Kind::Meter {
                label: "budget".into(),
                value: 2.0,
                max: 4.0,
            },
        )
        .id("meter"),
        Node::new(
            "value.tokens",
            Kind::Fact {
                value: Value::Int(12_400),
            },
        )
        .id("fact"),
    ]);
    let scene = DocumentUi::new(view, test_metrics()).frame_at(640, 480, Duration::ZERO);
    let mut texts = Vec::new();
    let mut rects = 0;
    let mut rules = 0;
    walk_ops(&scene.ops, &mut |op| match op {
        Op::Text { text, .. } => texts.push(text.clone()),
        Op::Rect { height, .. } if *height <= 1.0 => rules += 1,
        Op::Rect { .. } => rects += 1,
        _ => {}
    });
    assert!(texts.iter().any(|text| text.contains("quoted")));
    assert!(rules > 0, "a rule is drawn, never spelled with dashes");
    assert!(texts.iter().any(|text| text.contains("budget")));
    assert!(texts.iter().any(|text| text.contains("12k")));
    assert!(rects > 0);
}

#[test]
fn meter_widget_clips_bounded_fill_and_uses_palette_in_both_themes() {
    for (value, max, expected) in [
        (2.0, 4.0, 0.5),
        (-2.0, 4.0, 0.0),
        (8.0, 4.0, 1.0),
        (2.0, 0.0, 0.0),
        (f64::NAN, 4.0, 0.0),
    ] {
        let view = Node::new(
            "value.meter",
            Kind::Meter {
                label: "budget".into(),
                value,
                max,
            },
        )
        .id("meter");
        let mut app = DocumentUi::new(view, test_metrics());
        for light in [false, true] {
            app.set_light(light);
            let scene = app.frame(200, 120);
            let palette = crate::appearance::Palette::new(light);
            assert!(any_op(&scene.ops, |op| matches!(op, Op::Text { text, .. }
                if text.starts_with("budget: "))));
            assert!(any_op(
                &scene.ops,
                |op| matches!(op, Op::ClipRect { width: 160.0, height: 10.0, ops, .. }
                if matches!(&ops[0], Op::Rect { width: 160.0, style, .. } if *style == palette.meter)
                    && if expected == 0.0 { ops.len() == 1 } else {
                        matches!(&ops[1], Op::Rect { width, style, .. }
                            if *width == 160.0 * expected && *style == palette.accent)
                    })
            ));
        }
    }
}

#[test]
fn frame_theme_changes_styles_without_changing_the_view() {
    let mut app = DocumentUi::new(scene_view(), test_metrics());
    let dark = app.frame_at(800, 600, Duration::ZERO);
    app.set_light(true);
    let light = app.frame_at(800, 600, Duration::ZERO);
    let colours = |scene: &Scene| {
        let mut colours = Vec::new();
        walk_ops(&scene.ops, &mut |op| {
            if let Op::Text { style, .. } = op {
                colours.push(style.fg);
            }
        });
        colours
    };
    assert_ne!(colours(&dark), colours(&light));
}

#[test]
fn message_card_and_rail_paint_beneath_its_text() {
    let mut app = DocumentUi::new(scene_view(), test_metrics());
    app.frame_at(800, 600, Duration::ZERO);
    let ops = &app.retained.cached("msg.1").ops;
    let surface = Theme::dark().surface("message.user").unwrap();
    let card = ops
        .iter()
        .position(|op| matches!(op, Op::Rect { style, .. } if style.fg == surface.bg))
        .expect("no message card background");
    let rail = ops
        .iter()
        .position(|op| matches!(op, Op::Rect { width, .. } if *width == 2.0))
        .expect("no full-height rail");
    let text = ops
        .iter()
        .position(|op| matches!(op, Op::Group { .. }))
        .expect("no message content");
    assert!(card < rail && rail < text, "{card} {rail} {text}");
}

#[test]
fn only_a_painted_moving_indicator_animates() {
    let mut app = DocumentUi::new(
        Node::section("session")
            .id("session")
            .child(Node::section("turn").id("turn")),
        test_metrics(),
    );
    app.frame_at(640, 480, Duration::ZERO);
    assert!(
        !app.animating(),
        "a turn without a painted indicator is idle"
    );
    app.set_view(
        Node::section("session").id("session").child(
            Node::section("status.indicators").id("status").child(
                Node::new(
                    "indicator.activity",
                    Kind::Status {
                        text: "ready".into(),
                    },
                )
                .id("activity"),
            ),
        ),
    );
    app.frame_at(640, 480, Duration::ZERO);
    assert!(!app.animating(), "ready is not moving");
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Replace {
            id: "activity".into(),
            node: Node::new(
                "indicator.activity",
                Kind::Status {
                    text: "working".into(),
                },
            )
            .id("activity"),
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert!(!app.animating(), "an invalidated owner is not yet painted");
    app.frame_at(640, 480, Duration::ZERO);
    assert!(app.animating());
}

#[test]
fn elapsed_pulse_skips_wraps_and_preserves_unrelated_owners() {
    let mut app = DocumentUi::new(
        Node::section("session").id("session").children([
            Node::section("status.indicators").id("status").child(
                Node::new(
                    "indicator.activity",
                    Kind::Status {
                        text: "working".into(),
                    },
                )
                .id("activity"),
            ),
            Node::text("message.user", [Span::plain("retained")]).id("message"),
        ]),
        test_metrics(),
    );
    let ms = Duration::from_millis;
    app.frame_at(640, 480, ms(0));
    let status = app.retained.cached("status").ops.clone();
    let message = app.retained.cached("message").ops.clone();
    let pulse = |ops: &[Op]| -> String {
        fn collect(ops: &[Op], result: &mut String) {
            for op in ops {
                match op {
                    Op::Text { text, .. } => result.push_str(text),
                    Op::Group { ops, .. } | Op::ClipRect { ops, .. } => collect(ops, result),
                    _ => {}
                }
            }
        }
        let mut result = String::new();
        collect(ops, &mut result);
        result
    };
    let first = pulse(&status);
    let scene = app.frame_at(640, 480, ms(159));
    assert_eq!(app.retained.phase(), 0);
    assert!(Arc::ptr_eq(&status, &app.retained.cached("status").ops));
    assert_eq!(app.retained.rendered_nodes(), 0);
    let mut groups = 0;
    walk_ops(&scene.ops, &mut |op| {
        if matches!(op, Op::Group { .. }) {
            groups += 1;
        }
    });
    assert!(groups > 0);

    app.frame_at(640, 480, ms(320)); // Skip phase 1.
    assert_eq!(app.retained.phase(), 2);
    assert!(!Arc::ptr_eq(&status, &app.retained.cached("status").ops));
    assert!(!app.retained.contains("session"));
    assert_ne!(first, pulse(&app.retained.cached("status").ops));
    assert!(Arc::ptr_eq(&message, &app.retained.cached("message").ops));
    let phase_two = app.retained.cached("status").ops.clone();
    app.frame_at(640, 480, ms(960)); // Wrap to phase 2, no invalidation.
    assert_eq!(app.retained.phase(), 2);
    assert!(Arc::ptr_eq(&phase_two, &app.retained.cached("status").ops));
    app.frame_at(640, 480, ms(1120));
    assert_eq!(app.retained.phase(), 3);
    assert!(!Arc::ptr_eq(&phase_two, &app.retained.cached("status").ops));
    assert!(Arc::ptr_eq(&message, &app.retained.cached("message").ops));
    app.frame_at(640, 480, ms(1280));
    assert_eq!(app.retained.phase(), 0);
    assert_eq!(first, pulse(&app.retained.cached("status").ops));
}

#[test]
fn hidden_cached_indicator_does_not_keep_the_window_awake() {
    let mut app = DocumentUi::new(
        Node::section("session").id("session").child(
            Node::new(
                "details",
                Kind::Collapsible {
                    summary: vec![Span::plain("Details")],
                },
            )
            .id("details")
            .child(
                Node::section("status.indicators").id("status").child(
                    Node::new(
                        "indicator.activity",
                        Kind::Status {
                            text: "working".into(),
                        },
                    )
                    .id("activity"),
                ),
            ),
        ),
        test_metrics(),
    );
    app.frame_at(640, 480, Duration::ZERO);
    assert!(!app.animating());
    app.activate(Control::Disclosure("details".into()));
    app.frame_at(640, 480, Duration::ZERO);
    assert!(app.animating());
    let old = app.retained.cached("status").ops.clone();
    app.activate(Control::Disclosure("details".into()));
    app.frame_at(640, 480, Duration::from_millis(160));
    assert!(!app.animating());
    assert!(Arc::ptr_eq(&old, &app.retained.cached("status").ops));
    app.activate(Control::Disclosure("details".into()));
    app.frame_at(640, 480, Duration::from_millis(320));
    assert!(app.animating());
    assert!(!Arc::ptr_eq(&old, &app.retained.cached("status").ops));
}

#[test]
fn scrolling_a_moving_status_out_and_back_suspends_pulse_wakeups() {
    let mut transcript: Vec<_> = (0..70)
        .map(|i| {
            Node::text(
                "message.user",
                [Span::plain(format!(
                    "before {i}: {}",
                    "long text ".repeat(i % 5 + 8)
                ))],
            )
            .id(format!("before.{i}"))
        })
        .collect();
    transcript.push(
        Node::section("status.indicators").id("status").child(
            Node::new(
                "indicator.activity",
                Kind::Status {
                    text: "working".into(),
                },
            )
            .id("activity"),
        ),
    );
    transcript.extend((0..70).map(|i| {
        Node::text("message.user", [Span::plain(format!("after {i}"))]).id(format!("after.{i}"))
    }));
    let mut app = DocumentUi::new(
        Node::section("session").id("session").child(
            Node::section("transcript")
                .id("transcript")
                .children(transcript),
        ),
        test_metrics(),
    );
    let size = Size {
        width: 640,
        height: 240,
    };
    app.frame_at(size.width, size.height, Duration::ZERO);
    assert!(!app.retained.contains("status"));
    assert!(!app.animating());
    assert!(
        app.drive(Event::Redraw(size), Duration::from_millis(160))
            .deadline
            .is_none()
    );
    assert_eq!(app.retained.rendered_nodes(), 0);

    app.viewport
        .anchor(super::flow::FlowId::Node("status".into()), 0.0, 80.0);
    app.frame_at(size.width, size.height, Duration::from_millis(160));
    assert!(app.animating());
    let status = app.retained.cached("status").ops.clone();
    assert!(
        app.drive(Event::Redraw(size), Duration::from_millis(320))
            .deadline
            .is_some()
    );
    assert!(!Arc::ptr_eq(&status, &app.retained.cached("status").ops));

    app.scroll(100_000.0);
    assert!(
        !app.animating(),
        "wheel updates the visible pulse set before repaint"
    );
    app.frame_at(size.width, size.height, Duration::from_millis(480));
    assert!(!app.animating());
    assert!(
        app.drive(Event::Redraw(size), Duration::from_millis(640))
            .deadline
            .is_none()
    );
    assert_eq!(app.retained.rendered_nodes(), 0);
}

#[test]
fn idle_repaints_never_invalidate_retained_owners() {
    let mut app = DocumentUi::new(
        Node::section("session")
            .id("session")
            .child(Node::text("message.user", [Span::plain("idle")]).id("message")),
        test_metrics(),
    );
    app.frame_at(640, 480, Duration::ZERO);
    let owner = app.retained.cached("message").ops.clone();
    app.frame_at(640, 480, Duration::from_secs(10));
    assert!(!app.animating());
    assert_eq!(app.retained.rendered_nodes(), 0);
    assert!(Arc::ptr_eq(&owner, &app.retained.cached("message").ops));
}

#[test]
fn width_and_theme_invalidate_retained_geometry_but_idle_frames_do_not() {
    let mut app = DocumentUi::new(scene_view(), test_metrics());
    app.frame_at(640, 480, Duration::ZERO);
    let first = app.retained.cached("msg.1").ops.clone();
    app.frame_at(640, 480, Duration::from_secs(1));
    assert!(Arc::ptr_eq(&first, &app.retained.cached("msg.1").ops));
    assert_eq!(app.retained.rendered_nodes(), 0);

    app.frame_at(800, 480, Duration::from_secs(1));
    let wide = app.retained.cached("msg.1").ops.clone();
    assert!(!Arc::ptr_eq(&first, &wide));
    assert_eq!(app.retained.cached("msg.1").width, 760.0);
    app.set_light(true);
    app.frame_at(800, 480, Duration::from_secs(1));
    assert!(!Arc::ptr_eq(&wide, &app.retained.cached("msg.1").ops));
    app.frame_at(800, 480, Duration::from_secs(1));
    assert_eq!(app.retained.rendered_nodes(), 0);
}

#[test]
fn changing_local_theme_preserves_drafts_and_rebuilds_cached_colors() {
    let mut app = DocumentUi::new(form("panel.input", FieldKind::Inline), test_metrics());
    app.frame(640, 480);
    app.drive(Event::Text("private draft".into()), Duration::ZERO)
        .commands;
    let dark = app.frame(640, 480);
    app.set_light(true);
    let light = app.frame(640, 480);
    assert_eq!(
        app.field_text("panel.input", "value"),
        Some("private draft")
    );
    assert_ne!(dark.ops, light.ops);
    app.set_light(false);
    assert_eq!(dark.ops, app.frame(640, 480).ops);
}
use super::*;
use misa_proto::view::{Action, Field, Span};
use misa_value::Value;
#[test]
fn command_picker_filters_navigates_and_inserts_without_sending() {
    let mut view = form("compose", FieldKind::Inline);
    if let Kind::Fields { fields } = &mut view.kind {
        fields[0].id = "prompt".into();
    }
    let mut app = DocumentUi::new(view, test_metrics());
    app.declare_commands(serde_json::from_value(
        serde_json::json!([{ "id":"model", "label":"Model" }, { "id":"clear", "label":"Clear" }]),
    )
    .unwrap());
    app.key(Key::Commands);
    app.key(Key::Down);
    assert_eq!(
        app.overlays.picker().unwrap().selected().unwrap().value,
        "clear"
    );
    app.drive(Event::Text("mod".into()), Duration::ZERO)
        .commands;
    assert_eq!(app.overlays.picker().unwrap().matches().len(), 1);
    assert!(app.key(Key::Enter { newline: false }).is_empty());
    assert_eq!(app.field_text("compose", "prompt"), Some("/model "));
    assert!(app.overlays.picker().is_none());
    app.key(Key::Commands);
    app.drive(Event::Text("zzzz".into()), Duration::ZERO)
        .commands;
    assert!(app.key(Key::Enter { newline: false }).is_empty());
    assert!(app.overlays.picker().is_some());
    app.key(Key::Escape);
    assert_eq!(app.field_text("compose", "prompt"), Some("/model "));
}
#[test]
fn empty_picker_and_modal_input_preserve_draft() {
    let mut app = DocumentUi::new(form("form", FieldKind::Inline), test_metrics());
    app.frame(900, 720);
    app.key(Key::Commands);
    app.drive(Event::Text("query".into()), Duration::ZERO)
        .commands;
    assert!(app.pointer(80.0, 55.0, false).is_empty());
    assert!(app.key(Key::Enter { newline: false }).is_empty());
    let scene = app.frame(900, 720);
    assert!(any_op(
        &scene.ops,
        |op| matches!(op,Op::Text{text,..} if text == "No matching commands")
    ));
    app.key(Key::Escape);
    assert!(app.overlays.picker().is_none());
}
fn any_op(ops: &[Op], predicate: impl Fn(&Op) -> bool + Copy) -> bool {
    ops.iter().any(|op| {
        predicate(op)
            || matches!(op,Op::Group {ops,..} | Op::ClipRect {ops,..} if any_op(ops,predicate))
    })
}
#[test]
fn registered_composite_receives_its_indexed_subtree() {
    let view = Node::section("status.indicators").id("indicators").child(
        Node::section("indicator.model").id("model").child(
            Node::new(
                "value.text",
                Kind::Fact {
                    value: Value::str("scripted/model"),
                },
            )
            .id("model.value"),
        ),
    );
    let mut app = DocumentUi::new(view, test_metrics());
    let scene = app.frame(900, 120);
    assert!(any_op(
        &scene.ops,
        |op| matches!(op, Op::Text { text, .. } if text.contains("scripted/model"))
    ));
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Replace {
            id: "model.value".into(),
            node: Node::new(
                "value.text",
                Kind::Fact {
                    value: Value::str("changed/model"),
                },
            )
            .id("model.value"),
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    let scene = app.frame(900, 120);
    assert!(any_op(
        &scene.ops,
        |op| matches!(op, Op::Text { text, .. } if text.contains("changed/model"))
    ));
}
#[test]
fn status_footer_and_queue_components_render_natively() {
    let view = Node::section("session").id("session").children([
        Node::section("status.indicators").id("status").child(
            Node::section("indicator.model")
                .id("model")
                .label("model")
                .child(
                    Node::new(
                        "value.text",
                        Kind::Fact {
                            value: Value::str("claude"),
                        },
                    )
                    .id("model.value"),
                ),
        ),
        Node::section("message.group.footer")
            .id("footer")
            .child(Node::text("value.text", [Span::plain("12k tok")]).id("footer.tokens")),
        Node::section("queue").id("queue").children([
            Node::new(
                "queue.count",
                Kind::Fact {
                    value: Value::Int(2),
                },
            )
            .id("queue.count"),
            Node::text("queue.item", [Span::plain("first")]).id("queue.item.1"),
            Node::text("queue.item", [Span::plain("second")]).id("queue.item.2"),
        ]),
    ]);
    let mut app = DocumentUi::new(view, test_metrics());
    let scene = app.frame(900, 400);
    assert!(any_op(
        &scene.ops,
        |op| matches!(op, Op::Text { text, .. } if text == "claude")
    ));
    assert!(any_op(
        &scene.ops,
        |op| matches!(op, Op::Text { text, .. } if text == "12k tok")
    ));
    assert!(any_op(
        &scene.ops,
        |op| matches!(op, Op::Text { text, .. } if text == "Queued (2)")
    ));
    assert!(any_op(
        &scene.ops,
        |op| matches!(op, Op::Text { text, .. } if text == "first")
    ));
}

#[test]
fn semantic_action_button_grows_with_its_label_and_clips_only_without_room() {
    let label = "Run the whole thing";
    let view = Node::section("root").id("root").action(Action {
        id: "go".into(),
        on: ActionOn::Submit,
        label: Some(label.into()),
        args: Value::Null,
    });
    let mut app = DocumentUi::new(view, test_metrics());
    let measured = test_metrics().measure(label, FONT_SIZE);
    fn label_clip(ops: &[Op], label: &str) -> Option<f32> {
        ops.iter().find_map(|op| match op {
            Op::Group { ops, .. } => label_clip(ops, label),
            Op::ClipRect { width, ops, .. } => match &ops[0] {
                Op::Text { text, .. } if text == label => Some(*width),
                _ => label_clip(ops, label),
            },
            _ => None,
        })
    }
    // With room the button grows to its label: nothing is cut.
    let scene = app.frame_at(400, 200, Duration::ZERO);
    let hit = app
        .interaction
        .hits()
        .iter()
        .find(|hit| matches!(&hit.control, Control::Action { action, .. } if action == "go"))
        .unwrap();
    assert_eq!(hit.width, measured + 24.0);
    assert_eq!(label_clip(&scene.ops, label), Some(measured));
    assert!(hit.contains(hit.x + hit.width - 1.0, hit.y + 1.0));
    assert!(!hit.contains(hit.x + hit.width, hit.y + 1.0));

    // Out of room the allowance wins, and the hit bounds follow it.
    let scene = app.frame_at(60, 200, Duration::ZERO);
    let hit = app
        .interaction
        .hits()
        .iter()
        .find(|hit| matches!(&hit.control, Control::Action { action, .. } if action == "go"))
        .unwrap();
    assert_eq!(hit.width, 40.0);
    assert_eq!(label_clip(&scene.ops, label), Some(16.0));
}

fn form(id: &str, kind: FieldKind) -> Node {
    Node::new(
        "panel",
        Kind::Fields {
            fields: vec![Field {
                id: "value".into(),
                label: "Value".into(),
                value: String::new(),
                hint: None,
                kind,
                read_only: false,
                secret: false,
            }],
        },
    )
    .id(id)
    .action(Action {
        id: "answer".into(),
        on: ActionOn::Submit,
        label: None,
        args: Value::Null,
    })
}
// Locate the painted field viewport (not a text row or the outer document group).
// Coordinates are translated through cached groups as the painter sees them.
fn field_paint(scene: &Scene) -> (f32, f32, f32, f32, String, f32, f32, f32) {
    fn find(ops: &[Op], dx: f32, dy: f32) -> Option<(f32, f32, f32, f32, String, f32, f32, f32)> {
        for op in ops {
            match op {
                Op::Group { x, y, ops } => {
                    if let Some(result) = find(ops, dx + x, dy + y) {
                        return Some(result);
                    }
                }
                Op::ClipRect {
                    x,
                    y,
                    width,
                    height,
                    ops,
                } => {
                    // Clip children keep the outer coordinate space.
                    if let Some(result) = find(ops, dx, dy) {
                        return Some(result);
                    }
                    if let Some((cx, cy)) = ops.iter().find_map(|op| match op {
                        Op::Rect {
                            x, y, width: 1.5, ..
                        } => Some((*x + dx, *y + dy)),
                        _ => None,
                    }) {
                        let painted = ops
                            .iter()
                            .filter_map(|op| match op {
                                Op::Text { text, .. } => Some(text.as_str()),
                                _ => None,
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        let run_x = ops
                            .iter()
                            .find_map(|op| match op {
                                Op::Text { x, y, .. } if *y + dy == cy => Some(x + dx),
                                _ => None,
                            })
                            .expect("painted cursor line");
                        return Some((x + dx, y + dy, *width, *height, painted, cx, cy, run_x));
                    }
                }
                _ => {}
            }
        }
        None
    }
    find(&scene.ops, 0.0, 0.0).expect("focused field clip and caret")
}

#[test]
fn inline_field_scrolls_to_measured_cursor_and_home_restores_start() {
    let mut app = DocumentUi::new(form("one", FieldKind::Inline), test_metrics());
    let draft = "W界".repeat(25);
    app.drive(Event::Text(draft.clone()), Duration::ZERO);
    let end = field_paint(&app.frame(160, 400));
    assert_eq!(app.field_text("one", "value"), Some(draft.as_str()));
    assert_eq!(end.4, draft);
    assert!(end.5 >= end.0 && end.5 + 1.5 <= end.0 + end.2 + 0.01);
    assert!(app.drafts.viewport("one", "value").x > 0.0);
    app.drive(Event::Key(Key::Home), Duration::ZERO);
    let home = field_paint(&app.frame(160, 400));
    assert_eq!(home.5, home.0);
    assert_eq!(app.drafts.viewport("one", "value").x, 0.0);
    app.drive(Event::Key(Key::End), Duration::ZERO);
    let end_again = field_paint(&app.frame(160, 400));
    assert!((end_again.5 - end.5).abs() < 0.01);
    app.drive(Event::Text("終".into()), Duration::ZERO);
    let typed = field_paint(&app.frame(160, 400));
    assert!(typed.5 + 1.5 <= typed.0 + typed.2 + 0.01);
    assert!(typed.5 >= typed.0);
    assert!(app.field_text("one", "value").unwrap().ends_with("終"));
    let offset = app.drafts.viewport("one", "value").x;
    assert!((typed.7 - (typed.0 - offset)).abs() < 0.01);
    assert!(
        (typed.5
            - (typed.7
                + test_metrics().measure(app.field_text("one", "value").unwrap(), FONT_SIZE)))
        .abs()
            < 0.01
    );
    assert!(typed.5 - test_metrics().measure("終", FONT_SIZE) >= typed.0);
}

#[test]
fn secret_field_scroll_and_caret_use_bullet_advances_only() {
    let mut view = form("one", FieldKind::Inline);
    let Kind::Fields { fields } = &mut view.kind else {
        unreachable!()
    };
    fields[0].secret = true;
    let mut app = DocumentUi::new(view, test_metrics());
    app.drive(Event::Text("W界".repeat(20)), Duration::ZERO);
    let paint = field_paint(&app.frame(160, 400));
    assert_eq!(paint.4, "•".repeat(40));
    assert!(!paint.4.contains('W'));
    let offset = app.drafts.viewport("one", "value").x;
    assert!((paint.7 - (paint.0 - offset)).abs() < 0.01);
    assert!((paint.5 - (paint.7 + 40.0 * 9.0)).abs() < 0.01);
    assert!(paint.5 + 1.5 <= paint.0 + paint.2 + 0.01);
    app.key(Key::Home);
    let home = field_paint(&app.frame(160, 400));
    assert_eq!(home.5, home.0);
    assert_eq!(
        app.field_text("one", "value"),
        Some("W界".repeat(20).as_str())
    );
}

#[test]
fn capped_block_field_scrolls_lines_and_caret_inside_clip() {
    let mut app = DocumentUi::new(form("one", FieldKind::Block), test_metrics());
    let draft = (0..24)
        .map(|n| format!("line {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    app.drive(Event::Text(draft.clone()), Duration::ZERO);
    let bottom = field_paint(&app.frame(300, 500));
    assert_eq!(bottom.3, 171.0); // 180px box, inset by 6px top and 3px bottom
    assert!(bottom.4.contains("line 23"));
    assert!(!bottom.4.contains("line 0\n"));
    assert!(bottom.6 >= bottom.1 && bottom.6 + 21.0 <= bottom.1 + bottom.3);
    app.key(Key::Home);
    let home = field_paint(&app.frame(300, 500));
    assert_eq!(home.5, home.0);
    assert_eq!(home.6, bottom.6);
    app.drafts
        .editor_mut("one", "value")
        .unwrap()
        .move_cursor(Motion::First);
    app.invalidate("one");
    let top = field_paint(&app.frame(300, 500));
    assert!(top.4.starts_with("line 0"));
    assert!(!top.4.contains("line 23"));
    assert_eq!(top.5, top.0);
    assert_eq!(top.6, top.1);
    assert_eq!(app.field_text("one", "value"), Some(draft.as_str()));
}

#[test]
fn reports_and_rejected_prompts_preserve_local_typing() {
    let mut view = form("composer", FieldKind::Inline);
    let Kind::Fields { fields } = &mut view.kind else {
        unreachable!()
    };
    fields[0].id = "prompt".into();
    let mut app = DocumentUi::new(view.clone(), test_metrics());
    app.focus_control(Some(Control::Field {
        node: "composer".into(),
        field: "prompt".into(),
    }));
    app.drive(Event::Text("new draft".into()), Duration::ZERO)
        .commands;
    app.report(
        "Status".into(),
        Value::map([("needs_input", Value::Bool(true))]),
    );
    assert!(
        app.drive(
            Event::Text("must not reach composer".into()),
            Duration::ZERO
        )
        .commands
        .is_empty()
    );
    let scene = app.frame(800, 600);
    assert!(any_op(
        &scene.ops,
        |op| matches!(op, Op::Text {text, ..} if text == "needs input: yes")
    ));
    assert!(app.key(Key::Escape).is_empty());
    assert_eq!(app.field_text("composer", "prompt"), Some("new draft"));
    app.reject_prompt(
        "previous submission".into(),
        "Reply lost; execution is uncertain".into(),
    );
    assert_eq!(app.field_text("composer", "prompt"), Some("new draft"));
    assert_eq!(
        app.key(Key::Copy),
        vec![Command::Copy("previous submission".into())]
    );
    app.key(Key::Escape);
    app.drafts
        .editor_mut("composer", "prompt")
        .unwrap()
        .set_text("");
    app.reject_prompt("restored".into(), "Rejected".into());
    assert_eq!(app.field_text("composer", "prompt"), Some("restored"));
    assert_eq!(
        app.document.snapshot(),
        view,
        "local reports must not change authoritative content"
    );
}
#[test]
fn drafts_survive_updates_and_submit_only_the_target_panel() {
    let view = Node::section("root")
        .child(form("one", FieldKind::Inline))
        .child(form("two", FieldKind::Inline));
    let mut app = DocumentUi::new(view.clone(), test_metrics());
    app.focus_control(Some(Control::Field {
        node: "one".into(),
        field: "value".into(),
    }));
    app.drive(Event::Text("private draft".into()), Duration::ZERO)
        .commands;
    app.set_view(view);
    assert_eq!(app.field_text("one", "value"), Some("private draft"));
    let sent = app.submit("two", "answer");
    assert!(
        matches!(&sent[..],[Command::Intent(Intent::Action {node,fields,..})] if node=="two" && fields[0].value.is_empty())
    );
    assert_eq!(app.field_text("one", "value"), Some("private draft"));
}
#[test]
fn committed_text_types_once_and_special_keys_do_not_insert_text() {
    let mut app = DocumentUi::new(form("one", FieldKind::Inline), test_metrics());
    assert_eq!(
        app.interaction.focus().cloned(),
        Some(Control::Field {
            node: "one".into(),
            field: "value".into()
        })
    );
    assert!(
        app.drive(Event::Text("hé λ".into()), Duration::ZERO)
            .commands
            .is_empty()
    );
    assert_eq!(app.field_text("one", "value"), Some("hé λ"));
    assert!(
        app.drive(Event::Key(Key::Left), Duration::ZERO)
            .commands
            .is_empty()
    );
    assert!(
        app.drive(Event::Text("!".into()), Duration::ZERO)
            .commands
            .is_empty()
    );
    assert_eq!(app.field_text("one", "value"), Some("hé !λ"));
    app.drive(Event::Key(Key::SelectAll), Duration::ZERO);
    app.drive(Event::Text("once".into()), Duration::ZERO);
    assert_eq!(app.field_text("one", "value"), Some("once"));
}
#[test]
fn select_all_is_bound_to_the_focused_draft_across_tab_and_return() {
    let mut first = form("a", FieldKind::Inline);
    let mut second = form("b", FieldKind::Inline);
    first.actions.clear();
    second.actions.clear();
    let view = Node::section("root").id("root").child(first).child(second);
    let mut app = DocumentUi::new(view, test_metrics());
    app.frame(500, 500);
    let a = Control::Field {
        node: "a".into(),
        field: "value".into(),
    };
    let b = Control::Field {
        node: "b".into(),
        field: "value".into(),
    };
    let (x, y) = app.control_center(&a).unwrap();
    app.pointer(x, y, false);
    app.drive(Event::Text("alpha".into()), Duration::ZERO);
    let (x, y) = app.control_center(&b).unwrap();
    app.pointer(x, y, false);
    app.drive(Event::Text("beta".into()), Duration::ZERO);
    app.key(Key::SelectAll);
    app.key(Key::Tab { backward: true });
    assert_eq!(app.interaction.focus(), Some(&a));
    app.drive(Event::Text("!".into()), Duration::ZERO);
    assert_eq!(app.field_text("a", "value"), Some("alpha!"));
    app.key(Key::Tab { backward: false });
    assert_eq!(app.interaction.focus(), Some(&b));
    app.key(Key::Backspace);
    assert_eq!(app.field_text("b", "value"), Some("bet"));
    app.key(Key::SelectAll);
    app.key(Key::Tab { backward: true });
    app.key(Key::Tab { backward: false });
    app.drive(Event::Text("!".into()), Duration::ZERO);
    assert_eq!(app.field_text("b", "value"), Some("bet!"));
    app.key(Key::SelectAll);
    app.key(Key::Left);
    app.key(Key::Backspace);
    assert_eq!(app.field_text("b", "value"), Some("be!"));
    app.key(Key::SelectAll);
    app.key(Key::Backspace);
    assert_eq!(app.field_text("b", "value"), Some(""));
}

#[test]
fn select_all_expires_on_reset_and_field_removal() {
    let view = Node::section("root")
        .id("root")
        .child(form("a", FieldKind::Inline));
    let mut app = DocumentUi::new(view.clone(), test_metrics());
    app.focus_control(Some(Control::Field {
        node: "a".into(),
        field: "value".into(),
    }));
    app.drive(Event::Text("old".into()), Duration::ZERO);
    app.key(Key::SelectAll);
    app.set_view(view.clone());
    app.drive(Event::Text("!".into()), Duration::ZERO);
    assert_eq!(app.field_text("a", "value"), Some("old!"));
    app.key(Key::SelectAll);
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Remove { id: "a".into() }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    let mut replacement = form("a", FieldKind::Inline);
    if let Kind::Fields { fields } = &mut replacement.kind {
        fields[0].value = "seed".into();
    }
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Insert {
            parent: "root".into(),
            before: None,
            node: replacement,
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    app.drive(Event::Text("new".into()), Duration::ZERO);
    assert_eq!(app.field_text("a", "value"), Some("seednew"));
}

#[test]
fn boolean_keyboard_input_cannot_produce_invalid_values() {
    let mut app = DocumentUi::new(form("one", FieldKind::Bool), test_metrics());
    app.drive(Event::Text("nonsense".into()), Duration::ZERO)
        .commands;
    assert_eq!(app.field_text("one", "value"), Some(""));
    app.drive(Event::Text(" ".into()), Duration::ZERO).commands;
    assert_eq!(app.field_text("one", "value"), Some("true"));
    app.drive(Event::Text(" ".into()), Duration::ZERO).commands;
    assert_eq!(app.field_text("one", "value"), Some("false"));
}
#[test]
fn boolean_widget_uses_draft_hit_focus_and_existing_cycle_policy() {
    let mut view = form("one", FieldKind::Bool);
    let Kind::Fields { fields } = &mut view.kind else {
        unreachable!()
    };
    fields[0].value = "true".into();
    let mut app = DocumentUi::new(view.clone(), test_metrics());
    let control = Control::Field {
        node: "one".into(),
        field: "value".into(),
    };
    let scene = app.frame(400, 240);
    let hit = app
        .interaction
        .hits()
        .iter()
        .find(|hit| hit.control == control)
        .unwrap();
    let (x, y) = app.control_center(&control).unwrap();
    assert_eq!((x, y), (hit.x + hit.width / 2.0, hit.y + hit.height / 2.0));
    // One clipped checkbox, not an editor with a glyph or caret.
    assert!(any_op(
        &scene.ops,
        |op| matches!(op, Op::ClipRect { width, height, ops, .. }
        if *width == hit.width && *height == hit.height
            && matches!(&ops[..], [Op::Rect { .. }, Op::Rect { .. }, Op::Rect { .. }]))
    ));
    assert!(!any_op(
        &scene.ops,
        |op| matches!(op, Op::Text { text, .. } if text.contains("[✓]"))
    ));
    assert_eq!(app.field_text("one", "value"), Some("true"));
    assert!(app.pointer(x, y, false).is_empty());
    assert_eq!(app.interaction.focus(), Some(&control));
    assert_eq!(app.field_text("one", "value"), Some("false"));
    let off = app.frame(400, 240);
    assert_ne!(scene.ops, off.ops);
    assert!(
        app.drive(Event::Text(" ".into()), Duration::ZERO)
            .commands
            .is_empty()
    );
    assert_eq!(app.field_text("one", "value"), Some("true"));
    assert!(
        app.drive(Event::Text("invalid".into()), Duration::ZERO)
            .commands
            .is_empty()
    );
    assert_eq!(app.field_text("one", "value"), Some("true"));
    assert!(matches!(&app.submit("one", "answer")[..],
        [Command::Intent(Intent::Action { fields, .. })] if fields[0].value == "true"));
    view.kind = Kind::Fields {
        fields: vec![Field {
            id: "value".into(),
            label: "Value".into(),
            value: "false".into(),
            hint: None,
            kind: FieldKind::Bool,
            read_only: false,
            secret: false,
        }],
    };
    app.set_view(view);
    assert_eq!(app.field_text("one", "value"), Some("true"));
    app.frame(400, 240);
    app.pointer(x, y, false);
    assert_eq!(app.field_text("one", "value"), Some("false"));
    assert!(matches!(&app.submit("one", "answer")[..],
        [Command::Intent(Intent::Action { fields, .. })] if fields[0].value == "false"));
}

#[test]
fn read_only_boolean_has_no_hit_and_secret_boolean_stays_masked() {
    let mut view = form("one", FieldKind::Bool);
    let Kind::Fields { fields } = &mut view.kind else {
        unreachable!()
    };
    fields[0].value = "true".into();
    fields[0].read_only = true;
    let mut app = DocumentUi::new(view.clone(), test_metrics());
    app.frame(400, 240);
    assert!(
        app.control_center(&Control::Field {
            node: "one".into(),
            field: "value".into()
        })
        .is_none()
    );
    assert!(app.pointer(40.0, 70.0, false).is_empty());
    assert!(matches!(&app.submit("one", "answer")[..],
        [Command::Intent(Intent::Action { fields, .. })] if fields[0].value == "true"));
    let Kind::Fields { fields } = &mut view.kind else {
        unreachable!()
    };
    fields[0].read_only = false;
    fields[0].secret = true;
    app.set_view(view);
    let scene = app.frame(400, 240);
    assert!(any_op(
        &scene.ops,
        |op| matches!(op, Op::Text { text, .. } if text == "••••")
    ));
    assert!(!any_op(
        &scene.ops,
        |op| matches!(op, Op::Text { text, .. } if text == "true")
    ));
}

#[test]
fn disclosure_state_and_unicode_copy_are_local() {
    let view = Node::new(
        "tool",
        Kind::Collapsible {
            summary: vec![Span::plain("Details")],
        },
    )
    .id("tool")
    .child(Node::text("text", [Span::plain("héllo λ")]));
    let mut app = DocumentUi::new(view.clone(), test_metrics());
    app.frame(500, 500);
    let disclosure = Control::Disclosure("tool".into());
    let (x, y) = app.control_center(&disclosure).unwrap();
    app.pointer(x, y, false);
    assert!(app.interaction.is_expanded("tool"));
    let open = app.frame(500, 500);
    assert!(any_op(
        &open.ops,
        |op| matches!(op, Op::Text { text, .. } if text == "héllo λ")
    ));
    let cached = app.retained.cached("tool").ops.clone();
    app.frame(500, 500);
    assert!(Arc::ptr_eq(&cached, &app.retained.cached("tool").ops));
    app.set_view(view);
    let after_reset = app.frame(500, 500);
    assert!(any_op(
        &after_reset.ops,
        |op| matches!(op, Op::Text { text, .. } if text == "héllo λ")
    ));
    let (x, y) = app.control_center(&disclosure).unwrap();
    app.pointer(x, y, false);
    let closed = app.frame(500, 500);
    assert!(!app.interaction.is_expanded("tool"));
    assert!(!any_op(
        &closed.ops,
        |op| matches!(op, Op::Text { text, .. } if text == "héllo λ")
    ));
    let (x, y) = app.control_center(&disclosure).unwrap();
    app.pointer(x, y, false);
    app.frame(500, 500);
    app.focus_control(None);
    app.key(Key::SelectAll);
    assert_eq!(app.key(Key::Copy), vec![Command::Copy("héllo λ".into())]);
    assert!(app.interaction.is_expanded("tool"));
}
#[test]
fn save_destination_is_an_explicit_local_command() {
    let mut app = DocumentUi::new(Node::section("root"), test_metrics());
    app.activate(Control::Action {
        node: "attachment".into(),
        action: "attachment.save".into(),
    });
    app.drive(Event::Text("/tmp/my photo.png".into()), Duration::ZERO)
        .commands;
    assert_eq!(
        app.key(Key::Enter { newline: false }),
        vec![Command::Save {
            node: "attachment".into(),
            destination: "/tmp/my photo.png".into()
        }]
    );
    assert!(!app.overlays.saving());
}
#[test]
fn save_modal_confirms_only_nonblank_paths_and_cancels_without_effects() {
    let mut app = DocumentUi::new(Node::section("root"), test_metrics());
    app.activate(Control::Action {
        node: "attachment".into(),
        action: "attachment.save".into(),
    });
    assert!(app.key(Key::Enter { newline: false }).is_empty());
    assert_eq!(app.notice_text(), "Enter a local destination path");
    assert!(app.overlays.saving());
    app.drive(Event::Text("/tmp/secret path".into()), Duration::ZERO);
    assert_eq!(
        app.key(Key::Copy),
        vec![Command::Copy("/tmp/secret path".into())]
    );
    app.key(Key::SelectAll);
    app.drive(Event::Text("/tmp/new path".into()), Duration::ZERO);
    assert_eq!(
        app.activate(Control::SaveConfirm),
        vec![Command::Save {
            node: "attachment".into(),
            destination: "/tmp/new path".into()
        }]
    );
    app.activate(Control::Action {
        node: "attachment".into(),
        action: "attachment.save".into(),
    });
    app.drive(Event::Text("/tmp/cancelled".into()), Duration::ZERO);
    assert!(app.activate(Control::SaveCancel).is_empty());
    assert!(!app.overlays.saving());
}

#[test]
fn rich_shapes_draw_wrapped_table_meter_and_bitmap() {
    let view = Node::section("root")
        .child(Node::new(
            "table",
            Kind::Table {
                head: vec![vec![Span::plain("Heading")]],
                rows: vec![vec![vec![Span::plain(
                    "A long cell that wraps into multiple visible lines",
                )]]],
                align: Vec::new(),
            },
        ))
        .child(Node::new(
            "meter",
            Kind::Meter {
                label: "Used".into(),
                value: 5.0,
                max: 10.0,
            },
        ))
        .child(Node::new(
            "image",
            Kind::Image {
                blob: misa_proto::view::BlobRef {
                    hash: "image".into(),
                    len: 4,
                    media: Some("image/png".into()),
                },
                alt: "Picture".into(),
                width: 1,
                height: 1,
            },
        ));
    let mut app = DocumentUi::new(view, test_metrics());
    app.image(
        "image".into(),
        Arc::new(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([255, 0, 0, 255]),
        )),
    );
    let scene = app.frame(200, 600);
    assert!(any_op(&scene.ops, |op| matches!(op, Op::Image { .. })));
    assert!(any_op(
        &scene.ops,
        |op| matches!(op,Op::Rect {width,height,..} if *width==80.0 && *height==10.0)
    ));
    assert!(
        app.interaction
            .rows()
            .iter()
            .filter(|row| row.geometry.text.contains("cell")
                || row.geometry.text.contains("wrap")
                || row.geometry.text.contains("multiple"))
            .count()
            > 1,
        "long table cell must wrap"
    );
    assert!(any_op(
        &scene.ops,
        |op| matches!(op, Op::Image { image, .. } if image.get_pixel(0, 0).0 == [255, 0, 0, 255])
    ));
}

#[test]
fn evicted_images_release_retained_scenes_and_can_be_reloaded() {
    let reference = |hash: &str| misa_proto::view::BlobRef {
        hash: hash.into(),
        len: 4,
        media: Some("image/png".into()),
    };
    let node = |hash: &str| {
        Node::new(
            "image",
            Kind::Image {
                blob: reference(hash),
                alt: hash.into(),
                width: 2048,
                height: 2048,
            },
        )
        .id(hash)
    };
    let mut app = DocumentUi::new(
        Node::section("root")
            .id("root")
            .child(node("a"))
            .child(node("b"))
            .child(node("c")),
        test_metrics(),
    );
    let first = Arc::new(image::RgbaImage::new(2048, 2048));
    let weak = Arc::downgrade(&first);
    app.image("a".into(), first);
    drop(app.frame(800, 600));
    app.image("b".into(), Arc::new(image::RgbaImage::new(2048, 2048)));
    app.image("c".into(), Arc::new(image::RgbaImage::new(2048, 2048)));
    assert!(
        weak.upgrade().is_none(),
        "Cached display lists must not pin evicted bytes"
    );
    assert!(app.decoded_image_bytes() <= 32 * 1024 * 1024);
    app.pin_to_top();
    app.frame(800, 600);
    assert!(
        app.interaction
            .hits()
            .iter()
            .any(|hit| hit.control == Control::LoadImage(reference("a")))
    );
    assert_eq!(
        app.activate(Control::LoadImage(reference("a"))),
        vec![Command::LoadImage(reference("a"))]
    );
    app.image("a".into(), Arc::new(image::RgbaImage::new(1, 1)));
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Remove { id: "a".into() }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert!(!app.has_image("a"));
    app.image("a".into(), Arc::new(image::RgbaImage::new(1, 1)));
    assert!(
        !app.has_image("a"),
        "Late decode cannot repopulate a removed owner"
    );
}
#[test]
fn shared_image_reference_survives_one_owner_and_late_decode_after_reset_is_ignored() {
    let image_node = |id: &str| {
        Node::new(
            "image",
            Kind::Image {
                blob: misa_proto::view::BlobRef {
                    hash: "shared".into(),
                    len: 4,
                    media: Some("image/png".into()),
                },
                alt: id.into(),
                width: 1,
                height: 1,
            },
        )
        .id(id)
    };
    let mut app = DocumentUi::new(
        Node::section("root")
            .id("root")
            .child(image_node("first"))
            .child(image_node("second")),
        test_metrics(),
    );
    app.image("shared".into(), Arc::new(image::RgbaImage::new(1, 1)));
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Remove { id: "first".into() }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert!(app.has_image("shared"));
    app.set_view(Node::section("empty").id("empty"));
    assert!(!app.has_image("shared"));
    app.image("shared".into(), Arc::new(image::RgbaImage::new(1, 1)));
    assert!(!app.has_image("shared"));
}

#[test]
fn live_updates_do_not_steal_the_local_save_dialog() {
    let view = form("panel.input", FieldKind::Inline);
    let mut app = DocumentUi::new(view.clone(), test_metrics());
    app.activate(Control::Action {
        node: "image".into(),
        action: "attachment.save".into(),
    });
    app.drive(Event::Text("/tmp/".into()), Duration::ZERO)
        .commands;
    app.set_view(view);
    app.drive(Event::Text("photo.png".into()), Duration::ZERO)
        .commands;
    assert_eq!(
        app.key(Key::Enter { newline: false }),
        vec![Command::Save {
            node: "image".into(),
            destination: "/tmp/photo.png".into()
        }]
    );
}
#[test]
fn pointer_selection_uses_measured_unicode_boundaries() {
    let mut app = DocumentUi::new(Node::text("text", [Span::plain("界hi")]), test_metrics());
    app.frame(400, 200);
    app.pointer(20.0 + 18.1, 25.0, false);
    app.pointer(20.0 + 27.1, 25.0, true);
    assert_eq!(app.selected_text(), "h");
}
#[test]
fn context_menu_selection_keyboard_and_outside_preserve_focus() {
    let mut app = DocumentUi::new(Node::text("text", [Span::plain("界hi")]), test_metrics());
    app.frame(400, 200);
    app.pointer(38.1, 25.0, false);
    app.pointer(47.1, 25.0, true);
    assert_eq!(app.selected_text(), "h");
    app.drive(Event::ContextMenu { x: 40.0, y: 25.0 }, Duration::ZERO);
    assert!(app.menu.is_some());
    assert_eq!(
        app.key(Key::Enter { newline: false }),
        Vec::<Command>::new()
    ); // no highlight yet
    app.key(Key::Down);
    assert_eq!(
        app.key(Key::Enter { newline: false }),
        vec![Command::Copy("h".into())]
    );
    assert!(app.menu.is_none());
    app.key(Key::Menu);
    assert!(app.menu.is_some());
    app.key(Key::End);
    app.key(Key::Enter { newline: false });
    assert_eq!(app.selected_text(), "界hi");
    app.key(Key::Menu);
    app.pointer(399.0, 199.0, false);
    assert!(app.menu.is_none());
    assert_eq!(app.selected_text(), "界hi");
}
#[test]
fn context_menu_field_and_disclosure_are_local() {
    let mut app = DocumentUi::new(form("one", FieldKind::Inline), test_metrics());
    app.frame(400, 200);
    let control = Control::Field {
        node: "one".into(),
        field: "value".into(),
    };
    let (x, y) = app.control_center(&control).unwrap();
    app.focus_control(Some(control.clone()));
    app.drive(Event::Text("draft".into()), Duration::ZERO);
    app.drive(Event::ContextMenu { x, y }, Duration::ZERO);
    app.key(Key::Down);
    assert_eq!(
        app.key(Key::Enter { newline: false }),
        vec![Command::Copy("draft".into())]
    );
    assert_eq!(app.interaction.focus(), Some(&control));
    app.drive(Event::ContextMenu { x, y }, Duration::ZERO);
    app.key(Key::End);
    app.key(Key::Enter { newline: false });
    assert_eq!(app.interaction.focus(), Some(&control));
    let view = Node::new(
        "tool",
        Kind::Collapsible {
            summary: vec![Span::plain("Details")],
        },
    )
    .id("tool");
    let mut app = DocumentUi::new(view, test_metrics());
    app.frame(400, 200);
    let (x, y) = app
        .control_center(&Control::Disclosure("tool".into()))
        .unwrap();
    app.drive(Event::ContextMenu { x, y }, Duration::ZERO);
    app.key(Key::Home);
    app.key(Key::Enter { newline: false });
    assert!(app.interaction.is_expanded("tool"));
    app.report("Local report".into(), Value::str("content"));
    app.drive(Event::ContextMenu { x, y }, Duration::ZERO);
    assert!(app.menu.is_none(), "report overlay owns context input");
    app.key(Key::Menu);
    assert!(app.menu.is_none());
}
#[test]
fn blank_text_row_accepts_pointer_selection_across_its_width() {
    let mut app = DocumentUi::new(
        Node::text("text", [Span::plain("first\n\nlast")]),
        test_metrics(),
    );
    app.frame(320, 200);
    let blank = app
        .interaction
        .rows()
        .iter()
        .enumerate()
        .find(|(_, row)| row.geometry.text.is_empty())
        .map(|(index, row)| (index, row.x, row.y))
        .expect("blank row");
    app.pointer(blank.1 + 5.0, blank.2 + 1.0, false);
    assert_eq!(
        app.interaction.selection(),
        Some((
            (app.document.root().to_owned(), 6),
            (app.document.root().to_owned(), 6)
        ))
    );
}

#[test]
fn styled_narrow_unicode_rows_share_paint_hit_and_selection_positions() {
    let view = Node::text(
        "text",
        [Span::plain("界"), Span::strong("ill"), Span::plain(" W")],
    );
    let mut app = DocumentUi::new(view, test_metrics());
    let scene = app.frame(400, 200);
    let row = &app.interaction.rows()[0];
    assert_eq!(
        row.geometry.advances,
        vec![0.0, 18.0, 22.0, 26.0, 30.0, 34.0, 47.0]
    );
    assert!(any_op(
        &scene.ops,
        |op| matches!(op, Op::Text { x, text, .. } if text == "ill" && *x == 18.0)
    ));
    let x = row.x;
    let y = row.y + 1.0;
    app.pointer(x + 18.0, y, false);
    app.pointer(x + 30.0, y, true);
    assert_eq!(app.selected_text(), "ill");
    let selected = app.frame(400, 200);
    assert!(any_op(
        &selected.ops,
        |op| matches!(op, Op::Rect { x: left, width, .. } if *left == x + 18.0 && *width == 12.0)
    ));
    assert!(any_op(
        &selected.ops,
        |op| matches!(op, Op::Text { x: left, text, style, .. } if *left == x + 18.0 && text == "ill" && *style == Theme::dark().role("text").over(Theme::dark().role("bold")))
    ));
}

#[test]
fn giant_unwrapped_status_and_label_only_measure_and_paint_visible_prefixes() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Counted {
        longest: Arc<AtomicUsize>,
        total: Arc<AtomicUsize>,
    }
    impl TextMetrics for Counted {
        fn measure(&self, value: &str, size: f32) -> f32 {
            self.total.fetch_add(value.len(), Ordering::Relaxed);
            self.longest.fetch_max(value.len(), Ordering::Relaxed);
            TestMetrics.measure(value, size)
        }
        fn advances(&self, value: &str, size: f32) -> Vec<f32> {
            self.total.fetch_add(value.len(), Ordering::Relaxed);
            self.longest.fetch_max(value.len(), Ordering::Relaxed);
            TestMetrics.advances(value, size)
        }
        fn line_metrics(&self, size: f32) -> misa_pixel_ui::LineMetrics {
            TestMetrics.line_metrics(size)
        }
    }
    let longest = Arc::new(AtomicUsize::new(0));
    let total = Arc::new(AtomicUsize::new(0));
    let label = "界W".repeat(20_000);
    let value = "Wi界".repeat(20_000);
    let view = Node::new(
        "status",
        Kind::Status {
            text: value.clone(),
        },
    )
    .label(&label);
    let mut app = DocumentUi::new(
        view,
        Arc::new(Counted {
            longest: longest.clone(),
            total: total.clone(),
        }),
    );
    let scene = app.frame(100, 120);
    assert_eq!(app.interaction.rows().len(), 2);
    assert_eq!(app.interaction.rows()[0].geometry.text, label);
    assert_eq!(app.interaction.rows()[1].geometry.text, value);
    let assert_bounded = |scene: &Scene| {
        assert!(longest.load(Ordering::Relaxed) <= 32 * 4);
        assert!(total.load(Ordering::Relaxed) < 1024);
        walk_ops(&scene.ops, &mut |op| {
            if let Op::Text { text, .. } = op {
                assert!(text.len() <= 32 * 4, "invisible suffix reached paint");
            }
        });
    };
    assert_bounded(&scene);
    let row = &app.interaction.rows()[1];
    let (x, y) = (row.x, row.y + 1.0);
    app.pointer(x + 1.0, y, false);
    app.pointer(x + 14.0, y, true);
    assert_eq!(app.selected_text(), "W");
    assert_bounded(&app.frame(100, 120)); // selection repaint must not convert the tail
    app.key(Key::SelectAll);
    assert_eq!(
        app.key(Key::Copy),
        vec![Command::Copy(format!("{label}\n{value}"))]
    );
    assert_bounded(&app.frame(100, 120));
}

#[test]
fn narrow_quote_table_rows_clip_without_losing_copy_or_hit_bounds() {
    let view = Node::new("quote", Kind::Quote).child(Node::new(
        "table",
        Kind::Table {
            head: vec![vec![Span::plain("W")], vec![Span::plain("界")]],
            rows: vec![vec![vec![Span::plain("界W")], vec![Span::plain("W界")]]],
            align: vec![],
        },
    ));
    let mut app = DocumentUi::new(view, test_metrics());
    let scene = app.frame(95, 500);
    let cell_width = 55.0 / 2.0;
    let viewport = cell_width - 10.0 - GUTTER;
    assert!(app.interaction.rows().len() >= 4);
    let mut clips = Vec::new();
    walk_ops(&scene.ops, &mut |op| {
        if let Op::ClipRect { x, width, ops, .. } = op {
            clips.push((*x, *width, ops.clone()));
        }
    });
    // Row clips are cell-sized; the transcript region clip is window-sized.
    let rows_clipped = clips
        .iter()
        .filter(|(_, width, _)| *width == viewport)
        .count();
    assert_eq!(rows_clipped, app.interaction.rows().len());
    assert!(
        clips
            .iter()
            .all(|(_, width, _)| *width == viewport || *width == 95.0)
    );
    assert!(clips.iter().any(|(left, width, ops)| ops.iter().any(|op| matches!(op, Op::Text { x, text, .. } if text.contains('界') && x + app.metrics.measure(text, FONT_SIZE) > left + width))));
    for hit in app.interaction.hits() {
        if let Control::Text(index) = hit.control {
            let row = &app.interaction.rows()[index];
            assert_eq!(row.width, viewport);
            assert!(hit.x + hit.width <= row.x + viewport);
            // The document content is inset 20px from the window edge.
            let cell_right = if row.x < 20.0 + cell_width {
                20.0 + cell_width
            } else {
                20.0 + 2.0 * cell_width
            };
            assert!(hit.x + hit.width <= cell_right);
            assert!(!hit.contains(row.x + viewport, hit.y + 1.0));
        }
    }
    app.key(Key::SelectAll);
    let copied = app.selected_text();
    assert!(copied.contains('W') && copied.contains('界'));
    let selected = app.frame(95, 500);
    let selection = crate::appearance::Palette::new(app.light).selection;
    let mut painted = false;
    walk_ops(&selected.ops, &mut |op| {
        if matches!(op, Op::Rect { style, .. } if *style == selection) {
            painted = true;
        }
    });
    assert!(painted, "the selection paints inside its region");
}

#[test]
fn measured_wrap_handles_long_tokens_styled_runs_and_quote_prefixes() {
    let view = Node::new("quote", Kind::Quote).child(Node::text(
        "text",
        [
            Span::plain("WW"),
            Span::strong("iiiiiiiiiiiiii"),
            Span::plain("界界界"),
        ],
    ));
    let mut app = DocumentUi::new(view, test_metrics());
    app.frame(95, 250);
    assert!(app.interaction.rows().len() > 2);
    for row in app.interaction.rows() {
        assert!(
            row.edge(row.geometry.text.chars().count()) <= 55.0 - GUTTER + 0.01,
            "row exceeded available pixels: {:?}",
            row.geometry.text
        );
        // The rail is drawn beside the rows; a copied row carries content only.
        assert!(!row.geometry.text.contains('▏'));
    }
    let copied = app
        .interaction
        .rows()
        .iter()
        .map(|row| row.geometry.text.as_str())
        .collect::<String>();
    assert_eq!(copied, "WWiiiiiiiiiiiiii界界界");
}

#[test]
fn report_reflows_at_measured_pixel_width() {
    let mut app = DocumentUi::new(Node::section("root"), test_metrics());
    app.report("Report".into(), Value::str("WWWWiiii界界"));
    app.frame(132, 280);
    let lines = app.overlays.report_mut().unwrap().lines.clone();
    assert!(lines.len() > 2);
    assert_eq!(lines.concat(), "WWWWiiii界界");
    assert!(
        lines
            .iter()
            .all(|line| app.metrics.measure(line, FONT_SIZE) <= 36.0)
    );
    app.frame(300, 280);
    assert_eq!(
        app.overlays.report_mut().unwrap().lines,
        vec!["WWWWiiii界界"]
    );
}

#[test]
fn cached_groups_translate_measured_rows_without_remeasuring() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    // A counting wrapper makes unexpected reflow visible.
    struct Counted(Arc<AtomicUsize>);
    impl TextMetrics for Counted {
        fn measure(&self, value: &str, size: f32) -> f32 {
            self.0.fetch_add(1, Ordering::Relaxed);
            TestMetrics.measure(value, size)
        }
        fn advances(&self, value: &str, size: f32) -> Vec<f32> {
            self.0.fetch_add(1, Ordering::Relaxed);
            TestMetrics.advances(value, size)
        }
        fn line_metrics(&self, size: f32) -> misa_pixel_ui::LineMetrics {
            TestMetrics.line_metrics(size)
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let view = Node::section("session")
        .id("session")
        .child(Node::text("text", [Span::plain("界ill")]).id("child"))
        .children((0..20).map(|i| {
            Node::text("text", [Span::plain(format!("line {i}"))]).id(format!("tail.{i}"))
        }));
    let mut app = DocumentUi::new(view, Arc::new(Counted(calls.clone())));
    app.pin_to_top();
    app.frame(400, 100);
    app.scroll(20.0);
    app.frame(400, 100);
    let geometry = Arc::clone(&app.retained.cached("child").geometry.rows()[0].geometry);
    assert!(Arc::ptr_eq(&geometry, &app.interaction.rows()[0].geometry));
    let y = app.interaction.rows()[0].y;
    let measured = calls.load(Ordering::Relaxed);
    app.frame(400, 100);
    assert_eq!(calls.load(Ordering::Relaxed), measured);
    assert!(Arc::ptr_eq(&geometry, &app.interaction.rows()[0].geometry));
    app.scroll(-10.0);
    app.frame(400, 100);
    assert_eq!(calls.load(Ordering::Relaxed), measured);
    assert!(Arc::ptr_eq(
        &geometry,
        &app.retained.cached("child").geometry.rows()[0].geometry
    ));
    assert!(Arc::ptr_eq(&geometry, &app.interaction.rows()[0].geometry));
    assert_eq!(app.interaction.rows()[0].x, 20.0);
    assert_eq!(app.interaction.rows()[0].y, y + 10.0);

    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Replace {
            id: "child".into(),
            node: Node::text("text", [Span::plain("界ill updated")]).id("child"),
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    app.frame(400, 100);
    assert!(!Arc::ptr_eq(
        &geometry,
        &app.retained.cached("child").geometry.rows()[0].geometry
    ));
    assert!(Arc::ptr_eq(
        &app.retained.cached("child").geometry.rows()[0].geometry,
        &app.interaction.rows()[0].geometry
    ));
    assert_eq!(app.interaction.rows()[0].geometry.text, "界ill updated");
    assert!(calls.load(Ordering::Relaxed) > measured);
}

#[test]
fn deterministic_scene_and_unchanged_frames_do_no_layout() {
    for owners in [10, 1000] {
        let view = Node::section("session").id("session").child(
            Node::section("transcript")
                .id("transcript")
                .children((0..owners).map(|index| {
                    Node::text("message", [Span::plain("unchanged transcript")])
                        .id(format!("message.{index}"))
                })),
        );
        let mut app = DocumentUi::new(view.clone(), test_metrics());
        let mut first = None;
        for _ in 0..6 {
            app.set_view(view.clone());
            let scene = app.frame(800, 600);
            if let Some(expected) = &first {
                assert_eq!(&scene, expected, "scene oracle is not deterministic");
            } else {
                first = Some(scene);
            }
        }
        app.frame(800, 600);
        assert_eq!(app.retained.rendered_nodes(), 0);
    }
}
#[test]
fn scoped_document_transaction_settles_live_text_without_rebuilding_history() {
    use misa_proto::sync::Stream;
    for owners in [10, 1000] {
        let view = protocol_view(owners);
        let mut app = DocumentUi::new(Node::section("empty"), test_metrics());
        app.observed(&DocumentUpdate::Reset {
            tree: &view,
            streams: &[Stream {
                id: "answer.text".into(),
                role: "message.assistant".into(),
                text: "é".into(),
            }],
        })
        .unwrap();
        app.frame(800, 600);
        let retained_id = format!("message.{}", owners - 1);
        let retained = app.retained.cached(&retained_id).ops.clone();
        let answer = Node::text("message.assistant", [Span::plain("é終")]).id("answer");
        let update = DocumentUpdate::Changed {
            tree: &[ViewOp::Insert {
                parent: "transcript".into(),
                before: None,
                node: answer.clone(),
            }],
            live: &[StreamUpdate::End {
                id: "answer.text".into(),
            }],
            reset_live: false,
        };
        app.observed(&update).unwrap();
        assert!(app.document.streams_empty());
        assert!(app.document.contains("answer"));
        let scene = app.frame(800, 600);
        assert!(Arc::ptr_eq(
            &retained,
            &app.retained.cached(&retained_id).ops
        ));
        assert!(
            app.retained.rendered_nodes() <= 3,
            "only changed ancestors and final answer need layout"
        );
        let mut expected = view;
        expected.children[0].children.push(answer);
        let mut cold = DocumentUi::new(expected, test_metrics());
        assert_eq!(scene, cold.frame(800, 600));
    }
}
fn protocol_view(owners: usize) -> Node {
    Node::section("session").id("session").child(
        Node::section("transcript")
            .id("transcript")
            .children((0..owners).map(|index| {
                Node::text("message", [Span::plain("unchanged transcript")])
                    .id(format!("message.{index}"))
            })),
    )
}
// Independent cold tree/materialization oracle; no protocol cursor or wire adapter.
fn cold_scene(
    app: &mut DocumentUi,
    tree: &misa_proto::sync::IndexedTree,
    streams: &[misa_proto::sync::Stream],
) {
    let scene = app.frame(800, 600);
    let view = tree.snapshot();
    let mut cold = DocumentUi::new(view.clone(), test_metrics());
    cold.observed(&DocumentUpdate::Reset {
        tree: &view,
        streams,
    })
    .unwrap();
    let expected = cold.frame(800, 600);
    assert_eq!(scene, expected);
}
fn cold_stream_node(stream: &misa_proto::sync::Stream) -> Node {
    use misa_proto::view::State;
    let id = &stream.id;
    if super::document::thinking_stream(&stream.role) {
        return Node::new(
            &stream.role,
            Kind::Collapsible {
                summary: {
                    let mut summary = vec![Span::strong("thinking")];
                    if !stream.text.is_empty() {
                        let tail = stream.text.lines().rev().take(3).collect::<Vec<_>>();
                        summary.push(Span::plain(format!(
                            " · {}",
                            tail.into_iter().rev().collect::<Vec<_>>().join(" ↵ ")
                        )));
                    }
                    summary
                },
            },
        )
        .id(id)
        .state(State::Streaming)
        .child(
            Node::text(format!("{}.text", stream.role), [Span::plain(&stream.text)])
                .id(format!("{id}.body")),
        );
    }
    let mut blocks = misa_markdown::document(&stream.role, &stream.text, None).blocks;
    for (index, block) in blocks.iter_mut().enumerate() {
        block.id = format!("{id}.block.{index}");
        misa_proto::sync::address(block);
    }
    Node::section(&stream.role)
        .id(id)
        .state(State::Streaming)
        .children(blocks)
}
fn observed_changes(
    app: &mut DocumentUi,
    tree: Vec<ViewOp>,
    live: Vec<misa_proto::sync::StreamUpdate>,
    reset_live: bool,
) {
    app.observed(&DocumentUpdate::Changed {
        tree: &tree,
        live: &live,
        reset_live,
    })
    .unwrap();
}
#[test]
fn stream_append_and_subtree_replace_reuse_unchanged_owner_scenes() {
    use misa_proto::sync::{IndexedTree, Stream, StreamUpdate};
    for owners in [10, 1000] {
        let view = protocol_view(owners);
        let mut tree = IndexedTree::new(view.clone());
        let mut app = DocumentUi::new(view, test_metrics());
        app.frame(800, 600);
        let retained: Vec<_> = (0..owners)
            .filter_map(|index| {
                let id = format!("message.{index}");
                app.retained
                    .contains(&id)
                    .then(|| (id.clone(), app.retained.cached(&id).ops.clone()))
            })
            .collect();
        let mut stream = Stream {
            id: "live.text".into(),
            role: "message.assistant".into(),
            text: "hello".into(),
        };
        observed_changes(
            &mut app,
            vec![],
            vec![StreamUpdate::Current {
                stream: stream.clone(),
            }],
            true,
        );
        app.frame(800, 600);
        observed_changes(
            &mut app,
            vec![],
            vec![StreamUpdate::Append {
                id: stream.id.clone(),
                offset: 5,
                text: " world".into(),
            }],
            false,
        );
        stream.text.push_str(" world");
        app.frame(800, 600);
        assert!(app.retained.rendered_nodes() <= 3);
        for (id, ops) in &retained {
            assert!(Arc::ptr_eq(ops, &app.retained.cached(id).ops));
        }
        cold_scene(&mut app, &tree, &[stream.clone()]);
        let op = ViewOp::Replace {
            id: "message.0".into(),
            node: Node::text("message", [Span::plain("changed")]).id("message.0"),
        };
        tree.apply(&op).unwrap();
        observed_changes(&mut app, vec![op], vec![], false);
        app.frame(800, 600);
        assert!(app.retained.rendered_nodes() <= 2);
        for (id, ops) in &retained {
            if id != "message.0" {
                assert!(Arc::ptr_eq(ops, &app.retained.cached(id).ops));
            }
        }
        cold_scene(&mut app, &tree, &[stream]);
    }
}
#[test]
fn stream_completion_and_owner_removal_match_cold_rebuilds() {
    use misa_proto::sync::{IndexedTree, Stream, StreamUpdate};
    let view = protocol_view(4);
    let mut tree = IndexedTree::new(view.clone());
    let mut app = DocumentUi::new(view, test_metrics());
    let stream = Stream {
        id: "live.text".into(),
        role: "message.assistant".into(),
        text: "streamed answer".into(),
    };
    observed_changes(
        &mut app,
        vec![],
        vec![StreamUpdate::Current {
            stream: stream.clone(),
        }],
        true,
    );
    cold_scene(&mut app, &tree, &[stream.clone()]);
    let ops = vec![
        ViewOp::Insert {
            parent: "transcript".into(),
            before: Some("message.2".into()),
            node: Node::text("message", [Span::plain("committed answer")]).id("live"),
        },
        ViewOp::Remove {
            id: "message.0".into(),
        },
    ];
    for op in &ops {
        tree.apply(op).unwrap();
    }
    observed_changes(&mut app, ops, vec![], false);
    cold_scene(&mut app, &tree, &[stream]);
    observed_changes(
        &mut app,
        vec![],
        vec![StreamUpdate::End {
            id: "live.text".into(),
        }],
        false,
    );
    cold_scene(&mut app, &tree, &[]);
}
#[test]
fn markdown_stream_appends_reparse_partial_blocks_and_keep_prefix_ids() {
    use misa_proto::sync::{Stream, StreamUpdate};
    use misa_proto::view::SpanKind;
    let mut app = DocumentUi::new(protocol_view(0), test_metrics());
    let mut stream = Stream {
        id: "msg.4.text".into(),
        role: "message.assistant".into(),
        text: String::new(),
    };
    observed_changes(
        &mut app,
        vec![],
        vec![StreamUpdate::Current {
            stream: stream.clone(),
        }],
        false,
    );
    for delta in [
        "# Title\n\nfirst\n\nsecond\n\nthird\n\n*open",
        " and closed*",
        "\n\n```rust\nlet x = 1;",
        "\n```\n\n| a | b |\n| --- | :---: |",
        "\n| **yes** | no |",
    ] {
        let offset = stream.text.len();
        observed_changes(
            &mut app,
            vec![],
            vec![StreamUpdate::Append {
                id: stream.id.clone(),
                offset,
                text: delta.into(),
            }],
            false,
        );
        stream.text.push_str(delta);
        let live = app.document.stream_or_node(&stream.id).unwrap();
        let cold = cold_stream_node(&stream);
        assert_eq!(live, &cold, "after {delta:?}");
        assert_eq!(live.children[0].id, "msg.4.text.block.0");
        assert!(matches!(live.children[0].kind, Kind::Heading { .. }));
        let scene = app.frame(360, 600);
        let mut fresh = DocumentUi::new(protocol_view(0), test_metrics());
        let view = protocol_view(0);
        fresh
            .observed(&DocumentUpdate::Reset {
                tree: &view,
                streams: &[stream.clone()],
            })
            .unwrap();
        assert_eq!(scene, fresh.frame(360, 600));
    }
    let live = app.document.stream_or_node(&stream.id).unwrap();
    assert!(live.children.iter().any(|node| matches!(&node.kind, Kind::Code { lang: Some(lang), text } if lang == "rust" && text == "let x = 1;")));
    assert!(live.children.iter().any(|node| matches!(&node.kind, Kind::Table { rows, .. } if rows.iter().flatten().flatten().any(|span| span.kind == SpanKind::Strong))));
    assert!(live.children.iter().any(|node| matches!(&node.kind, Kind::Text { spans } if spans.iter().any(|span| span.kind == SpanKind::Emphasis && span.text == "open and closed"))));
}

#[test]
fn completed_markdown_stream_matches_settled_pixel_text_at_narrow_width() {
    use misa_proto::sync::Stream;
    let text = "# Heading\n\nA **bold** and *italic* word\n\n```rust\nlet x = 1;\n```\n\n| a | b |\n| --- | --- |\n| one | two |";
    let stream = Stream {
        id: "msg.4.text".into(),
        role: "message.assistant".into(),
        text: text.into(),
    };
    let view = protocol_view(0);
    let mut live = DocumentUi::new(view.clone(), test_metrics());
    live.observed(&DocumentUpdate::Reset {
        tree: &view,
        streams: &[stream.clone()],
    })
    .unwrap();
    let settled = Node::section("message.assistant")
        .id("msg.4")
        .children(misa_markdown::blocks(&stream.role, text));
    let mut view = view;
    view.children[0].children.push(settled);
    let mut cold = DocumentUi::new(view, test_metrics());
    fn painted_text(scene: &Scene) -> Vec<(Style, String)> {
        fn visit(ops: &[Op], out: &mut Vec<(Style, String)>) {
            for op in ops {
                match op {
                    Op::Text { style, text, .. } => out.push((*style, text.clone())),
                    Op::Group { ops, .. } | Op::ClipRect { ops, .. } => visit(ops, out),
                    _ => {}
                }
            }
        }
        let mut out = vec![];
        visit(&scene.ops, &mut out);
        out
    }
    assert_eq!(
        painted_text(&live.frame(185, 600)),
        painted_text(&cold.frame(185, 600))
    );
}

#[test]
fn current_rolls_back_markdown_and_end_and_reset_retire_it() {
    use misa_proto::sync::{Stream, StreamUpdate};
    let view = protocol_view(0);
    let mut app = DocumentUi::new(view.clone(), test_metrics());
    let mut stream = Stream {
        id: "msg.1.text".into(),
        role: "message.assistant".into(),
        text: "old **bold".into(),
    };
    observed_changes(
        &mut app,
        vec![],
        vec![StreamUpdate::Current {
            stream: stream.clone(),
        }],
        false,
    );
    assert_eq!(
        app.document.stream_or_node(&stream.id),
        Some(&cold_stream_node(&stream))
    );
    stream.text = "new | heading\n--- | ---\ncell | next".into();
    observed_changes(
        &mut app,
        vec![],
        vec![StreamUpdate::Current {
            stream: stream.clone(),
        }],
        false,
    );
    assert_eq!(
        app.document.stream_or_node(&stream.id),
        Some(&cold_stream_node(&stream))
    );
    assert!(matches!(
        app.document.stream_or_node(&stream.id).unwrap().children[0].kind,
        Kind::Table { .. }
    ));
    observed_changes(
        &mut app,
        vec![],
        vec![StreamUpdate::End {
            id: stream.id.clone(),
        }],
        false,
    );
    assert!(app.document.streams_empty());
    assert!(app.document.visible_streams().is_empty());
    observed_changes(
        &mut app,
        vec![],
        vec![StreamUpdate::Current { stream }],
        false,
    );
    app.observed(&DocumentUpdate::Reset {
        tree: &view,
        streams: &[],
    })
    .unwrap();
    assert!(app.document.streams_empty());
}

#[test]
fn thinking_precedes_answer_and_stays_plain_behind_disclosure() {
    use misa_proto::sync::Stream;
    let view = protocol_view(0);
    let mut app = DocumentUi::new(view.clone(), test_metrics());
    let streams = [
        Stream {
            id: "msg.1.text".into(),
            role: "message.assistant".into(),
            text: "**answer**".into(),
        },
        Stream {
            id: "msg.1.thinking".into(),
            role: "message.assistant.thinking".into(),
            text: "**literal**".into(),
        },
    ];
    app.observed(&DocumentUpdate::Reset {
        tree: &view,
        streams: &streams,
    })
    .unwrap();
    assert_eq!(
        app.document.visible_streams(),
        ["msg.1.thinking", "msg.1.text"]
    );
    let thinking = app.document.stream_or_node("msg.1.thinking").unwrap();
    assert!(matches!(thinking.kind, Kind::Collapsible { .. }));
    assert!(
        matches!(&thinking.children[0].kind, Kind::Text { spans } if spans[0].text == "**literal**")
    );
    let closed = app.frame(320, 500);
    app.activate(Control::Disclosure("msg.1.thinking".into()));
    let open = app.frame(320, 500);
    assert_ne!(closed, open);
    observed_changes(
        &mut app,
        vec![],
        vec![misa_proto::sync::StreamUpdate::Append {
            id: "msg.1.thinking".into(),
            offset: "**literal**".len(),
            text: "\nlast line".into(),
        }],
        false,
    );
    assert!(
        matches!(&app.document.stream_or_node("msg.1.thinking").unwrap().children[0].kind,
        Kind::Text { spans } if spans[0].text == "**literal**\nlast line")
    );
    assert_ne!(closed, app.frame(320, 500));
    app.observed(&DocumentUpdate::Reset {
        tree: &view,
        streams: &streams,
    })
    .unwrap();
    assert_eq!(
        app.document.visible_streams(),
        ["msg.1.thinking", "msg.1.text"]
    );
}

#[test]
fn headless_driver_uses_fake_clock_and_never_presents() {
    use misa_window_core::Clock;
    struct FakeClock(Duration);
    impl Clock for FakeClock {
        fn elapsed(&self) -> Duration {
            self.0
        }
    }
    let mut clock = FakeClock(Duration::ZERO);
    let mut app = DocumentUi::new(
        Node::section("status.indicators").id("status").child(
            Node::new(
                "indicator.activity",
                Kind::Status {
                    text: "working".into(),
                },
            )
            .id("activity"),
        ),
        test_metrics(),
    );
    let size = Size {
        width: 320,
        height: 200,
    };
    let resize = app.drive(Event::Resize(size), clock.elapsed());
    assert!(resize.redraw && resize.frame.is_none());
    let first = app.drive(Event::Redraw(size), clock.elapsed());
    assert_eq!(first.deadline, Some(PULSE_PERIOD));
    assert_eq!(first.frame.unwrap().width, 320.0);
    clock.0 = Duration::from_millis(160);
    assert_eq!(
        app.drive(Event::Redraw(size), clock.elapsed()).deadline,
        Some(Duration::from_millis(320))
    );
    assert!(
        app.drive(Event::Text("hello".into()), clock.elapsed())
            .redraw
    );
    assert!(
        app.drive(Event::Theme { light: true }, clock.elapsed())
            .redraw
    );
    assert!(
        app.drive(
            Event::Pointer {
                x: 0.0,
                y: 0.0,
                dragging: false
            },
            clock.elapsed()
        )
        .redraw
    );
    assert!(
        app.drive(Event::Wheel { delta: 12.0 }, clock.elapsed())
            .redraw
    );
}

#[test]
fn editing_a_field_does_not_relayout_the_transcript() {
    let view = Node::section("session")
        .id("session")
        .child(
            Node::section("transcript")
                .id("transcript")
                .child(Node::text("message", [Span::plain("old text")]).id("message.1")),
        )
        .child(form("panel.input", FieldKind::Inline));
    let mut app = DocumentUi::new(view, test_metrics());
    app.frame(800, 600);
    let owner = app.retained.cached("message.1").ops.clone();
    app.drive(Event::Text("draft".into()), Duration::ZERO)
        .commands;
    app.frame(800, 600);
    assert_eq!(app.retained.rendered_nodes(), 1);
    assert!(Arc::ptr_eq(&owner, &app.retained.cached("message.1").ops));
    assert_eq!(app.field_text("panel.input", "value"), Some("draft"));
    let sent = app.key(Key::Enter { newline: false });
    assert!(
        matches!(&sent[..],[Command::Intent(Intent::Action {fields,..})] if fields[0].value=="draft")
    );
    assert_eq!(app.field_text("panel.input", "value"), Some("draft"));
}

#[test]
fn list_item_edits_survive_reset_and_replace_without_reallocating_the_owner() {
    let list = || {
        Node::new(
            "list",
            Kind::List {
                ordered: false,
                items: vec![vec![form("nested", FieldKind::Inline)]],
                markers: vec![],
            },
        )
        .id("list")
    };
    let view = Node::section("root").id("root").child(list());
    let mut app = DocumentUi::new(view.clone(), test_metrics());
    assert_eq!(app.field_text("nested", "value"), Some(""));
    let editor = app.drafts.identity("nested", "value").unwrap();
    app.frame(400, 400);
    assert!(app.retained.contains("list"));
    app.focus_control(Some(Control::Field {
        node: "nested".into(),
        field: "value".into(),
    }));
    app.drive(Event::Text("my edit".into()), Duration::ZERO);
    assert!(
        !app.retained.contains("list"),
        "embedded edits invalidate the indexed list owner"
    );
    app.set_view(view);
    assert_eq!(app.drafts.identity("nested", "value"), Some(editor));
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Replace {
            id: "list".into(),
            node: list(),
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert_eq!(app.drafts.identity("nested", "value"), Some(editor));
    assert_eq!(app.field_text("nested", "value"), Some("my edit"));
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Remove { id: "list".into() }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert_eq!(app.field_text("nested", "value"), None);
    assert_eq!(app.drafts.identity("nested", "value"), None);
}

#[test]
fn clearing_secret_erases_its_viewport_and_only_invalidates_its_owner() {
    let mut secret = form("private", FieldKind::Inline);
    let Kind::Fields { fields } = &mut secret.kind else {
        unreachable!()
    };
    fields[0].secret = true;
    let mut app = DocumentUi::new(
        Node::section("root")
            .id("root")
            .child(secret)
            .child(Node::text("text", [Span::plain("other")]).id("other")),
        test_metrics(),
    );
    app.focus_control(Some(Control::Field {
        node: "private".into(),
        field: "value".into(),
    }));
    app.drive(Event::Text("secret".repeat(30)), Duration::ZERO);
    app.frame(180, 400);
    assert!(app.drafts.viewport("private", "value").x > 0.0);
    assert!(
        app.key(Key::Copy).is_empty(),
        "password fields must not copy plaintext"
    );
    let (x, y) = app
        .control_center(&Control::Field {
            node: "private".into(),
            field: "value".into(),
        })
        .unwrap();
    app.drive(Event::ContextMenu { x, y }, Duration::ZERO);
    assert!(matches!(
        app.menu.as_ref().unwrap().items.first(),
        Some(MenuItem::Action {
            id: MenuAction::Copy,
            enabled: false,
            ..
        })
    ));
    assert!(app.key(Key::Enter { newline: false }).is_empty());
    app.key(Key::Escape);
    let unrelated = app.retained.cached("other").ops.clone();
    app.clear_secret_drafts();
    assert_eq!(app.field_text("private", "value"), Some(""));
    assert_eq!(app.drafts.viewport("private", "value").x, 0.0);
    assert!(Arc::ptr_eq(&unrelated, &app.retained.cached("other").ops));
    assert!(!app.retained.contains("private"));
    app.frame(180, 400);
    assert_eq!(app.drafts.viewport("private", "value").x, 0.0);
}

#[test]
fn choice_read_only_and_secret_never_open_popup() {
    let choice = || FieldKind::Choice {
        options: vec![misa_proto::view::Choice {
            value: "private-value".into(),
            label: "Friendly label".into(),
            detail: None,
            metadata: None,
        }],
        selected: Some("private-value".into()),
    };
    let mut read_only = form("readonly", choice());
    let Kind::Fields { fields } = &mut read_only.kind else {
        unreachable!()
    };
    fields[0].read_only = true;
    let mut app = DocumentUi::new(read_only, test_metrics());
    let scene = app.frame(240, 400);
    assert!(format!("{:?}", scene.ops).contains("Friendly label"));
    assert!(
        app.control_center(&Control::Field {
            node: "readonly".into(),
            field: "value".into()
        })
        .is_none()
    );

    let mut secret = form("secret", choice());
    let Kind::Fields { fields } = &mut secret.kind else {
        unreachable!()
    };
    fields[0].secret = true;
    let mut app = DocumentUi::new(secret, test_metrics());
    let scene = app.frame(240, 400);
    assert!(!format!("{:?}", scene.ops).contains("Friendly label"));
    assert!(
        app.choice_widget(&Control::Field {
            node: "secret".into(),
            field: "value".into()
        })
        .is_none()
    );
}

#[test]
fn choice_popup_keeps_local_value_on_refresh() {
    let mut view = form(
        "choice",
        FieldKind::Choice {
            options: vec![
                misa_proto::view::Choice {
                    value: "first".into(),
                    label: "First".into(),
                    detail: None,
                    metadata: None,
                },
                misa_proto::view::Choice {
                    value: "second".into(),
                    label: "Second".into(),
                    detail: None,
                    metadata: None,
                },
            ],
            selected: Some("second".into()),
        },
    );
    let mut app = DocumentUi::new(view.clone(), test_metrics());
    assert_eq!(app.field_text("choice", "value"), Some("second"));
    app.frame(240, 400);
    let control = Control::Field {
        node: "choice".into(),
        field: "value".into(),
    };
    let (x, y) = app.control_center(&control).unwrap();
    assert!(app.pointer(x, y, false).is_empty());
    assert!(app.choice.is_some());
    let popup = app.frame(240, 400);
    assert!(matches!(popup.ops.last(), Some(Op::ClipRect { ops, .. })
        if format!("{ops:?}").contains("First") && format!("{ops:?}").contains("Second")));
    app.key(Key::Escape);
    assert!(app.choice.is_none());
    assert_eq!(app.interaction.focus(), Some(&control));
    assert_eq!(app.field_text("choice", "value"), Some("second"));
    app.key(Key::Enter { newline: false });
    app.key(Key::Home);
    app.key(Key::Enter { newline: false });
    assert!(app.choice.is_none());
    assert_eq!(app.field_text("choice", "value"), Some("first"));
    // Outside dismissal does not change either the focused control or local value.
    app.pointer(x, y, false);
    app.pointer(0.0, 0.0, false);
    assert!(app.choice.is_none());
    assert_eq!(app.interaction.focus(), Some(&control));
    assert_eq!(app.field_text("choice", "value"), Some("first"));
    let Kind::Fields { fields } = &mut view.kind else {
        unreachable!()
    };
    fields[0].value = "second".into();
    app.set_view(view);
    assert_eq!(app.field_text("choice", "value"), Some("first"));
    assert!(matches!(&app.submit("choice", "answer")[..],
        [Command::Intent(Intent::Action { fields, .. })] if fields[0].value == "first"));
}

fn choice_form(secret: bool, values: &[(&str, &str)]) -> Node {
    let mut view = form(
        "choice",
        FieldKind::Choice {
            options: values
                .iter()
                .map(|(value, label)| misa_proto::view::Choice {
                    value: (*value).into(),
                    label: (*label).into(),
                    detail: None,
                    metadata: None,
                })
                .collect(),
            selected: Some("first".into()),
        },
    );
    if let Kind::Fields { fields } = &mut view.kind {
        fields[0].secret = secret;
    }
    view
}

#[test]
fn secret_choice_cycles_on_click_and_space_without_exposing_labels() {
    let mut app = DocumentUi::new(
        choice_form(
            true,
            &[("first", "Hidden first"), ("second", "Hidden second")],
        ),
        test_metrics(),
    );
    let scene = app.frame(360, 300);
    let control = Control::Field {
        node: "choice".into(),
        field: "value".into(),
    };
    let (x, y) = app.control_center(&control).unwrap();
    assert!(!format!("{:?}", scene.ops).contains("Hidden"));
    assert!(!format!("{:?}", scene.ops).contains("first"));
    app.pointer(x, y, false);
    assert!(app.choice.is_none());
    assert_eq!(app.field_text("choice", "value"), Some("second"));
    let scene = app.frame(360, 300);
    assert!(!format!("{:?}", scene.ops).contains("Hidden"));
    assert!(!format!("{:?}", scene.ops).contains("second"));
    assert!(matches!(&app.submit("choice", "answer")[..],
        [Command::Intent(Intent::Action { fields, .. })] if fields[0].value == "second"));
    app.drive(Event::Text(" ".into()), Duration::ZERO);
    assert_eq!(app.field_text("choice", "value"), Some("first"));
}

#[test]
fn removed_selected_choice_is_visible_as_unavailable_until_replaced() {
    let mut app = DocumentUi::new(
        choice_form(false, &[("first", "First"), ("second", "Second")]),
        test_metrics(),
    );
    app.frame(360, 300);
    let control = Control::Field {
        node: "choice".into(),
        field: "value".into(),
    };
    app.key(Key::Down);
    app.key(Key::Enter { newline: false });
    assert_eq!(app.field_text("choice", "value"), Some("second"));
    let changed = choice_form(false, &[("first", "First new"), ("third", "Third")]);
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Replace {
            id: "choice".into(),
            node: changed,
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    let scene = app.frame(360, 300);
    assert!(any_op(
        &scene.ops,
        |op| matches!(op, Op::Text { text, .. } if text == "Unavailable selection")
    ));
    assert_eq!(app.field_text("choice", "value"), Some("second"));
    assert!(matches!(&app.submit("choice", "answer")[..],
        [Command::Intent(Intent::Action { fields, .. })] if fields[0].value == "second"));
    app.focus_control(Some(control));
    app.key(Key::Down);
    app.key(Key::Home);
    app.key(Key::Enter { newline: false });
    assert_eq!(app.field_text("choice", "value"), Some("first"));
}

#[test]
fn unrelated_tree_change_keeps_choice_open_but_owner_change_closes_it() {
    let mut app = DocumentUi::new(
        Node::section("root")
            .id("root")
            .child(choice_form(
                false,
                &[("first", "First"), ("second", "Second")],
            ))
            .child(Node::text("text", [Span::plain("old")]).id("other")),
        test_metrics(),
    );
    app.frame(360, 300);
    let control = Control::Field {
        node: "choice".into(),
        field: "value".into(),
    };
    app.focus_control(Some(control));
    app.key(Key::Down);
    assert!(app.choice.is_some());
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Replace {
            id: "other".into(),
            node: Node::text("text", [Span::plain("new")]).id("other"),
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert!(app.choice.is_some());
    app.key(Key::Escape);
    assert!(app.choice.is_none());
    app.key(Key::Down);
    assert!(app.choice.is_some());
    app.observed(&DocumentUpdate::Changed {
        tree: &[ViewOp::Replace {
            id: "choice".into(),
            node: choice_form(false, &[("first", "Changed"), ("third", "New")]),
        }],
        live: &[],
        reset_live: false,
    })
    .unwrap();
    assert!(app.choice.is_none());
    app.key(Key::Down);
    let scene = app.frame(360, 300);
    assert!(any_op(
        &scene.ops,
        |op| matches!(op, Op::Text { text, .. } if text == "Changed")
    ));
    assert!(!any_op(
        &scene.ops,
        |op| matches!(op, Op::Text { text, .. } if text == "Second")
    ));
}

#[test]
fn the_composer_pins_to_the_bottom_and_never_scrolls() {
    fn composer() -> Node {
        Node::new(
            "composer",
            Kind::Fields {
                fields: vec![Field {
                    id: "prompt".into(),
                    label: "Message".into(),
                    value: String::new(),
                    hint: None,
                    read_only: false,
                    secret: false,
                    kind: FieldKind::Block,
                }],
            },
        )
        .id("composer")
    }
    let mut view = Node::section("session").id("session");
    view.children = (0..30)
        .map(|i| {
            Node::text("message.user", [Span::plain(format!("message {i}"))]).id(format!("msg.{i}"))
        })
        .collect();
    view.children.push(composer());
    let mut app = DocumentUi::new(view, test_metrics());
    let prompt = Control::Field {
        node: "composer".into(),
        field: "prompt".into(),
    };
    app.frame(400, 200);
    let (_, at_tail) = app
        .control_center(&prompt)
        .expect("the composer paints at the tail");
    // Reading history must not carry the input away with it.
    app.scroll(-3_000.0);
    app.frame(400, 200);
    let (_, reading) = app
        .control_center(&prompt)
        .expect("the composer stays painted");
    assert_eq!(
        reading, at_tail,
        "the composer must not scroll with the transcript"
    );
    assert!(at_tail < 200.0);
    // Input takes no flow space: the reading flow never lands on it.
    assert!(
        app.viewport
            .visible()
            .iter()
            .all(|p| p.id != super::flow::FlowId::Node("composer".into()))
    );
    let theme = misa_render::Theme::dark();
    let mut cursor = super::flow::FlowId::Top;
    for _ in 0..200 {
        let Some(next) = app.document.next_flow(&cursor, &theme) else {
            break;
        };
        assert!(
            !matches!(&next, super::flow::FlowId::Node(id) if id == "composer"),
            "the reading flow must skip input"
        );
        cursor = next;
    }
}
