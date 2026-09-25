use misa_pixel_document::ui::{DocumentUi, DocumentUpdate};
use misa_pixel_ui::{Op, Scene};
use misa_proto::{
    sync::{IndexedTree, Stream, StreamUpdate, ViewOp},
    view::{Kind, Node, Span},
};
use misa_skia_paint::{draw_scene, png, raster, text_metrics};
use misa_style::Color;
use std::sync::Arc;
use std::time::Duration;

const BACKGROUND: Color = Color::Rgb(20, 22, 26);

fn view(owners: usize) -> Node {
    Node::section("session").id("session").child(
        Node::section("transcript")
            .id("transcript")
            .children((0..owners).map(|i| {
                Node::text("message", [Span::plain("unchanged transcript")])
                    .id(format!("message.{i}"))
            })),
    )
}

#[test]
fn scene_paints_to_png() {
    let metrics = text_metrics().expect("Skia text metrics for PNG test");
    let scene = DocumentUi::new(view(10), metrics).frame_at(800, 600, Duration::ZERO);
    let bytes = png(&scene, BACKGROUND).expect("a png");
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert!(
        bytes.len() > 1000,
        "the raster is suspiciously small: {} bytes",
        bytes.len()
    );
}

#[test]
fn raster_oracle_is_deterministic_after_reset() {
    for owners in [10, 1000] {
        let view = view(owners);
        let mut app = DocumentUi::new(
            view.clone(),
            text_metrics().expect("Skia text metrics for raster oracle"),
        );
        let mut hashes = std::collections::BTreeSet::new();
        for _ in 0..6 {
            app.set_view(view.clone());
            hashes.insert(png(&app.frame_at(800, 600, Duration::ZERO), BACKGROUND).unwrap());
        }
        assert_eq!(hashes.len(), 1, "render oracle is not deterministic");
    }
}

#[test]
fn retained_bitmap_reaches_raster() {
    let view = Node::section("session").child(Node::new(
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
    let mut app = DocumentUi::new(
        view,
        text_metrics().expect("Skia text metrics for bitmap test"),
    );
    app.image(
        "image".into(),
        Arc::new(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([255, 0, 0, 255]),
        )),
    );
    // The app's image layout gives the bitmap a visible drawn rectangle.
    let pixels = raster(&app.frame_at(200, 600, Duration::ZERO), BACKGROUND).unwrap();
    assert!(pixels.pixels().any(|pixel| pixel.0 == [255, 0, 0, 255]));
}

fn parity(app: &mut DocumentUi, tree: &IndexedTree, streams: &[Stream]) {
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
    let warm = app.frame_at(800, 600, Duration::ZERO);
    let cold = DocumentUi::new(
        view,
        text_metrics().expect("Skia text metrics for cold parity frame"),
    )
    .frame_at(800, 600, Duration::ZERO);
    assert_eq!(
        raster(&warm, BACKGROUND).unwrap(),
        raster(&cold, BACKGROUND).unwrap()
    );
}

fn changed(app: &mut DocumentUi, tree: &[ViewOp], live: &[StreamUpdate], reset_live: bool) {
    app.observed(&DocumentUpdate::Changed {
        tree,
        live,
        reset_live,
    })
    .unwrap();
}

#[test]
fn scoped_transaction_and_stream_changes_match_cold_pixels() {
    for owners in [10, 1000] {
        let view = view(owners);
        let mut tree = IndexedTree::new(view.clone());
        let mut app = DocumentUi::new(
            view,
            text_metrics().expect("Skia text metrics for stream test"),
        );
        let mut stream = Stream {
            id: "live.text".into(),
            role: "message.assistant".into(),
            text: "é".into(),
        };
        app.observed(&DocumentUpdate::Reset {
            tree: &tree.snapshot(),
            streams: &[stream.clone()],
        })
        .unwrap();
        parity(&mut app, &tree, &[stream.clone()]);
        changed(
            &mut app,
            &[],
            &[StreamUpdate::Append {
                id: stream.id.clone(),
                offset: 2,
                text: "終".into(),
            }],
            false,
        );
        stream.text.push('終');
        parity(&mut app, &tree, &[stream.clone()]);
        let replace = ViewOp::Replace {
            id: "message.0".into(),
            node: Node::text("message", [Span::plain("changed")]).id("message.0"),
        };
        tree.apply(&replace).unwrap();
        changed(&mut app, &[replace], &[], false);
        parity(&mut app, &tree, &[stream.clone()]);
        let answer = Node::text("message.assistant", [Span::plain(&stream.text)]).id("live");
        let insert = ViewOp::Insert {
            parent: "transcript".into(),
            before: None,
            node: answer,
        };
        tree.apply(&insert).unwrap();
        changed(
            &mut app,
            &[insert],
            &[StreamUpdate::End {
                id: stream.id.clone(),
            }],
            false,
        );
        parity(&mut app, &tree, &[]);
        let remove = ViewOp::Remove {
            id: "message.0".into(),
        };
        tree.apply(&remove).unwrap();
        changed(&mut app, &[remove], &[], false);
        parity(&mut app, &tree, &[]);
    }
}

#[test]
fn canvas_and_raster_share_clear_and_ops() {
    let scene = Scene {
        width: 8.0,
        height: 8.0,
        ops: vec![Op::Rect {
            x: 1.0,
            y: 1.0,
            width: 2.0,
            height: 2.0,
            style: misa_style::Style::rgb(255, 0, 0),
        }],
    };
    let mut surface = skia_safe::surfaces::raster_n32_premul((8, 8)).unwrap();
    draw_scene(surface.canvas(), &scene, BACKGROUND).unwrap();
    let pixels = raster(&scene, BACKGROUND).unwrap();
    assert_eq!(pixels.get_pixel(0, 0).0, [20, 22, 26, 255]);
    assert_eq!(pixels.get_pixel(1, 1).0, [255, 0, 0, 255]);
    let pixmap = surface.peek_pixels().unwrap();
    let bgra = pixmap.bytes().unwrap();
    assert_eq!(&bgra[..4], &[26, 22, 20, 255]);
}

#[test]
fn measured_advance_and_baseline_agree_with_painted_text() {
    let metrics = text_metrics().expect("a paint font");
    let size = 32.0;
    let advance = metrics.measure("MMMM", size);
    let line = metrics.line_metrics(size);
    assert!(advance.is_finite() && advance > 0.0);
    assert!(line.ascent < 0.0 && line.descent > 0.0);
    assert!(line.leading >= 0.0 && line.line_height >= line.descent - line.ascent);
    let x = 12.0;
    let y = 10.0;
    let scene = Scene {
        width: 250.0,
        height: 80.0,
        ops: vec![
            Op::Text {
                x,
                y,
                size,
                style: misa_style::Style::rgb(255, 0, 0),
                text: "MMMM".into(),
            },
            Op::Text {
                x: x + advance,
                y,
                size,
                style: misa_style::Style::rgb(0, 255, 0),
                text: "MMMM".into(),
            },
        ],
    };
    let pixels = raster(&scene, BACKGROUND).unwrap();
    let ink = |channel: usize| {
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for (px, py, color) in pixels.enumerate_pixels() {
            if color.0[channel] > 200 && color.0[1 - channel] < 100 {
                xs.push(px as f32);
                ys.push(py as f32);
            }
        }
        (xs.into_iter().reduce(f32::min).unwrap(), ys)
    };
    let (red_x, red_y) = ink(0);
    let (green_x, green_y) = ink(1);
    // Both runs have identical ink; their measured advance must position them.
    assert!(((green_x - red_x) - advance).abs() <= 1.0);
    for ys in [red_y, green_y] {
        assert!(
            ys.iter()
                .all(|py| *py >= y && *py <= y - line.ascent + line.descent)
        );
    }
}

#[test]
fn narrow_quote_table_selected_row_is_clipped_in_skia_pixels() {
    let view = Node::new("quote", Kind::Quote).child(Node::new(
        "table",
        Kind::Table {
            head: vec![vec![Span::plain("W")], vec![Span::plain("界")]],
            rows: vec![vec![vec![Span::plain("界W")], vec![Span::plain("W界")]]],
            align: vec![],
        },
    ));
    let mut app = DocumentUi::new(view, text_metrics().unwrap());
    app.frame_at(92, 500, Duration::ZERO);
    app.key(misa_pixel_document::ui::Key::SelectAll);
    let scene = app.frame_at(92, 500, Duration::ZERO);
    let cell_width = (92.0 - 40.0) / 2.0;
    let left = 20.0 + cell_width + 5.0;
    let right = 20.0 + 2.0 * cell_width - 5.0;
    // Pull actual retained paint ops (including the selection overlay) from the
    // second cell, keeping their parent translations. This removes the table's
    // full-width background so any leaked glyph or selection pixel is visible.
    fn cell_ops(ops: &[Op], dx: f32, dy: f32, left: f32, right: f32, out: &mut Vec<Op>) {
        for op in ops {
            match op {
                Op::Group { x, y, ops } => cell_ops(ops, dx + x, dy + y, left, right, out),
                Op::ClipRect { x, width, .. }
                    if (dx + x - left).abs() < 0.01 && (dx + x + width - right).abs() < 0.01 =>
                {
                    out.push(Op::Group {
                        x: dx,
                        y: dy,
                        ops: Arc::new(vec![op.clone()]),
                    });
                }
                _ => {}
            }
        }
    }
    let mut ops = Vec::new();
    cell_ops(&scene.ops, 0.0, 0.0, left, right, &mut ops);
    assert!(
        ops.len() >= 3,
        "second cell rows and selection must be present"
    );
    assert!(ops.iter().any(|op| matches!(op, Op::Group { ops, .. } if matches!(&ops[0], Op::ClipRect { ops, .. } if ops.iter().any(|op| matches!(op, Op::Rect { .. }))))));
    let metrics = text_metrics().unwrap();
    assert!(ops.iter().any(|op| matches!(op, Op::Group { x, ops, .. } if matches!(&ops[0], Op::ClipRect { ops, .. } if ops.iter().any(|op| matches!(op, Op::Text { x: run_x, size, text, .. } if x + run_x + metrics.measure(text, *size) > right))))),
        "test must exercise an oversized glyph in the cell");
    let isolated = Scene {
        width: 92.0,
        height: 500.0,
        ops,
    };
    let pixels = raster(&isolated, BACKGROUND).unwrap();
    let mut ink = 0;
    for (x, _, pixel) in pixels.enumerate_pixels() {
        if pixel.0 != [20, 22, 26, 255] {
            ink += 1;
            assert!(
                (x as f32) >= left && (x as f32) < right,
                "Skia painted outside the second table cell at {x} (bounds {left}..{right})"
            );
        }
    }
    assert!(ink > 0, "expected real Skia text and selection pixels");
}

#[test]
fn font_line_metrics_scale_with_the_paint_size() {
    let metrics = text_metrics().unwrap();
    let small = metrics.line_metrics(12.0);
    let large = metrics.line_metrics(24.0);
    assert!(small.line_height.is_finite() && small.line_height > 0.0);
    assert!(large.line_height > small.line_height);
    assert!(metrics.measure("W i é", 24.0) > metrics.measure("W i é", 12.0));
    assert_eq!(metrics.measure("", 12.0), 0.0);
}
