//! Toolkit-owned scene: no semantic tree or protocol types involved.
use misa_pixel_ui::{
    Button, Checkbox, FieldInsets, FieldMode, FieldViewport, ListBox, ListBoxState, ListKey,
    PlacedField, PlacedListBox, ProgressBar, Rect, TextField, TextFlow, TextMetrics,
};
use misa_pixel_ui::{Op, Scene};
use misa_style::Style;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeAction {
    Toggle,
    Note,
    Check,
    List,
    Progress,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Focus {
    Note,
    Button,
    Check,
    List,
}

pub struct Dashboard {
    pub selected: bool,
    list_state: ListBoxState,
    rows: usize,
    last_height: u32,
    focus: Option<Focus>,
    note: String,
    note_cursor: usize,
    note_focused: bool,
    checked: bool,
    note_viewport: FieldViewport,
}

impl Default for Dashboard {
    fn default() -> Self {
        Self {
            selected: false,
            list_state: ListBoxState::default(),
            rows: 20,
            last_height: 480,
            focus: None,
            note: String::new(),
            note_cursor: 0,
            note_focused: false,
            checked: false,
            note_viewport: FieldViewport::default(),
        }
    }
}

const ROW_HEIGHT: f32 = 26.0;

impl Dashboard {
    pub fn scroll(&mut self, delta: f32) {
        if delta.is_finite() {
            self.list_state.scroll = (self.list_state.scroll + delta).max(0.0);
        }
        // Placement reconciles the offset with the current viewport.
    }

    pub fn append_row(&mut self) {
        self.rows += 1;
    }

    pub fn offset(&self) -> f32 {
        self.list_state.scroll
    }
    pub fn list_selection(&self) -> Option<usize> {
        self.list_state.selected
    }
    pub fn checked(&self) -> bool {
        self.checked
    }

    pub fn key(&mut self, key: ListKey, width: u32, height: u32, metrics: &dyn TextMetrics) {
        if self.focus == Some(Focus::List) {
            let list = self.list(width, height, metrics);
            list.key(&mut self.list_state, key);
        } else if key == ListKey::Enter && self.focus == Some(Focus::Button) {
            self.toggle();
        }
    }

    pub fn space(&mut self) {
        match self.focus {
            Some(Focus::Check) => self.checked = !self.checked,
            Some(Focus::Button) => self.toggle(),
            _ => {}
        }
    }

    pub fn tab(&mut self, backward: bool) {
        let next = match (self.focus, backward) {
            (None, false) | (Some(Focus::List), false) => Focus::Note,
            (Some(Focus::Note), false) => Focus::Check,
            (Some(Focus::Check), false) => Focus::Button,
            (Some(Focus::Button), false) => Focus::List,
            (None, true) | (Some(Focus::Note), true) => Focus::List,
            (Some(Focus::List), true) => Focus::Button,
            (Some(Focus::Button), true) => Focus::Check,
            (Some(Focus::Check), true) => Focus::Note,
        };
        self.focus = Some(next);
        self.note_focused = next == Focus::Note;
    }

    pub fn toggle(&mut self) {
        self.selected = !self.selected;
    }

    pub fn note(&self) -> &str {
        &self.note
    }
    pub fn note_focused(&self) -> bool {
        self.note_focused
    }
    pub fn insert_note(&mut self, text: &str) {
        if self.note_focused {
            self.note.insert_str(self.note_cursor, text);
            self.note_cursor += text.len();
        }
    }
    pub fn backspace_note(&mut self) {
        if self.note_focused && self.note_cursor > 0 {
            let start = self.note[..self.note_cursor]
                .char_indices()
                .next_back()
                .unwrap()
                .0;
            self.note.replace_range(start..self.note_cursor, "");
            self.note_cursor = start;
        }
    }
    fn note_field(&mut self, width: u32, metrics: &dyn TextMetrics) -> PlacedField<NativeAction> {
        TextField {
            id: NativeAction::Note,
            bounds: Rect {
                x: 36.0,
                y: 44.0,
                width: (width as f32 - 72.0).max(1.0),
                height: 76.0,
            },
            label: self.note.clone(),
            mode: FieldMode::WordWrap,
            font_size: 15.0,
            focused: self.note_focused,
            cursor: Some(self.note_cursor),
            insets: FieldInsets {
                left: 7.0,
                top: 6.0,
                right: 7.0,
                bottom: 3.0,
            },
            border_width: 1.0,
            caret_width: 1.5,
            background: Style::rgb(35, 40, 48),
            border: Style::rgb(65, 74, 86),
            focus_border: Style::rgb(45, 105, 150),
            foreground: Style::rgb(230, 232, 236),
        }
        .place(metrics, &mut self.note_viewport)
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

    fn checkbox(
        &self,
        width: u32,
        metrics: &dyn TextMetrics,
    ) -> misa_pixel_ui::PlacedCheckbox<NativeAction> {
        Checkbox {
            id: NativeAction::Check,
            bounds: Rect {
                x: 36.0,
                y: 125.0,
                width: (width as f32 - 72.0).max(1.0),
                height: 22.0,
            },
            label: "Enable local option".into(),
            checked: self.checked,
            focused: self.focus == Some(Focus::Check),
            font_size: 14.0,
            background: Style::rgb(35, 40, 48),
            foreground: Style::rgb(230, 232, 236),
            accent: Style::rgb(45, 105, 150),
        }
        .place(metrics)
    }

    fn list_bounds(&self, width: u32, height: u32, metrics: &dyn TextMetrics) -> Rect {
        let flow = TextFlow::new(metrics, 15.0);
        let card_width = (width as f32 - 40.0).max(1.0);
        let rows = flow.wrap(
            vec![(
                Style::rgb(230, 232, 236),
                "This card is built from Scene / Op, not misa-proto.".into(),
            )],
            (card_width - 32.0).max(0.0),
        );
        let top = 205.0 + rows.len() as f32 * flow.line_height() + 16.0;
        Rect {
            x: 36.0,
            y: top,
            width: (width as f32 - 72.0).max(1.0),
            height: (height as f32 - top - 60.0).max(0.0),
        }
    }

    fn list(
        &mut self,
        width: u32,
        height: u32,
        metrics: &dyn TextMetrics,
    ) -> PlacedListBox<NativeAction> {
        ListBox {
            id: NativeAction::List,
            bounds: self.list_bounds(width, height, metrics),
            count: self.rows,
            label: |i| format!("Local item {}", i + 1),
            font_size: 14.0,
            row_height: ROW_HEIGHT,
            focused: self.focus == Some(Focus::List),
            background: Style::rgb(35, 40, 48),
            foreground: Style::rgb(230, 232, 236),
            highlight: Style::rgb(45, 105, 150),
        }
        .place(metrics, &mut self.list_state)
    }

    pub fn click(&mut self, x: f32, y: f32, width: u32, metrics: &dyn TextMetrics) -> bool {
        self.note_focused = self.note_field(width, metrics).bounds.contains(x, y);
        self.focus = self.note_focused.then_some(Focus::Note);
        if self.checkbox(width, metrics).click(x, y).is_some() {
            self.checked = !self.checked;
            self.focus = Some(Focus::Check);
        } else if self.button(width, metrics).bounds.contains(x, y) {
            self.toggle();
            self.focus = Some(Focus::Button);
        } else {
            let list = self.list(width, self.last_height, metrics);
            if list.click(&mut self.list_state, x, y).is_some() {
                self.focus = Some(Focus::List);
            }
        }
        self.focus.is_some()
    }

    pub fn frame(&mut self, width: u32, height: u32, metrics: &dyn TextMetrics) -> Scene {
        self.last_height = height;
        let button = self.button(width, metrics);
        let card_width = (width as f32 - 40.0).max(1.0);
        let flow = TextFlow::new(metrics, 15.0);
        let description = "This card is built from Scene / Op, not misa-proto.";
        let available = (card_width - 32.0).max(0.0);
        let description_rows = flow.wrap(
            vec![(Style::rgb(230, 232, 236), description.into())],
            available,
        );
        let description_top = 205.0;
        let list_top = description_top + description_rows.len() as f32 * flow.line_height() + 16.0;
        let mut scene = Scene {
            width: width as f32,
            height: height as f32,
            ops: vec![Op::Rect {
                x: 20.0,
                y: 200.0,
                width: card_width,
                height: (list_top - 200.0).max(1.0),
                style: Style::rgb(35, 40, 48),
            }],
        };
        scene.ops.extend(button.ops);
        scene.ops.extend(self.note_field(width, metrics).ops);
        scene.ops.extend(self.checkbox(width, metrics).widget.ops);
        scene.ops.extend(
            ProgressBar {
                id: NativeAction::Progress,
                bounds: Rect {
                    x: 36.0,
                    y: 195.0,
                    width: (width as f32 - 72.0).max(1.0),
                    height: 5.0,
                },
                fraction: self
                    .list_state
                    .selected
                    .map_or(0.0, |i| (i + 1) as f32 / self.rows as f32),
                background: Style::rgb(65, 74, 86),
                foreground: Style::rgb(45, 105, 150),
            }
            .place()
            .ops,
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
            24.0,
            height as f32 - 48.0,
            14.0,
            "Enter / Space or click the button to toggle",
        );
        for (line, runs) in description_rows.into_iter().enumerate() {
            // Reserve the footer even when the window is too short to show the
            // complete description; its remaining rows never paint over it.
            if description_top + (line + 1) as f32 * flow.line_height() > height as f32 - 60.0 {
                break;
            }
            flow.place(
                Rect {
                    x: 36.0,
                    y: description_top + line as f32 * flow.line_height(),
                    width: available,
                    height: flow.line_height(),
                },
                runs,
                &mut scene.ops,
            );
        }
        // The list owns the shared paint/hit clip and its scroll viewport.
        scene
            .ops
            .extend(self.list(width, height, metrics).widget.ops);
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
    fn local_note_editing_respects_focus_and_unicode_boundaries() {
        let mut dashboard = Dashboard::default();
        dashboard.insert_note("ignored");
        assert_eq!(dashboard.note(), "");
        assert!(dashboard.click(40.0, 50.0, 130, &Metrics));
        dashboard.insert_note("aé");
        dashboard.backspace_note();
        assert_eq!(dashboard.note(), "a");
        dashboard.click(0.0, 0.0, 130, &Metrics);
        dashboard.backspace_note();
        assert_eq!(dashboard.note(), "a");
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
                .take(scene.ops.len() - 1) // the final clip is the list
                .filter_map(|op| match op {
                    Op::ClipRect {
                        x: 36.0,
                        y,
                        width,
                        ops,
                        ..
                    } if *y >= 205.0 && *y < 205.0 + 15.0 * 12.0 => Some((*width, ops.clone())),
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
        assert!(reconstructed.starts_with("This card"));
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
