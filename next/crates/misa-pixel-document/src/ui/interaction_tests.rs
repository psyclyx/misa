use super::Control;
use super::interaction::{Hit, InteractionMap, PointerResult};
use misa_pixel_ui::{LaidOutRow, Rect};
use misa_window_core::PointerPhase;

fn row(map: &mut InteractionMap, text: &str) {
    map.add_row(
        10.0,
        20.0,
        80.0,
        18.0,
        LaidOutRow {
            bounds: Rect {
                x: 10.0,
                y: 20.0,
                width: 80.0,
                height: 18.0,
            },
            text: text.into(),
            advances: (0..=text.len()).map(|index| index as f32 * 8.0).collect(),
            runs: vec![],
        },
        0,
    );
}

#[test]
fn selection_survives_repaint_but_not_retired_rows() {
    let mut map = InteractionMap::default();
    row(&mut map, "abc");
    assert!(matches!(
        map.pointer(10.0, 21.0, PointerPhase::Press),
        PointerResult::SelectionChanged
    ));
    assert!(matches!(
        map.pointer(26.0, 21.0, PointerPhase::Move),
        PointerResult::SelectionChanged
    ));
    assert_eq!(map.selected_text(), "ab");
    map.begin_frame();
    row(&mut map, "abc");
    map.finish_frame();
    assert_eq!(map.selected_text(), "ab");
    map.begin_frame();
    map.finish_frame();
    assert_eq!(map.selected_text(), "ab");
    map.retire(&[String::new()].into());
    assert_eq!(map.selection(), None);
}

#[test]
fn last_hit_wins_without_drags_activating_controls() {
    let mut map = InteractionMap::default();
    map.add_hit(Hit {
        x: 0.0,
        y: 0.0,
        width: 20.0,
        height: 20.0,
        control: Control::SaveCancel,
    });
    map.add_hit(Hit {
        x: 0.0,
        y: 0.0,
        width: 20.0,
        height: 20.0,
        control: Control::SaveConfirm,
    });
    assert!(matches!(
        map.pointer(5.0, 5.0, PointerPhase::Move),
        PointerResult::None
    ));
    assert!(matches!(
        map.pointer(5.0, 5.0, PointerPhase::Press),
        PointerResult::None
    ));
    assert!(matches!(
        map.pointer(5.0, 5.0, PointerPhase::Release),
        PointerResult::Activate(Control::SaveConfirm)
    ));
    assert!(map.focused(&Control::SaveConfirm));
}
