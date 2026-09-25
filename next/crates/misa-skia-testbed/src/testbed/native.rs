//! Toolkit-owned scene: no semantic tree or protocol types involved.
use crate::{Op, Scene};
use misa_pixel_ui::{Button, Rect, TextMetrics};
use misa_style::Style;

#[derive(Clone, Copy)]
enum NativeAction {
    Toggle,
}

pub(super) struct Dashboard {
    pub selected: bool,
}

impl Dashboard {
    pub fn toggle(&mut self) {
        self.selected = !self.selected;
    }

    fn button(
        &self,
        width: u32,
        metrics: &dyn TextMetrics,
    ) -> misa_pixel_ui::PlacedButton<NativeAction> {
        Button {
            id: NativeAction::Toggle,
            bounds: Rect {
                x: 36.0,
                y: 150.0,
                width: (width as f32 - 72.0).clamp(1.0, 180.0),
                height: 42.0,
            },
            label: if self.selected {
                "Selected"
            } else {
                "Select me"
            }
            .into(),
            font_size: 15.0,
            background: if self.selected {
                Style::rgb(45, 105, 150)
            } else {
                Style::rgb(65, 74, 86)
            },
            foreground: Style::rgb(230, 232, 236),
        }
        .place(metrics)
    }

    pub fn click(&mut self, x: f32, y: f32, width: u32, metrics: &dyn TextMetrics) -> bool {
        let button = self.button(width, metrics);
        if button.bounds.contains(x, y) {
            match button.id {
                NativeAction::Toggle => self.toggle(),
            }
            true
        } else {
            false
        }
    }

    pub fn frame(&self, width: u32, height: u32, metrics: &dyn TextMetrics) -> Scene {
        let button = self.button(width, metrics);
        let card_width = (width as f32 - 40.0).max(1.0);
        let card_height = (height as f32 - 160.0).clamp(1.0, 310.0);
        let mut scene = Scene {
            width: width as f32,
            height: height as f32,
            ops: vec![Op::Rect {
                x: 20.0,
                y: 74.0,
                width: card_width,
                height: card_height,
                style: Style::rgb(35, 40, 48),
            }],
        };
        scene.ops.extend(button.ops);
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
            24.0,
            height as f32 - 48.0,
            14.0,
            "Enter / Space or click the button to toggle",
        );
        scene
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Metrics;
    impl TextMetrics for Metrics {
        fn measure(&self, text: &str, size: f32) -> f32 {
            text.len() as f32 * size / 2.0
        }
        fn advances(&self, text: &str, size: f32) -> Vec<f32> {
            (0..=text.len()).map(|i| i as f32 * size / 2.0).collect()
        }
        fn line_metrics(&self, size: f32) -> misa_pixel_ui::LineMetrics {
            misa_pixel_ui::LineMetrics {
                ascent: -size,
                descent: 0.0,
                leading: 0.0,
                line_height: size,
            }
        }
    }
    #[test]
    fn button_is_local_and_resizes_with_viewport() {
        let mut dashboard = Dashboard { selected: false };
        assert!(!dashboard.click(0.0, 0.0, 500, &Metrics));
        assert!(dashboard.click(40.0, 155.0, 500, &Metrics));
        assert!(dashboard.selected);
        let narrow = dashboard.frame(130, 300, &Metrics);
        let wide = dashboard.frame(500, 300, &Metrics);
        assert_eq!(narrow.width, 130.0);
        assert_eq!(wide.width, 500.0);
        assert!(matches!(narrow.ops[1], Op::Rect { width: 58.0, .. }));
        assert!(!dashboard.click(94.0, 155.0, 130, &Metrics));
    }
}
