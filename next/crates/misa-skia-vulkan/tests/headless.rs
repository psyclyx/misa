use misa_proto::view::{Node, Span};
use misa_render::{Color, Theme};
use misa_skia_ui::{Layout, Op, Scene, app::App};
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
fn app_drives_two_deterministic_gpu_frames() {
    let tree = Node::section("session")
        .id("session")
        .child(Node::text("message.user", [Span::plain("A real Skia canvas")]).id("message"));
    let mut app = App::new(tree.clone());
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
    // Independently constructed semantic tree -> scene also renders on this context.
    let scene = misa_skia_ui::scene(&tree, &Theme::dark(), 24, 6, Layout::default());
    assert!(!scene.ops.is_empty());
    assert!(renderer.render(&scene, BG).is_ok());
}
