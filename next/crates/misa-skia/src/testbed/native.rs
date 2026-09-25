//! Toolkit-owned scene: no semantic tree or protocol types involved.
use crate::{Op, Scene};
use misa_render::Style;

pub(super) struct Dashboard {
    pub selected: bool,
}

impl Dashboard {
    pub fn toggle(&mut self) {
        self.selected = !self.selected;
    }

    pub fn click(&mut self, x: f32, y: f32, width: u32) -> bool {
        let (left, top, button_width) = button_bounds(width);
        if x >= left && x < left + button_width && y >= top && y < top + 42.0 {
            self.toggle();
            true
        } else {
            false
        }
    }

    pub fn frame(&self, width: u32, height: u32) -> Scene {
        let (left, top, button_width) = button_bounds(width);
        let card_width = (width as f32 - 40.0).max(1.0);
        let card_height = (height as f32 - 160.0).clamp(1.0, 310.0);
        let mut scene = Scene {
            width: width as f32,
            height: height as f32,
            ops: vec![],
        };
        let mut rect = |x, y, w, h, color: (u8, u8, u8)| {
            scene.ops.push(Op::Rect {
                x,
                y,
                width: w,
                height: h,
                style: Style::rgb(color.0, color.1, color.2),
            })
        };
        rect(20.0, 74.0, card_width, card_height, (35, 40, 48));
        rect(
            left,
            top,
            button_width,
            42.0,
            if self.selected {
                (45, 105, 150)
            } else {
                (65, 74, 86)
            },
        );
        let mut text = |x, y, size, content: &str| {
            scene.ops.push(Op::Text {
                x,
                y,
                size,
                style: Style::rgb(230, 232, 236),
                text: content.into(),
            })
        };
        text(24.0, 20.0, 20.0, "Native dashboard");
        text(
            36.0,
            95.0,
            15.0,
            "This card is built from Scene / Op, not misa-proto.",
        );
        text(
            left + 12.0,
            top + 10.0,
            15.0,
            if self.selected {
                "Selected"
            } else {
                "Select me"
            },
        );
        text(
            24.0,
            height as f32 - 48.0,
            14.0,
            "Enter / Space or click the button to toggle",
        );
        scene
    }
}

fn button_bounds(width: u32) -> (f32, f32, f32) {
    (36.0, 150.0, (width as f32 - 72.0).clamp(1.0, 180.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn button_is_local_and_resizes_with_viewport() {
        let mut dashboard = Dashboard { selected: false };
        assert!(!dashboard.click(0.0, 0.0, 500));
        assert!(dashboard.click(40.0, 155.0, 500));
        assert!(dashboard.selected);
        let narrow = dashboard.frame(130, 300);
        let wide = dashboard.frame(500, 300);
        assert_eq!(narrow.width, 130.0);
        assert_eq!(wide.width, 500.0);
        assert!(matches!(narrow.ops[1], Op::Rect { width, .. } if width == 58.0));
    }
}
