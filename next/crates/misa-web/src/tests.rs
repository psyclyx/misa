use super::*;
use misa_proto::view::{Action, BlobRef, Field};

async fn invoke(runtime: &Runtime, command: &str, input: Value) -> misa_proto::invocation::Outcome {
    use misa_protocol::invocation::{CallContext, Dispatcher};
    Dispatcher::new(
        CallContext {
            principal: "web-fixture".into(),
            connection: 1,
        },
        1,
        Default::default(),
        Default::default(),
    )
    .dispatch(
        runtime,
        misa_proto::invocation::Invocation {
            id: next_id(),
            scope: runtime.scope(),
            command: command.into(),
            input,
        },
    )
    .await
    .outcome
}
fn read(runtime: &Runtime, id: &str) -> misa_proto::observation::Content {
    use misa_proto::observation::{Member, Selection};
    let definition = runtime
        .query_exports()
        .into_iter()
        .find(|definition| definition.id == id)
        .expect("installed query");
    let result = runtime
        .read_selection(&Selection {
            scope: runtime.scope(),
            members: std::collections::BTreeMap::from([(
                "result".into(),
                Member {
                    query: misa_proto::Query::new(id),
                    encoding: definition.result.encoding(),
                    contract: definition.contract,
                    optional: false,
                },
            )]),
        })
        .unwrap();
    result.members["result"].clone()
}

#[tokio::test]
async fn attachment_button_downloads_kernel_confirmed_bytes_as_a_file() {
    let kernel = Arc::new(misa_kernel::LocalKernel::new(
        misa_kernel::ScriptedProvider::always("done"),
    ));
    let blobs = kernel.blobs().clone();
    let stored = blobs.put(PNG, Some("image/png")).unwrap();
    let runtime = Runtime::start(
        "save",
        "Save",
        None,
        kernel,
        "scripted",
        "test",
        misa_value::Value::Null,
    );
    assert!(matches!(
        invoke(
            &runtime,
            "session.prompt",
            Value::map([
                ("text", Value::str("save it")),
                (
                    "attachments",
                    serde_json::from_value(serde_json::to_value(vec![stored.clone()]).unwrap())
                        .unwrap()
                )
            ])
        )
        .await,
        misa_proto::invocation::Outcome::Accepted { .. }
    ));
    fn target(node: &Node) -> Option<String> {
        if node
            .actions
            .iter()
            .any(|action| action.id == "attachment.save")
        {
            return Some(node.id.clone());
        }
        node.children.iter().find_map(target)
    }
    let mut revision = runtime.watch_rev();
    let (tree, node) = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let misa_proto::observation::Content::Document(document) =
                read(&runtime, "conversation.presentation")
            else {
                panic!("conversation is a document");
            };
            let tree = document.tree;
            if let Some(node) = target(&tree) {
                break (tree, node);
            }
            revision.changed().await.unwrap();
        }
    })
    .await
    .expect("attachment was not durably recorded");
    let html = render_main(&tree);
    assert!(html.contains("action=\"./download\""));
    assert!(html.contains("Save attachment"));
    let response = local_download(
        &runtime,
        &Source::Local(blobs),
        std::collections::HashMap::from([("node".into(), node)]),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers()[axum::http::header::CONTENT_DISPOSITION],
        format!("attachment; filename=\"{}.png\"", stored.hash)
    );
    assert_eq!(
        axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap()
            .as_ref(),
        PNG
    );
}

#[tokio::test]
async fn usage_presentation_is_a_typed_finite_document() {
    use misa_value::Value;
    let kernel = misa_kernel::LocalKernel::new(misa_kernel::ScriptedProvider::new([]));
    let runtime = misa_session::Runtime::start(
        "usage",
        "Usage",
        None,
        std::sync::Arc::new(kernel),
        "scripted",
        "test",
        Value::Null,
    );
    assert!(matches!(
        invoke(
            &runtime,
            "session.usage.refresh",
            Value::map([] as [(&str, Value); 0])
        )
        .await,
        misa_proto::invocation::Outcome::Completed { .. }
    ));
    let facts = misa_kernel::usage::parse(
        "kimi",
        &serde_json::json!({"usage":{"limit":100,"used":25}}),
    );
    assert!(
        runtime
            .dispatch(
                misa_reframe::Event::new("kernel/usage")
                    .with("id", Value::str("usage.1"))
                    .with("provider", Value::str("scripted"))
                    .with("facts", facts)
            )
            .is_empty()
    );
    let misa_proto::observation::Content::Value(value) = read(&runtime, "usage.report") else {
        panic!("usage report is data");
    };
    assert_eq!(
        value
            .get("session")
            .and_then(|session| session.get("cost_micros"))
            .and_then(Value::as_i64),
        Some(0)
    );
    let misa_proto::observation::Content::Document(document) = read(&runtime, "usage.presentation")
    else {
        panic!("usage presentation is a document");
    };
    let tree = document.tree;
    misa_proto::view::validate(&tree).unwrap();
    let html = render_main(&tree);
    let terminal = misa_lines::to_plain(&misa_lines::render(
        &tree,
        &misa_render::Theme::plain(),
        100,
    ));
    for expected in ["Kimi", "remaining", "75", "$0"] {
        assert!(html.contains(expected), "{expected}: {html}");
        assert!(terminal.contains(expected), "{expected}: {terminal}");
    }
    assert!(html.contains("value.money"));
}

/// Bytes the store will recognise as a picture, which is what a store sniffs for.
const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 1, 2, 3];

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
    .action(Action {
        id: "composer.submit".into(),
        on: ActionOn::Submit,
        label: Some("Send".into()),
        args: misa_value::Value::Null,
    })
    .child(Node::text("composer.hint", [Span::plain("enter to send")]))
}

fn view() -> Node {
    Node::section("session")
        .id("session")
        .child(
            Node::text("message.user", [Span::plain("what is <this>?")])
                .id("msg.1")
                .state(NodeState::Done),
        )
        .child(
            Node::new(
                "tool.call",
                Kind::Collapsible {
                    summary: vec![Span::plain("echo")],
                },
            )
            .id("call.1")
            .child(Node::new(
                "tool.result",
                Kind::Code {
                    lang: Some("rust".into()),
                    text: "let x = 1;".into(),
                },
            )),
        )
        .child(Node::new(
            "tool.batch",
            Kind::List {
                ordered: true,
                items: vec![
                    vec![Node::text("x", [Span::plain("first")])],
                    vec![Node::text("x", [Span::plain("second")])],
                ],
                markers: Vec::new(),
            },
        ))
        .child(composer())
}

#[test]
fn a_submission_with_attachments_is_made_the_way_the_protocol_declares_one() {
    // A form of text fields cannot carry bytes, so when this client is holding some the
    // submission stops being "forward the session's action" and becomes the declared
    // text-with-attachments intent. Every other action is forwarded untouched: what an
    // action means is the session's business, and a client that guessed would be a client
    // with a second opinion about the session's own vocabulary.
    let pending = vec![BlobRef {
        hash: "a".repeat(64),
        len: 12,
        media: Some("image/png".into()),
    }];
    let mut form = HashMap::new();
    form.insert("node".to_string(), "composer".to_string());
    form.insert("action".to_string(), "composer.submit".to_string());
    form.insert("prompt".to_string(), "look at this".to_string());
    assert_eq!(
        submitted_intent(&form, &pending),
        Intent::Prompt {
            text: "look at this".into(),
            attachments: pending.clone()
        }
    );

    // Nothing pending: the session's own action, with its fields, exactly as submitted.
    match submitted_intent(&form, &[]) {
        Intent::Action { action, fields, .. } => {
            assert_eq!(action, "composer.submit");
            assert!(
                fields
                    .iter()
                    .any(|field| field.id == "prompt" && field.value == "look at this")
            );
        }
        other => panic!("a form with nothing pending became a `{}`", other.name()),
    }

    // Something pending, but not a submission: still the action, because an attachment is
    // not a reason to turn a button into a prompt.
    let mut other = HashMap::new();
    other.insert("action".to_string(), "queue.take".to_string());
    match submitted_intent(&other, &pending) {
        Intent::Action { action, .. } => assert_eq!(action, "queue.take"),
        other => panic!("a queue button became a `{}`", other.name()),
    }
}

#[test]
fn the_strip_says_what_is_waiting_and_offers_the_way_to_add_more() {
    // The upload form is there even with nothing pending, because a client that could only
    // attach once it already had an attachment could never attach the first one.
    let empty = lead(&[]);
    assert!(empty.contains("action=\"./attach\""), "{empty}");
    assert!(empty.contains("multipart/form-data"), "{empty}");
    assert!(!empty.contains("/detach"), "{empty}");

    let pending = vec![
        BlobRef {
            hash: "a".repeat(64),
            len: 12,
            media: Some("image/png".into()),
        },
        BlobRef {
            hash: "b".repeat(64),
            len: 3,
            media: None,
        },
    ];
    let strip = lead(&pending);
    assert!(strip.contains("2 attachments"), "{strip}");
    assert!(strip.contains(&"a".repeat(64)), "{strip}");
    assert!(strip.contains("image/png"), "{strip}");
    assert!(strip.contains("action=\"./detach\""), "{strip}");
    // The bytes are never in the document: only their names are, which is what keeps a
    // transcript and a strip small however large the file is.
    assert!(strip.len() < 1_000, "{strip}");
}

#[tokio::test]
async fn a_blob_is_served_from_the_store_under_a_name_that_cannot_change() {
    let blobs = Arc::new(misa_kernel::Blobs::in_memory());
    let stored = blobs.put(PNG, None).expect("a blob");
    let source = Source::Local(blobs);

    let response = blob_response(Some(&source), &stored.hash).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
    // Named by the hash of its own content, so the bytes behind the name can never change:
    // a browser may keep them for as long as it likes, and a transcript that scrolls back
    // fetches nothing.
    let cache = response.headers()[header::CACHE_CONTROL].to_str().unwrap();
    assert!(cache.contains("immutable"), "{cache}");
    assert!(
        cache.contains("private"),
        "authenticated blob bytes must not enter a shared HTTP cache"
    );

    let body = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(&body[..], PNG);
}

#[tokio::test]
async fn a_name_that_is_not_a_content_hash_is_answered_before_it_is_asked() {
    let blobs = Arc::new(misa_kernel::Blobs::in_memory());
    let source = Source::Local(blobs);
    for name in [
        "../../etc/shadow",
        "not a hash",
        &"a".repeat(63),
        &"A".repeat(64),
    ] {
        let response = blob_response(Some(&source), name).await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "`{name}` was answered"
        );
    }
    // A client with no store behind it says so rather than pretending the blob is missing
    // from a store it does not have.
    let hash = "a".repeat(64);
    assert_eq!(
        blob_response(None, &hash).await.status(),
        StatusCode::NOT_FOUND
    );
}

#[test]
fn a_message_becomes_a_paragraph_and_its_roles_become_classes() {
    let html = render_main(&view());
    assert!(
        html.contains("<p class=\"n-message.user\" id=\"msg.1\" data-state=\"done\">"),
        "{html}"
    );
    assert!(html.contains("what is &lt;this&gt;?"), "{html}");
}

#[test]
fn nothing_a_session_sends_reaches_the_page_unescaped() {
    let node = Node::text(
        "message.assistant",
        [Span::plain("<script>alert(1)</script>")],
    );
    let html = render_main(&node);
    assert!(!html.contains("<script>"), "{html}");
    assert!(html.contains("&lt;script&gt;"), "{html}");
}

#[test]
fn a_collapsible_is_a_details_element_because_html_already_has_one() {
    let html = render_main(&view());
    assert!(html.contains("<details"), "{html}");
    assert!(html.contains("<summary>"), "{html}");
    assert!(
        !html.contains("<details open"),
        "a closed node was rendered open"
    );
}

#[test]
fn a_code_block_carries_its_text_and_language_and_no_highlighting() {
    let html = render_main(&view());
    assert!(html.contains("data-lang=\"rust\""), "{html}");
    assert!(html.contains("let x = 1;"), "{html}");
    // Highlighting is the browser's; the session sent no capture.
    assert!(!html.contains("data-token"), "{html}");
}

#[test]
fn a_form_is_a_form_and_works_without_the_script() {
    let html = render_main(&view());
    assert!(
        html.contains("method=\"post\" action=\"./intent\""),
        "{html}"
    );
    assert!(html.contains("<textarea name=\"prompt\""), "{html}");
    assert!(
        html.contains("<input type=\"hidden\" name=\"action\" value=\"composer.submit\">"),
        "{html}"
    );
    assert!(html.contains("type=\"submit\""), "{html}");
}

#[test]
fn secret_policy_masks_every_field_shape() {
    for kind in [
        FieldKind::Inline,
        FieldKind::Block,
        FieldKind::Bool,
        FieldKind::Choice {
            options: vec![],
            selected: Some("private".into()),
        },
    ] {
        for read_only in [false, true] {
            let node = Node::new(
                "secret",
                Kind::Fields {
                    fields: vec![Field {
                        id: "value".into(),
                        label: "Secret".into(),
                        value: "private".into(),
                        hint: None,
                        kind: kind.clone(),
                        read_only,
                        secret: true,
                    }],
                },
            );
            let html = render_main(&node);
            assert!(!html.contains("private"), "{html}");
            assert!(
                html.contains(if read_only {
                    "••••"
                } else {
                    "type=\"password\""
                }),
                "{html}"
            );
        }
    }
}

#[test]
fn a_read_only_block_keeps_its_shape_without_offering_an_edit() {
    let node = Node::new(
        "report",
        Kind::Fields {
            fields: vec![Field {
                id: "body".into(),
                label: "Report".into(),
                value: "one\ntwo".into(),
                hint: None,
                kind: FieldKind::Block,
                read_only: true,
                secret: false,
            }],
        },
    );
    let html = render_main(&node);
    assert!(html.contains("<pre>one\ntwo</pre>"), "{html}");
    assert!(!html.contains("textarea"), "{html}");
}

#[test]
fn a_panel_is_a_report_whose_buttons_work_without_the_script() {
    // The shape a session opens for `/login`: a row that is a fact, a field somebody types
    // into, and a dismiss. A row is not a text box, and each button posts on its own —
    // a button outside a form is a button that does nothing.
    let panel = Node::section("panel")
        .id("authorize")
        .label("Authorize `kimi-coding`")
        .child(
            Node::new(
                "panel.row",
                Kind::Fields {
                    fields: vec![Field {
                        id: "row.0".into(),
                        label: "code".into(),
                        value: "AAAA-BBBB".into(),
                        hint: None,
                        read_only: true,
                        secret: false,
                        kind: FieldKind::Inline,
                    }],
                },
            )
            .id("panel.row.0"),
        )
        .action(Action {
            id: "panel.close".into(),
            on: ActionOn::Click,
            label: Some("Dismiss".into()),
            args: misa_value::Value::Null,
        });
    let html = render_main(&panel);
    assert!(html.contains("<dt>code</dt><dd>AAAA-BBBB</dd>"), "{html}");
    assert!(
        !html.contains("<input type=\"text\" name=\"row.0\""),
        "a row became an input: {html}"
    );
    assert!(
        html.contains("method=\"post\" action=\"./intent\"")
            && html.contains("value=\"panel.close\""),
        "a panel's button does not post anything: {html}"
    );
    assert!(
        html.contains("value=\"authorize\""),
        "the form does not name the node: {html}"
    );
}

#[test]
fn an_image_is_a_reference_rather_than_bytes() {
    let node = Node::new(
        "screenshot",
        Kind::Image {
            blob: BlobRef {
                hash: "a".repeat(64),
                len: 9,
                media: Some("image/png".into()),
            },
            alt: "a chart".into(),
            width: 10,
            height: 10,
        },
    );
    let html = render_main(&node);
    assert!(html.contains("/blob/"), "{html}");
    assert!(html.contains("alt=\"a chart\""), "{html}");
}

#[test]
fn a_document_is_a_document() {
    let commands = vec![
        misa_kit::intent::Command::new("model", "Model", "choose a model").arg(
            misa_proto::preparation::Arg::new("model", "Model")
                .required()
                .from("models"),
        ),
    ];
    let html = document_parts("a demo", "demo", &commands, &render_main(&view()), "");
    assert!(html.starts_with("<!doctype html>"), "{html}");
    assert!(html.contains("<main id=\"main\">"), "{html}");
    assert!(html.contains("/style.css"), "{html}");
    assert!(html.contains("/app.js"), "{html}");
    assert!(html.contains("data-session=\"demo\""), "{html}");
    assert!(html.contains("id=\"theme\""), "{html}");
}

#[test]
fn nested_disclosures_do_not_rewrite_their_ancestors() {
    let tree = Node::section("session")
        .id("session")
        .child(Node::text("message", [Span::plain("before")]))
        .child(
            Node::new(
                "tool.call",
                Kind::Collapsible {
                    summary: vec![Span::plain("details")],
                },
            )
            .id("call")
            .child(Node::text("body", [Span::plain("inside")])),
        )
        .child(Node::text("message", [Span::plain("after")]));
    let html = render_main(&tree);
    assert!(html.starts_with("<section "), "{html}");
    assert_eq!(html.matches("<details ").count(), 1, "{html}");
    assert_eq!(html.matches("</details>").count(), 1, "{html}");
    assert!(html.ends_with("</section>"), "{html}");
    assert!(html.find("before").unwrap() < html.find("<details ").unwrap());
    assert!(html.find("</details>").unwrap() < html.find("after").unwrap());
}

#[test]
fn the_stylesheet_themes_roles_and_nothing_else() {
    assert!(
        STYLE.contains(".n-message\\.user"),
        "no role rule in the stylesheet"
    );
    assert!(STYLE.contains("data-token"), "captures have no styling");
    // A stylesheet that named a colour inline per node would be a stylesheet
    // that had left the theming model.
    assert!(!STYLE.contains("style=\""), "{STYLE}");
}
#[test]
fn a_submitted_form_becomes_the_intent_both_paths_send() {
    let mut form = HashMap::new();
    form.insert("node".to_string(), "panel".to_string());
    form.insert("action".to_string(), "panel.submit".to_string());
    form.insert("value".to_string(), "sk-a-secret".to_string());
    match intent_from_form(&form) {
        Intent::Action {
            node,
            action,
            fields,
            ..
        } => {
            assert_eq!(node, "panel");
            assert_eq!(action, "panel.submit");
            // The two hidden fields are not values a panel asked for.
            assert_eq!(fields.len(), 1);
            assert_eq!(fields[0].id, "value");
        }
        other => panic!("expected an action, got {other:?}"),
    }
}

#[test]
fn document_namespaces_do_not_rewrite_command_targets() {
    let node = Node::section("plugin.panel")
        .id("root")
        .child(
            Node::new(
                "items",
                Kind::List {
                    ordered: false,
                    items: vec![vec![Node::text("item", [Span::plain("Pet")]).id("item")]],
                    markers: Vec::new(),
                },
            )
            .id("list"),
        )
        .action(Action {
            id: "pet.feed".into(),
            on: ActionOn::Click,
            label: Some("Feed".into()),
            args: Value::Null,
        });
    let first = render_scoped(&node, "panel-1:");
    let second = render_scoped(&node, "panel-2:");
    for (html, prefix) in [(&first, "panel-1:"), (&second, "panel-2:")] {
        for id in ["root", "list", "item"] {
            assert!(html.contains(&format!("id=\"{prefix}{id}\"")), "{html}");
        }
        assert!(html.contains("name=\"node\" value=\"root\""), "{html}");
        assert!(
            html.contains("name=\"action\" value=\"pet.feed\""),
            "{html}"
        );
    }
    assert!(!first.contains("panel-2:"));
    assert!(!second.contains("panel-1:"));
}

#[test]
fn status_keeps_semantic_children_and_actions_in_the_dom() {
    let node = Node::section("status.indicators").id("status").child(
        Node::section("indicator.plan")
            .id("plan")
            .child(
                Node::new(
                    "value.money",
                    Kind::Fact {
                        value: Value::Int(1_240_000),
                    },
                )
                .id("cost"),
            )
            .action(Action {
                id: "usage.open".into(),
                on: ActionOn::Click,
                label: Some("Usage details".into()),
                args: Value::Null,
            }),
    );
    let html = render_main(&node);
    assert!(html.contains("id=\"plan\""), "{html}");
    assert!(html.contains("id=\"cost\""), "{html}");
    assert!(
        html.contains("<data value=\"1240000\">$1.24</data>"),
        "{html}"
    );
    assert!(html.contains("name=\"node\" value=\"plan\""), "{html}");
    assert!(
        html.contains("name=\"action\" value=\"usage.open\""),
        "{html}"
    );
    assert!(html.contains(">Usage details</button>"), "{html}");
}

#[test]
fn a_fact_is_marked_up_with_its_value_and_written_by_the_clients_formatter() {
    let node = Node::section("value.money").child(Node::new(
        "value.money",
        Kind::Fact {
            value: misa_value::Value::Int(1_240_000),
        },
    ));
    let html = render_main(&node);
    assert!(
        html.contains("<data value=\"1240000\">$1.24</data>"),
        "{html}"
    );
}

#[test]
fn the_declarations_become_the_browsers_own_completion() {
    let commands = vec![
        misa_kit::intent::Command::new("model", "Model", "choose a model").arg(
            misa_proto::preparation::Arg::new("model", "Model")
                .required()
                .from("models"),
        ),
    ];
    let html = command_declarations(&commands);
    assert!(html.contains("<datalist id=\"misa-commands\">"), "{html}");
    assert!(html.contains("value=\"/model\""), "{html}");
    // No script, no round trip, and it works with JavaScript switched off.
    assert!(!html.contains("<script"), "{html}");
}

#[test]
fn a_session_with_nothing_declared_adds_nothing_to_the_document() {
    assert!(command_declarations(&[]).is_empty());
}

#[test]
fn a_heading_picks_its_level_and_a_quote_becomes_a_blockquote() {
    let node = Node::section("message.assistant")
        .child(Node::new(
            "message.assistant.markdown.heading",
            Kind::Heading {
                level: 2,
                spans: vec![Span::plain("Title")],
            },
        ))
        .child(
            Node::new("message.assistant.markdown.quote", Kind::Quote).child(Node::text(
                "message.assistant.markdown.paragraph",
                [Span::plain("quoted")],
            )),
        )
        .child(Node::new("message.assistant.markdown.rule", Kind::Rule));
    let html = render_main(&node);
    assert!(
        html.contains("<h2 class=\"n-message.assistant.markdown.heading\"><span>Title</span></h2>"),
        "{html}"
    );
    assert!(html.contains("<blockquote"), "{html}");
    // A rule is void: one tag, and no closing tag to mismatch.
    assert!(
        html.contains("<hr class=\"n-message.assistant.markdown.rule\">"),
        "{html}"
    );
    assert!(!html.contains("</hr>"), "{html}");
}

#[test]
fn a_link_closes_as_an_anchor_and_a_combined_mark_nests() {
    let node = Node::text(
        "message.assistant",
        [
            Span::link("docs", "https://example.com"),
            Span::plain(" "),
            Span {
                text: "x".into(),
                kind: misa_proto::view::SpanKind::StrongEmphasis,
            },
        ],
    );
    let html = render_scoped(&node, "");
    assert!(
        html.contains("<a href=\"https://example.com\">docs</a>"),
        "{html}"
    );
    assert!(html.contains("<strong><em>x</em></strong>"), "{html}");
}

#[test]
fn a_task_list_renders_its_ballot_box() {
    let node = Node::new(
        "items",
        Kind::List {
            ordered: false,
            items: vec![
                vec![Node::text("item", [Span::plain("todo")])],
                vec![Node::text("item", [Span::plain("done")])],
            ],
            markers: vec![Some(false), Some(true)],
        },
    );
    let html = render_scoped(&node, "");
    assert!(html.contains("class=\"task\""), "{html}");
    assert!(html.contains("☐"), "{html}");
    assert!(html.contains("☑"), "{html}");
}
