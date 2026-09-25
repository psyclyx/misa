//! Toolkit-owned scene: no semantic tree or protocol types involved.
use misa_pixel_ui::{Button, Rect, TextFlow, TextMetrics, Viewport};
use misa_pixel_ui::{Op, Scene};
use misa_style::Style;
use std::sync::Arc;

#[derive(Clone, Copy)]
enum NativeAction {
    Toggle,
}

pub struct Dashboard {
    pub selected: bool,
    viewport: Viewport,
    rows: usize,
}

impl Default for Dashboard {
    fn default() -> Self {
        let mut viewport = Viewport::new(0.0, 0.0);
        viewport.pin_to_top();
        Self {
            selected: false,
            viewport,
            rows: 20,
        }
    }
}

const LIST_TOP: f32 = 205.0;
const ROW_HEIGHT: f32 = 26.0;

impl Dashboard {
    pub fn scroll(&mut self, delta: f32) {
        self.viewport.scroll(delta);
    }

    /// Resume following new rows, rather than changing the scroll offset directly.
    pub fn follow_tail(&mut self) {
        self.viewport.follow_tail();
    }

    pub fn append_row(&mut self) {
        self.rows += 1;
    }

    pub fn offset(&self) -> f32 {
        self.viewport.offset()
    }

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

    pub fn frame(&mut self, width: u32, height: u32, metrics: &dyn TextMetrics) -> Scene {
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
            24.0,
            height as f32 - 48.0,
            14.0,
            "Enter / Space or click the button to toggle",
        );
        let flow = TextFlow::new(metrics, 15.0);
        let description = "This card is built from Scene / Op, not misa-proto.";
        let available = (card_width - 32.0).max(0.0);
        for (line, runs) in flow
            .wrap(
                vec![(Style::rgb(230, 232, 236), description.into())],
                available,
            )
            .into_iter()
            .enumerate()
        {
            flow.place(
                Rect {
                    x: 36.0,
                    y: 95.0 + line as f32 * flow.line_height(),
                    width: available,
                    height: flow.line_height(),
                },
                runs,
                &mut scene.ops,
            );
        }
        // The list lives in its own clipped card. Width and height are measured
        // anew each frame, while the viewport retains the wheel/follow policy.
        let list_height = (height as f32 - LIST_TOP - 60.0).max(1.0);
        let list_width = (width as f32 - 72.0).max(1.0);
        self.viewport
            .reconcile(self.rows as f32 * ROW_HEIGHT, list_height, 0.0);
        let mut rows = Vec::new();
        for index in 0..self.rows {
            let top = index as f32 * ROW_HEIGHT;
            if !self.viewport.visible(top, top + ROW_HEIGHT) {
                continue;
            }
            let y = LIST_TOP + self.viewport.position(top);
            rows.push(Op::Rect {
                x: 36.0,
                y,
                width: list_width,
                height: ROW_HEIGHT,
                style: if index % 2 == 0 {
                    Style::rgb(45, 105, 150)
                } else {
                    Style::rgb(65, 74, 86)
                },
            });
            rows.push(Op::Text {
                x: 40.0,
                y: y + 3.0,
                size: 14.0,
                style: Style::rgb(230, 232, 236),
                text: format!("Local item {}", index + 1),
            });
        }
        scene.ops.push(Op::ClipRect {
            x: 36.0,
            y: LIST_TOP,
            width: list_width,
            height: list_height,
            ops: Arc::new(rows),
        });
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
    fn description_wraps_and_paints_inside_narrow_card() {
        let mut dashboard = Dashboard::default();
        let narrow = dashboard.frame(130, 300, &Metrics);
        let wide = dashboard.frame(500, 300, &Metrics);
        let rows = |scene: &Scene| {
            scene
                .ops
                .iter()
                .filter_map(|op| match op {
                    Op::ClipRect {
                        x: 36.0,
                        y,
                        width,
                        ops,
                        ..
                    } if *y >= 95.0 && *y < 150.0 => Some((*width, ops.clone())),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let narrow_rows = rows(&narrow);
        assert!(narrow_rows.len() > 1);
        assert_eq!(rows(&wide).len(), 1);
        assert!(narrow_rows.iter().all(|(width, _)| *width == 58.0));
        let reconstructed: String = narrow_rows
            .iter()
            .flat_map(|(_, ops)| ops.iter())
            .filter_map(|op| match op {
                Op::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(reconstructed.starts_with("This card is built"));
    }

    #[test]
    fn button_is_local_and_resizes_with_viewport() {
        let mut dashboard = Dashboard::default();
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
