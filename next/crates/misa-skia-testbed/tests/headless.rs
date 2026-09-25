//! Real Vulkan/Ganesh integration tests: a missing ICD is a failure, not a skip.
use misa_skia_testbed::testbed::headless::Headless;
use misa_skia_ui::Op;
use misa_window_core::{Event, Key, Size};
use std::time::Duration;

fn text(ops: &[Op]) -> String {
    let mut result = String::new();
    for op in ops {
        match op {
            Op::Text { text, .. } => result.push_str(text),
            Op::Group { ops, .. } | Op::ClipRect { ops, .. } => result.push_str(&text(ops)),
            _ => {}
        }
    }
    result
}

#[test]
fn native_key_pointer_and_resize_reach_gpu_readback() {
    let mut host = Headless::new(Size {
        width: 500,
        height: 320,
    })
    .expect("Vulkan/Ganesh required (lavapipe works)");
    eprintln!("Headless Vulkan device: {}", host.device_name());
    let initial = host.frame().unwrap();
    assert!(text(&initial.scene.ops).contains("Select me"));
    assert_eq!(initial.pixels.dimensions(), (500, 320));
    assert_eq!(initial.pixels.get_pixel(0, 0).0, [20, 22, 26, 255]);
    host.input(Event::Key(Key::Enter { newline: false }));
    let keyed = host.frame().unwrap();
    assert!(text(&keyed.scene.ops).contains("Selected"));
    assert_ne!(keyed.pixels, initial.pixels);
    assert_eq!(keyed.pixels.get_pixel(50, 160).0, [45, 105, 150, 255]);
    host.input(Event::Pointer {
        x: 40.0,
        y: 155.0,
        dragging: false,
    });
    let clicked = host.frame().unwrap();
    assert_eq!(clicked.pixels, initial.pixels);
    host.input(Event::Resize(Size {
        width: 130,
        height: 300,
    }));
    let resized = host.frame().unwrap();
    assert_eq!(resized.pixels.dimensions(), (130, 300));
    assert_eq!((resized.scene.width, resized.scene.height), (130.0, 300.0));
    assert_eq!(resized.pixels.get_pixel(0, 0).0, [20, 22, 26, 255]);
}

#[test]
fn semantic_text_pointer_resize_and_fake_clock_paint_glyphs_on_gpu() {
    let mut host = Headless::new(Size {
        width: 600,
        height: 440,
    })
    .expect("Vulkan/Ganesh required (lavapipe works)");
    host.select_semantic();
    let first = host.frame().unwrap();
    let first_text = text(&first.scene.ops);
    assert!(first_text.contains("Local draft"));
    assert!(
        first_text.contains("Semantic fixture·"),
        "activity glyph at t=0"
    );
    assert_eq!(first.deadline, Some(Duration::from_millis(160)));
    assert_eq!(first.pixels.get_pixel(0, 0).0, [20, 22, 26, 255]);
    assert!(first.pixels.pixels().any(|p| p.0 != [20, 22, 26, 255]));
    // The form starts focused; committed text uses Event::Text, not a winit key.
    host.input(Event::Text("!".into()));
    let edited = host.frame().unwrap();
    assert!(text(&edited.scene.ops).contains("Local draft!"));
    assert_ne!(first.pixels, edited.pixels);
    // Hit a semantic field through the same pointer event the native host sends.
    // The field location is obtained from the App's hit map, not guessed geometry.
    let field = host.field_hit().expect("visible note field");
    host.input(Event::Pointer {
        x: field.0,
        y: field.1,
        dragging: false,
    });
    host.input(Event::Text("?".into()));
    let pointed = host.frame().unwrap();
    assert!(text(&pointed.scene.ops).contains("Local draft!?"));
    assert_ne!(pointed.pixels, edited.pixels);
    host.advance_to(Duration::from_millis(159));
    let same_phase = host.frame().unwrap();
    assert_eq!(same_phase.pixels, pointed.pixels);
    assert_eq!(same_phase.deadline, Some(Duration::from_millis(160)));
    host.advance_to(Duration::from_millis(320));
    let pulse = host.frame().unwrap();
    assert_ne!(
        text(&pulse.scene.ops),
        text(&same_phase.scene.ops),
        "activity glyph must advance"
    );
    assert_ne!(
        pulse.pixels, same_phase.pixels,
        "changed glyph must reach GPU readback"
    );
    assert_eq!(pulse.deadline, Some(Duration::from_millis(480)));
    host.input(Event::Resize(Size {
        width: 480,
        height: 400,
    }));
    let resized = host.frame().unwrap();
    assert_eq!(resized.pixels.dimensions(), (480, 400));
    assert!(text(&resized.scene.ops).contains("Local draft!?"));
}
