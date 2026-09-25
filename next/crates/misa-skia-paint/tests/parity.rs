use misa_proto::{
    sync::{IndexedTree, Stream, StreamUpdate, ViewOp},
    view::{Kind, Node, Span},
};
use misa_render::{Color, Theme};
use misa_skia_paint::{draw_scene, png, raster};
use misa_skia_ui::{
    Layout, Op, Scene,
    app::{App, DocumentUpdate},
    scene,
};
use std::sync::Arc;

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
    let scene = scene(&view(10), &Theme::dark(), 80, 24, Layout::default());
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
        let mut app = App::new(view.clone());
        let mut hashes = std::collections::BTreeSet::new();
        for _ in 0..6 {
            app.set_view(view.clone());
            hashes.insert(png(&app.frame(800, 600), BACKGROUND).unwrap());
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
    let mut app = App::new(view);
    app.images.insert(
        "image".into(),
        Arc::new(image::RgbaImage::from_pixel(
            1,
            1,
            image::Rgba([255, 0, 0, 255]),
        )),
    );
    // The app's image layout gives the bitmap a visible drawn rectangle.
    let pixels = raster(&app.frame(200, 600), BACKGROUND).unwrap();
    assert!(pixels.pixels().any(|pixel| pixel.0 == [255, 0, 0, 255]));
}

fn parity(app: &mut App, tree: &IndexedTree, streams: &[Stream]) {
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
    let warm = app.frame(800, 600);
    let cold = App::new(view).frame(800, 600);
    assert_eq!(
        raster(&warm, BACKGROUND).unwrap(),
        raster(&cold, BACKGROUND).unwrap()
    );
}

fn changed(app: &mut App, tree: &[ViewOp], live: &[StreamUpdate], reset_live: bool) {
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
        let mut app = App::new(view);
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
            style: misa_render::Style::rgb(255, 0, 0),
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
