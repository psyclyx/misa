use misa_proto::view::{Node, Span};
use misa_render::Color;
use misa_skia_ui::{Op, Scene, app::App};
use misa_skia_vulkan::Renderer;
use std::{sync::Arc, time::Duration};

const BG: Color = Color::Rgb(20, 22, 26);

#[test]
fn ganesh_draws_scene_ops_and_reads_rgba() {
    let mut renderer = Renderer::new().expect("Vulkan/Ganesh device required (lavapipe is fine)");
    eprintln!(
        "Vulkan device: {} (queue {})",
        renderer.device_name, renderer.queue_family
    );
    let mut image = image::RgbaImage::new(2, 2);
    image.fill(0);
    image.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
    let group = vec![
        Op::Rect {
            x: 0.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
            style: misa_render::Style::rgb(0, 255, 0),
        },
        Op::Image {
            x: 30.0,
            y: 0.0,
            width: 20.0,
            height: 20.0,
            image: Arc::new(image),
        },
    ];
    let scene = Scene {
        width: 100.0,
        height: 60.0,
        ops: vec![
            Op::Group {
                x: 10.0,
                y: 10.0,
                ops: Arc::new(group),
            },
            Op::Text {
                x: 10.0,
                y: 32.0,
                size: 18.0,
                style: misa_render::Style::rgb(255, 255, 255),
                text: "GPU".into(),
            },
        ],
    };
    let a = renderer.render(&scene, BG).expect("GPU draw/readback");
    let b = renderer
        .render(&scene, BG)
        .expect("second GPU draw/readback");
    assert_eq!(a, b, "two GPU frames of the same scene must match");
    assert_eq!(a.dimensions(), (100, 60));
    assert_eq!(a.get_pixel(0, 0).0, [20, 22, 26, 255]);
    assert_eq!(a.get_pixel(15, 15).0, [0, 255, 0, 255]);
    assert_eq!(a.get_pixel(45, 15).0, [255, 0, 0, 255]);
    assert!(
        a.pixels().any(|p| p.0 == [255, 255, 255, 255]),
        "text must paint"
    );
}

#[test]
fn measured_text_clip_reaches_the_headless_gpu_painter() {
    let mut renderer = Renderer::new().expect("headless Vulkan device");
    let scene = Scene {
        width: 120.0,
        height: 50.0,
        ops: vec![Op::ClipRect {
            x: 10.0,
            y: 5.0,
            width: 20.0,
            height: 30.0,
            ops: Arc::new(vec![Op::Text {
                x: 10.0,
                y: 5.0,
                size: 24.0,
                style: misa_render::Style::rgb(255, 255, 255),
                text: "WWWWWWWW".into(),
            }]),
        }],
    };
    let pixels = renderer.render(&scene, BG).expect("GPU readback");
    let bg = [20, 22, 26, 255];
    assert!(
        (10..30).any(|x| (5..35).any(|y| pixels.get_pixel(x, y).0 != bg)),
        "some measured glyph ink must remain in the viewport"
    );
    assert!(
        (31..120).all(|x| (0..50).all(|y| pixels.get_pixel(x, y).0 == bg)),
        "the GPU must not paint the overflowing glyphs beyond the clip"
    );
}

#[test]
fn app_drives_two_deterministic_gpu_frames() {
    let tree = Node::section("session")
        .id("session")
        .child(Node::text("message.user", [Span::plain("A real Skia canvas")]).id("message"));
    let metrics = misa_skia_paint::text_metrics().expect("Skia text metrics for GPU test");
    let mut app = App::new(tree.clone(), metrics.clone());
    let mut renderer = Renderer::new().expect("Vulkan/Ganesh device required (lavapipe is fine)");
    let a = renderer
        .frame_at(&mut app, 320, 160, Duration::ZERO, BG)
        .expect("first App frame");
    let b = renderer
        .frame_at(&mut app, 320, 160, Duration::ZERO, BG)
        .expect("second App frame");
    assert_eq!(a, b);
    assert_eq!(a.dimensions(), (320, 160));
    assert!(a.pixels().any(|p| p.0 != [20, 22, 26, 255]));
    // Independently constructed App layout also renders on this context.
    let scene = App::new(tree, metrics).frame_at(320, 160, Duration::ZERO);
    assert!(!scene.ops.is_empty());
    assert!(renderer.render(&scene, BG).is_ok());
}
