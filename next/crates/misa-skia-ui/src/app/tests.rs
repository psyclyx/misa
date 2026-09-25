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
fn test_metrics() -> Arc<dyn TextMetrics> {
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
fn frame_positions_nonempty_runs_and_uses_syntax_colours() {
    let mut app = App::new(scene_view(), test_metrics());
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
    let mut app = App::new(view, test_metrics());
    let scene = app.frame(144, 180);
    assert_eq!(app.rows.len(), 3);
    assert_eq!(app.rows[0].geometry.text, "rust");
    let visible = app.clip(&raw, 104.0); // 144px frame minus the 40px document inset
    assert_eq!(app.rows[1].geometry.text, visible);
    assert_eq!(app.rows[2].geometry.text, app.clip("let y = 2;", 104.0));
    let theme = Theme::dark();
    assert_eq!(
        app.rows[0].geometry.runs[0].0,
        theme.role("markdown.code.label")
    );
    assert!(
        app.rows[1]
            .geometry
            .runs
            .iter()
            .any(|(style, text, _)| *style == theme.token("keyword") && text == "let")
    );
    assert!(
        app.rows[1]
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
        app.rows[2]
            .geometry
            .runs
            .iter()
            .any(|(style, text, _)| *style == theme.token("keyword") && text == "let")
    );
    app.key(Key::SelectAll);
    assert_eq!(
        app.selected_text(),
        format!("rust\n{visible}\n{}", app.rows[2].geometry.text)
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
    let scene = App::new(view, test_metrics()).frame_at(640, 480, Duration::ZERO);
    let mut texts = Vec::new();
    let mut rects = 0;
    walk_ops(&scene.ops, &mut |op| match op {
        Op::Text { text, .. } => texts.push(text.clone()),
        Op::Rect { .. } => rects += 1,
        _ => {}
    });
    assert!(texts.iter().any(|text| text.contains("quoted")));
    assert!(texts.iter().any(|text| text.contains('─')));
    assert!(texts.iter().any(|text| text.contains("budget")));
    assert!(texts.iter().any(|text| text.contains("12k")));
    assert!(rects > 0);
}

#[test]
fn frame_theme_changes_styles_without_changing_the_view() {
    let mut app = App::new(scene_view(), test_metrics());
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
    let mut app = App::new(scene_view(), test_metrics());
    app.frame_at(800, 600, Duration::ZERO);
    let ops = &app.cache["msg.1"].ops;
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
    let mut app = App::new(
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
    let mut app = App::new(
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
    let status = app.cache["status"].ops.clone();
    let root = app.cache["session"].ops.clone();
    let message = app.cache["message"].ops.clone();
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
    assert_eq!(app.tick, 0);
    assert!(Arc::ptr_eq(&status, &app.cache["status"].ops));
    assert_eq!(app.rendered_nodes, 0);
    assert!(scene.ops.iter().any(|op| matches!(op, Op::Group { .. })));

    app.frame_at(640, 480, ms(320)); // Skip phase 1.
    assert_eq!(app.tick, 2);
    assert!(!Arc::ptr_eq(&status, &app.cache["status"].ops));
    assert!(!Arc::ptr_eq(&root, &app.cache["session"].ops));
    assert_ne!(first, pulse(&app.cache["status"].ops));
    assert!(Arc::ptr_eq(&message, &app.cache["message"].ops));
    let phase_two = app.cache["status"].ops.clone();
    app.frame_at(640, 480, ms(960)); // Wrap to phase 2, no invalidation.
    assert_eq!(app.tick, 2);
    assert!(Arc::ptr_eq(&phase_two, &app.cache["status"].ops));
    app.frame_at(640, 480, ms(1120));
    assert_eq!(app.tick, 3);
    assert!(!Arc::ptr_eq(&phase_two, &app.cache["status"].ops));
    assert!(Arc::ptr_eq(&message, &app.cache["message"].ops));
    app.frame_at(640, 480, ms(1280));
    assert_eq!(app.tick, 0);
    assert_eq!(first, pulse(&app.cache["status"].ops));
}

#[test]
fn hidden_cached_indicator_does_not_keep_the_window_awake() {
    let mut app = App::new(
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
    app.expanded.insert("details".into());
    app.invalidate("details");
    app.frame_at(640, 480, Duration::ZERO);
    assert!(app.animating());
    let old = app.cache["status"].ops.clone();
    app.expanded.remove("details");
    app.invalidate("details");
    app.frame_at(640, 480, Duration::from_millis(160));
    assert!(!app.animating());
    assert!(Arc::ptr_eq(&old, &app.cache["status"].ops));
    app.expanded.insert("details".into());
    app.invalidate("details");
    app.frame_at(640, 480, Duration::from_millis(320));
    assert!(app.animating());
    assert!(!Arc::ptr_eq(&old, &app.cache["status"].ops));
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
    let mut app = App::new(
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
    let status = app.cache["status"].ops.clone();
    let before = app.cache["before.0"].ops.clone();
    let after = app.cache["after.0"].ops.clone();
    let bounds = app.cache["session"].indicators[0].clone();
    assert_eq!(bounds.id, "status");
    assert!(bounds.top > size.height as f32);
    assert!(bounds.bottom < app.content_height - size.height as f32);
    assert!(
        !app.animating(),
        "follow scroll leaves the status above the viewport"
    );
    assert!(
        app.drive(Event::Redraw(size), Duration::from_millis(160))
            .deadline
            .is_none()
    );
    assert_eq!(app.rendered_nodes, 0);
    assert!(Arc::ptr_eq(&status, &app.cache["status"].ops));

    // Intersection uses the group's actual top and height, including nested groups.
    let below = 20.0 + bounds.top - size.height as f32;
    app.scroll(below - app.scroll);
    assert!(!app.animating());
    app.scroll(1.0);
    assert!(app.animating());
    let above = 20.0 + bounds.bottom;
    app.scroll(above - app.scroll);
    assert!(!app.animating());
    app.scroll(-1.0);
    assert!(app.animating());

    // Center on the actual cached group placement, rather than a tree index.
    let scroll_to_status = 20.0 + bounds.top - 80.0;
    app.scroll(scroll_to_status - app.scroll);
    assert!(
        app.animating(),
        "wheel scrolling updates visibility immediately"
    );
    assert!(
        app.drive(Event::Redraw(size), Duration::from_millis(320))
            .deadline
            .is_some()
    );
    assert!(!Arc::ptr_eq(&status, &app.cache["status"].ops));
    let visible = app.cache["status"].ops.clone();
    assert!(Arc::ptr_eq(&before, &app.cache["before.0"].ops));
    assert!(Arc::ptr_eq(&after, &app.cache["after.0"].ops));

    app.scroll(100_000.0);
    assert!(!app.animating());
    assert!(
        app.drive(Event::Redraw(size), Duration::from_millis(480))
            .deadline
            .is_none()
    );
    assert_eq!(app.rendered_nodes, 0);
    assert!(Arc::ptr_eq(&visible, &app.cache["status"].ops));
    app.scroll(scroll_to_status - app.scroll);
    assert!(
        app.drive(Event::Redraw(size), Duration::from_millis(640))
            .deadline
            .is_some()
    );
    assert!(!Arc::ptr_eq(&visible, &app.cache["status"].ops));
    assert!(Arc::ptr_eq(&before, &app.cache["before.0"].ops));
    assert!(Arc::ptr_eq(&after, &app.cache["after.0"].ops));
}

#[test]
fn idle_repaints_never_invalidate_retained_owners() {
    let mut app = App::new(
        Node::section("session")
            .id("session")
            .child(Node::text("message.user", [Span::plain("idle")]).id("message")),
        test_metrics(),
    );
    app.frame_at(640, 480, Duration::ZERO);
    let owner = app.cache["message"].ops.clone();
    app.frame_at(640, 480, Duration::from_secs(10));
    assert!(!app.animating());
    assert_eq!(app.rendered_nodes, 0);
    assert!(Arc::ptr_eq(&owner, &app.cache["message"].ops));
}

#[test]
fn changing_local_theme_preserves_drafts_and_rebuilds_cached_colors() {
    let mut app = App::new(form("panel.input", FieldKind::Inline), test_metrics());
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
    let mut app = App::new(view, test_metrics());
    app.commands = serde_json::from_value(
        serde_json::json!([{ "id":"model", "label":"Model" }, { "id":"clear", "label":"Clear" }]),
    )
    .unwrap();
    app.key(Key::Commands);
    app.key(Key::Down);
    assert_eq!(
        app.picker.as_ref().unwrap().selected().unwrap().value,
        "clear"
    );
    app.drive(Event::Text("mod".into()), Duration::ZERO)
        .commands;
    assert_eq!(app.picker.as_ref().unwrap().matches().len(), 1);
    assert!(app.key(Key::Enter { newline: false }).is_empty());
    assert_eq!(app.field_text("compose", "prompt"), Some("/model "));
    assert!(app.picker.is_none());
    app.key(Key::Commands);
    app.drive(Event::Text("zzzz".into()), Duration::ZERO)
        .commands;
    assert!(app.key(Key::Enter { newline: false }).is_empty());
    assert!(app.picker.is_some());
    app.key(Key::Escape);
    assert_eq!(app.field_text("compose", "prompt"), Some("/model "));
}
#[test]
fn empty_picker_and_modal_input_preserve_draft() {
    let mut app = App::new(form("form", FieldKind::Inline), test_metrics());
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
    assert!(app.picker.is_none());
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
    let mut app = App::new(view, test_metrics());
    let scene = app.frame(900, 120);
    assert!(any_op(
        &scene.ops,
        |op| matches!(op, Op::Text { text, .. } if text.contains("scripted/model"))
    ));
    app.apply_tree([&ViewOp::Replace {
        id: "model.value".into(),
        node: Node::new(
            "value.text",
            Kind::Fact {
                value: Value::str("changed/model"),
            },
        )
        .id("model.value"),
    }])
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
    let mut app = App::new(view, test_metrics());
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
fn semantic_action_uses_measured_button_paint_and_hit_bounds() {
    let view = Node::section("root").id("root").action(Action {
        id: "go".into(),
        on: ActionOn::Submit,
        label: Some("Run".into()),
        args: Value::Null,
    });
    let mut app = App::new(view, test_metrics());
    let scene = app.frame_at(400, 200, Duration::ZERO);
    let hit = app
        .hits
        .iter()
        .find(|hit| matches!(&hit.control, Control::Action { action, .. } if action == "go"))
        .unwrap();
    let bounds = misa_pixel_ui::Rect {
        x: hit.x,
        y: hit.y,
        width: hit.width,
        height: hit.height,
    };
    assert!(bounds.contains(hit.x + 1.0, hit.y + 1.0));
    assert!(!bounds.contains(hit.x + hit.width, hit.y + 1.0));
    fn button_clip(ops: &[Op]) -> bool {
        ops.iter().any(|op| match op {
            Op::Group { ops, .. } => button_clip(ops),
            Op::ClipRect { width, ops, .. } => {
                *width == 248.0 && matches!(&ops[0], Op::Text { text, .. } if text == "Run")
            }
            _ => false,
        })
    }
    assert!(button_clip(&scene.ops));
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
    let mut app = App::new(form("one", FieldKind::Inline), test_metrics());
    let draft = "W界".repeat(25);
    app.drive(Event::Text(draft.clone()), Duration::ZERO);
    let end = field_paint(&app.frame(160, 400));
    assert_eq!(app.field_text("one", "value"), Some(draft.as_str()));
    assert_eq!(end.4, draft);
    assert!(end.5 >= end.0 && end.5 + 1.5 <= end.0 + end.2 + 0.01);
    assert!(app.field_viewports[&("one".into(), "value".into())].x > 0.0);
    app.drive(Event::Key(Key::Home), Duration::ZERO);
    let home = field_paint(&app.frame(160, 400));
    assert_eq!(home.5, home.0);
    assert_eq!(app.field_viewports[&("one".into(), "value".into())].x, 0.0);
    app.drive(Event::Key(Key::End), Duration::ZERO);
    let end_again = field_paint(&app.frame(160, 400));
    assert!((end_again.5 - end.5).abs() < 0.01);
    app.drive(Event::Text("終".into()), Duration::ZERO);
    let typed = field_paint(&app.frame(160, 400));
    assert!(typed.5 + 1.5 <= typed.0 + typed.2 + 0.01);
    assert!(typed.5 >= typed.0);
    assert!(app.field_text("one", "value").unwrap().ends_with("終"));
    let offset = app.field_viewports[&("one".into(), "value".into())].x;
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
    let mut app = App::new(view, test_metrics());
    app.drive(Event::Text("W界".repeat(20)), Duration::ZERO);
    let paint = field_paint(&app.frame(160, 400));
    assert_eq!(paint.4, "•".repeat(40));
    assert!(!paint.4.contains('W'));
    let offset = app.field_viewports[&("one".into(), "value".into())].x;
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
    let mut app = App::new(form("one", FieldKind::Block), test_metrics());
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
        .get_mut(&("one".into(), "value".into()))
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
    let mut app = App::new(view.clone(), test_metrics());
    app.focus = Some(Control::Field {
        node: "composer".into(),
        field: "prompt".into(),
    });
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
        .get_mut(&("composer".into(), "prompt".into()))
        .unwrap()
        .set_text("");
    app.reject_prompt("restored".into(), "Rejected".into());
    assert_eq!(app.field_text("composer", "prompt"), Some("restored"));
    assert_eq!(
        app.tree.snapshot(),
        view,
        "local reports must not change authoritative content"
    );
}
#[test]
fn drafts_survive_updates_and_submit_only_the_target_panel() {
    let view = Node::section("root")
        .child(form("one", FieldKind::Inline))
        .child(form("two", FieldKind::Inline));
    let mut app = App::new(view.clone(), test_metrics());
    app.focus = Some(Control::Field {
        node: "one".into(),
        field: "value".into(),
    });
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
    let mut app = App::new(form("one", FieldKind::Inline), test_metrics());
    assert_eq!(
        app.focus,
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
fn boolean_keyboard_input_cannot_produce_invalid_values() {
    let mut app = App::new(form("one", FieldKind::Bool), test_metrics());
    app.drive(Event::Text("nonsense".into()), Duration::ZERO)
        .commands;
    assert_eq!(app.field_text("one", "value"), Some(""));
    app.drive(Event::Text(" ".into()), Duration::ZERO).commands;
    assert_eq!(app.field_text("one", "value"), Some("true"));
    app.drive(Event::Text(" ".into()), Duration::ZERO).commands;
    assert_eq!(app.field_text("one", "value"), Some("false"));
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
    let mut app = App::new(view.clone(), test_metrics());
    app.activate(Control::Disclosure("tool".into()));
    app.set_view(view);
    app.frame(500, 500);
    app.focus = None;
    app.key(Key::SelectAll);
    assert_eq!(app.key(Key::Copy), vec![Command::Copy("héllo λ".into())]);
    assert!(app.expanded.contains("tool"));
}
#[test]
fn save_destination_is_an_explicit_local_command() {
    let mut app = App::new(Node::section("root"), test_metrics());
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
    assert!(app.save.is_none());
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
    let mut app = App::new(view, test_metrics());
    app.images.insert(
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
        app.rows
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
    let mut app = App::new(
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
    assert!(
        app.images
            .values()
            .map(|image| image.as_raw().len())
            .sum::<usize>()
            <= 32 * 1024 * 1024
    );
    app.frame(800, 600);
    assert!(
        app.hits
            .iter()
            .any(|hit| hit.control == Control::LoadImage(reference("a")))
    );
    assert_eq!(
        app.activate(Control::LoadImage(reference("a"))),
        vec![Command::LoadImage(reference("a"))]
    );
    app.image("a".into(), Arc::new(image::RgbaImage::new(1, 1)));
    app.apply_tree([&ViewOp::Remove { id: "a".into() }])
        .unwrap();
    assert!(!app.images.contains_key("a"));
    app.image("a".into(), Arc::new(image::RgbaImage::new(1, 1)));
    assert!(
        !app.images.contains_key("a"),
        "Late decode cannot repopulate a removed owner"
    );
}
#[test]
fn live_updates_do_not_steal_the_local_save_dialog() {
    let view = form("panel.input", FieldKind::Inline);
    let mut app = App::new(view.clone(), test_metrics());
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
    let mut app = App::new(Node::text("text", [Span::plain("界hi")]), test_metrics());
    app.frame(400, 200);
    app.pointer(20.0 + 18.1, 25.0, false);
    app.pointer(20.0 + 27.1, 25.0, true);
    assert_eq!(app.selected_text(), "h");
}
#[test]
fn blank_text_row_accepts_pointer_selection_across_its_width() {
    let mut app = App::new(
        Node::text("text", [Span::plain("first\n\nlast")]),
        test_metrics(),
    );
    app.frame(320, 200);
    let blank = app
        .rows
        .iter()
        .enumerate()
        .find(|(_, row)| row.geometry.text.is_empty())
        .map(|(index, row)| (index, row.x, row.y))
        .expect("blank row");
    app.pointer(blank.1 + 5.0, blank.2 + 1.0, false);
    assert_eq!(app.selection, Some(((blank.0, 0), (blank.0, 0))));
}

#[test]
fn styled_narrow_unicode_rows_share_paint_hit_and_selection_positions() {
    let view = Node::text(
        "text",
        [Span::plain("界"), Span::strong("ill"), Span::plain(" W")],
    );
    let mut app = App::new(view, test_metrics());
    let scene = app.frame(400, 200);
    let row = &app.rows[0];
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
    let mut app = App::new(
        view,
        Arc::new(Counted {
            longest: longest.clone(),
            total: total.clone(),
        }),
    );
    let scene = app.frame(100, 120);
    assert_eq!(app.rows.len(), 2);
    assert_eq!(app.rows[0].geometry.text, label);
    assert_eq!(app.rows[1].geometry.text, value);
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
    let row = &app.rows[1];
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
    let mut app = App::new(view, test_metrics());
    let scene = app.frame(95, 500);
    let cell_width = 55.0 / 2.0;
    let viewport = cell_width - 10.0;
    assert!(app.rows.len() >= 4);
    let mut clips = Vec::new();
    walk_ops(&scene.ops, &mut |op| {
        if let Op::ClipRect { x, width, ops, .. } = op {
            clips.push((*x, *width, ops.clone()));
        }
    });
    assert_eq!(clips.len(), app.rows.len());
    assert!(clips.iter().all(|(_, width, _)| *width == viewport));
    assert!(clips.iter().any(|(left, width, ops)| ops.iter().any(|op| matches!(op, Op::Text { x, text, .. } if text.contains('界') && x + app.measure(text) > left + width))));
    for hit in &app.hits {
        if let Control::Text(index) = hit.control {
            let row = &app.rows[index];
            assert_eq!(row.width, viewport);
            assert!(hit.x + hit.width <= row.x + viewport);
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
    assert!(selected.ops.iter().any(|op| matches!(op, Op::ClipRect { ops, .. } if ops.iter().any(|op| matches!(op, Op::Rect { style, .. } if *style == app.colors().selection)))));
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
    let mut app = App::new(view, test_metrics());
    app.frame(95, 250);
    assert!(app.rows.len() > 2);
    for row in &app.rows {
        assert!(
            row.edge(row.geometry.text.chars().count()) <= 55.0 + 0.01,
            "row exceeded available pixels: {:?}",
            row.geometry.text
        );
        assert!(row.geometry.text.starts_with("▏ "));
    }
    let copied = app
        .rows
        .iter()
        .map(|row| row.geometry.text.trim_start_matches("▏ "))
        .collect::<String>();
    assert_eq!(copied, "WWiiiiiiiiiiiiii界界界");
}

#[test]
fn report_reflows_at_measured_pixel_width() {
    let mut app = App::new(Node::section("root"), test_metrics());
    app.report("Report".into(), Value::str("WWWWiiii界界"));
    app.frame(132, 280);
    let lines = &app.report.as_ref().unwrap().lines;
    assert!(lines.len() > 2);
    assert_eq!(lines.concat(), "WWWWiiii界界");
    assert!(lines.iter().all(|line| app.measure(line) <= 36.0));
    app.frame(300, 280);
    assert_eq!(app.report.as_ref().unwrap().lines, vec!["WWWWiiii界界"]);
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
    let mut app = App::new(view, Arc::new(Counted(calls.clone())));
    app.frame(400, 100);
    let geometry = Arc::clone(&app.cache["child"].rows[0].geometry);
    assert!(Arc::ptr_eq(&geometry, &app.rows[0].geometry));
    let y = app.rows[0].y;
    let scroll = app.scroll;
    let measured = calls.load(Ordering::Relaxed);
    app.frame(400, 100);
    assert_eq!(calls.load(Ordering::Relaxed), measured);
    assert!(Arc::ptr_eq(&geometry, &app.rows[0].geometry));
    app.scroll(-10.0);
    app.frame(400, 100);
    assert_eq!(calls.load(Ordering::Relaxed), measured);
    assert!(Arc::ptr_eq(&geometry, &app.cache["child"].rows[0].geometry));
    assert!(Arc::ptr_eq(&geometry, &app.rows[0].geometry));
    assert_eq!(app.rows[0].x, 20.0);
    assert!(app.scroll < scroll);
    assert_eq!(app.rows[0].y, y + scroll - app.scroll);

    app.apply_tree([&ViewOp::Replace {
        id: "child".into(),
        node: Node::text("text", [Span::plain("界ill updated")]).id("child"),
    }])
    .unwrap();
    app.frame(400, 100);
    assert!(!Arc::ptr_eq(
        &geometry,
        &app.cache["child"].rows[0].geometry
    ));
    assert!(Arc::ptr_eq(
        &app.cache["child"].rows[0].geometry,
        &app.rows[0].geometry
    ));
    assert_eq!(app.rows[0].geometry.text, "界ill updated");
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
        let mut app = App::new(view.clone(), test_metrics());
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
        assert_eq!(app.rendered_nodes, 0);
    }
}
#[test]
fn scoped_document_transaction_settles_live_text_without_rebuilding_history() {
    use misa_proto::sync::Stream;
    for owners in [10, 1000] {
        let view = protocol_view(owners);
        let mut app = App::new(Node::section("empty"), test_metrics());
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
        let retained = app.cache["message.0"].ops.clone();
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
        assert!(app.streams.is_empty());
        assert!(app.tree.contains("answer"));
        let scene = app.frame(800, 600);
        assert!(Arc::ptr_eq(&retained, &app.cache["message.0"].ops));
        assert!(
            app.rendered_nodes <= 3,
            "only changed ancestors and final answer need layout"
        );
        let mut expected = view;
        expected.children[0].children.push(answer);
        let mut cold = App::new(expected, test_metrics());
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
    app: &mut App,
    tree: &misa_proto::sync::IndexedTree,
    streams: &[misa_proto::sync::Stream],
) {
    let mut view = tree.snapshot();
    let mut overlay = Node::section("streams").id("streams");
    for stream in streams {
        let owner = stream
            .id
            .rsplit_once('.')
            .map_or(stream.id.as_str(), |(owner, _)| owner);
        if !stream.text.is_empty() && !tree.contains(owner) {
            overlay.children.push(
                Node::text(&stream.role, [Span::plain(&stream.text)])
                    .id(&stream.id)
                    .state(misa_proto::view::State::Streaming),
            );
        }
    }
    if !overlay.children.is_empty() {
        view.children[0].children.push(overlay);
    }
    let scene = app.frame(800, 600);
    let expected = App::new(view, test_metrics()).frame(800, 600);
    assert_eq!(scene, expected);
}
fn observed_changes(
    app: &mut App,
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
        let mut app = App::new(view, test_metrics());
        app.frame(800, 600);
        let retained: Vec<_> = (0..owners)
            .map(|index| app.cache[&format!("message.{index}")].ops.clone())
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
        assert_eq!(app.rendered_nodes, 4);
        for (index, ops) in retained.iter().enumerate() {
            assert!(Arc::ptr_eq(
                ops,
                &app.cache[&format!("message.{index}")].ops
            ));
        }
        cold_scene(&mut app, &tree, &[stream.clone()]);
        let op = ViewOp::Replace {
            id: "message.0".into(),
            node: Node::text("message", [Span::plain("changed")]).id("message.0"),
        };
        tree.apply(&op).unwrap();
        observed_changes(&mut app, vec![op], vec![], false);
        app.frame(800, 600);
        assert_eq!(app.rendered_nodes, 3);
        for (index, ops) in retained.iter().enumerate().skip(1) {
            assert!(Arc::ptr_eq(
                ops,
                &app.cache[&format!("message.{index}")].ops
            ));
        }
        cold_scene(&mut app, &tree, &[stream]);
    }
}
#[test]
fn stream_completion_and_owner_removal_match_cold_rebuilds() {
    use misa_proto::sync::{IndexedTree, Stream, StreamUpdate};
    let view = protocol_view(4);
    let mut tree = IndexedTree::new(view.clone());
    let mut app = App::new(view, test_metrics());
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
fn headless_driver_uses_fake_clock_and_never_presents() {
    use misa_window_core::Clock;
    struct FakeClock(Duration);
    impl Clock for FakeClock {
        fn elapsed(&self) -> Duration {
            self.0
        }
    }
    let mut clock = FakeClock(Duration::ZERO);
    let mut app = App::new(
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
    let mut app = App::new(view, test_metrics());
    app.frame(800, 600);
    let owner = app.cache["transcript"].ops.clone();
    app.drive(Event::Text("draft".into()), Duration::ZERO)
        .commands;
    app.frame(800, 600);
    assert_eq!(app.rendered_nodes, 2);
    assert!(Arc::ptr_eq(&owner, &app.cache["transcript"].ops));
    assert_eq!(app.field_text("panel.input", "value"), Some("draft"));
    let sent = app.key(Key::Enter { newline: false });
    assert!(
        matches!(&sent[..],[Command::Intent(Intent::Action {fields,..})] if fields[0].value=="draft")
    );
    assert_eq!(app.field_text("panel.input", "value"), Some("draft"));
}
